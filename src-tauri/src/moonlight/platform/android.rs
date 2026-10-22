//! Android Activity/surface owner. All renderer changes go through the runtime actor.
//!
//! Command integration: create_stream_window (on a worker) returns (window, token),
//! then start_stream(token, request).await and present_generation(token).
//! attach_initial_surface is available for callers with their own startup sequencing.
//! Do not cancel an in-flight actor attach/detach: the owner pins its reference even
//! on errors, since a failed/dropped ACK does not prove the renderer stopped using it.

use std::{
    collections::{BTreeSet, VecDeque},
    ffi::c_void,
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};

use jni::{
    objects::{JClass, JObject, JString, JValue},
    sys::{jboolean, jfloat, jint, jlong, JNI_FALSE, JNI_TRUE},
    JNIEnv, JavaVM,
};
use tauri::{AppHandle, Manager};
use tokio::sync::{Mutex as AsyncMutex, Notify};

use super::window::NativeSurfaceDescriptor;
use crate::{
    input::{
        event::{ButtonState, MouseButton},
        manager::InputManager,
        state::MouseMode,
    },
    moonlight::{
        composition::MoonlightManager,
        domain::{MoonlightError, SessionState},
        native,
        runtime::{MoonlightRuntimeHandle, NativeStartRequest},
    },
    services::app_context::AppContext,
};

const SURFACE_WAIT: Duration = Duration::from_secs(10);
const RECOVERY_WAIT: Duration = Duration::from_secs(5);
const CONTROLLER_RETRY: Duration = Duration::from_millis(250);
// moonlight-common-c/src/Limelight.h; Kotlin supports standard buttons + Guide.
const CONTROLLER_TYPE_XBOX: u8 = 0x01;
const CONTROLLER_BUTTONS: u32 = 0xF7FF;
const CONTROLLER_CAPABILITIES: u16 = 0x01; // Analog triggers only; no haptic/sensor callbacks.
const BRIDGE_CLASS: &str = "com.noland.connect.stream.NativeStreamBridge";
static COORDINATOR: OnceLock<Arc<Coordinator>> = OnceLock::new();

#[link(name = "android")]
unsafe extern "C" {
    fn ANativeWindow_fromSurface(
        env: *mut jni::sys::JNIEnv,
        surface: jni::sys::jobject,
    ) -> *mut c_void;
    fn ANativeWindow_acquire(window: *mut c_void);
    fn ANativeWindow_release(window: *mut c_void);
}

struct NativeWindow(usize);
impl Drop for NativeWindow {
    fn drop(&mut self) {
        unsafe { ANativeWindow_release(self.0 as *mut c_void) };
    }
}
impl NativeWindow {
    fn acquire(&self) -> Arc<Self> {
        unsafe { ANativeWindow_acquire(self.0 as *mut c_void) };
        Arc::new(Self(self.0))
    }
}

#[derive(Clone)]
struct Surface {
    activity: i64,
    serial: i64,
    window: Arc<NativeWindow>,
    width: u32,
    height: u32,
}
impl Surface {
    fn descriptor(&self) -> NativeSurfaceDescriptor {
        NativeSurfaceDescriptor {
            surface_type: native::nl_surface_type_NL_SURFACE_ANDROID_NATIVE_WINDOW,
            window_handle: self.window.0,
            display_handle: 0,
            width: self.width,
            height: self.height,
            // SurfaceHolder dimensions are physical pixels, not Android dp.
            scale_factor: 1.0,
        }
    }
    fn same(&self, other: &Self) -> bool {
        self.activity == other.activity
            && self.serial == other.serial
            && self.width == other.width
            && self.height == other.height
    }
}

