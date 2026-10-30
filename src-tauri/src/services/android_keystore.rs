//! Shared Android Keystore bridge for the real state-secret backend and feedback vault.
//! JNI uses the application classloader; failures never fall back to plaintext/file keys.
use jni::{
    objects::{JClass, JObject, JString, JValue},
    JavaVM,
};

const ERROR: &str = "Android Keystore storage is unavailable; no plaintext fallback is permitted";

pub fn call(request: &serde_json::Value) -> Result<serde_json::Value, String> {
    let encoded = serde_json::to_string(request).map_err(|_| ERROR.to_string())?;
    let android =
        std::panic::catch_unwind(ndk_context::android_context).map_err(|_| ERROR.to_string())?;
    if android.vm().is_null() || android.context().is_null() {
        return Err(ERROR.into());
    }
    let vm = unsafe { JavaVM::from_raw(android.vm().cast()) }.map_err(|_| ERROR.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|_| ERROR.to_string())?;
    let result: Result<serde_json::Value, jni::errors::Error> = env.with_local_frame(24, |env| {
        // This is a borrowed ndk global reference; only the new local reference is released.
        let context = unsafe { JObject::from_raw(android.context().cast()) };
        let local = env.new_local_ref(&context)?;
        let loader = env
            .call_method(&local, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
            .l()?;
        let name =
            JObject::from(env.new_string("com.noland.connect.feedback.FeedbackNativeBridge")?);
        let class = JClass::from(
            env.call_method(
                loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[JValue::Object(&name)],
            )?
            .l()?,
        );
        let request = JObject::from(env.new_string(&encoded)?);
        let result = env
            .call_static_method(
                class,
                "call",
                "(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&local), JValue::Object(&request)],
            )?
            .l()?;
        let text = JString::from(result);
        let text: String = env.get_string(&text)?.into();
        Ok(serde_json::from_str(&text).unwrap_or(serde_json::Value::Null))
    });
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    let response = result.map_err(|_| ERROR.to_string())?;
    if response["ok"].as_bool() != Some(true) {
        return Err(ERROR.into());
    }
    Ok(response)
}

pub fn account(
    action: &str,
    service: &str,
    account: &str,
    value: Option<&str>,
) -> Result<Option<String>, String> {
    let response = call(
        &serde_json::json!({"action":action,"service":service,"account":account,"value":value}),
    )?;
    Ok(response
        .get("value")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string))
}
