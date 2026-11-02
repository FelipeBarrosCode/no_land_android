//! Optional contribution control plane. It never owns or requests a paid provider operation.
#[cfg(target_os = "android")]
mod android;
pub mod dto;
pub mod outbox;
mod secure;
mod telemetry;

use super::app_context::AppContext;
use crate::{
    models::{
        app_state::{OfferCandidate, OrchestrationState, PersistedAppState},
        feedback::*,
        vast::VastInstance,
    },
    moonlight::composition::MoonlightManager,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use outbox::{Journal, Milestone, Outbox, Rental};
use reqwest::Method;
use secure::{DeletionReceipt, EnrollmentAttempt, Participant, Vault};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use tokio::sync::{Mutex, Notify};

pub struct FeedbackProgramService {
    outbox: Outbox,
    api_url: Option<String>,
    http: Option<reqwest::Client>,
    inner: Mutex<Local>,
    network: Mutex<()>,
    controls: Mutex<()>,
    lifecycle_ready: AtomicBool,
    collecting: AtomicBool,
    desired_disabled: AtomicBool,
    stop_generation: AtomicU64,
    foreground: AtomicBool,
    cancel: Notify,
    pub wake: Notify,
}
struct Local {
    initialized: bool,
    vault: Option<Vault>,
    journal: Option<Journal>,
    metrics: telemetry::Accumulator,
    notices: Option<ProgramDocument>,
    message: Option<String>,
    next_delivery: Instant,
    retry_at: Instant,
    failures: u32,
    dirty: bool,
}
impl Default for Local {
    fn default() -> Self {
        Self {
            initialized: false,
            vault: None,
            journal: None,
            metrics: Default::default(),
            notices: None,
            message: None,
            next_delivery: Instant::now(),
            retry_at: Instant::now(),
            failures: 0,
            dirty: false,
        }
    }
}
struct ApiFailure {
    message: String,
    retryable: bool,
    after: Duration,
    unauthorized: bool,
}
impl ApiFailure {
    fn local(message: &str) -> Self {
        Self {
            message: message.into(),
            retryable: true,
            after: Duration::from_secs(30),
            unauthorized: false,
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConsentAck {
    consent_epoch: u64,
    enabled: bool,
    optional_context_enabled: bool,
}

impl FeedbackProgramService {
    pub fn new(root: &Path) -> Self {
        Self {
            outbox: Outbox::new(root),
            api_url: configured_url(option_env!("NOLAND_FEEDBACK_API_URL")),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(3))
                .timeout(Duration::from_secs(8))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .ok(),
            inner: Mutex::new(Local::default()),
            network: Mutex::new(()),
            controls: Mutex::new(()),
            lifecycle_ready: AtomicBool::new(false),
            collecting: AtomicBool::new(false),
            desired_disabled: AtomicBool::new(false),
            stop_generation: AtomicU64::new(0),
            foreground: AtomicBool::new(true),
            cancel: Notify::new(),
            wake: Notify::new(),
        }
    }
    pub fn set_foreground(&self, foreground: bool) {
        self.foreground.store(foreground, Ordering::SeqCst);
        if !foreground {
            self.cancel.notify_waiters();
        }
    }
    fn stop_local(&self) {
        self.collecting.store(false, Ordering::SeqCst);
        self.cancel.notify_waiters();
    }
    fn collection_consent(&self, p: &FeedbackProgramState) -> bool {
        self.collecting.load(Ordering::SeqCst)
            && !self.desired_disabled.load(Ordering::SeqCst)
            && self.api_url.is_some()
            && self.http.is_some()
            && cfg!(all(target_os = "android", noland_feedback_native_android))
            && self.lifecycle_ready.load(Ordering::SeqCst)
            && p.enrollment_status == "enrolled"
            && p.schema_version == 1
            && p.consent_epoch > 0
            && ((p.enabled && p.pending_control_action.is_none())
                || p.pending_control_action.as_ref().is_some_and(|c| {
                    c.kind == ControlKind::Consent && c.enabled && c.preserve_sessions
                }))
    }
    fn can_collect(&self, p: &FeedbackProgramState) -> bool {
        self.foreground.load(Ordering::SeqCst) && self.collection_consent(p)
    }
    fn permitted(&self, p: &FeedbackProgramState) -> bool {
        self.can_collect(p) && p.enabled && p.pending_control_action.is_none()
    }
    fn owns_journal(&self, p: &FeedbackProgramState, local: &Local) -> bool {
        self.collection_consent(p)
            && local.vault.as_ref().is_some_and(|v| {
                v.installation_id.as_str() == p.installation_id.as_deref().unwrap_or("")
                    && v.receipt.is_none()
                    && !v.collection_paused
                    && v.api_url == self.api_url
                    && v.participant.as_ref().is_some_and(|a| {
                        a.enabled
                            && valid_participant(a)
                            && p.user_id.as_ref() == Some(&a.user_id)
                            && p.consent_epoch == a.consent_epoch
                            && p.privacy_version.as_ref() == Some(&a.privacy_version)
                            && p.terms_version.as_ref() == Some(&a.terms_version)
                    })
                    && p.credential_ref.as_deref() == Some(CREDENTIAL_REF)
                    && p.deletion_code_ref.as_deref() == Some(CREDENTIAL_REF)
            })
            && local
                .journal
                .as_ref()
                .is_some_and(|j| Some(&j.installation_id) == p.installation_id.as_ref())
    }
    async fn initialize(&self, context: &AppContext) {
        let p = context.load_state().await.feedback_program;
        let mut local = self.inner.lock().await;
        if local.initialized {
            return;
        }
        local.initialized = true;
        // Reading an absent vault creates no key, identity, journal, or network request.
        // Always inspect it so resetting/recovering state.json cannot strand an offline deletion.
        let mut vault = match secure::read().await {
            Ok(Some(v)) => v,
            Ok(None) => {
                if p.credential_ref.is_some() {
                    local.message = Some(
                        "Protected feedback credentials are missing; feedback stays off".into(),
                    );
                }
                return;
            }
            Err(e) => {
                local.message = Some(e);
                return;
            }
        };
        if vault.api_url != self.api_url {
            local.message = Some("Feedback service configuration changed; protected ownership material is retained and no credentials will be sent to the new service".into());
            local.vault = Some(vault);
            return;
        }
        // The atomic public state records Stop/Delete before any network lock. Prefer its newer intent
        // over a protected copy from an older in-flight acknowledgment.
        if vault.receipt.is_none()
            && p.installation_id.as_ref() == Some(&vault.installation_id)
            && p.pending_control_action.is_some()
        {
            vault.pending_control = p.pending_control_action.clone();
            if let Err(e) = secure::write(&vault).await {
                local.message = Some(e);
                local.vault = Some(vault);
                return;
            }
        }
        let authorized = !vault.collection_paused
            && vault.pending_control.is_none()
            && vault.receipt.is_none()
            && vault
                .participant
                .as_ref()
                .is_some_and(|v| matches_participant(&p, &vault, v));
        if authorized
            || vault
                .pending_control
                .as_ref()
                .is_some_and(|c| c.preserve_sessions)
            || p.pending_control_action
                .as_ref()
                .is_some_and(|c| c.preserve_sessions)
        {
            match self.outbox.load(&vault.installation_id) {
                Ok(journal) => {
                    // Persist the process-gap downgrade before any delivery, retaining known success.
                    if let Err(e) = self.outbox.save(&journal) {
                        local.message = Some(e);
                    } else {
                        local.journal = Some(journal);
                    }
                }
                Err(e) => local.message = Some(e),
            }
        } else if p.pending_control_action.is_some() || !p.enabled {
            if let Err(e) = self.outbox.clear() {
                local.message = Some(e);
            }
        }
        let usable = authorized && local.journal.is_some();
        let recovery = if let Some(control) = vault.pending_control.clone() {
            let mut state = vault
                .participant
                .as_ref()
                .map(|a| state_from_participant(&vault, a))
                .unwrap_or_default();
            state.enabled = false;
            state.enrollment_status = if control.kind == ControlKind::Deletion {
                "deletion_pending"
            } else {
                "enrolled"
            }
            .into();
            state.pending_control_action = Some(control);
            Some(state)
        } else if vault.receipt.is_some() {
            let mut state = FeedbackProgramState::default();
            state.installation_id = Some(vault.installation_id.clone());
            state.credential_ref = Some(CREDENTIAL_REF.into());
            state.deletion_code_ref = Some(CREDENTIAL_REF.into());
            state.enrollment_status = "deleted".into();
            Some(state)
        } else if vault.enrollment.is_some() && vault.participant.is_none() {
            let mut state = FeedbackProgramState::default();
            state.enrollment_status = "enrolling".into();
            state.installation_id = Some(vault.installation_id.clone());
            state.credential_ref = Some(CREDENTIAL_REF.into());
            state.deletion_code_ref = Some(CREDENTIAL_REF.into());
            Some(state)
        } else if !authorized && p.pending_control_action.is_none() {
            vault.participant.as_ref().map(|a| {
                // A recovered/reset state is never consent. Preserve ownership for explicit controls.
                let mut state = state_from_participant(&vault, a);
                state.enabled = false;
                state.enrollment_status = "withdrawn".into();
                state
            })
        } else {
            None
        };
        local.vault = Some(vault);
        self.collecting.store(usable, Ordering::SeqCst);
        drop(local);
        if let Some(recovered) = recovery {
            let _ = context
                .update_state(|s| s.feedback_program = recovered)
                .await;
        }
    }
    pub fn start(app: AppHandle, context: AppContext) {
        #[cfg(target_os = "android")]
        android::install(context.clone());
        // Watch actual decoded-output/submitted telemetry. Connected/input counters are insufficient.
        let moonlight = app.state::<MoonlightManager>();
        let mut stats = moonlight.runtime.subscribe_statistics();
        let active = moonlight.active_stream_instance_id.clone();
        let frame_context = context.clone();
        tauri::async_runtime::spawn(async move {
            while stats.changed().await.is_ok() {
                if !frame_context
                    .feedback
                    .can_collect(&frame_context.load_state().await.feedback_program)
                {
                    continue;
                }
                let s = stats.borrow().clone();
                if s.state == "idle" || s.session_generation == 0 {
                    continue;
                }
                let Some(host) = s.feedback_host_id.as_ref() else {
                    continue;
                };
                let state = frame_context.load_state().await;
                let id = state
                    .provisioned_servers
                    .iter()
                    .find(|r| &r.embedded_moonlight_host_id == host)
                    .map(|r| r.instance_id);
                let Some(id) = id else {
                    continue;
                };
                if active.lock().ok().and_then(|id| *id) != Some(id) {
                    continue;
                }
                if cfg!(target_os = "android")
                    && s.sampled_at.elapsed() <= Duration::from_secs(3)
                    && s.renderer_decoded_output_frame_count > 0
                {
                    frame_context
                        .feedback
                        .milestone(&frame_context, id, Milestone::FirstFrame)
                        .await;
                }
            }
        });
        tauri::async_runtime::spawn(async move {
            context.feedback.initialize(&context).await;
            let mut tick = tokio::time::interval(Duration::from_secs(30));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! { _ = tick.tick() => {}, _ = context.feedback.wake.notified() => {} }
                context.feedback.checkpoint(&context, false).await;
                context.feedback.pump(&context).await;
            }
        });
    }
    pub async fn status(&self, context: &AppContext) -> FeedbackStatus {
        self.initialize(context).await;
        let mut p = context.load_state().await.feedback_program;
        let capability = secure::available().await;
        let supported = cfg!(all(target_os = "android", noland_feedback_native_android))
            && capability.is_ok()
            && self.lifecycle_ready.load(Ordering::SeqCst);
        if !supported && self.lifecycle_ready.load(Ordering::SeqCst) {
            self.stop_local();
        }
        p.enabled = supported && self.permitted(&p);
        let local = self.inner.lock().await;
        let message = if self.api_url.is_none() {
            Some("Feedback is unconfigured: NOLAND_FEEDBACK_API_URL is absent or is not a public HTTPS base URL".into())
        } else if let Err(e) = capability {
            Some(e)
        } else if p
            .pending_control_action
            .as_ref()
            .is_some_and(|c| c.preserve_sessions)
        {
            Some("Scope synchronization pending; basic observations remain local and uploads are paused".into())
        } else if p.pending_control_action.is_some() {
            Some(
                "Local collection is stopped; a durable account control is awaiting acknowledgment"
                    .into(),
            )
        } else {
            local.message.clone().or_else(|| local.journal.as_ref().filter(|j| j.dropped_sessions > 0).map(|_| "Some old feedback was evicted at the local storage/age limit; no terminal outcome was invented".into()))
        };
        let receipt = local.vault.as_ref().and_then(|v| v.receipt.as_ref());
        FeedbackStatus {
            program: p,
            configured: self.api_url.is_some(),
            supported,
            status_message: message,
            pending_sessions: local.journal.as_ref().map_or(0, |j| {
                j.sessions
                    .values()
                    .filter(|r| {
                        r.accepted_revision < r.snapshot.revision
                            || r.review
                                .as_ref()
                                .is_some_and(|v| v.revision > r.accepted_review_revision)
                    })
                    .count()
            }),
            jitter_capability:
                "unsupported: operational agent has no received-sample watermark/full-window evidence; N/A",
            frame_evidence: "android_mediacodec_decoded_output_submitted_to_surface_not_scanout",
            receipt_id: receipt.map(|r| r.receipt_id.clone()),
            deletion_status: receipt.map(|r| r.status.clone()),
        }
    }
    pub async fn notices(&self) -> Result<ProgramDocument, String> {
        if Instant::now() < self.inner.lock().await.retry_at {
            return Err("Feedback API retry backoff is active".into());
        }
        let doc: ProgramDocument = match self
            .request(Method::GET, "/program", None, None, None, None)
            .await
        {
            Ok(doc) => doc,
            Err(error) => {
                let message = error.message.clone();
                self.failure(error).await;
                return Err(message);
            }
        };
        if doc.schema_version != 1 || doc.privacy_version.is_empty() || doc.terms_version.is_empty()
        {
            return Err("Unsupported feedback notices".into());
        }
        if doc.collection_paused || !doc.available {
            self.stop_local();
        }
        let mut local = self.inner.lock().await;
        let incompatible = local
            .vault
            .as_ref()
            .and_then(|v| v.participant.as_ref())
            .is_some_and(|p| {
                p.privacy_version != doc.privacy_version || p.terms_version != doc.terms_version
            });
        if incompatible {
            self.stop_local();
        }
        if let Some(vault) = local.vault.as_mut() {
            vault.collection_paused = doc.collection_paused || !doc.available || incompatible;
            secure::write(vault).await?;
        }
        local.notices = Some(doc.clone());
        Ok(doc)
    }
    pub async fn enroll(
        &self,
        context: &AppContext,
        mut request: EnrollmentRequest,
    ) -> Result<FeedbackStatus, String> {
        let stop_generation = self.stop_generation.load(Ordering::SeqCst);
        self.initialize(context).await;
        let _controls = self.controls.lock().await;
        if !cfg!(all(target_os = "android", noland_feedback_native_android))
            || !self.lifecycle_ready.load(Ordering::SeqCst)
        {
            return Err("The full Android native lifecycle/frame adapter is not ready".into());
        }
        if !request.participation_accepted {
            return Err("Explicit participation consent is required".into());
        }
        if !request.optional_context_enabled {
            request.optional_profile = Default::default();
        }
        request.optional_profile.normalize()?;
        secure::available().await?;
        let _network = self.network.lock().await;
        let p = context.load_state().await.feedback_program;
        if p.pending_control_action.is_some() {
            return Err("Resolve the pending withdrawal/deletion before rejoining".into());
        }
        let notices = self.notices().await?;
        if !notices.available
            || notices.collection_paused
            || notices.privacy_version != request.privacy_version
            || notices.terms_version != request.terms_version
        {
            return Err(
                "Read and accept the currently available feedback policies before joining".into(),
            );
        }
        let mut vault = secure::read().await?.unwrap_or_else(|| Vault {
            schema_version: 1,
            api_url: self.api_url.clone(),
            installation_id: uuid::Uuid::new_v4().to_string(),
            enrollment: None,
            participant: None,
            receipt: None,
            pending_control: None,
            control_blocked: false,
            collection_paused: false,
        });
        if vault.api_url != self.api_url {
            return Err("Protected credentials belong to a different feedback service; enrollment is unavailable".into());
        }
        if vault.pending_control.is_some() {
            return Err("Resolve the durable account control before enrolling".into());
        }
        if vault.receipt.is_some() {
            return Err("This installation has requested deletion; retain its receipt rather than reenrolling".into());
        }
        if let Some(existing) = &vault.participant {
            if matches_participant(&p, &vault, existing) {
                return Ok(self.status(context).await);
            }
            if !existing.enabled || p.enrollment_status == "withdrawn" {
                return Err("Use consent settings to rejoin the existing installation".into());
            }
            if existing.privacy_version != request.privacy_version
                || existing.terms_version != request.terms_version
            {
                return Err(
                    "Updated policy consent requires the server's supported policy migration flow"
                        .into(),
                );
            }
        } else {
            if let Some(attempt) = &vault.enrollment {
                if chrono::Utc::now()
                    .timestamp()
                    .saturating_sub(attempt.created_at)
                    >= 24 * 60 * 60
                {
                    return Err("Enrollment recovery expired; no new enrollment was created. Use the published ownership-recovery contact".into());
                }
                if attempt.request != request {
                    return Err(
                        "Retry the original pending enrollment before changing its consent choices"
                            .into(),
                    );
                }
            } else {
                vault.enrollment = Some(EnrollmentAttempt {
                    request_id: uuid::Uuid::new_v4().to_string(),
                    recovery_proof: URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>()),
                    created_at: chrono::Utc::now().timestamp(),
                    request: request.clone(),
                });
            }
            // Proof and request ID are encrypted and durable BEFORE the first enrollment request.
            secure::write(&vault).await?;
            context
                .update_state(|state| {
                    state.feedback_program.enrollment_status = "enrolling".into();
                    state.feedback_program.installation_id = Some(vault.installation_id.clone());
                    state.feedback_program.credential_ref = Some(CREDENTIAL_REF.into());
                    state.feedback_program.deletion_code_ref = Some(CREDENTIAL_REF.into());
                    state.feedback_program.enabled = false;
                })
                .await
                .map_err(|_| "Could not persist enrollment state")?;
            let attempt = vault
                .enrollment
                .as_ref()
                .ok_or("Enrollment proof is missing")?;
            let payload = json!({"installationId":vault.installation_id, "requestId":attempt.request_id,
                "recoveryProof":attempt.recovery_proof, "privacyVersion":request.privacy_version,
                "termsVersion":request.terms_version, "participationAccepted":true,
                "optionalContextEnabled":request.optional_context_enabled, "optionalProfile":request.optional_profile});
            let enrollment_result = self
                .request::<Participant>(
                    Method::POST,
                    "/enrollments",
                    Some(payload),
                    None,
                    None,
                    None,
                )
                .await;
            let mut participant = match enrollment_result {
                Ok(participant) => participant,
                Err(error) => {
                    let message = error.message.clone();
                    self.failure(error).await;
                    return Err(message);
                }
            };
            if !valid_participant(&participant)
                || participant.privacy_version != request.privacy_version
                || participant.terms_version != request.terms_version
            {
                return Err(
                    "Feedback enrollment returned invalid credentials or policy versions".into(),
                );
            }
            participant.enabled = true;
            participant.optional_context_enabled = request.optional_context_enabled;
            vault.participant = Some(participant);
            vault.enrollment = None;
            secure::write(&vault).await?;
        }
        let participant = vault
            .participant
            .as_ref()
            .ok_or("Enrollment credentials are missing")?
            .clone();
        let journal = Journal::empty(vault.installation_id.clone());
        if context
            .load_state()
            .await
            .feedback_program
            .pending_control_action
            .is_none()
        {
            self.outbox.save(&journal)?;
        }
        let accepted_state = context
            .update_state(|state| {
                let pending = state.feedback_program.pending_control_action.clone();
                state.feedback_program = state_from_participant(&vault, &participant);
                if let Some(mut control) = pending {
                    if control.expected_epoch == 0 {
                        control.expected_epoch = participant.consent_epoch;
                    }
                    state.feedback_program.enabled = false;
                    if control.kind == ControlKind::Deletion {
                        state.feedback_program.enrollment_status = "deletion_pending".into();
                    }
                    state.feedback_program.pending_control_action = Some(control);
                }
            })
            .await
            .map_err(|_| "Could not persist accepted feedback enrollment")?;
        vault.pending_control = accepted_state
            .feedback_program
            .pending_control_action
            .clone();
        secure::write(&vault).await?;
        {
            let mut local = self.inner.lock().await;
            local.vault = Some(vault);
            local.journal = accepted_state
                .feedback_program
                .pending_control_action
                .is_none()
                .then_some(journal);
            local.metrics.reset();
            local.message = None;
        }
        if self.stop_generation.load(Ordering::SeqCst) == stop_generation {
            self.desired_disabled.store(false, Ordering::SeqCst);
        }
        self.collecting.store(true, Ordering::SeqCst);
        drop(_network);
        Ok(self.status(context).await)
    }
    pub async fn consent(
        &self,
        context: &AppContext,
        enabled: bool,
        optional: bool,
    ) -> Result<FeedbackStatus, String> {
        if !enabled {
            return self.queue_stop(context, false).await;
        }
        let stop_generation = self.stop_generation.load(Ordering::SeqCst);
        self.initialize(context).await;
        // Invalidate in-flight delivery before waiting for disk/network locks.
        let was_active = self.collecting.load(Ordering::SeqCst);
        self.stop_local();
        let _controls = self.controls.lock().await;
        self.stop_local();
        let p = context.load_state().await.feedback_program;
        if p.pending_control_action.is_some() {
            self.retry_control().await?;
            self.wake.notify_one();
            return Ok(self.status(context).await);
        }
        if p.user_id.is_none() {
            return Err("Feedback is not enrolled".into());
        }
        if enabled {
            secure::available().await?;
        }
        let preserve = enabled && p.enabled && was_active;
        if enabled {
            let notices = self.notices().await?;
            if !notices.available
                || notices.collection_paused
                || p.privacy_version.as_ref() != Some(&notices.privacy_version)
                || p.terms_version.as_ref() != Some(&notices.terms_version)
            {
                return Err("Current policies/availability do not authorize rejoining".into());
            }
        }
        let control = PendingControl {
            kind: if enabled {
                ControlKind::Consent
            } else {
                ControlKind::Withdrawal
            },
            request_id: uuid::Uuid::new_v4().to_string(),
            expected_epoch: p.consent_epoch,
            enabled,
            optional_context_enabled: optional && enabled,
            preserve_sessions: preserve,
            previous: None,
        };
        self.persist_control(&control).await?;
        if enabled && self.stop_generation.load(Ordering::SeqCst) == stop_generation {
            self.desired_disabled.store(false, Ordering::SeqCst);
        }
        context.update_state(|s| {
            if s.feedback_program.pending_control_action.is_some() { return; }
            s.feedback_program.enabled = false;
            s.feedback_program.optional_context_enabled = optional && enabled;
            s.feedback_program.pending_control_action = Some(control);
        }).await.map_err(|_| "Local feedback is stopped, but the account control could not be persisted; retry it")?;
        {
            let mut local = self.inner.lock().await;
            local.metrics.reset();
            if preserve {
                if let Some(journal) = local.journal.as_mut() {
                    for rental in journal.sessions.values_mut() {
                        rental.gap();
                    }
                    self.outbox.save(journal)?;
                }
            }
            if !preserve {
                local.journal = None;
                let _ = self.outbox.clear();
            }
            // The allowlisted rental context is required setup context; no optional profile is in snapshots.
            if !optional {
                if let Some(v) = local.vault.as_mut() {
                    v.enrollment = None;
                    secure::write(v).await?;
                }
            }
            local.next_delivery = Instant::now();
        }
        // Scope changes keep basic observations local, never send under an unacknowledged epoch.
        self.collecting.store(preserve, Ordering::SeqCst);
        self.wake.notify_one();
        Ok(self.status(context).await)
    }
    pub async fn delete(&self, context: &AppContext) -> Result<FeedbackStatus, String> {
        self.queue_stop(context, true).await
    }
    async fn queue_stop(
        &self,
        context: &AppContext,
        deletion: bool,
    ) -> Result<FeedbackStatus, String> {
        self.stop_generation.fetch_add(1, Ordering::SeqCst);
        self.desired_disabled.store(true, Ordering::SeqCst);
        self.stop_local();
        self.initialize(context).await;
        let p = context.load_state().await.feedback_program;
        if p.enrollment_status == "deleted" {
            self.retry_control().await?;
            self.wake.notify_one();
            return Ok(self.status(context).await);
        }
        if p.installation_id.is_none() {
            return Err("No protected feedback enrollment is available".into());
        }
        let kind = if deletion {
            ControlKind::Deletion
        } else {
            ControlKind::Withdrawal
        };
        // No account/network mutex here: durable withdrawal must not wait for a slow in-flight API.
        let updated = context.update_state(|s| {
            let p = &mut s.feedback_program;
            p.enabled = false;
            p.optional_context_enabled = false;
            if p.pending_control_action.as_ref().is_some_and(|c| c.kind == kind || c.kind == ControlKind::Deletion) { return; }
            let previous = if !deletion { p.pending_control_action.take().map(Box::new) } else { None };
            p.pending_control_action = Some(PendingControl { kind, request_id: uuid::Uuid::new_v4().to_string(), expected_epoch: p.consent_epoch,
                enabled: false, optional_context_enabled: false, preserve_sessions: false, previous });
            if deletion { p.enrollment_status = "deletion_pending".into(); }
        }).await.map_err(|_| "Local collection is stopped, but the durable account intent could not be saved; retry it")?;
        {
            let mut local = self.inner.lock().await;
            local.journal = None;
            local.metrics.reset();
            if let Err(e) = self.outbox.clear() {
                local.message = Some(e);
            }
            local.next_delivery = Instant::now().max(local.retry_at);
        }
        if let Some(control) = &updated.feedback_program.pending_control_action {
            self.persist_control(control).await?;
        }
        self.wake.notify_one();
        Ok(self.status(context).await)
    }

    async fn persist_control(&self, control: &PendingControl) -> Result<(), String> {
        let mut local = self.inner.lock().await;
        let mut vault = secure::read().await?.ok_or(
            "Protected ownership credentials are missing; account control was not discarded",
        )?;
        if vault.participant.is_none()
            && !(matches!(
                control.kind,
                ControlKind::Deletion | ControlKind::Withdrawal
            ) && vault.enrollment.is_some())
        {
            return Err("No participant credentials exist; retry pending enrollment first".into());
        }
        vault.pending_control = Some(control.clone());
        vault.control_blocked = false;
        // Pending enrollment proofs must survive withdrawal/deletion until ownership is recovered.
        if vault.participant.is_some() {
            vault.enrollment = None;
        }
        secure::write(&vault).await?;
        local.vault = Some(vault);
        Ok(())
    }
    async fn retry_control(&self) -> Result<(), String> {
        let mut local = self.inner.lock().await;
        if let Some(vault) = local.vault.as_mut() {
            vault.control_blocked = false;
            secure::write(vault).await?;
        }
        local.next_delivery = Instant::now().max(local.retry_at);
        Ok(())
    }
    pub async fn begin_rental(
        &self,
        context: &AppContext,
        offer: &OfferCandidate,
    ) -> Option<String> {
        let p = context.load_state().await.feedback_program;
        if !self.can_collect(&p) || !valid_provider_id(&offer.id.to_string()) {
            return None;
        }
        let hardware = HardwareSnapshot {
            gpu_name: bounded_label(&offer.gpu_name, 128),
            rented_gpu_count: (offer.gpu_count > 0).then_some(offer.gpu_count),
            country: country_code(&offer.country),
            region: bounded_label(&offer.region, 80),
        };
        let rental = Rental::pending(
            p.consent_epoch,
            offer.machine_id.clone().filter(|s| valid_provider_id(s)),
            offer.id.to_string(),
            hardware,
        );
        let id = rental.id.clone();
        let mut local = self.inner.lock().await;
        if !self.owns_journal(&p, &local) {
            return None;
        }
        let mut next = local.journal.clone()?;
        next.sessions.insert(id.clone(), rental);
        next.prune(chrono::Utc::now().timestamp());
        if !next.sessions.contains_key(&id) {
            return None;
        }
        if let Err(e) = self.outbox.save(&next) {
            local.message = Some(e);
            return None;
        }
        local.journal = Some(next);
        Some(id)
    }
    pub async fn confirmed_rental(
        &self,
        context: &AppContext,
        attempt: &str,
        instance_id: u64,
        complete: bool,
    ) {
        let p = context.load_state().await.feedback_program;
        if !self.can_collect(&p) || !valid_provider_id(&instance_id.to_string()) {
            return;
        }
        let session = {
            let mut local = self.inner.lock().await;
            if !self.owns_journal(&p, &local) {
                return;
            }
            let Some(mut next) = local.journal.clone() else {
                return;
            };
            if next.rental(&instance_id.to_string()).is_some() {
                return;
            }
            let Some(rental) = next.sessions.get_mut(attempt) else {
                return;
            };
            if rental.snapshot.consent_epoch != p.consent_epoch {
                return;
            }
            rental.confirm(instance_id.to_string(), complete);
            let machine = rental.snapshot.provider_machine_id.clone();
            if let Err(e) = self.outbox.save(&next) {
                local.message = Some(e);
                return;
            }
            local.journal = Some(next);
            (attempt.to_string(), machine)
        };
        // These are references, not credentials. The spool remains authoritative after local removal.
        let _ = context
            .update_state(|s| {
                if s.instance.instance_id == Some(instance_id) {
                    s.instance.machine_id = (!session.1.is_empty()).then(|| session.1.clone());
                    s.instance.feedback_session_id = Some(session.0.clone());
                    s.instance.feedback_journal_ref =
                        Some(format!("feedback-v1/outbox.json#{}", session.0));
                }
                let record = match s
                    .provisioned_servers
                    .iter_mut()
                    .find(|r| r.instance_id == instance_id)
                {
                    Some(r) => r,
                    None => {
                        s.provisioned_servers.push(
                            crate::models::app_state::ProvisionedServerState::new(instance_id),
                        );
                        s.provisioned_servers.last_mut().unwrap()
                    }
                };
                record.machine_id = (!session.1.is_empty()).then(|| session.1.clone());
                record.feedback_session_id = Some(session.0.clone());
                record.feedback_journal_ref =
                    Some(format!("feedback-v1/outbox.json#{}", session.0));
            })
            .await;
        self.wake.notify_one();
    }
    pub async fn has_pending_creation(&self) -> bool {
        self.inner.lock().await.journal.as_ref().is_some_and(|j| {
            j.sessions
                .values()
                .any(|r| !r.creation_reconciled && r.snapshot.lifecycle.destroyed_at.is_none())
        })
    }
    pub async fn attached_rental(&self, context: &AppContext, instance_id: u64) {
        let p = context.load_state().await.feedback_program;
        let mut local = self.inner.lock().await;
        if !self.owns_journal(&p, &local) {
            return;
        }
        let Some(mut next) = local.journal.clone() else {
            return;
        };
        let Some(r) = next.rental_mut(instance_id) else {
            return;
        };
        if r.creation_reconciled {
            return;
        }
        r.creation_reconciled = true;
        if let Err(e) = self.outbox.save(&next) {
            local.message = Some(e);
        } else {
            local.journal = Some(next);
        }
    }
    /// Only exact native-generated provider labels recover an uncertain create. No offer/host guessing.
    pub async fn recover_created(
        &self,
        context: &AppContext,
        instances: &[VastInstance],
    ) -> Option<u64> {
        let p = context.load_state().await.feedback_program;
        if !self.can_collect(&p) {
            return None;
        }
        let attempts: Vec<String> = self
            .inner
            .lock()
            .await
            .journal
            .as_ref()?
            .sessions
            .values()
            .filter(|r| !r.creation_reconciled && r.snapshot.lifecycle.destroyed_at.is_none())
            .map(|r| r.id.clone())
            .collect();
        for attempt in attempts {
            let label = format!("Noland Connect Session {attempt}");
            let matches: Vec<_> = instances.iter().filter(|i| i.label == label).collect();
            if matches.len() == 1 {
                self.confirmed_rental(context, &attempt, matches[0].id, false)
                    .await;
                return Some(matches[0].id);
            }
        }
        None
    }
    pub async fn observe_state(&self, _before: &PersistedAppState, after: &PersistedAppState) {
        if !self.collection_consent(&after.feedback_program) {
            return;
        }
        let mut local = self.inner.lock().await;
        if !self.owns_journal(&after.feedback_program, &local) {
            return;
        }
        let Some(mut next) = local.journal.clone() else {
            return;
        };
        let mut changed = false;
        for record in &after.provisioned_servers {
            let Some(r) = next.rental_mut(record.instance_id) else {
                continue;
            };
            if let Some(machine) = record
                .machine_id
                .as_ref()
                .filter(|id| valid_provider_id(id))
            {
                if r.snapshot.provider_machine_id.is_empty() {
                    r.snapshot.provider_machine_id = machine.clone();
                    r.snapshot.revision += 1;
                    changed = true;
                } else if &r.snapshot.provider_machine_id != machine {
                    r.blocked = true;
                    changed = true;
                }
            }
            let ready = matches!(record.last_state, OrchestrationState::Ready);

            let paired = record.embedded_moonlight_paired || record.steps.pairing_completed;
            if ready {
                changed |= r.mark(Milestone::Ready);
            }
            if paired {
                changed |= r.mark(Milestone::Paired);
            }
        }
        if let Some(id) = after.post_wireguard_setup.current_instance_id {
            if let Some(r) = next.rental_mut(id) {
                if after.post_wireguard_setup.setup_complete {
                    changed |= r.mark(Milestone::Ready);
                }
                if after.post_wireguard_setup.paired {
                    changed |= r.mark(Milestone::Paired);
                }
            }
        }
        if changed {
            if let Err(e) = self.outbox.save(&next) {
                local.message = Some(e);
                self.stop_local();
            } else {
                local.journal = Some(next);
                self.wake.notify_one();
            }
        }
    }
    pub async fn milestone(&self, context: &AppContext, id: u64, event: Milestone) {
        let p = context.load_state().await.feedback_program;
        if !self.collection_consent(&p) {
            return;
        }
        let mut local = self.inner.lock().await;
        let Some(mut next) = local.journal.clone() else {
            return;
        };
        if !self.owns_journal(&p, &local) {
            return;
        }
        if !next.rental_mut(id).is_some_and(|r| r.mark(event)) {
            return;
        }
        if let Err(e) = self.outbox.save(&next) {
            local.message = Some(e);
            self.stop_local();
        } else {
            local.journal = Some(next);
            self.wake.notify_one();
        }
    }
    pub async fn unknown_missing(&self, context: &AppContext, ids: &[u64]) {
        let p = context.load_state().await.feedback_program;
        if !self.permitted(&p) {
            return;
        }
        let mut local = self.inner.lock().await;
        let Some(mut next) = local.journal.clone() else {
            return;
        };
        let mut changed = false;
        for id in ids {
            if let Some(r) = next.rental_mut(*id) {
                changed |= r.gap();
            }
        }
        if changed {
            if let Err(e) = self.outbox.save(&next) {
                local.message = Some(e);
                self.stop_local();
            } else {
                local.journal = Some(next);
            }
        }
    }
    pub async fn state_reloaded(&self, context: &AppContext) {
        self.stop_local();
        {
            let mut local = self.inner.lock().await;
            local.initialized = false;
            local.journal = None;
            local.metrics.reset();
            local.vault = None;
        }
        self.initialize(context).await;
    }
    pub async fn observe_frame(
        &self,
        context: &AppContext,
        s: &crate::moonlight::runtime::RuntimeStatistics,
    ) {
        let p = context.load_state().await.feedback_program;
        if !self.can_collect(&p)
            || !cfg!(target_os = "android")
            || s.session_generation == 0
            || s.sampled_at.elapsed() > Duration::from_secs(3)
            || s.renderer_decoded_output_frame_count == 0
        {
            return;
        }
        let Some(host) = &s.feedback_host_id else {
            return;
        };
        let state = context.load_state().await;
        if let Some(record) = state
            .provisioned_servers
            .iter()
            .find(|r| &r.embedded_moonlight_host_id == host)
        {
            self.milestone(context, record.instance_id, Milestone::FirstFrame)
                .await;
        }
    }
    pub async fn checkpoint(&self, context: &AppContext, suspension: bool) {
        let p = context.load_state().await.feedback_program;
        // Suspension first sets foreground=false; still checkpoint already consented observations.
        if !self.collection_consent(&p) {
            return;
        }
        let mut local = self.inner.lock().await;
        if suspension {
            local.metrics.reset();
            if let Some(j) = local.journal.as_mut() {
                for r in j.sessions.values_mut() {
                    r.gap();
                }
            }
            local.dirty = true;
        }
        if !self.owns_journal(&p, &local) {
            return;
        }
        if let Some(j) = local.journal.as_mut() {
            let dropped = j.dropped_sessions;
            j.prune(chrono::Utc::now().timestamp());
            if j.dropped_sessions != dropped {
                local.dirty = true;
            }
        }
        if !local.dirty {
            return;
        }
        if let Some(j) = local.journal.as_ref() {
            if let Err(e) = self.outbox.save(j) {
                local.message = Some(e);
                self.stop_local();
                return;
            }
        }
        local.dirty = false;
    }
    pub async fn observe_jitter(
        &self,
        context: &AppContext,
        id: u64,
        segment: &str,
        sequence: u64,
        measured: Instant,
        received: u64,
        jitter: f64,
        route: &str,
    ) {
        let p = context.load_state().await.feedback_program;
        // The operational Android source does not echo the processed received-sample watermark.
        // Do not promote a client send timestamp, heartbeat, or stream RTT variance to this metric.
        const CANONICAL_JITTER_SUPPORTED: bool = false;
        if !CANONICAL_JITTER_SUPPORTED || !self.permitted(&p) {
            return;
        }
        let mut local = self.inner.lock().await;
        if !self.owns_journal(&p, &local) {
            return;
        }
        let Local {
            journal,
            metrics,
            dirty,
            ..
        } = &mut *local;
        if let Some(r) = journal.as_mut().and_then(|j| j.rental_mut(id)) {
            if metrics.observe(
                r,
                segment,
                sequence,
                measured,
                received,
                jitter,
                route,
                Instant::now(),
            ) {
                *dirty = true;
            }
        }
    }
    pub async fn review_eligible(&self, context: &AppContext, id: &str) -> bool {
        let p = context.load_state().await.feedback_program;
        valid_provider_id(id)
            && self.permitted(&p)
            && self
                .inner
                .lock()
                .await
                .journal
                .as_ref()
                .and_then(|j| j.rental(id))
                .is_some_and(Rental::review_eligible)
    }
    pub async fn stage_review(
        &self,
        context: &AppContext,
        id: &str,
        score: Option<u8>,
        comment: Option<String>,
    ) -> Result<(), String> {
        if !self.review_eligible(context, id).await {
            return Err("This rental has no eligible native first-frame evidence".into());
        }
        let mut local = self.inner.lock().await;
        let mut next = local
            .journal
            .clone()
            .ok_or("Feedback journal unavailable")?;
        let r = next
            .sessions
            .values_mut()
            .find(|r| r.snapshot.provider_instance_id == id)
            .ok_or("Untracked rental")?;
        if !r.review_eligible() {
            return Err("Rental already ended or is not review eligible".into());
        }
        if !self.permitted(&context.load_state().await.feedback_program) {
            return Err("Feedback permission changed before draft persistence".into());
        }
        let revision = r
            .review_revision
            .max(r.review.as_ref().map_or(0, |review| review.revision))
            .max(r.accepted_review_revision)
            .checked_add(1)
            .ok_or("Review revision exhausted")?;
        // Only active, frame-eligible rentals reach here. No review can have been
        // published yet: the sender requires confirmed destruction first.
        // Persist a local tombstone, never an empty ReviewInput/PUT.
        r.review = if score.is_none() && comment.is_none() {
            None
        } else {
            Some(ReviewInput::new(revision, score, comment)?)
        };
        r.review_revision = revision;
        r.review_blocked = false;
        // No HTTP here. Destroy/backup never waits for a review upload.
        self.outbox.save(&next)?;
        local.journal = Some(next);
        Ok(())
    }

    pub async fn read_api<T: DeserializeOwned>(
        &self,
        context: &AppContext,
        method: Method,
        path: &str,
        payload: Option<Value>,
    ) -> Result<T, String> {
        let _network = self.network.lock().await;
        if Instant::now() < self.inner.lock().await.retry_at {
            return Err("Feedback delivery is backing off; rental/search remains available".into());
        }
        let p = context.load_state().await.feedback_program;
        let token = self.token(context, &p).await?;
        let result = self
            .request(
                method,
                path,
                payload,
                Some(&token),
                Some((context, p.consent_epoch)),
                None,
            )
            .await;
        match result {
            Ok(value) => Ok(value),
            Err(error) => {
                let message = error.message.clone();
                self.failure(error).await;
                Err(message)
            }
        }
    }
    async fn token(
        &self,
        context: &AppContext,
        p: &FeedbackProgramState,
    ) -> Result<String, String> {
        if !self.permitted(p) {
            return Err(
                "Feedback is disabled, unavailable, or awaiting consent synchronization".into(),
            );
        }
        let vault = secure::read()
            .await
            .map_err(|e| {
                self.stop_local();
                e
            })?
            .ok_or_else(|| {
                self.stop_local();
                "Protected feedback credentials are missing"
            })?;
        if vault.api_url != self.api_url {
            self.stop_local();
            return Err("Feedback credential service binding changed".into());
        }
        let v = vault
            .participant
            .as_ref()
            .filter(|v| {
                !vault.collection_paused
                    && vault.pending_control.is_none()
                    && matches_participant(p, &vault, v)
            })
            .ok_or_else(|| {
                self.stop_local();
                "Feedback credentials do not match current consent"
            })?;
        let current = context.load_state().await.feedback_program;
        if !self.permitted(&current) || current.consent_epoch != p.consent_epoch {
            return Err("Feedback permission changed".into());
        }
        Ok(v.credential.clone())
    }
    async fn pump(&self, context: &AppContext) {
        if !self.foreground.load(Ordering::SeqCst) || self.api_url.is_none() {
            return;
        }
        let Ok(_controls) = self.controls.try_lock() else {
            return;
        };
        let Ok(_network) = self.network.try_lock() else {
            return;
        };
        if Instant::now() < self.inner.lock().await.next_delivery {
            return;
        }
        let p = context.load_state().await.feedback_program;
        if p.enrollment_status == "deleted" {
            self.poll_deletion().await;
            return;
        }
        if let Some(control) = p.pending_control_action.clone() {
            if self
                .inner
                .lock()
                .await
                .vault
                .as_ref()
                .is_some_and(|v| v.control_blocked)
            {
                return;
            }
            let issued = control.previous.as_deref().unwrap_or(&control);
            match self.sync_control(context, &p, issued).await {
                Ok(()) => {
                    let mut l = self.inner.lock().await;
                    l.failures = 0;
                    l.next_delivery = Instant::now();
                    l.message = None;
                }
                Err(e) => self.failure(e).await,
            }
            return;
        }
        if !self.permitted(&p) {
            return;
        }
        // Only enabled participants make automatic program requests. Outages leave local consented accumulation intact.
        if let Err(message) = self.notices().await {
            self.failure(ApiFailure::local(&message)).await;
            return;
        }
        if !self.permitted(&context.load_state().await.feedback_program) {
            return;
        }
        let token = match self.token(context, &p).await {
            Ok(t) => t,
            Err(e) => {
                self.inner.lock().await.message = Some(e);
                return;
            }
        };
        // Never transmit a higher in-memory revision before its cumulative snapshot is durable.
        self.checkpoint(context, false).await;
        if !self.permitted(&context.load_state().await.feedback_program) {
            return;
        }
        let rentals = self
            .inner
            .lock()
            .await
            .journal
            .as_ref()
            .map(|j| j.sessions.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for rental in rentals {
            if !rental.deliverable(p.consent_epoch) {
                continue;
            }
            let mut session_accepted = rental.accepted_revision >= rental.snapshot.revision;
            if !session_accepted {
                let payload = match serde_json::to_value(&rental.snapshot) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                let result: Result<Value, _> = self
                    .request(
                        Method::PUT,
                        &format!("/sessions/{}", rental.id),
                        Some(payload),
                        Some(&token),
                        Some((context, p.consent_epoch)),
                        None,
                    )
                    .await;
                match result {
                    Ok(ack) if ack["revision"].as_u64() == Some(rental.snapshot.revision) => {
                        self.ack(&rental.id, rental.snapshot.revision, false, false)
                            .await;
                        session_accepted = true;
                    }
                    Ok(_) => {
                        self.failure(ApiFailure::local(
                            "Feedback server did not acknowledge the snapshot revision",
                        ))
                        .await;
                        return;
                    }
                    Err(e) => {
                        if !e.retryable && !e.unauthorized {
                            self.ack(&rental.id, 0, false, true).await;
                        }
                        self.failure(e).await;
                        return;
                    }
                }
            }
            if session_accepted
                && rental.snapshot.lifecycle.destroyed_at.is_some()
                && !rental.review_blocked
            {
                if let Some(review) = rental
                    .review
                    .filter(|r| r.revision > rental.accepted_review_revision)
                {
                    let result: Result<Value, _> = self
                        .request(
                            Method::PUT,
                            &format!("/sessions/{}/review", rental.id),
                            serde_json::to_value(&review).ok(),
                            Some(&token),
                            Some((context, p.consent_epoch)),
                            None,
                        )
                        .await;
                    match result {
                        Ok(ack)
                            if ack
                                .get("reviewRevision")
                                .or_else(|| ack.get("revision"))
                                .and_then(Value::as_u64)
                                == Some(review.revision) =>
                        {
                            self.ack(&rental.id, review.revision, true, false).await
                        }
                        Ok(_) => {
                            self.failure(ApiFailure::local(
                                "Feedback server did not acknowledge the review revision",
                            ))
                            .await;
                            return;
                        }
                        Err(e) => {
                            if !e.retryable && !e.unauthorized {
                                self.ack(&rental.id, 0, true, true).await;
                            }
                            self.failure(e).await;
                            return;
                        }
                    }
                }
            }
        }
        let mut local = self.inner.lock().await;
        local.failures = 0;
        local.next_delivery = Instant::now() + Duration::from_secs(60);
    }
    async fn ack(&self, id: &str, revision: u64, review: bool, blocked: bool) {
        let mut local = self.inner.lock().await;
        let Some(mut next) = local.journal.clone() else {
            return;
        };
        if let Some(r) = next.sessions.get_mut(id) {
            if blocked {
                if review {
                    r.review_blocked = true;
                } else {
                    r.blocked = true;
                }
            } else if review {
                r.accepted_review_revision = r.accepted_review_revision.max(revision);
            } else {
                r.accepted_revision = r.accepted_revision.max(revision);
            }
        }
        if let Err(e) = self.outbox.save(&next) {
            local.message = Some(e);
            self.stop_local();
        } else {
            local.journal = Some(next);
        }
    }
    async fn failure(&self, error: ApiFailure) {
        if error.unauthorized {
            self.stop_local();
        }
        let mut local = self.inner.lock().await;
        if error.unauthorized || !error.retryable {
            if let Ok(Some(mut vault)) = secure::read().await {
                if error.unauthorized {
                    if let Some(p) = vault.participant.as_mut() {
                        p.enabled = false;
                    }
                }
                if vault.pending_control.is_some() || vault.receipt.is_some() {
                    vault.control_blocked = true;
                }
                if secure::write(&vault).await.is_ok() {
                    local.vault = Some(vault);
                }
            }
        }
        local.failures = local.failures.saturating_add(1).min(10);
        let backoff = Duration::from_secs((1_u64 << local.failures).min(900));
        local.next_delivery = Instant::now()
            + backoff.max(error.after)
            + Duration::from_millis(rand::random::<u16>() as u64 % 1000);
        local.retry_at = local.next_delivery;
        local.message = Some(error.message);
    }
    async fn poll_deletion(&self) {
        let vault = self.inner.lock().await.vault.clone();
        let Some(mut vault) = vault else {
            return;
        };
        if vault.api_url != self.api_url || vault.control_blocked {
            return;
        }
        let Some(receipt) = vault.receipt.clone() else {
            return;
        };
        // Receipt authentication is separate from revoked participant authentication.
        if vault.participant.is_some() {
            vault.participant = None;
            if secure::write(&vault).await.is_err() {
                return;
            }
        }
        if receipt.status != "completed" {
            #[derive(Deserialize)]
            struct ReceiptStatus {
                status: String,
            }
            let result: Result<ReceiptStatus, _> = self
                .request(
                    Method::GET,
                    &format!("/deletions/{}", receipt.receipt_id),
                    None,
                    Some(&receipt.status_token),
                    None,
                    None,
                )
                .await;
            match result {
                Ok(status) if matches!(status.status.as_str(), "pending" | "completed") => {
                    if let Some(receipt) = vault.receipt.as_mut() {
                        receipt.status = status.status;
                    }
                    if secure::write(&vault).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    self.failure(error).await;
                    return;
                }
                _ => return,
            }
        }
        let mut local = self.inner.lock().await;
        local.vault = Some(vault);
        local.next_delivery = Instant::now() + Duration::from_secs(3600);
    }
    async fn sync_control(
        &self,
        context: &AppContext,
        p: &FeedbackProgramState,
        c: &PendingControl,
    ) -> Result<(), ApiFailure> {
        let mut vault = secure::read()
            .await
            .map_err(|e| ApiFailure::local(&e))?
            .ok_or_else(|| {
                ApiFailure::local(
                    "Protected control credentials are unavailable; the request is retained",
                )
            })?;
        if vault.api_url != self.api_url {
            return Err(ApiFailure::local(
                "Protected control is bound to a different API",
            ));
        }
        if vault.participant.is_none()
            && matches!(c.kind, ControlKind::Deletion | ControlKind::Withdrawal)
        {
            let attempt = vault
                .enrollment
                .as_ref()
                .ok_or_else(|| ApiFailure::local("Protected deletion ownership is unavailable"))?;
            // A lost enrollment reply is recovered only with the original proof, never installationId alone.
            let request = &attempt.request;
            let mut recovered: Participant = self.request(Method::POST, "/enrollments", Some(json!({
                "installationId":vault.installation_id, "requestId":attempt.request_id, "recoveryProof":attempt.recovery_proof,
                "privacyVersion":request.privacy_version,"termsVersion":request.terms_version,"participationAccepted":true,
                "optionalContextEnabled":request.optional_context_enabled,"optionalProfile":request.optional_profile
            })), None, None, None).await?;
            if !valid_participant(&recovered) {
                return Err(ApiFailure::local("Invalid enrollment ownership recovery"));
            }
            recovered.optional_context_enabled = request.optional_context_enabled;
            vault.participant = Some(recovered);
            vault.enrollment = None;
            secure::write(&vault)
                .await
                .map_err(|e| ApiFailure::local(&e))?;
        }
        let participant = vault
            .participant
            .as_ref()
            .ok_or_else(|| ApiFailure::local("Protected control credentials are unavailable"))?
            .clone();
        if vault.api_url != self.api_url {
            return Err(ApiFailure::local(
                "Account control belongs to another feedback service; authentication retained",
            ));
        }
        if Some(&vault.installation_id) != p.installation_id.as_ref() {
            return Err(ApiFailure::local("Account control identity does not match"));
        }
        let current = context.load_state().await.feedback_program;
        if !current
            .pending_control_action
            .as_ref()
            .is_some_and(|pending| {
                pending.request_id == c.request_id
                    || pending
                        .previous
                        .as_ref()
                        .is_some_and(|prior| prior.request_id == c.request_id)
            })
        {
            return Err(ApiFailure::local(
                "A newer durable account control superseded this request",
            ));
        }
        vault.pending_control = current.pending_control_action.clone();
        let mut bound = c.clone();
        if bound.kind == ControlKind::Withdrawal && bound.expected_epoch == 0 {
            bound.expected_epoch = participant.consent_epoch;
            context
                .update_state(|s| {
                    if let Some(pending) = s.feedback_program.pending_control_action.as_mut() {
                        if pending.request_id == bound.request_id {
                            pending.expected_epoch = bound.expected_epoch;
                        }
                    }
                })
                .await
                .map_err(|_| ApiFailure::local("Could not bind the recovered withdrawal epoch"))?;
            vault.pending_control = context
                .load_state()
                .await
                .feedback_program
                .pending_control_action;
        }
        secure::write(&vault)
            .await
            .map_err(|e| ApiFailure::local(&e))?;
        let c = &bound;
        if c.kind == ControlKind::Deletion {
            // Recovery capability + stable requestId makes a lost deletion response retryable even
            // after the participant bearer has been revoked. The receipt token is separate.
            let receipt: DeletionReceipt = if let Some(receipt) = vault.receipt.clone() {
                receipt
            } else {
                self.request(
                    Method::POST,
                    "/deletions",
                    Some(
                        json!({"deletionCode":participant.deletion_code,"requestId":c.request_id}),
                    ),
                    None,
                    None,
                    Some(&c.request_id),
                )
                .await?
            };
            if uuid::Uuid::parse_str(&receipt.receipt_id).is_err()
                || receipt.status_token.len() < 32
                || !matches!(receipt.status.as_str(), "pending" | "completed")
            {
                return Err(ApiFailure::local(
                    "Deletion receipt was incomplete; credentials are retained",
                ));
            }
            vault.receipt = Some(receipt);
            vault.enrollment = None;
            vault.pending_control = None;
            vault.control_blocked = false;
            // Retain the participant until receipt AND state acknowledgment are durable.
            secure::write(&vault)
                .await
                .map_err(|e| ApiFailure::local(&e))?;
            context
                .update_state(|s| {
                    s.feedback_program.enabled = false;
                    s.feedback_program.enrollment_status = "deleted".into();
                    s.feedback_program.pending_control_action = None;
                    s.feedback_program.optional_context_enabled = false;
                })
                .await
                .map_err(|_| ApiFailure::local("Could not persist deletion acknowledgment"))?;
            vault.participant = None;
            secure::write(&vault)
                .await
                .map_err(|e| ApiFailure::local(&e))?;
            let mut local = self.inner.lock().await;
            local.vault = Some(vault);
            local.journal = None;
            self.outbox.clear().map_err(|e| ApiFailure::local(&e))?;
            return Ok(());
        }
        let ack: ConsentAck = self.request(Method::PUT, "/me/consent", Some(json!({"requestId":c.request_id,"expectedEpoch":c.expected_epoch,"enabled":c.enabled,"optionalContextEnabled":c.optional_context_enabled})), Some(&participant.credential), None, Some(&c.request_id)).await?;
        if ack.consent_epoch > 9_007_199_254_740_991
            || ack.consent_epoch <= c.expected_epoch
            || ack.enabled != c.enabled
            || ack.optional_context_enabled != c.optional_context_enabled
        {
            return Err(ApiFailure::local(
                "Consent acknowledgment did not match the durable request",
            ));
        }
        self.stop_local();
        let mut participant = participant;
        participant.enabled = ack.enabled;
        participant.optional_context_enabled = ack.optional_context_enabled;
        participant.consent_epoch = ack.consent_epoch;
        vault.participant = Some(participant.clone());
        secure::write(&vault)
            .await
            .map_err(|e| ApiFailure::local(&e))?;
        let journal = {
            let local = self.inner.lock().await;
            if c.preserve_sessions {
                local
                    .journal
                    .clone()
                    .unwrap_or_else(|| Journal::empty(vault.installation_id.clone()))
            } else {
                Journal::empty(vault.installation_id.clone())
            }
        };
        let mut journal = journal;
        journal.rebase_epoch(ack.consent_epoch);
        let latest = context.load_state().await.feedback_program;
        let superseded_by_stop = latest
            .pending_control_action
            .as_ref()
            .is_some_and(|next| next.request_id != c.request_id);
        if ack.enabled && !superseded_by_stop && !self.desired_disabled.load(Ordering::SeqCst) {
            self.outbox
                .save(&journal)
                .map_err(|e| ApiFailure::local(&e))?;
        } else {
            self.outbox.clear().map_err(|e| ApiFailure::local(&e))?;
        }
        context
            .update_state(|s| {
                // A later deletion intent must never be overwritten by an earlier consent response.
                if s.feedback_program
                    .pending_control_action
                    .as_ref()
                    .is_some_and(|p| p.request_id == c.request_id)
                {
                    s.feedback_program = state_from_participant(&vault, &participant);
                    if !ack.enabled {
                        s.feedback_program.enrollment_status = "withdrawn".into();
                    }
                } else if let Some(mut next) = s.feedback_program.pending_control_action.clone() {
                    if next
                        .previous
                        .as_ref()
                        .is_some_and(|prior| prior.request_id == c.request_id)
                    {
                        next.previous = None;
                        next.expected_epoch = ack.consent_epoch;
                        s.feedback_program = state_from_participant(&vault, &participant);
                        s.feedback_program.enabled = false;
                        s.feedback_program.optional_context_enabled = false;
                        s.feedback_program.pending_control_action = Some(next);
                    }
                }
            })
            .await
            .map_err(|_| ApiFailure::local("Could not persist consent acknowledgment"))?;
        vault.pending_control = context
            .load_state()
            .await
            .feedback_program
            .pending_control_action;
        vault.control_blocked = false;
        vault.collection_paused = false;
        secure::write(&vault)
            .await
            .map_err(|e| ApiFailure::local(&e))?;
        let current = context.load_state().await.feedback_program;
        let mut local = self.inner.lock().await;
        local.vault = Some(vault);
        local.journal = (ack.enabled
            && current.pending_control_action.is_none()
            && !self.desired_disabled.load(Ordering::SeqCst))
        .then_some(journal);
        local.metrics.reset();
        self.collecting.store(
            ack.enabled && current.pending_control_action.is_none() && current.enabled,
            Ordering::SeqCst,
        );
        drop(local);
        let snapshot = context.load_state().await;
        self.observe_state(&snapshot, &snapshot).await;
        Ok(())
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        payload: Option<Value>,
        token: Option<&str>,
        gate: Option<(&AppContext, u64)>,
        request_id: Option<&str>,
    ) -> Result<T, ApiFailure> {
        let base = self
            .api_url
            .as_ref()
            .ok_or_else(|| ApiFailure::local("Feedback API is not configured"))?;
        let cancelled = self.cancel.notified();
        tokio::pin!(cancelled);
        cancelled.as_mut().enable();
        if let Some((context, epoch)) = gate {
            let p = context.load_state().await.feedback_program;
            if !self.permitted(&p) || p.consent_epoch != epoch {
                return Err(ApiFailure::local(
                    "Feedback permission changed before delivery",
                ));
            }
        }
        let client = self
            .http
            .as_ref()
            .ok_or_else(|| ApiFailure::local("Feedback HTTPS client unavailable"))?;
        let mut request = client.request(method, format!("{base}{path}"));
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(id) = request_id {
            request = request
                .header("X-Request-Id", id)
                .header("Idempotency-Key", id);
        }
        if let Some(payload) = payload {
            request = request.json(&payload);
        }
        let response = if gate.is_some() {
            tokio::select! { r = request.send() => r, _ = &mut cancelled => return Err(ApiFailure::local("Feedback delivery cancelled after local consent/lifecycle change")) }
        } else {
            request.send().await
        };
        let mut response = response.map_err(|_| {
            ApiFailure::local(
                "Feedback API is unavailable; normal rental and streaming are unaffected",
            )
        })?;
        let status = response.status();
        let after = retry_after(
            response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| ApiFailure::local("Feedback response was interrupted"))?
        {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err(ApiFailure::local(
                    "Feedback response exceeded its size limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            let structured: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let unauthorized = status.as_u16() == 401 || status.as_u16() == 403;
            let retryable = status.as_u16() == 429
                || status.is_server_error()
                || structured["retryable"].as_bool() == Some(true);
            return Err(ApiFailure {
                message: if unauthorized {
                    "Feedback authorization or consent is no longer usable; collection is paused"
                        .into()
                } else {
                    format!(
                        "Feedback request was not accepted (HTTP {}); {}",
                        status.as_u16(),
                        if retryable {
                            "retry is queued"
                        } else {
                            "this contribution is paused"
                        }
                    )
                },
                retryable,
                after,
                unauthorized,
            });
        }
        if bytes.is_empty() {
            bytes.extend_from_slice(b"{}");
        }
        if let Some((context, epoch)) = gate {
            let current = context.load_state().await.feedback_program;
            if !self.permitted(&current) || current.consent_epoch != epoch {
                return Err(ApiFailure::local(
                    "Feedback permission changed while receiving the response",
                ));
            }
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| ApiFailure::local("Feedback server returned an invalid response"))
    }
}

fn bounded_label(value: &str, limit: usize) -> Option<String> {
    let value: String = value
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect();
    (!value.is_empty()).then_some(value)
}
fn country_code(value: &str) -> Option<String> {
    let code = value.trim().to_ascii_uppercase();
    (code.len() == 2 && code.bytes().all(|b| b.is_ascii_uppercase())).then_some(code)
}
fn valid_participant(p: &Participant) -> bool {
    uuid::Uuid::parse_str(&p.user_id).is_ok()
        && p.credential.len() >= 32
        && p.credential.len() <= 4096
        && p.deletion_code.len() >= 32
        && p.deletion_code.len() <= 4096
        && p.consent_epoch > 0
        && p.consent_epoch <= 9_007_199_254_740_991
        && chrono::DateTime::parse_from_rfc3339(&p.consented_at).is_ok()
        && !p.credential.chars().any(char::is_control)
        && !p.deletion_code.chars().any(char::is_control)
}
fn matches_participant(p: &FeedbackProgramState, v: &Vault, a: &Participant) -> bool {
    p.enabled
        && a.enabled
        && valid_participant(a)
        && p.pending_control_action.is_none()
        && p.installation_id.as_ref() == Some(&v.installation_id)
        && p.user_id.as_ref() == Some(&a.user_id)
        && p.consent_epoch == a.consent_epoch
        && p.privacy_version.as_ref() == Some(&a.privacy_version)
        && p.terms_version.as_ref() == Some(&a.terms_version)
        && p.credential_ref.as_deref() == Some(CREDENTIAL_REF)
        && p.optional_context_enabled == a.optional_context_enabled
}
fn state_from_participant(v: &Vault, p: &Participant) -> FeedbackProgramState {
    FeedbackProgramState {
        schema_version: 1,
        enabled: p.enabled,
        enrollment_status: "enrolled".into(),
        installation_id: Some(v.installation_id.clone()),
        user_id: Some(p.user_id.clone()),
        credential_ref: Some(CREDENTIAL_REF.into()),
        deletion_code_ref: Some(CREDENTIAL_REF.into()),
        consent_epoch: p.consent_epoch,
        privacy_version: Some(p.privacy_version.clone()),
        terms_version: Some(p.terms_version.clone()),
        consented_at: Some(p.consented_at.clone()),
        optional_context_enabled: p.optional_context_enabled,
        pending_control_action: None,
    }
}
fn configured_url(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(raw).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    Some(if raw.ends_with("/v1") {
        raw.to_string()
    } else {
        format!("{raw}/v1")
    })
}
fn retry_after(value: Option<&str>) -> Duration {
    let Some(v) = value else {
        return Duration::ZERO;
    };
    if let Ok(seconds) = v.parse::<u64>() {
        return Duration::from_secs(seconds.min(7 * 24 * 3600));
    }
    chrono::DateTime::parse_from_rfc2822(v)
        .ok()
        .map(|at| {
            Duration::from_secs(
                at.timestamp()
                    .saturating_sub(chrono::Utc::now().timestamp())
                    .clamp(0, 7 * 24 * 3600) as u64,
            )
        })
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_url_and_non_https_are_unconfigured() {
        for v in [
            None,
            Some(""),
            Some("http://localhost"),
            Some("https://a.example/?token=secret"),
        ] {
            assert!(configured_url(v).is_none());
        }
        assert_eq!(
            configured_url(Some("https://example.test/v1/")).as_deref(),
            Some("https://example.test/v1")
        );
    }
    #[test]
    fn persisted_true_without_matching_protected_consent_never_authorizes() {
        let p = FeedbackProgramState::default();
        let service = FeedbackProgramService::new(Path::new("."));
        assert!(!service.permitted(&p));
    }
    #[test]
    fn retry_after_seconds_are_respected() {
        assert!(retry_after(Some("120")) >= Duration::from_secs(120));
    }
}
