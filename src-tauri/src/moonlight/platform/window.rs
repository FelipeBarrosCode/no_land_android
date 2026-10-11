use std::{
    ffi::c_void,
    sync::atomic::{AtomicBool, Ordering},
};

#[cfg(not(target_os = "android"))]
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};

#[cfg(all(target_os = "macos", not(test)))]
unsafe extern "C" {
    fn noland_macos_resolve_stream_target_view(ns_view: *mut c_void) -> *mut c_void;
}
use tauri::{AppHandle, Manager, Runtime, Window};

use crate::moonlight::{
    domain::MoonlightError, native, platform::desktop_input::uninstall_native_stream_input,
};

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub const STREAM_WINDOW_LABEL: &str = "moonlight-stream";
#[cfg(any(target_os = "ios", target_os = "android"))]
pub const STREAM_WINDOW_LABEL: &str = "main";

#[cfg(target_os = "ios")]
unsafe extern "C" {
    fn noland_ios_stream_surface(root: *mut c_void) -> *mut c_void;
    fn noland_ios_stream_present() -> i32;
    fn noland_ios_stream_close();
}

#[derive(Debug, Default)]
pub struct StreamWindowCloseState {
    allow_close: AtomicBool,
    closing_in_progress: AtomicBool,
}

impl StreamWindowCloseState {
    pub fn allow_close(&self) -> bool {
        self.allow_close.load(Ordering::SeqCst)
    }

    pub fn set_allow_close(&self, value: bool) {
        self.allow_close.store(value, Ordering::SeqCst);
    }

    pub fn begin_close_intercept(&self) -> bool {
        !self.closing_in_progress.swap(true, Ordering::SeqCst)
    }

    pub fn finish_close_intercept(&self) {
        self.closing_in_progress.store(false, Ordering::SeqCst);
    }