#[derive(Default)]
struct State {
    next_token: i64,
    next_activity: i64,
    token: i64,
    activity: i64,
    minimum_surface_serial: i64,
    surface: Option<Surface>,
    // Separate acquired reference, including while an attach ACK is outstanding.
    attached: Option<Surface>,
    ready: bool,
    input_registered: bool,
    focused: bool,
    closing: bool,
    disconnecting: bool,
    failure: Option<String>,
    loss_revision: u64,
    width: u32,
    height: u32,
    dashboard: bool,
    presenting: bool,
    input_suspended: bool,
    input_revision: u64,
    shift_keys: u8,
    controller_mask: u16,
    neutral_mask: u16,
    controllers: VecDeque<ControllerInput>,
    desired_controllers: [Option<ControllerState>; 16],
    controller_announced: u16,
    controller_dirty: u16,
    controller_retry_blocked: u16,
    controller_session: Option<u64>,
    keyboard: VecDeque<KeyboardInput>,
    pressed_keys: BTreeSet<u16>,
    key_releases: BTreeSet<(i64, u16)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ControllerState {
    number: i16,
    mask: u16,
    buttons: i32,
    left_trigger: u8,
    right_trigger: u8,
    left_x: i16,
    left_y: i16,
    right_x: i16,
    right_y: i16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ControllerRoute {
    token: i64,
    activity: i64,
    revision: u64,
    session_generation: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ControllerInput {
    route: ControllerRoute,
    state: ControllerState,
}

#[derive(Debug, PartialEq, Eq)]
struct KeyboardInput {
    token: i64,
    activity: i64,
    revision: u64,
    event: KeyboardEvent,
}

#[derive(Debug, PartialEq, Eq)]
enum KeyboardEvent {
    Key {
        key: u16,
        pressed: bool,
        modifiers: u8,
    },
    Text(String),
}

impl State {
    fn accepts_input(&self, token: i64, activity: i64, pointer_or_gamepad: bool) -> bool {
        Coordinator::valid(self, token, activity)
            && self.input_registered
            && self.ready
            && self.focused
            && !self.dashboard
            && (!pointer_or_gamepad || !self.input_suspended)
    }

    fn activity_can_stop(&self, token: i64, activity: i64) -> bool {
        Coordinator::valid(self, token, activity) && !self.dashboard
    }

    fn set_input_suspended(&mut self, suspended: bool) -> bool {
        let release = suspended && !self.input_suspended;
        self.input_suspended = suspended;
        if release {
            self.release_inputs();
        }
        release
    }

    fn controller_route(&self) -> Option<ControllerRoute> {
        (self.token != 0 && self.activity != 0 && self.ready && !self.closing).then_some(
            ControllerRoute {
                token: self.token,
                activity: self.activity,
                revision: self.input_revision,
                session_generation: self.controller_session,
            },
        )
    }

    fn reset_controllers(&mut self) {
        self.controller_mask = 0;
        self.neutral_mask = 0;
        self.controllers.clear();
        self.desired_controllers = [None; 16];
        self.controller_announced = 0;
        self.controller_dirty = 0;
        self.controller_retry_blocked = 0;
        self.controller_session = None;
    }

    fn ensure_player_one(&mut self) {
        if self.token != 0 && !self.closing {
            self.controller_mask |= 1;
            if self.desired_controllers[0].is_none() {
                self.desired_controllers[0] =
                    Some(ControllerState::neutral(0, self.controller_mask));
                self.controller_dirty |= 1;
            }
        }
    }

    fn neutralize_controllers(&mut self) {
        self.ensure_player_one();
        self.controllers.clear();
        self.neutral_mask |= self.controller_mask;
        self.controller_retry_blocked = 0;
        for number in 0..16 {
            let bit = 1u16 << number;
            if self.controller_mask & bit != 0 || self.desired_controllers[number].is_some() {
                self.desired_controllers[number] = Some(ControllerState::neutral(
                    number as i16,
                    self.controller_mask,
                ));
                self.controller_dirty |= bit;
            }
        }
    }

    fn observe_controller_session(&mut self, generation: u64) {
        if self
            .controller_session
            .is_some_and(|previous| previous != generation)
        {
            self.controller_announced = 0;
            // Never carry held buttons or pre-reconnect FIFO edges to a new transport.
            self.neutralize_controllers();
        }
        self.controller_session = Some(generation);
        self.ensure_player_one();
    }

    fn controller_retry_tick(&mut self) {
        self.controller_retry_blocked = 0;
    }

    fn queue_controller(&mut self, mut controller: ControllerState) -> bool {
        controller.mask |= 1; // Physical + virtual player one never hot-unplugs.
        if !self.accepts_input(self.token, self.activity, true) {
            controller = ControllerState::neutral(controller.number, controller.mask);
        }
        let removed = self.controller_mask & !controller.mask;
        self.neutral_mask |= removed;
        self.controller_announced &= controller.mask;
        self.controller_mask = controller.mask;
        self.controllers
            .retain(|pending| controller.mask & (1u16 << pending.state.number) != 0);
        for number in 0..16 {
            let bit = 1u16 << number;
            if removed & bit != 0
                || (controller.mask & bit != 0 && self.desired_controllers[number].is_none())
            {
                self.desired_controllers[number] =
                    Some(ControllerState::neutral(number as i16, controller.mask));
                self.controller_dirty |= bit;
            } else if let Some(state) = self.desired_controllers[number].as_mut() {
                state.mask = controller.mask;
            }
        }
        let overflow = self.controllers.len() >= 256;
        if overflow {
            self.neutralize_controllers();
            controller = ControllerState::neutral(controller.number, controller.mask);
        }
        self.desired_controllers[controller.number as usize] = Some(controller);
        self.controller_dirty |= 1u16 << controller.number;
        if let Some(route) = self.controller_route() {
            self.controllers.push_back(ControllerInput {
                route,
                state: controller,
            });
        }
        overflow
    }

    fn take_controller_neutral(&mut self) -> Option<ControllerInput> {
        let route = self.controller_route()?;
        let available = self.neutral_mask & !self.controller_retry_blocked;
        if available == 0 {
            return None;
        }
        let number = available.trailing_zeros() as i16;
        self.neutral_mask &= !(1u16 << number);
        Some(ControllerInput {
            route,
            state: ControllerState::neutral(number, self.controller_mask),
        })
    }

    fn take_controller_input(&mut self) -> Option<ControllerInput> {
        let route = self.controller_route()?;
        while let Some(mut input) = self.controllers.pop_front() {
            if input.route != route
                || self.controller_retry_blocked & (1u16 << input.state.number) != 0
            {
                continue;
            }
            input.state.mask = self.controller_mask;
            return Some(input);
        }
        let pending = (self.controller_dirty | (self.controller_mask & !self.controller_announced))
            & !self.controller_retry_blocked;
        if pending == 0 {
            return None;
        }
        let number = pending.trailing_zeros() as usize;
        let state = self.desired_controllers[number]
            .unwrap_or_else(|| ControllerState::neutral(number as i16, self.controller_mask));
        Some(ControllerInput { route, state })
    }

    fn controller_failed(&mut self, input: &ControllerInput) {
        if self.controller_route() == Some(input.route) {
            let bit = 1u16 << input.state.number;
            self.controller_dirty |= bit;
            self.controller_retry_blocked |= bit;
        }
    }

    fn keyboard_is_current(&self, input: &KeyboardInput) -> bool {
        self.accepts_input(input.token, input.activity, false)
            && self.input_revision == input.revision
    }

    fn queue_keyboard(&mut self, event: KeyboardEvent) -> bool {
        if self.keyboard.len() >= 256 {
            // A dropped up edge must not leave its already-sent down edge held.
            // Cancel the batch rather than executing Enter after dropping text.
            self.invalidate_keyboard();
            return false;
        }
        self.keyboard.push_back(KeyboardInput {
            token: self.token,
            activity: self.activity,
            revision: self.input_revision,
            event,
        });
        true
    }

    // Called only in actor-producer order, never by the JNI enqueue path.
    fn track_key(&mut self, key: u16, pressed: bool) {
        if pressed {
            self.pressed_keys.insert(key);
        } else {
            self.pressed_keys.remove(&key);
        }
        self.shift_keys = u8::from(self.pressed_keys.contains(&0x10))
            | (u8::from(self.pressed_keys.contains(&0xA0)) << 1)
            | (u8::from(self.pressed_keys.contains(&0xA1)) << 2);
    }

    fn invalidate_keyboard(&mut self) {
        self.input_revision = self.input_revision.wrapping_add(1);
        for key in std::mem::take(&mut self.pressed_keys) {
            self.key_releases.insert((self.token, key));
        }
        self.shift_keys = 0;
        self.keyboard.clear();
        // The shared input revision also invalidates controller retry snapshots.
        self.neutralize_controllers();
    }

    // Invalidate queued work, not player presence. The worker sends neutral states
    // with the retained active mask before consuming any subsequently queued input.
    fn release_inputs(&mut self) {
        self.invalidate_keyboard();
    }

    fn enter_dashboard(&mut self) {
        self.dashboard = true;
        self.input_suspended = true;
        self.focused = false;
        self.surface = None;
        self.loss_revision = self.loss_revision.wrapping_add(1);
        self.release_inputs();
    }

    fn claim_activity(&mut self) -> i64 {
        self.next_activity += 1;
        self.activity = self.next_activity;
        self.minimum_surface_serial = 1;
        self.surface = None;
        self.focused = false;
        self.input_registered = false;
        self.dashboard = false;
        self.input_suspended = false;
        self.loss_revision = self.loss_revision.wrapping_add(1);
        self.release_inputs();
        self.activity
    }

    fn surface_loss_expired(&self, token: i64, revision: u64) -> bool {
        self.token == token
            && self.loss_revision == revision
            && self.surface.is_none()
            && !self.closing
            && !self.dashboard
            && !self.presenting
    }

    // Called under the state lock after detach ACK. request_stop may have claimed
    // cleanup during that await; preserve its generation so it can reset disconnecting.
    fn finish_surface_close(&mut self, token: i64) {
        if self.token == token && !self.disconnecting {
            self.surface = None;
            self.token = 0;
            self.activity = 0;
            self.ready = false;
            self.closing = false;
            self.dashboard = false;
            self.presenting = false;
            self.reset_controllers();
            self.release_inputs();
        }
    }
}

struct Coordinator {
    app: AppHandle,
    runtime: MoonlightRuntimeHandle,
    input: Arc<InputManager>,
    state: Mutex<State>,
    changed: Condvar,
    renderer: AsyncMutex<()>,
    wake: Notify,
    input_wake: Notify,
    presentation: AsyncMutex<()>,
    statistics: Mutex<()>,
}

fn error(message: impl Into<String>) -> MoonlightError {
    MoonlightError::Native(message.into())
}
fn coordinator() -> Result<&'static Arc<Coordinator>, MoonlightError> {
    COORDINATOR
        .get()
        .ok_or_else(|| error("Android stream coordinator is not initialized"))
}

/// Idempotent; create_or_reuse_stream_window calls this automatically. Terminal
/// watching is armed per generation by start_stream, after the runtime start ACK.
pub fn init(app: &AppHandle) {
    COORDINATOR.get_or_init(|| {
        let manager = app.state::<MoonlightManager>();
        let bridge = Arc::new(Coordinator {
            app: app.clone(),
            runtime: manager.runtime.clone(),
            input: manager.input.clone(),
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            renderer: AsyncMutex::new(()),
            wake: Notify::new(),
            input_wake: Notify::new(),
            presentation: AsyncMutex::new(()),
            statistics: Mutex::new(()),
        });
        let worker = bridge.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                worker.wake.notified().await;
                if let Err(err) = worker.reconcile().await {
                    tracing::error!(%err, "Android surface recovery failed; disconnecting");
                    worker.request_stop_current();
                }
            }
        });

        let worker = bridge.clone();
        tauri::async_runtime::spawn(async move {
            let mut retry = tokio::time::interval_at(
                tokio::time::Instant::now() + CONTROLLER_RETRY,
                CONTROLLER_RETRY,
            );
            retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = worker.input_wake.notified() => {},
                    _ = retry.tick() => {
                        worker.state.lock().unwrap().controller_retry_tick();
                    },
                }
                worker.flush_actor_input().await;
            }
        });
        bridge
    });
}

/// Launches and waits at most ten seconds for a nonzero SurfaceHolder size.
/// Must run on a Tokio/blocking worker, NEVER the Android UI thread. JNI rejects
/// that misuse before entering the Condvar wait. Overlapping launches are rejected.
pub fn create(app: &AppHandle, width: u32, height: u32) -> Result<i64, MoonlightError> {
    init(app);
    let c = coordinator()?;
    if width == 0 || height == 0 || width > i32::MAX as u32 || height > i32::MAX as u32 {
        return Err(error("Invalid Android stream dimensions"));
    }
    let token = {
        let mut s = c.state.lock().unwrap();
        if s.token != 0 || s.disconnecting || s.closing {
            return Err(error("Android stream is already open or still cleaning up"));
        }
        s.next_token += 1;
        s.token = s.next_token;
        s.failure = None;
        s.ready = false;
        s.width = width;
        s.height = height;
        s.dashboard = false;
        s.presenting = false;
        s.input_suspended = false;
        s.reset_controllers();
        s.release_inputs();
        s.token
    };
    c.input.set_stream_dimensions(width, height);
    if let Err(err) = java_call("launch", token, width as i32, height as i32) {
        c.fail_launch(token, err.to_string());
        return Err(err);
    }
    let deadline = Instant::now() + SURFACE_WAIT;
    let mut s = c.state.lock().unwrap();
    loop {
        if s.token != token || s.closing || s.failure.is_some() {
            return Err(error(
                s.failure
                    .clone()
                    .unwrap_or_else(|| "Android stream launch cancelled".into()),
            ));
        }
        if s.surface.is_some() {
            return Ok(token);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            drop(s);
            c.fail_launch(
                token,
                "Timed out waiting for Android stream Activity/surface".into(),
            );
            return Err(error(
                "Timed out waiting for Android stream Activity/surface (10s)",
            ));
        }
        s = c.changed.wait_timeout(s, remaining).unwrap().0;
    }
}

/// Worker-only command entry point: captures the generation as part of creation,
/// not in a later lookup that could accidentally observe a replacement route.
pub fn create_stream_window(
    app: &AppHandle,
    width: u32,
    height: u32,
) -> Result<(tauri::Window, i64), MoonlightError> {
    let window = app
        .get_window(super::window::STREAM_WINDOW_LABEL)
        .ok_or_else(|| error("The Android application window is unavailable"))?;
    let token = create(app, width, height)?;
    app.state::<super::window::StreamWindowCloseState>().reset();
    Ok((window, token))
}

/// Preferred initial attach. Serializes with recreation and close; retains the
/// acquired ANativeWindow until the runtime's detach ACK. Call before runtime.start.
/// The generation argument prevents a delayed command from attaching a newer route.
pub async fn attach_initial_surface(token: i64) -> Result<(), MoonlightError> {
    let c = coordinator()?.clone();
    // A spawned operation survives cancellation of the calling command future.
    tauri::async_runtime::spawn(async move {
        let _renderer = c.renderer.lock().await;
        {
            let mut s = c.state.lock().unwrap();
            if s.token != token || s.closing || s.surface.is_none() {
                return Err(error("Android initial surface is no longer available"));
            }
            s.ready = true;
        }
        c.reconcile_locked().await?;
        c.check_attached(token)
    })
    .await
    .map_err(|err| error(format!("Android initial attach task failed: {err}")))?
}

