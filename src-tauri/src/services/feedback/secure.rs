//! A dedicated feedback namespace over the same Keystore adapter used by state secrets.
use crate::models::feedback::EnrollmentRequest;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Vault {
    pub schema_version: u32,
    #[serde(default)]
    pub api_url: Option<String>,
    pub installation_id: String,
    pub enrollment: Option<EnrollmentAttempt>,
    pub participant: Option<Participant>,
    pub receipt: Option<DeletionReceipt>,
    #[serde(default)]
    pub pending_control: Option<crate::models::feedback::PendingControl>,
    #[serde(default)]
    pub control_blocked: bool,
    #[serde(default)]
    pub collection_paused: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnrollmentAttempt {
    pub request_id: String,
    pub recovery_proof: String,
    pub created_at: i64,
    pub request: EnrollmentRequest,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub user_id: String,
    pub credential: String,
    pub deletion_code: String,
    pub consent_epoch: u64,
    pub privacy_version: String,
    pub terms_version: String,
    pub consented_at: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub optional_context_enabled: bool,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletionReceipt {
    pub receipt_id: String,
    pub status_token: String,
    pub status: String,
}

pub async fn read() -> Result<Option<Vault>, String> {
    let response = call(serde_json::json!({"action":"read"})).await?;
    match response.get("value").and_then(serde_json::Value::as_str) {
        None => Ok(None),
        Some(s) => {
            let vault: Vault = serde_json::from_str(s)
                .map_err(|_| "Protected feedback credentials are invalid")?;
            if vault.schema_version != 1
                || uuid::Uuid::parse_str(&vault.installation_id).is_err()
                || vault.pending_control.as_ref().is_some_and(|c| !c.valid())
                || vault
                    .participant
                    .as_ref()
                    .is_some_and(|p| !super::valid_participant(p))
                || vault.receipt.as_ref().is_some_and(|r| {
                    uuid::Uuid::parse_str(&r.receipt_id).is_err()
                        || r.status_token.len() < 32
                        || r.status_token.len() > 4096
                        || !matches!(r.status.as_str(), "pending" | "completed")
                })
                || vault.enrollment.as_ref().is_some_and(|a| {
                    uuid::Uuid::parse_str(&a.request_id).is_err()
                        || a.recovery_proof.len() < 43
                        || a.recovery_proof.len() > 128
                })
            {
                return Err("Protected feedback credential version is unsupported".into());
            }
            Ok(Some(vault))
        }
    }
}
pub async fn write(vault: &Vault) -> Result<(), String> {
    let value = serde_json::to_string(vault)
        .map_err(|_| "Could not encode protected feedback credentials")?;
    call(serde_json::json!({"action":"write","value":value})).await?;
    Ok(())
}
pub async fn available() -> Result<(), String> {
    call(serde_json::json!({"action":"probe"}))
        .await
        .map(|_| ())
}
#[cfg(not(target_os = "android"))]
async fn call(_: serde_json::Value) -> Result<serde_json::Value, String> {
    Err("This build has no verified Android Keystore feedback adapter".into())
}
#[cfg(target_os = "android")]
async fn call(request: serde_json::Value) -> Result<serde_json::Value, String> {
    tokio::task::spawn_blocking(move || super::super::android_keystore::call(&request))
        .await
        .map_err(|_| "Android feedback secure adapter task failed".to_string())?
}
