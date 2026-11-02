//! Public, non-secret feedback state and wire DTOs. Credentials never enter app state.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SCHEMA_VERSION: u32 = 1;
pub const METRIC_VERSION: &str = "rtt_jitter_ewma_window60_report1s_v1";
pub const THRESHOLD_VERSION: &str = "noland_jitter_bands_v1";
pub const CREDENTIAL_REF: &str = "android-keystore://noland-feedback/installation-v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeedbackProgramState {
    pub schema_version: u32,
    pub enabled: bool,
    pub enrollment_status: String,
    pub installation_id: Option<String>,
    pub user_id: Option<String>,
    pub credential_ref: Option<String>,
    pub deletion_code_ref: Option<String>,
    pub consent_epoch: u64,
    pub privacy_version: Option<String>,
    pub terms_version: Option<String>,
    pub consented_at: Option<String>,
    pub optional_context_enabled: bool,
    pub pending_control_action: Option<PendingControl>,
}

impl Default for FeedbackProgramState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            enabled: false,
            enrollment_status: "not_enrolled".into(),
            installation_id: None,
            user_id: None,
            credential_ref: None,
            deletion_code_ref: None,
            consent_epoch: 0,
            privacy_version: None,
            terms_version: None,
            consented_at: None,
            optional_context_enabled: false,
            pending_control_action: None,
        }
    }
}