/// Attach and start under the same lifecycle lock used by stop and surface recovery.
/// Prefer this over a direct runtime.start so Back during startup cannot resurrect
/// a stream after full application cleanup. The operation survives caller cancellation.
pub async fn start_stream(token: i64, request: NativeStartRequest) -> Result<(), MoonlightError> {
    let c = coordinator()?.clone();
    tauri::async_runtime::spawn(async move {
        let result = async {
            let _renderer = c.renderer.lock().await;
            {
                let mut s = c.state.lock().unwrap();
                if s.token != token || s.closing || s.surface.is_none() {
                    return Err(error("Android stream startup was cancelled"));
                }
                s.ready = true;
            }
            c.reconcile_locked().await?;
            c.check_attached(token)?;
            c.runtime.start(request).await?;
            c.watch_terminal_state(token);
            c.check_attached(token)
        }
        .await;
        if result.is_err() {
            c.request_stop(token);
        }
        result
    })
    .await
    .map_err(|err| error(format!("Android stream startup task failed: {err}")))?
}

/// For user actions targeting the current route. Startup continuations must retain
/// the token returned by create/create_stream_window instead of looking it up later.
pub fn current_generation() -> Result<i64, MoonlightError> {
    let c = coordinator()?;
    let s = c.state.lock().unwrap();
    if s.token == 0 || s.closing {
        return Err(error("No active Android stream route"));
    }
    Ok(s.token)
}

/// A naked borrowed descriptor cannot safely escape to command code: a surface-loss
/// callback could detach/release it before the command enqueues its initial attach.
/// Kept as an actionable error for legacy generic-window callers. Use the serialized
/// start_stream/attach_initial_surface APIs, which construct the type-6 descriptor.
pub fn surface_descriptor() -> Result<NativeSurfaceDescriptor, MoonlightError> {
    Err(error("Android requires platform::android::start_stream(generation, request) or attach_initial_surface(generation), not a raw window descriptor"))
}

pub fn present() -> Result<(), MoonlightError> {
    present_generation(current_generation()?)
}

/// Generation-aware presentation for command continuations after an async start.
pub fn present_generation(token: i64) -> Result<(), MoonlightError> {
    let c = coordinator()?;
    c.check_attached(token)?;
    java_call("present", token, 0, 0)
}

/// Reopen the private Activity for an existing runtime, without starting it again.
/// The detached task survives command cancellation; only the Condvar wait runs on
/// a blocking worker, and renderer rebinding uses the normal lifecycle lock.
pub async fn present_stream() -> Result<(), MoonlightError> {
    let c = coordinator()?.clone();
    let token = current_generation()?;
    tauri::async_runtime::spawn(async move {
        let _presentation = c.presentation.lock().await;
        if c.check_attached(token).is_ok() && !c.state.lock().unwrap().dashboard {
            return present_generation(token);
        }
        let (width, height) = {
            let mut s = c.state.lock().unwrap();
            if s.token != token || s.closing || !s.ready {
                return Err(error("Android stream is not available for presentation"));
            }
            s.presenting = true;
            s.failure = None;
            (s.width, s.height)
        };
        let worker = c.clone();
        let result = async {
            tauri::async_runtime::spawn_blocking(move || {
                java_call("launch", token, width as i32, height as i32)?;
                let deadline = Instant::now() + SURFACE_WAIT;
                let mut s = worker.state.lock().unwrap();
                loop {
                    if s.token != token || s.closing {
                        return Err(error("Android stream presentation cancelled"));
                    }
                    if let Some(failure) = &s.failure {
                        return Err(error(failure.clone()));
                    }
                    if !s.dashboard && s.surface.is_some() {
                        return Ok(());
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(error(
                            "Timed out reopening Android stream Activity/surface (10s)",
                        ));
                    }
                    s = worker.changed.wait_timeout(s, remaining).unwrap().0;
                }
            })
            .await
            .map_err(|err| error(format!("Android presentation worker failed: {err}")))??;
            let _renderer = c.renderer.lock().await;
            c.reconcile_locked().await?;
            c.check_attached(token)?;
            present_generation(token)
        }
        .await;
        let close_failed_activity = {
            let mut s = c.state.lock().unwrap();
            if s.token == token && !s.closing {
                s.presenting = false;
                if result.is_err() {
                    // A failed reentry must not tear down the still-running session.
                    s.enter_dashboard();
                    c.input.set_focus(false);
                    c.input_wake.notify_one();
                    c.wake.notify_one();
                }
                result.is_err()
            } else {
                false
            }
        };
        c.changed.notify_all();
        if close_failed_activity {
            // Ensure a retry can create a fresh Activity rather than reusing one
            // whose surface was rejected. dashboard already protects its teardown.
            let _ = tauri::async_runtime::spawn_blocking(move || {
                let _ = java_statistics(token, "");
                java_call("close", token, 0, 0)
            })
            .await;
        }
        result
    })
    .await
    .map_err(|err| error(format!("Android presentation task failed: {err}")))?
}

/// Snapshot both identities, so an updater also refreshes after Activity recreation.
pub(super) fn statistics_route() -> Option<(i64, i64)> {
    let c = COORDINATOR.get()?;
    let s = c.state.lock().unwrap();
    (s.token != 0
        && s.activity != 0
        && s.ready
        && !s.closing
        && !s.dashboard
        && s.surface.is_some())
    .then_some((s.token, s.activity))
}

pub(super) fn update_statistics(route: (i64, i64), text: &str) -> Result<(), MoonlightError> {
    let c = coordinator()?;
    let _statistics = c.statistics.lock().unwrap();
    if statistics_route() == Some(route) {
        java_bridge("updateStatistics", route.0, 0, 0, Some(text))?;
    }
    Ok(())
}

/// A retiring route may receive one empty update, never an Activity launch.
pub(super) fn clear_statistics(token: i64) -> Result<(), MoonlightError> {
    let c = coordinator()?;
    let _statistics = c.statistics.lock().unwrap();
    // The same token may already have returned with a new Activity/surface.
    if !statistics_route().is_some_and(|route| route.0 == token) {
        java_bridge("updateStatistics", token, 0, 0, Some(""))?;
    }
    Ok(())
}

/// Nonblocking close entry for the shared window API. The coordinator performs its
/// own actor detach even if the command already detached; only its ACK releases refs.
pub fn close() -> Result<(), MoonlightError> {
    let Some(c) = COORDINATOR.get() else {
        return Ok(());
    };
    let token = {
        let mut s = c.state.lock().unwrap();
        if s.token == 0 {
            return Ok(());
        }
        s.closing = true;
        s.input_registered = false;
        s.focused = false;
        s.release_inputs();
        s.token
    };
    c.input.end_capture();
    c.input_wake.notify_one();
    c.changed.notify_all();
    c.wake.notify_one();
    let _ = java_statistics(token, "");
    java_call("close", token, 0, 0)
}

/// Route startup errors/explicit stop through the application's full disconnect
/// path. Stale generations are ignored; create stays blocked during cleanup.
pub fn request_stop(token: i64) {
    if let Ok(c) = coordinator() {
        c.request_stop(token);
    }
}

pub fn install_input(input: &Arc<InputManager>) -> Result<(), MoonlightError> {
    let c = coordinator()?;
    let mut s = c.state.lock().unwrap();
    if s.token == 0
        || s.activity == 0
        || s.surface.is_none()
        || s.closing
        || !Arc::ptr_eq(input, &c.input)
    {
        return Err(error("Android Activity/JNI input bridge is not registered"));
    }
    s.input_registered = true;
    Ok(())
}
pub fn uninstall_input() {
    if let Ok(c) = coordinator() {
        let mut s = c.state.lock().unwrap();
        s.input_registered = false;
        s.release_inputs();
        c.input.end_capture();
        c.input_wake.notify_one();
    }
}
pub fn activate_input() -> Result<bool, MoonlightError> {
    let c = coordinator()?;
    let s = c.state.lock().unwrap();
    if !s.input_registered || s.closing {
        return Ok(false);
    }
    // Android touch is intentionally absolute, irrespective of desktop preference.
    c.input.begin_capture(MouseMode::Absolute);
    c.input.set_focus(s.focused);
    Ok(true)
}
pub fn release_input() {
    if let Ok(c) = coordinator() {
        // Tauri main loses focus when StreamActivity is shown. Release keys, but do
        // not disable Activity input or interpret this as stream termination.
        let mut s = c.state.lock().unwrap();
        s.release_inputs();
        c.input.release_all_inputs();
        c.input_wake.notify_one();
        if s.input_registered && !s.closing {
            c.input.set_focus(s.focused);
        }
    }
}

