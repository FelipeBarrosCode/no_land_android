//! Allowlisted read DTOs: unknown server properties cannot leak participant data to the WebView.
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineSummaries {
    pub summaries: Vec<MachineSummary>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineSummary {
    pub provider_machine_id: String,
    pub reliability: Reliability,
    pub jitter: Option<Jitter>,
    #[serde(default)]
    pub jitter_groups: Vec<JitterGroup>,
    pub reviews: ReviewSummary,
    pub as_of: String,
    pub first_observed_at: Option<String>,
    pub last_observed_at: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reliability {
    pub source: String,
    pub ratio: Option<f64>,
    pub eligible_sessions: u64,
    pub successful_sessions: u64,
    pub failed_sessions: u64,
    pub basis: String,
    pub scope: String,
    pub first_observed_at: Option<String>,
    pub last_observed_at: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Jitter {
    pub mean_ms: f64,
    pub min_ms: f64,
    pub max_ms: f64,
    pub sample_count: u64,
    pub contributing_sessions: u64,
    pub high_sample_ratio: f64,
    pub severe_sample_ratio: f64,
    pub metric_version: String,
    pub scope: String,
    pub nominal_report_interval_ms: Option<u64>,
    pub completeness: Option<String>,
    pub threshold_version: Option<String>,
    pub high_threshold_ms: Option<f64>,
    pub severe_threshold_ms: Option<f64>,
}
impl Jitter {
    pub fn canonical(&self) -> bool {
        self.metric_version == crate::models::feedback::METRIC_VERSION
            && self.completeness.as_deref() == Some("full_window")
            && self.nominal_report_interval_ms == Some(1000)
            && self.threshold_version.as_deref() == Some(crate::models::feedback::THRESHOLD_VERSION)
            && self.high_threshold_ms == Some(7.0)
            && self.severe_threshold_ms == Some(15.0)
            && self.sample_count > 0
            && self.contributing_sessions > 0
            && [self.mean_ms, self.min_ms, self.max_ms]
                .iter()
                .all(|x| x.is_finite() && *x >= 0.0)
            && self.min_ms <= self.mean_ms
            && self.mean_ms <= self.max_ms
            && (0.0..=1.0).contains(&self.high_sample_ratio)
            && (0.0..=self.high_sample_ratio).contains(&self.severe_sample_ratio)
    }
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JitterGroup {
    #[serde(flatten)]
    pub jitter: Jitter,
    pub route_mode: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSummary {
    pub comment_count: u64,
    pub rating_count: u64,
    pub average_score: Option<f64>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPage {
    pub reviews: Vec<PublicReview>,
    pub next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicReview {
    pub review_id: String,
    pub score: Option<u8>,
    pub comment: Option<String>,
    pub published_date: String,
    pub hardware_snapshot: Option<crate::models::feedback::HardwareSnapshot>,
}