impl FeedbackProgramState {
    /// Validate the original object, not a default-merged version: partial consent is not consent.
    pub fn from_untrusted(value: Option<&Value>) -> Self {
        let Some(mut state) = value
            .cloned()
            .and_then(|v| serde_json::from_value::<Self>(v).ok())
        else {
            return Self::default();
        };
        if state.schema_version != SCHEMA_VERSION
            || !matches!(
                state.enrollment_status.as_str(),
                "not_enrolled"
                    | "enrolling"
                    | "enrolled"
                    | "withdrawn"
                    | "deletion_pending"
                    | "deleted"
                    | "unavailable"
            )
            || state.consent_epoch > 9_007_199_254_740_991
            || state
                .installation_id
                .as_deref()
                .is_some_and(|s| uuid::Uuid::parse_str(s).is_err())
            || state
                .user_id
                .as_deref()
                .is_some_and(|s| uuid::Uuid::parse_str(s).is_err())
            || state
                .pending_control_action
                .as_ref()
                .is_some_and(|c| !c.valid())
        {
            return Self::default();
        }
        if state.enrollment_status != "enrolled"
            || state.user_id.is_none()
            || state.installation_id.is_none()
            || state.consent_epoch == 0
            || state.credential_ref.as_deref() != Some(CREDENTIAL_REF)
            || state.deletion_code_ref.as_deref() != Some(CREDENTIAL_REF)
            || state.privacy_version.as_deref().is_none_or(str::is_empty)
            || state.terms_version.as_deref().is_none_or(str::is_empty)
            || state.consented_at.as_deref().is_none_or(str::is_empty)
            || state.pending_control_action.is_some()
        {
            state.enabled = false;
        }
        state
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PendingControl {
    pub kind: ControlKind,
    pub request_id: String,
    pub expected_epoch: u64,
    pub enabled: bool,
    pub optional_context_enabled: bool,
    #[serde(default)]
    pub preserve_sessions: bool,
    /// At most one already-issued consent change must settle before an offline withdrawal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Box<PendingControl>>,
}
impl PendingControl {
    pub(crate) fn valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.request_id).is_ok()
            && self.expected_epoch <= 9_007_199_254_740_991
            && self.previous.as_ref().is_none_or(|p| {
                self.kind == ControlKind::Withdrawal
                    && p.kind == ControlKind::Consent
                    && p.previous.is_none()
                    && p.valid()
            })
            && match self.kind {
                ControlKind::Consent => self.enabled && self.expected_epoch > 0,
                ControlKind::Withdrawal => {
                    !self.enabled && !self.optional_context_enabled && !self.preserve_sessions
                }
                ControlKind::Deletion => {
                    !self.enabled && !self.optional_context_enabled && !self.preserve_sessions
                }
            }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    Consent,
    Withdrawal,
    Deletion,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackStatus {
    #[serde(flatten)]
    pub program: FeedbackProgramState,
    pub configured: bool,
    pub supported: bool,
    pub status_message: Option<String>,
    pub pending_sessions: usize,
    pub jitter_capability: &'static str,
    pub frame_evidence: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deletion_status: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollmentRequest {
    pub privacy_version: String,
    pub terms_version: String,
    pub participation_accepted: bool,
    pub optional_context_enabled: bool,
    #[serde(default)]
    pub optional_profile: OptionalProfile,
}
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OptionalProfile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bandwidth_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
}

impl OptionalProfile {
    pub fn normalize(&mut self) -> Result<(), String> {
        for (field, max) in [
            (&mut self.country, 2),
            (&mut self.region, 80),
            (&mut self.connection_category, 32),
            (&mut self.bandwidth_tier, 32),
        ] {
            if let Some(value) = field.take() {
                let value = value.trim();
                if value.encode_utf16().count() > max || value.chars().any(char::is_control) {
                    return Err("Invalid optional connection context".into());
                }
                if !value.is_empty() {
                    *field = Some(value.to_string());
                }
            }
        }
        if self
            .country
            .as_ref()
            .is_some_and(|s| s.len() != 2 || !s.bytes().all(|c| c.is_ascii_uppercase()))
        {
            return Err("Optional country must be an uppercase two-letter code".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgramDocument {
    pub schema_version: u32,
    pub available: bool,
    pub collection_paused: bool,
    pub privacy_version: String,
    pub terms_version: String,
    pub privacy_url: String,
    pub terms_url: String,
    pub deletion_url: String,
    pub contact: String,
    pub disclosure: String,
    pub metrics: Vec<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventOrder {
    pub rental_started: u64,
    pub ready: Option<u64>,
    pub paired: Option<u64>,
    pub setup_success: Option<u64>,
    pub first_stream_frame: Option<u64>,
    pub destroyed: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Lifecycle {
    pub rental_started_at: String,
    pub ready_at: Option<String>,
    pub paired_at: Option<String>,
    pub setup_success_at: Option<String>,
    pub first_stream_frame_at: Option<String>,
    pub destroyed_at: Option<String>,
    pub observation_complete: bool,
    pub last_event_sequence: u64,
    pub event_order: EventOrder,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionContext {
    pub platform: String,
    pub app_version: String,
    pub collector_version: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HardwareSnapshot {
    pub gpu_name: Option<String>,
    pub rented_gpu_count: Option<u32>,
    pub country: Option<String>,
    pub region: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionSnapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub consent_epoch: u64,
    pub provider: String,
    pub provider_machine_id: String,
    pub provider_instance_id: String,
    pub provider_offer_id: Option<String>,
    pub lifecycle: Lifecycle,
    pub context: SessionContext,
    pub hardware_snapshot: HardwareSnapshot,
    pub jitter_buckets: Vec<JitterBucket>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JitterBucket {
    pub metric_version: String,
    pub route_mode: String,
    pub completeness: String,
    pub nominal_report_interval_ms: u64,
    pub threshold_version: String,
    pub high_threshold_ms: f64,
    pub severe_threshold_ms: f64,
    pub sample_count: u64,
    pub sum_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub high_sample_count: u64,
    pub severe_sample_count: u64,
    pub observed_duration_ms: u64,
    pub first_observed_at: String,
    pub last_observed_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReviewInput {
    pub revision: u64,
    pub score: Option<u8>,
    pub comment: Option<String>,
}
impl ReviewInput {
    pub fn new(revision: u64, score: Option<u8>, comment: Option<String>) -> Result<Self, String> {
        let comment = comment
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if score.is_some_and(|n| !(1..=10).contains(&n))
            || comment.as_ref().is_some_and(|s| {
                s.encode_utf16().count() > 2000
                    || s.chars()
                        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            })
            || (score.is_none() && comment.is_none())
        {
            return Err("Provide a score from 1 to 10 and/or a plain-text comment of at most 2,000 characters".into());
        }
        Ok(Self {
            revision,
            score,
            comment,
        })
    }
}

/// IDs must come from integer/string provider JSON, never a floating-point conversion.
pub fn provider_id(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => valid_provider_id(s).then(|| s.clone()),
        Value::Number(n) => n
            .as_u64()
            .filter(|n| *n > 0 && *n <= i64::MAX as u64)
            .map(|n| n.to_string()),
        _ => None,
    }
}
pub fn valid_provider_id(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('0')
        && s.bytes().all(|b| b.is_ascii_digit())
        && s.parse::<i64>().is_ok_and(|n| n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn strict_decimal_ids_never_round_or_guess() {
        assert_eq!(
            provider_id(&json!(9007199254740993_u64)).as_deref(),
            Some("9007199254740993")
        );
        for v in [
            json!(1.0),
            json!("01"),
            json!(0),
            json!(-1),
            json!("9223372036854775808"),
        ] {
            assert!(provider_id(&v).is_none());
        }
    }
    #[test]
    fn controls_cannot_encode_accidental_reenable_or_scope_expansion() {
        let mut control = PendingControl {
            kind: ControlKind::Withdrawal,
            request_id: uuid::Uuid::new_v4().to_string(),
            expected_epoch: 1,
            enabled: false,
            optional_context_enabled: false,
            preserve_sessions: false,
            previous: None,
        };
        assert!(control.valid());
        control.enabled = true;
        assert!(!control.valid());
        control.kind = ControlKind::Consent;
        assert!(control.valid());
        control.expected_epoch = 0;
        assert!(!control.valid());
        control.kind = ControlKind::Deletion;
        control.enabled = false;
        assert!(control.valid());
    }
    #[test]
    fn pending_withdrawal_can_settle_one_issued_scope_change_without_reenabling() {
        let issued = PendingControl {
            kind: ControlKind::Consent,
            request_id: uuid::Uuid::new_v4().to_string(),
            expected_epoch: 1,
            enabled: true,
            optional_context_enabled: false,
            preserve_sessions: true,
            previous: None,
        };
        let mut stop = PendingControl {
            kind: ControlKind::Withdrawal,
            request_id: uuid::Uuid::new_v4().to_string(),
            expected_epoch: 1,
            enabled: false,
            optional_context_enabled: false,
            preserve_sessions: false,
            previous: Some(Box::new(issued.clone())),
        };
        assert!(stop.valid());
        stop.previous.as_mut().unwrap().previous = Some(Box::new(issued));
        assert!(!stop.valid());
    }
    #[test]
    fn optional_profile_is_allowlisted_bounded_and_truly_optional() {
        let mut profile = OptionalProfile::default();
        assert!(profile.normalize().is_ok());
        profile.country = Some("CA".into());
        profile.region = Some("   ".into());
        assert!(profile.normalize().is_ok());
        assert!(profile.region.is_none());
        profile.country = Some("Canada".into());
        assert!(profile.normalize().is_err());
        assert!(serde_json::from_value::<OptionalProfile>(json!({"email":"not-allowed"})).is_err());
    }
    #[test]
    fn partial_future_and_malformed_consent_are_off() {
        for v in [
            json!({"enabled": true}),
            json!({"schemaVersion": 99, "enabled": true}),
            Value::Null,
        ] {
            assert!(!FeedbackProgramState::from_untrusted(Some(&v)).enabled);
        }
        assert!(ReviewInput::new(1, None, Some("   ".into())).is_err());
        assert!(ReviewInput::new(1, Some(0), None).is_err());
    }
}
