use crate::{
    models::feedback::*,
    services::{
        app_context::AppContext,
        feedback::dto::{MachineSummaries, ReviewPage},
    },
};
use reqwest::Method;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

#[tauri::command]
pub async fn feedback_program_status(
    context: State<'_, AppContext>,
) -> Result<FeedbackStatus, String> {
    Ok(context.feedback.status(context.inner()).await)
}
#[tauri::command]
pub async fn feedback_program_notices(
    context: State<'_, AppContext>,
) -> Result<ProgramDocument, String> {
    context.feedback.notices().await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn feedback_program_enroll(
    context: State<'_, AppContext>,
    request: EnrollmentRequest,
) -> Result<FeedbackStatus, String> {
    context.feedback.enroll(context.inner(), request).await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn feedback_program_consent(
    context: State<'_, AppContext>,
    enabled: bool,
    optional_context_enabled: bool,
) -> Result<FeedbackStatus, String> {
    context
        .feedback
        .consent(context.inner(), enabled, optional_context_enabled)
        .await
}
#[tauri::command]
pub async fn feedback_program_delete(
    context: State<'_, AppContext>,
) -> Result<FeedbackStatus, String> {
    context.feedback.delete(context.inner()).await
}
#[tauri::command(rename_all = "camelCase")]
pub async fn feedback_machine_summaries(
    context: State<'_, AppContext>,
    machine_ids: Vec<String>,
) -> Result<MachineSummaries, String> {
    if machine_ids.is_empty()
        || machine_ids.len() > 100
        || machine_ids.iter().any(|s| !valid_provider_id(s))
    {
        return Err("Supply 1–100 positive decimal machine ID strings".into());
    }
    let ids: std::collections::BTreeSet<_> = machine_ids.into_iter().collect();
    let mut response: MachineSummaries = context
        .feedback
        .read_api(
            context.inner(),
            Method::POST,
            "/machines/summaries",
            Some(json!({"provider":"vast","machineIds":ids})),
        )
        .await?;
    let returned: std::collections::BTreeSet<_> = response
        .summaries
        .iter()
        .map(|s| s.provider_machine_id.clone())
        .collect();
    if response.summaries.len() != ids.len() || returned != ids {
        return Err("Incomplete machine feedback response; use Vast fallback".into());
    }
    for summary in &mut response.summaries {
        if summary.reliability.eligible_sessions < 2 {
            summary.reliability.ratio = None;
        }
        if summary
            .reliability
            .ratio
            .is_some_and(|r| !r.is_finite() || !(0.0..=1.0).contains(&r))
        {
            return Err("Invalid community reliability response; use Vast fallback".into());
        }
        if summary.jitter.as_ref().is_some_and(|j| !j.canonical()) {
            summary.jitter = None;
        }
        summary.jitter_groups.retain(|j| j.jitter.canonical());
    }
    Ok(response)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn feedback_machine_reviews(
    context: State<'_, AppContext>,
    machine_id: String,
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<ReviewPage, String> {
    if !valid_provider_id(&machine_id) {
        return Err("Invalid machine ID".into());
    }
    let limit = limit.unwrap_or(20);
    if !(1..=50).contains(&limit) || cursor.as_ref().is_some_and(|s| s.len() > 4096) {
        return Err("Invalid review pagination".into());
    }
    let mut path = format!("/machines/{machine_id}/reviews?limit={limit}");
    if let Some(cursor) = cursor {
        path.push_str("&cursor=");
        path.push_str(&urlencoding::encode(&cursor));
    }
    let page: ReviewPage = context
        .feedback
        .read_api(context.inner(), Method::GET, &path, None)
        .await?;
    if page.reviews.len() > limit as usize
        || page.reviews.iter().any(|r| {
            r.comment
                .as_ref()
                .is_some_and(|s| s.encode_utf16().count() > 2000)
                || uuid::Uuid::parse_str(&r.review_id).is_err()
                || r.score.is_some_and(|s| !(1..=10).contains(&s))
        })
    {
        return Err("Invalid review page".into());
    }
    Ok(page)
}
#[tauri::command(rename_all = "camelCase")]
pub async fn feedback_review_report(
    context: State<'_, AppContext>,
    review_id: String,
    reason: String,
) -> Result<Value, String> {
    if uuid::Uuid::parse_str(&review_id).is_err()
        || reason.trim().is_empty()
        || reason.encode_utf16().count() > 500
        || reason.chars().any(char::is_control)
    {
        return Err("Invalid review report".into());
    }
    let _: Value = context
        .feedback
        .read_api(
            context.inner(),
            Method::POST,
            &format!("/reviews/{review_id}/reports"),
            Some(json!({"reason":reason.trim()})),
        )
        .await?;
    Ok(json!({"ok":true}))
}
#[tauri::command(rename_all = "camelCase")]
pub async fn feedback_review_block_author(
    context: State<'_, AppContext>,
    review_id: String,
) -> Result<Value, String> {
    if uuid::Uuid::parse_str(&review_id).is_err() {
        return Err("Invalid review ID".into());
    }
    let _: Value = context
        .feedback
        .read_api(
            context.inner(),
            Method::POST,
            &format!("/reviews/{review_id}/block-author"),
            Some(json!({})),
        )
        .await?;
    Ok(json!({"ok":true}))
}
#[derive(Serialize)]
pub struct ReviewEligibility {
    eligible: bool,
}
#[tauri::command(rename_all = "camelCase")]
pub async fn feedback_review_eligibility(
    context: State<'_, AppContext>,
    provider_instance_id: String,
) -> Result<ReviewEligibility, String> {
    Ok(ReviewEligibility {
        eligible: context
            .feedback
            .review_eligible(context.inner(), &provider_instance_id)
            .await,
    })
}
#[tauri::command(rename_all = "camelCase")]
pub async fn feedback_stage_review(
    context: State<'_, AppContext>,
    provider_instance_id: String,
    score: Option<u8>,
    comment: Option<String>,
) -> Result<Value, String> {
    context
        .feedback
        .stage_review(context.inner(), &provider_instance_id, score, comment)
        .await?;
    Ok(json!({"staged":true}))
}