impl Coordinator {
    // One keyboard FIFO preserves commit/key ordering. Controllers have their
    // own bounded FIFO; releases have priority over both queues.
    async fn flush_actor_input(&self) {
        let _renderer = self.renderer.lock().await;
        for _ in 0..64 {
            self.release_keyboard_locked().await;
            enum Work {
                Controller(ControllerInput),
                Keyboard(KeyboardInput),
            }
            let work = {
                let mut s = self.state.lock().unwrap();
                if s.token == 0 || s.closing || !s.ready {
                    return;
                }
                s.observe_controller_session(self.runtime.latest_statistics().session_generation);
                let work = if let Some(input) = s.take_controller_neutral() {
                    Work::Controller(input)
                } else if let Some(input) = s.keyboard.pop_front() {
                    Work::Keyboard(input)
                } else if let Some(input) = s.take_controller_input() {
                    Work::Controller(input)
                } else {
                    return;
                };
                work
            };
            let result = match work {
                Work::Controller(input) => self.send_controller_input(input).await,
                Work::Keyboard(input) => self.send_keyboard_input(input).await,
            };
            if let Err(err) = result {
                tracing::warn!(%err, "Android actor input failed");
            }
        }
        // Bound the lifecycle-lock hold even under a continuous input producer.
        self.input_wake.notify_one();
    }

    async fn send_controller_input(
        &self,
        mut input: ControllerInput,
    ) -> Result<(), MoonlightError> {
        let bit = 1u16 << input.state.number;
        let announce = {
            let mut s = self.state.lock().unwrap();
            s.observe_controller_session(self.runtime.latest_statistics().session_generation);
            if s.controller_route() != Some(input.route) {
                return Ok(());
            }
            input.state.mask = s.controller_mask;
            input.state.mask & bit != 0 && s.controller_announced & bit == 0
        };
        if announce {
            let result = self
                .runtime
                .send_controller_arrival(
                    input.state.number as u8,
                    input.state.mask,
                    CONTROLLER_TYPE_XBOX,
                    CONTROLLER_BUTTONS,
                    CONTROLLER_CAPABILITIES,
                )
                .await;
            let mut s = self.state.lock().unwrap();
            s.observe_controller_session(self.runtime.latest_statistics().session_generation);
            if let Err(err) = result {
                s.controller_failed(&input);
                return Err(err);
            }
            // A suspension, Activity claim or reconnect may have happened during ACK.
            if s.controller_route() != Some(input.route) {
                return Ok(());
            }
            s.controller_announced |= bit & s.controller_mask;
        }
        {
            let mut s = self.state.lock().unwrap();
            s.observe_controller_session(self.runtime.latest_statistics().session_generation);
            if s.controller_route() != Some(input.route) {
                return Ok(());
            }
            input.state.mask = s.controller_mask;
            if input.state.mask & bit == 0
                || !s.accepts_input(input.route.token, input.route.activity, true)
            {
                input.state = ControllerState::neutral(input.state.number, input.state.mask);
            }
        }
        let p = input.state;
        let result = self
            .runtime
            .send_controller_state(
                p.number,
                p.mask as i16,
                p.buttons,
                p.left_trigger,
                p.right_trigger,
                p.left_x,
                p.left_y,
                p.right_x,
                p.right_y,
            )
            .await;
        let mut s = self.state.lock().unwrap();
        s.observe_controller_session(self.runtime.latest_statistics().session_generation);
        if result.is_err() {
            s.controller_failed(&input);
        } else if s.controller_route() == Some(input.route)
            && s.desired_controllers[p.number as usize] == Some(p)
        {
            s.controller_dirty &= !bit;
        }
        result
    }

    async fn release_keyboard_locked(&self) {
        let keys = {
            let mut s = self.state.lock().unwrap();
            std::mem::take(&mut s.key_releases)
        };
        for (token, key) in keys {
            if let Err(err) = self.send_key_release(token, key, 0).await {
                tracing::warn!(%err, "Android keyboard cleanup failed");
            }
        }
    }

    // Cleanup deliberately ignores Activity/revision/focus: those may have been
    // invalidated after a down was sent. The renderer lock pins the runtime token.
    async fn send_key_release(
        &self,
        token: i64,
        key: u16,
        modifiers: u8,
    ) -> Result<(), MoonlightError> {
        {
            let mut s = self.state.lock().unwrap();
            if token == 0 || s.token != token {
                return Ok(());
            }
            s.track_key(key, false);
            s.key_releases.remove(&(token, key));
        }
        self.runtime.send_keyboard(key, false, modifiers).await
    }

    async fn send_key_edge(
        &self,
        input: &KeyboardInput,
        key: u16,
        pressed: bool,
        modifiers: u8,
    ) -> Result<bool, MoonlightError> {
        {
            let mut s = self.state.lock().unwrap();
            if !s.keyboard_is_current(input) {
                return Ok(false);
            }
            // Register before the actor await, so concurrent lifecycle invalidation
            // can pair an in-flight down with an unconditional release.
            s.track_key(key, pressed);
        }
        let result = self.runtime.send_keyboard(key, pressed, modifiers).await;
        if result.is_err() {
            let _ = self.send_key_release(input.token, key, 0).await;
        }
        result.map(|()| true)
    }

    async fn send_keyboard_input(&self, input: KeyboardInput) -> Result<(), MoonlightError> {
        if !self.state.lock().unwrap().keyboard_is_current(&input) {
            return Ok(());
        }
        match &input.event {
            KeyboardEvent::Key {
                key,
                pressed,
                modifiers,
            } => {
                self.send_key_edge(&input, *key, *pressed, *modifiers)
                    .await?;
            }
            KeyboardEvent::Text(value) if !value.is_ascii() => {
                self.runtime.send_utf8_text(value.clone()).await?;
            }
            KeyboardEvent::Text(value) => {
                for ch in value.bytes() {
                    let hardware_shift = {
                        let s = self.state.lock().unwrap();
                        if !s.keyboard_is_current(&input) {
                            break;
                        }
                        s.shift_keys != 0
                    };
                    let Some((key, shift)) = ascii_key(ch) else {
                        continue;
                    };
                    let modifiers = u8::from(shift);
                    let press_shift = shift && !hardware_shift;
                    if press_shift && !self.send_key_edge(&input, 0xA0, true, 1).await? {
                        break;
                    }
                    let result = self.send_key_edge(&input, key, true, modifiers).await;
                    // Complete the pair even when cancellation happened between
                    // the down and up, or between synthetic Shift and the key.
                    let release = if matches!(&result, Ok(false)) {
                        Ok(())
                    } else {
                        self.send_key_release(input.token, key, modifiers).await
                    };
                    let shift_release = if press_shift {
                        self.send_key_release(input.token, 0xA0, 0).await
                    } else {
                        Ok(())
                    };
                    let sent = result?;
                    release?;
                    shift_release?;
                    if !sent {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn watch_terminal_state(self: &Arc<Self>, token: i64) {
        // Only called after a successful start ACK: the actor publishes Connecting
        // before native start/ACK. Idle here is terminal, not its pre-start value.
        let mut states = self.runtime.subscribe_state();
        let c = self.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                {
                    let s = c.state.lock().unwrap();
                    if s.token != token || s.closing {
                        return;
                    }
                }
                // Check the latest value before waiting: watch may have coalesced
                // every active update into Idle before this task was scheduled.
                let current = states.borrow_and_update().clone();
                if current == SessionState::Idle {
                    c.request_stop(token);
                    return;
                }
                if states.changed().await.is_err() {
                    c.request_stop(token);
                    return;
                }
            }
        });
    }

    fn check_attached(&self, token: i64) -> Result<(), MoonlightError> {
        let s = self.state.lock().unwrap();
        if s.token == token
            && !s.closing
            && s.surface
                .as_ref()
                .zip(s.attached.as_ref())
                .is_some_and(|(surface, attached)| surface.same(attached))
        {
            Ok(())
        } else {
            Err(error(
                "Android stream surface changed or startup was cancelled",
            ))
        }
    }
    fn valid(s: &State, token: i64, activity: i64) -> bool {
        s.token == token && token != 0 && s.activity == activity && activity != 0 && !s.closing
    }
    fn fail_launch(self: &Arc<Self>, token: i64, message: String) {
        {
            let mut s = self.state.lock().unwrap();
            if s.token != token {
                return;
            }
            s.failure = Some(message);
            s.closing = true;
        }
        self.changed.notify_all();
        self.request_stop(token);
    }
    fn request_stop_current(self: &Arc<Self>) {
        let token = self.state.lock().unwrap().token;
        self.request_stop(token);
    }
    fn request_stop(self: &Arc<Self>, token: i64) {
        self.request_stop_checked(token, None, None);
    }

    fn request_stop_checked(
        self: &Arc<Self>,
        token: i64,
        activity: Option<i64>,
        loss_revision: Option<u64>,
    ) {
        {
            let mut s = self.state.lock().unwrap();
            if token == 0
                || s.token != token
                || s.disconnecting
                || activity.is_some_and(|activity| !s.activity_can_stop(token, activity))
                || loss_revision.is_some_and(|revision| !s.surface_loss_expired(token, revision))
            {
                return;
            }
            s.disconnecting = true;
            s.closing = true;
            s.input_registered = false;
            s.focused = false;
            s.release_inputs();
        }
        self.input.end_capture();
        self.changed.notify_all();
        let c = self.clone();
        tauri::async_runtime::spawn(async move {
            // Serialize with startup/recovery. close() is nonblocking and never
            // acquires renderer, so application cleanup can call it under this lock.
            let _renderer = c.renderer.lock().await;
            c.release_keyboard_locked().await;
            if let Err(err) = crate::commands::moonlight_disconnect_stream(
                c.app.clone(),
                c.app.state::<AppContext>(),
                c.app.state::<MoonlightManager>(),
            )
            .await
            {
                tracing::error!(?err, "Android full stream disconnect failed");
            }
            // Retry detach even if application cleanup returned early. On failure
            // keep refs and block new launches, rather than risk a use-after-free.
            if let Err(err) = c.runtime.detach_surface().await {
                tracing::error!(%err, "Android detach failed; retaining native window references");
                return;
            }
            let _ = java_statistics(token, "");
            let _ = java_call("close", token, 0, 0).map_err(|err| {
                tracing::error!(%err, "Could not close Android stream Activity");
            });
            let mut s = c.state.lock().unwrap();
            if s.token == token {
                s.attached = None;
                s.surface = None;
                s.token = 0;
                s.activity = 0;
                s.ready = false;
                s.closing = false;
                s.disconnecting = false;
                s.dashboard = false;
                s.presenting = false;
                s.reset_controllers();
                s.release_inputs();
            }
            c.changed.notify_all();
        });
    }
    async fn reconcile(&self) -> Result<(), MoonlightError> {
        let _renderer = self.renderer.lock().await;
        self.reconcile_locked().await
    }
    async fn reconcile_locked(&self) -> Result<(), MoonlightError> {
        loop {
            self.release_keyboard_locked().await;
            let (token, desired, attached, closing) = {
                let s = self.state.lock().unwrap();
                if s.token == 0 || (!s.ready && !s.closing) {
                    return Ok(());
                }
                (
                    s.token,
                    if s.closing { None } else { s.surface.clone() },
                    s.attached.clone(),
                    s.closing,
                )
            };
            if let (Some(a), Some(b)) = (&attached, &desired) {
                if a.same(b) {
                    return Ok(());
                }
            }
            if attached.is_some() {
                self.runtime.detach_surface().await?;
                self.state.lock().unwrap().attached = None;
                // `attached` remains alive until this iteration ends, after ACK.
            }
            if closing {
                // close() can invalidate keys after the loop's initial drain but
                // before this snapshot/detach ACK. Drain again before retiring token.
                self.release_keyboard_locked().await;
                let mut s = self.state.lock().unwrap();
                s.finish_surface_close(token);
                self.changed.notify_all();
                return Ok(());
            }
            // Re-read after detach: UI callbacks may have replaced/destroyed the
            // candidate while the actor was acknowledging the old surface.
            let desired = {
                let mut s = self.state.lock().unwrap();
                if s.token != token || s.closing {
                    continue;
                }
                let Some(mut desired) = s.surface.clone() else {
                    return Ok(());
                };
                desired.window = desired.window.acquire();
                s.attached = Some(desired.clone());
                desired
            };
            self.runtime.attach_surface(desired.descriptor()).await?;
            // Loop until attached matches the latest surface/size/closing state.
        }
    }
    fn schedule_surface_loss(self: &Arc<Self>, token: i64, revision: u64) {
        let c = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(RECOVERY_WAIT).await;
            let lost = {
                let s = c.state.lock().unwrap();
                s.surface_loss_expired(token, revision)
            };
            if lost {
                tracing::warn!("Android surface did not return within recovery deadline");
                c.request_stop_checked(token, None, Some(revision));
            }
        });
    }
}