    pub fn reset(&self) {
        self.allow_close.store(false, Ordering::SeqCst);
        self.closing_in_progress.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NativeSurfaceDescriptor {
    pub surface_type: native::nl_surface_type_t,
    pub window_handle: usize,
    pub display_handle: usize,
    pub width: u32,
    pub height: u32,
    pub scale_factor: f32,
}

impl NativeSurfaceDescriptor {
    pub fn to_native(&self) -> native::nl_surface_descriptor_t {
        native::nl_surface_descriptor_t {
            surface_type: self.surface_type,
            window_handle: self.window_handle as *mut c_void,
            display_handle: self.display_handle as *mut c_void,
            width: self.width,
            height: self.height,
            scale_factor: self.scale_factor,
        }
    }
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub fn create_or_reuse_stream_window<R: Runtime>(
    app: &AppHandle<R>,
    width: u32,
    height: u32,
    title: &str,
) -> Result<Window<R>, MoonlightError> {
    if let Some(window) = app.get_window(STREAM_WINDOW_LABEL) {
        app.state::<StreamWindowCloseState>().reset();
        let _ = window.set_fullscreen(false);
        let _ = window.set_title(title);
        window
            .hide()
            .map_err(|error| MoonlightError::Native(error.to_string()))?;
        return Ok(window);
    }

    app.state::<StreamWindowCloseState>().reset();
    let builder = tauri::window::WindowBuilder::new(app, STREAM_WINDOW_LABEL)
        .title(title)
        .inner_size(width as f64, height as f64)
        .resizable(true)
        .decorations(cfg!(target_os = "macos"))
        .visible(false);
    #[cfg(target_os = "macos")]
    let builder = builder
        .title_bar_style(tauri::utils::TitleBarStyle::Overlay)
        .hidden_title(true);
    #[cfg(target_os = "windows")]
    let builder = builder.shadow(true);
    let window = builder
        .build()
        .map_err(|error| MoonlightError::Native(error.to_string()))?;
    Ok(window)
}

#[cfg(any(target_os = "ios", target_os = "android"))]
pub fn create_or_reuse_stream_window<R: Runtime>(
    app: &AppHandle<R>,
    _width: u32,
    _height: u32,
    _title: &str,
) -> Result<Window<R>, MoonlightError> {
    app.state::<StreamWindowCloseState>().reset();
    app.get_window(STREAM_WINDOW_LABEL).ok_or_else(|| {
        MoonlightError::Native(format!(
            "The {} application window is unavailable",
            if cfg!(target_os = "android") {
                "Android"
            } else {
                "iOS"
            }
        ))
    })
}

pub fn present_stream_window<R: Runtime>(window: &Window<R>) -> Result<(), MoonlightError> {
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        window
            .show()
            .map_err(|error| MoonlightError::Native(error.to_string()))?;
        window
            .set_fullscreen(true)
            .map_err(|error| MoonlightError::Native(error.to_string()))?;
        window
            .set_focus()
            .map_err(|error| MoonlightError::Native(error.to_string()))?;
    }
    #[cfg(target_os = "ios")]
    {
        let _ = window;
        if unsafe { noland_ios_stream_present() } != 0 {
            return Err(MoonlightError::Native(
                "The iOS stream presentation is unavailable".into(),
            ));
        }
    }
    #[cfg(target_os = "android")]
    {
        let _ = window;
        Err(MoonlightError::Native(
            "Android native stream presentation is not connected yet".into(),
        ))
    }
    #[cfg(not(target_os = "android"))]
    Ok(())
}

pub fn close_stream_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), MoonlightError> {
    app.state::<StreamWindowCloseState>().set_allow_close(true);
    app.state::<StreamWindowCloseState>()
        .finish_close_intercept();
    if let Some(window) = app.get_window(STREAM_WINDOW_LABEL) {
        let _ = uninstall_native_stream_input(&window);
        #[cfg(not(any(target_os = "ios", target_os = "android")))]
        window
            .close()
            .map_err(|error| MoonlightError::Native(error.to_string()))?;
    }
    #[cfg(target_os = "ios")]
    unsafe {
        noland_ios_stream_close();
    }
    Ok(())
}

#[cfg(target_os = "android")]
pub fn stream_window_surface_descriptor<R: Runtime>(
    _window: &Window<R>,
) -> Result<NativeSurfaceDescriptor, MoonlightError> {
    Err(MoonlightError::Native(
        "Android stream surface bridge is not connected yet".into(),
    ))
}

#[cfg(not(target_os = "android"))]
pub fn stream_window_surface_descriptor<R: Runtime>(
    window: &Window<R>,
) -> Result<NativeSurfaceDescriptor, MoonlightError> {
    let size = window
        .inner_size()
        .map_err(|error| MoonlightError::Native(error.to_string()))?;
    let scale_factor = window
        .scale_factor()
        .map_err(|error| MoonlightError::Native(error.to_string()))? as f32;
    let raw_window = window
        .window_handle()
        .map_err(|error| MoonlightError::Native(error.to_string()))?
        .as_raw();
    let raw_display = window
        .display_handle()
        .map_err(|error| MoonlightError::Native(error.to_string()))?
        .as_raw();

    surface_descriptor_from_raw_handles(
        raw_window,
        raw_display,
        size.width,
        size.height,
        scale_factor,
    )
}

#[cfg(not(target_os = "android"))]
fn surface_descriptor_from_raw_handles(
    raw_window: RawWindowHandle,
    raw_display: RawDisplayHandle,
    width: u32,
    height: u32,
    scale_factor: f32,
) -> Result<NativeSurfaceDescriptor, MoonlightError> {
    match raw_window {
        #[cfg(target_os = "ios")]
        RawWindowHandle::UiKit(handle) => {
            let surface = unsafe { noland_ios_stream_surface(handle.ui_view.as_ptr()) };
            if surface.is_null() {
                return Err(MoonlightError::Native(
                    "Could not create the iOS stream surface".into(),
                ));
            }
            Ok(NativeSurfaceDescriptor {
                surface_type: native::nl_surface_type_NL_SURFACE_IOS_UIVIEW,
                window_handle: surface as usize,
                display_handle: 0,
                width,
                height,
                scale_factor,
            })
        }
        RawWindowHandle::AppKit(handle) => {
            let resolved_view = {
                #[cfg(all(target_os = "macos", not(test)))]
                {
                    let resolved =
                        unsafe { noland_macos_resolve_stream_target_view(handle.ns_view.as_ptr()) };
                    if resolved.is_null() {
                        handle.ns_view.as_ptr()
                    } else {
                        resolved
                    }
                }
                #[cfg(any(not(target_os = "macos"), test))]
                {
                    handle.ns_view.as_ptr()
                }
            };
            Ok(NativeSurfaceDescriptor {
                surface_type: native::nl_surface_type_NL_SURFACE_MACOS_NSVIEW,
                window_handle: resolved_view as usize,
                display_handle: match raw_display {
                    RawDisplayHandle::AppKit(_) => 0,
                    _ => 0,
                },
                width,
                height,
                scale_factor,
            })
        }
        RawWindowHandle::Win32(handle) => Ok(NativeSurfaceDescriptor {
            surface_type: native::nl_surface_type_NL_SURFACE_WINDOWS_HWND,
            window_handle: handle.hwnd.get() as usize,
            display_handle: handle
                .hinstance
                .map(|value| value.get() as usize)
                .unwrap_or(0),
            width,
            height,
            scale_factor,
        }),
        RawWindowHandle::Xlib(handle) => {
            let display_handle = match raw_display {
                RawDisplayHandle::Xlib(display) => display
                    .display
                    .map(|value| value.as_ptr() as usize)
                    .unwrap_or(0),
                _ => 0,
            };
            Ok(NativeSurfaceDescriptor {
                surface_type: native::nl_surface_type_NL_SURFACE_X11_WINDOW,
                window_handle: handle.window as usize,
                display_handle,
                width,
                height,
                scale_factor,
            })
        }
        RawWindowHandle::Wayland(handle) => {
            let display_handle = match raw_display {
                RawDisplayHandle::Wayland(display) => display.display.as_ptr() as usize,
                _ => 0,
            };
            Ok(NativeSurfaceDescriptor {
                surface_type: native::nl_surface_type_NL_SURFACE_WAYLAND_SURFACE,
                window_handle: handle.surface.as_ptr() as usize,
                display_handle,
                width,
                height,
                scale_factor,
            })
        }
        other => Err(MoonlightError::Native(format!(
            "unsupported raw window handle for moonlight stream surface: {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::c_void, num::NonZeroIsize, ptr::NonNull};

    use raw_window_handle::{
        AppKitDisplayHandle, AppKitWindowHandle, RawDisplayHandle, RawWindowHandle,
        Win32WindowHandle, WindowsDisplayHandle,
    };

    use super::surface_descriptor_from_raw_handles;
    use crate::moonlight::native;

    #[test]
    fn maps_appkit_surface_descriptor() {
        let view = NonNull::<c_void>::dangling();
        let descriptor = surface_descriptor_from_raw_handles(
            RawWindowHandle::AppKit(AppKitWindowHandle::new(view)),
            RawDisplayHandle::AppKit(AppKitDisplayHandle::new()),
            1920,
            1080,
            2.0,
        )
        .unwrap();
        assert_eq!(
            descriptor.surface_type,
            native::nl_surface_type_NL_SURFACE_MACOS_NSVIEW
        );
        assert_eq!(descriptor.window_handle, view.as_ptr() as usize);
    }

    #[test]
    fn maps_win32_surface_descriptor() {
        let hwnd = NonZeroIsize::new(100).unwrap();
        let descriptor = surface_descriptor_from_raw_handles(
            RawWindowHandle::Win32(Win32WindowHandle::new(hwnd)),
            RawDisplayHandle::Windows(WindowsDisplayHandle::new()),
            1280,
            720,
            1.0,
        )
        .unwrap();
        assert_eq!(
            descriptor.surface_type,
            native::nl_surface_type_NL_SURFACE_WINDOWS_HWND
        );
        assert_eq!(descriptor.window_handle, 100isize as usize);
    }
}
