//! Activity suspension is a conservative lifecycle gap, not a setup failure.
use super::super::app_context::AppContext;
use jni::{objects::JObject, sys::jboolean, JNIEnv};
use std::sync::OnceLock;

static CONTEXT: OnceLock<AppContext> = OnceLock::new();

pub fn install(context: AppContext) {
    let _ = CONTEXT.set(context.clone());
    tauri::async_runtime::spawn(async move {
        let installed = tokio::task::spawn_blocking(|| {
            super::super::android_keystore::call(&serde_json::json!({"action":"installLifecycle"}))
        })
        .await;
        context.feedback.lifecycle_ready.store(
            matches!(installed, Ok(Ok(_))),
            std::sync::atomic::Ordering::SeqCst,
        );
        if !context
            .feedback
            .lifecycle_ready
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            context.feedback.stop_local();
            context.feedback.inner.lock().await.message =
                Some("Android lifecycle bridge unavailable; feedback is disabled".into());
        }
    });
}

#[no_mangle]
pub extern "system" fn Java_com_noland_connect_feedback_FeedbackNativeBridge_nativeLifecycle(
    _env: JNIEnv<'_>,
    _object: JObject<'_>,
    foreground: jboolean,
) {
    if let Some(context) = CONTEXT.get().cloned() {
        let foreground = foreground != 0;
        context.feedback.set_foreground(foreground);
        tauri::async_runtime::spawn(async move {
            context.feedback.checkpoint(&context, !foreground).await;
            if foreground {
                context.feedback.wake.notify_one();
            }
        });
    }
}