/// Same class-loader pattern as services/android_vpn.rs; attached native threads
/// cannot resolve application classes with FindClass. Never load the library again.
fn java_call(method: &str, token: i64, width: i32, height: i32) -> Result<(), MoonlightError> {
    java_bridge(method, token, width, height, None)
}

fn java_statistics(token: i64, text: &str) -> Result<(), MoonlightError> {
    let c = coordinator()?;
    let _statistics = c.statistics.lock().unwrap();
    java_bridge("updateStatistics", token, 0, 0, Some(text))
}

fn java_bridge(
    method: &str,
    token: i64,
    width: i32,
    height: i32,
    text: Option<&str>,
) -> Result<(), MoonlightError> {
    let context = std::panic::catch_unwind(ndk_context::android_context)
        .map_err(|_| error("Android application context is not initialized"))?;
    if context.vm().is_null() || context.context().is_null() {
        return Err(error("Android JVM/context unavailable"));
    }
    let vm =
        unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(|err| error(err.to_string()))?;
    let mut env = vm
        .attach_current_thread()
        .map_err(|err| error(err.to_string()))?;
    let result = env.with_local_frame(16, |env| -> jni::errors::Result<bool> {
        // Copy ndk_context's borrowed global without wrapping/adopting it. This
        // also permits concurrent JNI calls on independently attached workers.
        let raw_env = env.get_raw();
        let new_local_ref = unsafe { (**raw_env).NewLocalRef }
            .ok_or(jni::errors::Error::NullPtr("JNI NewLocalRef"))?;
        let local = unsafe { new_local_ref(raw_env, context.context().cast()) };
        if local.is_null() {
            return Err(jni::errors::Error::NullPtr("Android application context"));
        }
        let context = unsafe { JObject::from_raw(local) };
        let loader = env
            .call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
            .l()?;
        let name = JObject::from(env.new_string(BRIDGE_CLASS)?);
        let class = JClass::from(
            env.call_method(
                loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&name)],
            )?
            .l()?,
        );
        if let Some(text) = text {
            let text = JObject::from(env.new_string(text)?);
            env.call_static_method(
                class,
                method,
                "(JLjava/lang/String;)V",
                &[JValue::Long(token), JValue::Object(&text)],
            )?;
            return Ok(true);
        }
        env.call_static_method(
            class,
            method,
            "(Landroid/content/Context;JII)Z",
            &[
                JValue::Object(&context),
                JValue::Long(token),
                JValue::Int(width),
                JValue::Int(height),
            ],
        )?
        .z()
    });
    match result {
        Ok(true) => Ok(()),
        Ok(false) => Err(error(format!(
            "Android stream bridge `{method}` rejected the request"
        ))),
        Err(err) => {
            // Print the Java stack before clearing it so native worker failures are
            // actionable in logcat rather than silently swallowed by JNI.
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            Err(error(format!(
                "Android stream JNI `{method}` failed: {err}"
            )))
        }
    }
}

// All callbacks carry BOTH route generation and Activity incarnation. Surface
// callbacks additionally carry a monotonic SurfaceHolder revision.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeClaimActivity(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
) -> jlong {
    let Ok(c) = coordinator() else {
        return 0;
    };
    let mut s = c.state.lock().unwrap();
    if s.token != token || token == 0 || s.closing {
        return 0;
    }
    let activity = s.claim_activity();
    let revision = s.loss_revision;
    c.input.set_focus(false);
    drop(s);
    c.wake.notify_one();
    c.input_wake.notify_one();
    c.schedule_surface_loss(token, revision);
    activity
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeSurface(
    env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    serial: jlong,
    surface: JObject,
    width: jint,
    height: jint,
) -> jboolean {
    let Ok(c) = coordinator() else {
        return JNI_FALSE;
    };
    if width <= 0 || height <= 0 || serial <= 0 || surface.is_null() {
        return JNI_FALSE;
    }
    let mut s = c.state.lock().unwrap();
    if !Coordinator::valid(&s, token, activity) || s.dashboard {
        return JNI_FALSE;
    }
    if serial < s.minimum_surface_serial {
        return JNI_FALSE;
    }
    let raw = unsafe { ANativeWindow_fromSurface(env.get_raw(), surface.as_raw()) };
    if raw.is_null() {
        return JNI_FALSE;
    }
    s.minimum_surface_serial = serial;
    s.surface = Some(Surface {
        activity,
        serial,
        window: Arc::new(NativeWindow(raw as usize)),
        width: width as u32,
        height: height as u32,
    });
    s.input_registered = true;
    s.loss_revision += 1;
    drop(s);
    c.changed.notify_all();
    c.wake.notify_one();
    JNI_TRUE
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeSurfaceLost(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    serial: jlong,
) {
    let Ok(c) = coordinator() else {
        return;
    };
    let mut s = c.state.lock().unwrap();
    if !Coordinator::valid(&s, token, activity) || serial < s.minimum_surface_serial {
        return;
    }
    s.surface = None;
    s.minimum_surface_serial = serial.saturating_add(1);
    s.focused = false;
    s.release_inputs();
    s.loss_revision += 1;
    let revision = s.loss_revision;
    c.input.set_focus(false);
    drop(s);
    c.changed.notify_all();
    c.wake.notify_one();
    c.input_wake.notify_one();
    c.schedule_surface_loss(token, revision);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeFocus(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    focused: jboolean,
) {
    let Ok(c) = coordinator() else {
        return;
    };
    let mut s = c.state.lock().unwrap();
    if !Coordinator::valid(&s, token, activity) {
        return;
    }
    s.focused = focused != JNI_FALSE && s.surface.is_some() && !s.dashboard;
    if !s.focused {
        s.release_inputs();
        c.input_wake.notify_one();
    }
    if s.input_registered && s.ready {
        c.input.begin_capture(MouseMode::Absolute);
    }
    c.input.set_focus(s.focused);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeFailure(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
) {
    if let Ok(c) = coordinator() {
        tracing::error!("Android Activity launch failed (see logcat for Java stack)");
        let mut s = c.state.lock().unwrap();
        if s.token != token || token == 0 || s.closing {
            return;
        }
        if s.presenting {
            s.failure = Some("Android Activity launch failed; see logcat".into());
            drop(s);
            c.changed.notify_all();
        } else if !s.ready {
            drop(s);
            c.fail_launch(token, "Android Activity launch failed; see logcat".into());
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeStop(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
) {
    let Ok(c) = coordinator() else {
        return;
    };
    c.request_stop_checked(token, Some(activity), None);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeTouch(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    action: jint,
    x: jfloat,
    y: jfloat,
    width: jint,
    height: jint,
) {
    let Ok(c) = coordinator() else {
        return;
    };
    let s = c.state.lock().unwrap();
    if !s.accepts_input(token, activity, true) {
        return;
    }
    if action == 3 {
        c.input.release_all_inputs();
        return;
    }
    if width <= 0 || height <= 0 || !x.is_finite() || !y.is_finite() {
        return;
    }
    c.input.begin_capture(MouseMode::Absolute);
    c.input
        .absolute_motion_in_content(x as f64, y as f64, width as f64, height as f64);
    match action {
        0 => c
            .input
            .mouse_button(MouseButton::Left, ButtonState::Pressed),
        1 => c
            .input
            .mouse_button(MouseButton::Left, ButtonState::Released),
        _ => {}
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeKey(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    key: jint,
    pressed: jboolean,
    modifiers: jint,
) {
    let Ok(c) = coordinator() else {
        return;
    };
    let mut s = c.state.lock().unwrap();
    if !s.accepts_input(token, activity, false) || !(1..=255).contains(&key) {
        return;
    }
    if !s.queue_keyboard(KeyboardEvent::Key {
        key: key as u16,
        pressed: pressed != JNI_FALSE,
        modifiers: modifiers as u8 & 0x0f,
    }) {
        tracing::warn!("Android keyboard queue full; cancelling batch and releasing held keys");
    }
    drop(s);
    c.input_wake.notify_one();
}

// Instance ABI (excluding the implicit JNIEnv/JObject):
// Pointer (JJIFFII)V, Button (JJIZ)V, Scroll (JJIIZ)V, Text (JJLjava/lang/String;)V,
// Controller (JJIIIIIIIII)V: NINE ints, SuspendInput (JJZ)V, Dashboard (JJ)Z.
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativePointer(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    kind: jint,
    x: jfloat,
    y: jfloat,
    width: jint,
    height: jint,
) {
    let Ok(c) = coordinator() else { return };
    let s = c.state.lock().unwrap();
    if !s.accepts_input(token, activity, true) || !x.is_finite() || !y.is_finite() {
        return;
    }
    match kind {
        0 if width > 0 && height > 0 => {
            c.input.begin_capture(MouseMode::Absolute);
            c.input
                .absolute_motion_in_content(x as f64, y as f64, width as f64, height as f64);
        }
        1 => {
            c.input.begin_capture(MouseMode::Relative);
            // Bound a malformed/huge delta before it reaches the i32 accumulator.
            c.input.relative_motion(
                x.round().clamp(-32768.0, 32767.0) as i32,
                y.round().clamp(-32768.0, 32767.0) as i32,
            );
        }
        _ => {}
    }
}

fn mouse_button(button: jint) -> Option<MouseButton> {
    match button {
        1 => Some(MouseButton::Left),
        2 => Some(MouseButton::Right),
        3 => Some(MouseButton::Middle),
        4 => Some(MouseButton::X1),
        5 => Some(MouseButton::X2),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeButton(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    button: jint,
    pressed: jboolean,
) {
    let Ok(c) = coordinator() else { return };
    let Some(button) = mouse_button(button) else {
        return;
    };
    let s = c.state.lock().unwrap();
    if !s.accepts_input(token, activity, true) {
        return;
    }
    c.input.mouse_button(
        button,
        if pressed != JNI_FALSE {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        },
    );
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeScroll(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    vertical: jint,
    horizontal: jint,
    hires: jboolean,
) {
    let Ok(c) = coordinator() else { return };
    let s = c.state.lock().unwrap();
    if !s.accepts_input(token, activity, true) {
        return;
    }
    if vertical != 0 {
        c.input.vertical_scroll(vertical, hires != JNI_FALSE);
    }
    if horizontal != 0 {
        c.input.horizontal_scroll(horizontal, hires != JNI_FALSE);
    }
}

// Same US virtual-key table and whole-commit ASCII/UTF-8 decision as iOS.
fn ascii_key(ch: u8) -> Option<(u16, bool)> {
    match ch {
        b'a'..=b'z' => return Some(((ch - b'a' + b'A') as u16, false)),
        b'A'..=b'Z' => return Some((ch as u16, true)),
        b'0'..=b'9' => return Some((ch as u16, false)),
        b'\n' | b'\r' => return Some((0x0D, false)),
        b'\t' => return Some((0x09, false)),
        _ => {}
    }
    let plain = b" `-=[]\\;',./";
    let upper = b" ~_+{}|:\"<>?";
    let keys = [
        0x20, 0xC0, 0xBD, 0xBB, 0xDB, 0xDD, 0xDC, 0xBA, 0xDE, 0xBC, 0xBE, 0xBF,
    ];
    for (index, key) in keys.into_iter().enumerate() {
        if ch == plain[index] {
            return Some((key, false));
        }
        if ch == upper[index] {
            return Some((key, true));
        }
    }
    b")!@#$%^&*("
        .iter()
        .position(|value| *value == ch)
        .map(|index| (0x30 + index as u16, true))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeText(
    mut env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    text: JString,
) {
    let Ok(c) = coordinator() else { return };
    let revision = {
        let s = c.state.lock().unwrap();
        if !s.accepts_input(token, activity, false) || text.is_null() {
            return;
        }
        s.input_revision
    };
    // Bound the Java allocation before copying it, as well as the UTF-8 queue below.
    match env
        .call_method(&text, "length", "()I", &[])
        .and_then(|length| length.i())
    {
        Ok(1..=16384) => {}
        Ok(_) => return,
        Err(err) => {
            tracing::warn!(%err, "Android text length check failed");
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            return;
        }
    }
    let value: String = match env.get_string(&text) {
        Ok(value) => value.into(),
        Err(err) => {
            tracing::warn!(%err, "Android text decoding failed");
            if env.exception_check().unwrap_or(false) {
                let _ = env.exception_clear();
            }
            return;
        }
    };
    let mut s = c.state.lock().unwrap();
    if !s.accepts_input(token, activity, false) || s.input_revision != revision {
        return;
    }
    if value.len() > 16 * 1024 {
        tracing::warn!("Android text paste limit reached");
        return;
    }
    if !s.queue_keyboard(KeyboardEvent::Text(value)) {
        tracing::warn!("Android keyboard queue full; cancelling batch and releasing held keys");
    }
    drop(s);
    c.input_wake.notify_one();
}

impl ControllerState {
    fn neutral(number: i16, mask: u16) -> Self {
        Self {
            number,
            mask,
            buttons: 0,
            left_trigger: 0,
            right_trigger: 0,
            left_x: 0,
            left_y: 0,
            right_x: 0,
            right_y: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn from_jni(
        number: jint,
        mask: jint,
        buttons: jint,
        left_trigger: jint,
        right_trigger: jint,
        left_x: jint,
        left_y: jint,
        right_x: jint,
        right_y: jint,
    ) -> Option<Self> {
        if !(0..16).contains(&number) || !(0..=0xffff).contains(&mask) {
            return None;
        }
        if mask & (1 << number) == 0 {
            return Some(Self::neutral(number as i16, mask as u16));
        }
        Some(Self {
            number: number as i16,
            mask: mask as u16,
            buttons,
            left_trigger: left_trigger.clamp(0, 255) as u8,
            right_trigger: right_trigger.clamp(0, 255) as u8,
            left_x: left_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            left_y: left_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            right_x: right_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
            right_y: right_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        })
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeController(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    controller_number: jint,
    active_mask: jint,
    buttons: jint,
    left_trigger: jint,
    right_trigger: jint,
    left_x: jint,
    left_y: jint,
    right_x: jint,
    right_y: jint,
) {
    let Ok(c) = coordinator() else { return };
    let Some(controller) = ControllerState::from_jni(
        controller_number,
        active_mask,
        buttons,
        left_trigger,
        right_trigger,
        left_x,
        left_y,
        right_x,
        right_y,
    ) else {
        return;
    };
    let mut s = c.state.lock().unwrap();
    if !Coordinator::valid(&s, token, activity) || !s.ready {
        return;
    }
    s.observe_controller_session(c.runtime.latest_statistics().session_generation);
    // Menu/focus callbacks may still carry snapshots. Cache a neutral one, never
    // replay the held state when the transport's announcement eventually succeeds.
    if s.queue_controller(controller) {
        tracing::warn!("Android controller queue full; neutralizing held controls");
    }
    drop(s);
    c.input_wake.notify_one();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeSuspendInput(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
    suspended: jboolean,
) {
    let Ok(c) = coordinator() else { return };
    let mut s = c.state.lock().unwrap();
    if !Coordinator::valid(&s, token, activity) || s.dashboard {
        return;
    }
    let suspended = suspended != JNI_FALSE;
    if s.set_input_suspended(suspended) {
        c.input.release_all_inputs();
        c.input_wake.notify_one();
    }
    // Keep InputManager active: nativeKey/nativeText remain usable for the IME.
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_com_noland_connect_stream_NativeStreamBridge_nativeDashboard(
    _env: JNIEnv,
    _object: JObject,
    token: jlong,
    activity: jlong,
) -> jboolean {
    let Ok(c) = coordinator() else {
        return JNI_FALSE;
    };
    let mut s = c.state.lock().unwrap();
    if !Coordinator::valid(&s, token, activity) || !s.ready {
        return JNI_FALSE;
    }
    // Commit before Kotlin starts MainActivity (singleTask can destroy this Activity).
    s.enter_dashboard();
    c.input.set_focus(false);
    drop(s);
    c.changed.notify_all();
    c.input_wake.notify_one();
    c.wake.notify_one();
    let _ = java_statistics(token, "");
    JNI_TRUE
}

#[cfg(test)]
mod tests {
    use super::{
        ascii_key, mouse_button, ControllerState, Coordinator, KeyboardEvent, MouseButton, State,
    };

    fn active_route() -> State {
        State {
            token: 42,
            activity: 7,
            next_token: 42,
            next_activity: 7,
            ready: true,
            focused: true,
            input_registered: true,
            width: 1920,
            height: 1080,
            ..State::default()
        }
    }

    fn key(key: u16, pressed: bool) -> KeyboardEvent {
        KeyboardEvent::Key {
            key,
            pressed,
            modifiers: 0,
        }
    }

    #[test]
    fn keyboard_fifo_keeps_commit_before_enter() {
        let mut s = active_route();
        let expected = [
            KeyboardEvent::Text("committed".into()),
            key(0x0D, true),
            key(0x0D, false),
        ];
        for event in expected {
            assert!(s.queue_keyboard(event));
        }
        assert_eq!(
            s.keyboard.pop_front().unwrap().event,
            KeyboardEvent::Text("committed".into())
        );
        assert_eq!(s.keyboard.pop_front().unwrap().event, key(0x0D, true));
        assert_eq!(s.keyboard.pop_front().unwrap().event, key(0x0D, false));
    }

    #[test]
    fn keyboard_fifo_keeps_commit_backspace_replacement_order() {
        let mut s = active_route();
        s.queue_keyboard(KeyboardEvent::Text("old".into()));
        s.queue_keyboard(key(0x08, true));
        s.queue_keyboard(key(0x08, false));
        s.queue_keyboard(KeyboardEvent::Text("replacement".into()));
        let events: Vec<_> = s.keyboard.drain(..).map(|input| input.event).collect();
        assert_eq!(
            events,
            vec![
                KeyboardEvent::Text("old".into()),
                key(0x08, true),
                key(0x08, false),
                KeyboardEvent::Text("replacement".into())
            ]
        );
    }

    #[test]
    fn cancellation_between_keyboard_items_releases_sent_down_and_drops_remaining_items() {
        let mut s = active_route();
        s.queue_keyboard(key(0x41, true));
        s.queue_keyboard(KeyboardEvent::Text("cancelled".into()));
        let down = s.keyboard.pop_front().unwrap();
        assert!(s.keyboard_is_current(&down));
        s.track_key(0x41, true); // The actor producer registered this down before its await.
        s.release_inputs();
        assert!(!s.keyboard_is_current(&down));
        assert!(s.keyboard.is_empty());
        assert!(s.pressed_keys.is_empty());
        assert!(s.key_releases.contains(&(42, 0x41)));
        assert!(s.queue_keyboard(KeyboardEvent::Text("new revision".into())));
        assert!(s.keyboard_is_current(s.keyboard.front().unwrap()));
    }

    #[test]
    fn keyboard_shift_state_follows_processing_not_future_enqueues() {
        let mut s = active_route();
        s.queue_keyboard(KeyboardEvent::Text("A".into()));
        s.queue_keyboard(key(0xA0, true));
        assert_eq!(s.shift_keys, 0);
        s.keyboard.pop_front(); // The text must synthesize its own Shift.
        s.track_key(0xA0, true); // Processing the queued hardware down.
        assert_ne!(s.shift_keys, 0);
        s.queue_keyboard(KeyboardEvent::Text("B".into()));
        s.queue_keyboard(key(0xA0, false));
        assert_ne!(s.shift_keys, 0); // A future up must not change earlier text.
        s.track_key(0xA0, false);
        assert_eq!(s.shift_keys, 0);
    }

    #[test]
    fn keyboard_tags_reject_old_activity_and_generation() {
        let mut s = active_route();
        s.queue_keyboard(key(0x0D, true));
        let input = s.keyboard.pop_front().unwrap();
        assert_eq!((input.token, input.activity, input.revision), (42, 7, 0));
        s.activity = 8;
        assert!(!s.keyboard_is_current(&input));
        s.activity = 7;
        s.token = 43;
        assert!(!s.keyboard_is_current(&input));
    }

    #[test]
    fn keyboard_overflow_cancels_batch_and_preserves_pending_releases() {
        let mut s = active_route();
        s.track_key(0xA0, true);
        s.track_key(0x41, true);
        for _ in 0..256 {
            assert!(s.queue_keyboard(KeyboardEvent::Text("pending".into())));
        }
        assert!(!s.queue_keyboard(key(0x0D, true)));
        assert!(s.keyboard.is_empty());
        assert!(s.pressed_keys.is_empty());
        assert_eq!(s.shift_keys, 0);
        assert_eq!(
            s.key_releases.iter().copied().collect::<Vec<_>>(),
            vec![(42, 0x41), (42, 0xA0)]
        );
        assert_eq!(s.input_revision, 1);
    }

    #[test]
    fn dashboard_teardown_preserves_session_and_player_presence() {
        let mut s = active_route();
        s.controller_mask = 0b101;
        s.queue_controller(ControllerState::neutral(0, 0b101));
        s.enter_dashboard();
        assert!(Coordinator::valid(&s, 42, 7)); // SurfaceLost still needs a detach ACK.
        assert!(!s.activity_can_stop(42, 7));
        assert!(!s.surface_loss_expired(42, s.loss_revision));
        assert!(!s.accepts_input(42, 7, true));
        assert!(!s.accepts_input(42, 7, false));
        assert_eq!((s.token, s.ready, s.closing), (42, true, false));
        assert_eq!((s.width, s.height), (1920, 1080));
        assert_eq!((s.controller_mask, s.neutral_mask), (0b101, 0b101));
        assert!(s.controllers.is_empty());
    }

    #[test]
    fn dashboard_reentry_claim_rejects_old_activity_without_restarting_route() {
        let mut s = active_route();
        s.controller_mask = 1;
        s.enter_dashboard();
        s.presenting = true;
        let activity = s.claim_activity();
        assert_eq!(activity, 8);
        assert_eq!((s.token, s.ready, s.controller_mask), (42, true, 1));
        assert!(!s.dashboard);
        assert!(!s.input_suspended);
        assert!(!s.activity_can_stop(42, 7));
        assert!(s.activity_can_stop(42, activity));
        assert!(!s.surface_loss_expired(42, s.loss_revision)); // Reentry has its own deadline.
        assert!(!s.accepts_input(42, activity, true)); // Await surface and focus.
        s.presenting = false;
        s.focused = true;
        s.input_registered = true;
        assert!(s.accepts_input(42, activity, true));
    }

    #[test]
    fn suspension_releases_controls_but_allows_ime_and_preserves_mask() {
        let mut s = active_route();
        s.controller_mask = 0x8001;
        s.shift_keys = 1;
        s.queue_controller(ControllerState::neutral(0, 0x8001));
        let revision = s.input_revision;
        assert!(s.set_input_suspended(true));
        assert_eq!(s.controller_mask, 0x8001);
        assert_eq!(s.neutral_mask, 0x8001);
        assert!(s.controllers.is_empty());
        assert_eq!(s.shift_keys, 0);
        assert_ne!(s.input_revision, revision);
        assert!(!s.accepts_input(42, 7, true));
        assert!(s.accepts_input(42, 7, false));
        assert!(!s.accepts_input(41, 7, false));
        assert!(s.queue_keyboard(KeyboardEvent::Text("IME".into())));
        assert!(!s.set_input_suspended(true)); // Repeated suspension must not cancel IME text.
        assert_eq!(s.keyboard.len(), 1);
        assert!(!s.set_input_suspended(false));
        assert!(s.accepts_input(42, 7, true));
        assert_eq!(s.controller_mask, 0x8001);
    }

    #[test]
    fn surface_timeout_is_only_for_current_unexpected_loss() {
        let mut s = active_route();
        assert!(s.surface_loss_expired(42, 0));
        assert!(!s.surface_loss_expired(41, 0));
        assert!(!s.surface_loss_expired(42, 1));
        s.presenting = true;
        assert!(!s.surface_loss_expired(42, 0));
        s.presenting = false;
        s.closing = true;
        assert!(!s.surface_loss_expired(42, 0));
    }

    #[test]
    fn ascii_virtual_keys_match_ios_including_shift_and_punctuation() {
        for ch in 32..=126 {
            assert!(ascii_key(ch).is_some(), "missing ASCII {ch}");
        }
        for (ch, key, shift) in [
            (b'a', 0x41, false),
            (b'Z', 0x5A, true),
            (b' ', 0x20, false),
            (b'!', 0x31, true),
            (b')', 0x30, true),
            (b'(', 0x39, true),
            (b'\\', 0xDC, false),
            (b'|', 0xDC, true),
            (b'?', 0xBF, true),
            (b'\n', 0x0D, false),
            (b'\r', 0x0D, false),
            (b'\t', 0x09, false),
        ] {
            assert_eq!(ascii_key(ch), Some((key, shift)));
        }
        assert_eq!(ascii_key(0), None);
        assert_eq!(ascii_key(0x7f), None);
        assert_eq!(ascii_key(0xc3), None);
    }

    #[test]
    fn android_button_order_is_not_native_middle_right_order() {
        assert_eq!(mouse_button(1), Some(MouseButton::Left));
        assert_eq!(mouse_button(2), Some(MouseButton::Right));
        assert_eq!(mouse_button(3), Some(MouseButton::Middle));
        assert_eq!(mouse_button(4), Some(MouseButton::X1));
        assert_eq!(mouse_button(5), Some(MouseButton::X2));
        assert_eq!(mouse_button(0), None);
        assert_eq!(mouse_button(6), None);
    }

    #[test]
    fn controller_bounds_keep_all_sixteen_slots_and_mask_bits() {
        for number in [-1, 16, i32::MAX] {
            assert!(ControllerState::from_jni(number, 1, 0, 0, 0, 0, 0, 0, 0).is_none());
        }
        for mask in [-1, 65536] {
            assert!(ControllerState::from_jni(0, mask, 0, 0, 0, 0, 0, 0, 0).is_none());
        }
        let p =
            ControllerState::from_jni(15, 65535, 123, -1, 256, i32::MIN, i32::MAX, -32768, 32767)
                .unwrap();
        assert_eq!(p.number, 15);
        assert_eq!(p.mask as i16, -1); // Runtime API uses an i16 mask, preserving all bits.
        assert_eq!((p.left_trigger, p.right_trigger), (0, 255));
        assert_eq!(
            (p.left_x, p.left_y, p.right_x, p.right_y),
            (-32768, 32767, -32768, 32767)
        );
        assert_eq!(
            ControllerState::from_jni(0, 0, 123, 99, 99, 10, 10, 10, 10),
            Some(ControllerState::neutral(0, 0))
        );
    }

    #[test]
    fn failed_controller_arrival_remains_pending_until_retry_tick() {
        let mut s = active_route();
        s.observe_controller_session(7);
        let mut press = ControllerState::neutral(0, 1);
        press.buttons = 0x1000;
        s.queue_controller(press);
        let input = s.take_controller_input().unwrap();
        s.controller_failed(&input);
        assert_eq!(s.controller_announced, 0);
        assert!(s.take_controller_input().is_none());
        assert_eq!(s.desired_controllers[0], Some(press));
        s.controller_retry_tick(); // The independent 250 ms timer calls this without input.
        assert_eq!(s.take_controller_input().unwrap().state, press);
    }

    #[test]
    fn controller_reconnect_invalidates_announcements_and_held_desired_state() {
        let mut s = active_route();
        s.observe_controller_session(7);
        let mut press = ControllerState::neutral(0, 1);
        press.buttons = 0x1000;
        s.queue_controller(press);
        let stale = s.take_controller_input().unwrap();
        s.controller_announced = 1;
        s.observe_controller_session(8);
        assert_eq!(s.controller_announced, 0);
        assert!(s.controllers.is_empty());
        assert_ne!(s.controller_route(), Some(stale.route));
        assert_eq!(
            s.desired_controllers[0],
            Some(ControllerState::neutral(0, 1))
        );
        assert_eq!(
            s.take_controller_neutral().unwrap().state,
            ControllerState::neutral(0, 1)
        );
        s.controller_failed(&stale); // A late old-transport ACK must not block the new route.
        assert_eq!(s.controller_retry_blocked, 0);
    }

    #[test]
    fn menu_controller_snapshots_are_neutral_without_player_one_hot_unplug() {
        let mut s = active_route();
        s.observe_controller_session(7);
        s.controller_announced = 1;
        let mut press = ControllerState::neutral(0, 1);
        press.buttons = 0x1000;
        press.left_trigger = 255;
        s.queue_controller(press);
        s.set_input_suspended(true);
        s.queue_controller(press); // Kotlin may send a final held snapshot during suspension.
        assert_eq!(s.controller_mask, 1);
        assert_eq!(s.controller_announced, 1);
        assert_eq!(
            s.desired_controllers[0],
            Some(ControllerState::neutral(0, 1))
        );
        s.queue_controller(ControllerState::neutral(0, 0)); // Virtual pad toggled off.
        assert_eq!(s.controller_mask, 1);
        assert!(s
            .controllers
            .iter()
            .all(|input| input.state == ControllerState::neutral(0, 1)));
    }

    #[test]
    fn controller_retries_rebind_to_current_activity_and_input_revision() {
        let mut s = active_route();
        s.observe_controller_session(7);
        s.queue_controller(ControllerState::neutral(0, 1));
        let stale = s.take_controller_input().unwrap();
        s.controller_failed(&stale);
        s.claim_activity();
        assert!(s.controllers.is_empty());
        let retry = s.take_controller_neutral().unwrap();
        assert_eq!(retry.route.token, stale.route.token);
        assert_ne!(retry.route.activity, stale.route.activity);
        assert_ne!(retry.route.revision, stale.route.revision);
        assert_eq!(retry.state, ControllerState::neutral(0, 1));
    }

    #[test]
    fn failed_arrival_retries_latest_neutral_not_a_released_press() {
        let mut s = active_route();
        s.observe_controller_session(7);
        let mut press = ControllerState::neutral(0, 1);
        press.buttons = 0x1000;
        s.queue_controller(press);
        let input = s.take_controller_input().unwrap();
        s.controller_failed(&input);
        s.queue_controller(ControllerState::neutral(0, 1));
        assert!(s.take_controller_input().is_none());
        s.controller_retry_tick();
        assert_eq!(
            s.take_controller_input().unwrap().state,
            ControllerState::neutral(0, 1)
        );
    }

    #[test]
    fn controller_neutral_presence_exists_without_any_kotlin_snapshot() {
        let mut s = active_route();
        s.observe_controller_session(7);
        assert_eq!(s.controller_mask, 1);
        assert_eq!(
            s.take_controller_input().unwrap().state,
            ControllerState::neutral(0, 1)
        );
    }

    #[test]
    fn controller_fifo_preserves_edges_and_overflow_neutralizes() {
        let mut s = active_route();
        let mut press = ControllerState::neutral(0, 1);
        press.buttons = 1;
        assert!(!s.queue_controller(press));
        assert!(!s.queue_controller(ControllerState::neutral(0, 1)));
        assert_eq!(s.controllers.pop_front().unwrap().state.buttons, 1);
        assert_eq!(s.controllers.pop_front().unwrap().state.buttons, 0);
        for _ in 0..256 {
            assert!(!s.queue_controller(press));
        }
        assert!(s.queue_controller(ControllerState::neutral(0, 1)));
        assert_eq!(s.controllers.len(), 1);
        assert_eq!((s.neutral_mask, s.controller_mask), (1, 1));
    }

    #[test]
    fn mask_change_drops_disconnected_slot_work_and_releases_it() {
        let mut s = active_route();
        s.queue_controller(ControllerState::neutral(15, 0x8001));
        s.queue_controller(ControllerState::neutral(0, 1));
        assert_eq!(s.controller_mask, 1);
        assert_eq!(s.neutral_mask, 0x8000);
        assert_eq!(s.controllers.len(), 1);
        assert_eq!(s.controllers[0].state.number, 0);
    }

    #[test]
    fn current_route_and_activity_are_accepted() {
        assert!(Coordinator::valid(&active_route(), 42, 7));
    }

    #[test]
    fn stale_route_is_rejected_even_when_activity_matches() {
        assert!(!Coordinator::valid(&active_route(), 41, 7));
    }

    #[test]
    fn stale_activity_is_rejected_after_recreation_in_same_route() {
        let mut state = active_route();
        state.activity = 8;
        assert!(!Coordinator::valid(&state, 42, 7));
        assert!(Coordinator::valid(&state, 42, 8));
    }

    #[test]
    fn zero_route_and_activity_are_rejected_even_when_they_match() {
        let mut state = active_route();
        state.token = 0;
        assert!(!Coordinator::valid(&state, 0, 7));
        state.token = 42;
        state.activity = 0;
        assert!(!Coordinator::valid(&state, 42, 0));
        assert!(!Coordinator::valid(&State::default(), 0, 0));
    }

    #[test]
    fn closing_route_rejects_current_activity_callbacks() {
        let mut state = active_route();
        state.closing = true;
        assert!(!Coordinator::valid(&state, 42, 7));
    }

    #[test]
    fn cleanup_claimed_during_detach_preserves_generation_for_cleanup_owner() {
        let mut state = active_route();
        state.closing = true;
        assert!(!state.disconnecting); // State before reconciliation awaits detach.
        let token = state.token;

        // request_stop claims full cleanup while the detach ACK is pending.
        state.disconnecting = true;
        state.finish_surface_close(token);

        assert_eq!(state.token, 42);
        assert_eq!(state.activity, 7);
        assert!(state.ready);
        assert!(state.closing);
        assert!(state.disconnecting);
    }

    #[test]
    fn unclaimed_close_retires_route_after_detach() {
        let mut state = active_route();
        state.closing = true;
        state.next_token = 42;
        state.next_activity = 7;

        state.finish_surface_close(42);

        assert_eq!(state.token, 0);
        assert_eq!(state.activity, 0);
        assert!(!state.ready);
        assert!(!state.closing);
        assert!(!state.disconnecting);
        assert_eq!(state.next_token, 42);
        assert_eq!(state.next_activity, 7);
    }

    #[test]
    fn stale_close_does_not_retire_replacement_route() {
        let mut state = active_route();
        state.token = 43;
        state.activity = 8;
        state.closing = true;

        state.finish_surface_close(42);

        assert_eq!(state.token, 43);
        assert_eq!(state.activity, 8);
        assert!(state.ready);
        assert!(state.closing);
        assert!(!state.disconnecting);
    }
}
