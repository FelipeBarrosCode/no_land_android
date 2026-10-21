//! Synchronous Rust side of the Android `VpnService` control contract.
//! Calls run from existing blocking tasks and attach those threads to the JVM.

use std::{path::Path, sync::Mutex};

use jni::{
    objects::{JClass, JObject, JString, JValue},
    JNIEnv, JavaVM,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::errors::{AppError, AppResult};

const BRIDGE_CLASS: &str = "com.noland.connect.vpn.NolandVpnBridge";
const BRIDGE_SIGNATURE: &str = "(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/String;";
const REFERENCE_PREFIX: &str = "vpn.configuration.";
static BRIDGE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VpnStatus {
    pub ok: bool,
    pub status: String,
    pub configuration_reference: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Runtime {
    pub ok: bool,
    pub error: Option<String>,
    pub active: bool,
    pub launch_id: String,
    pub config_fingerprint: String,
    pub peer_public_key: String,
    pub endpoint: String,
    pub mtu: u16,
    pub latest_handshake_age_secs: Option<u64>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallRequest<'a> {
    configuration_reference: &'a str,
    configuration: &'a str,
    operation_id: &'a str,
    instance_id: u64,
    launch_id: &'a str,
    config_fingerprint: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReferenceRequest<'a> {
    configuration_reference: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRequest<'a> {
    configuration_reference: &'a str,
    configuration: &'a str,
    expected_config_fingerprint: &'a str,
    config_fingerprint: &'a str,
    transition_id: uuid::Uuid,
}

#[derive(Deserialize)]
struct ConfigurationResponse {
    ok: bool,
    configuration: Option<String>,
    error: Option<String>,
}

impl From<jni::errors::Error> for AppError {
    fn from(value: jni::errors::Error) -> Self {
        AppError::Command(format!("Android JNI call failed: {value}"))
    }
}

fn java_exception_message(env: &mut JNIEnv<'_>) -> Option<String> {
    if !env.exception_check().ok()? {
        return None;
    }
    let exception = env.exception_occurred().ok()?;
    env.exception_clear().ok()?;
    let rendered = env
        .call_method(exception, "toString", "()Ljava/lang/String;", &[])
        .ok()?
        .l()
        .ok()?;
    if rendered.is_null() {
        return None;
    }
    let rendered = JString::from(rendered);
    let message: String = env.get_string(&rendered).ok()?.into();
    Some(message)
}

fn jni_failure(env: &mut JNIEnv<'_>, method: &str, error: jni::errors::Error) -> AppError {
    let detail = java_exception_message(env).unwrap_or_else(|| error.to_string());
    AppError::Command(format!(
        "Android VPN bridge method `{method}` failed: {detail}"
    ))
}

fn new_local_context<'local>(
    env: &mut JNIEnv<'local>,
    global_context: jni::sys::jobject,
) -> AppResult<JObject<'local>> {
    let raw_env = env.get_raw();
    let new_local_ref = unsafe { (**raw_env).NewLocalRef }.ok_or_else(|| {
        AppError::State("Android JNI environment does not provide NewLocalRef".into())
    })?;
    let local_context = unsafe { new_local_ref(raw_env, global_context) };
    if env.exception_check()? {
        let detail = java_exception_message(env)
            .unwrap_or_else(|| "Java exception while copying the Android context".into());
        return Err(AppError::Command(format!(
            "Could not create a local Android context reference: {detail}"
        )));
    }
    if local_context.is_null() {
        return Err(AppError::State(
            "Android JNI NewLocalRef returned no application context".into(),
        ));
    }

    Ok(unsafe { JObject::from_raw(local_context) })
}

fn call<T: DeserializeOwned>(method: &str, value: &impl Serialize) -> AppResult<T> {
    let _bridge_guard = BRIDGE_LOCK
        .lock()
        .map_err(|_| AppError::State("Android VPN bridge lock was poisoned".into()))?;
    let encoded = serde_json::to_string(value)?;
    let android_context = std::panic::catch_unwind(ndk_context::android_context)
        .map_err(|_| AppError::State("Android application context is not initialized".into()))?;
    if android_context.vm().is_null() || android_context.context().is_null() {
        return Err(AppError::State(
            "Android application context is unavailable".into(),
        ));
    }

    let vm = unsafe { JavaVM::from_raw(android_context.vm().cast()) }
        .map_err(|error| AppError::Command(format!("Could not access Android JVM: {error}")))?;
    let mut env = vm.attach_current_thread().map_err(|error| {
        AppError::Command(format!(
            "Could not attach Android VPN bridge thread: {error}"
        ))
    })?;

    env.with_local_frame(16, |env| {
        let context = new_local_context(env, android_context.context().cast())?;

        let loader_result =
            env.call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[]);
        let class_loader = match loader_result {
            Ok(value) => value.l()?,
            Err(error) => return Err(jni_failure(env, method, error)),
        };
        if class_loader.is_null() {
            return Err(AppError::State(
                "Android VPN bridge class loader is unavailable".into(),
            ));
        }

        let class_name = env.new_string(BRIDGE_CLASS)?;
        let class_name_object = JObject::from(class_name);
        let class_result = env.call_method(
            &class_loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&class_name_object)],
        );
        let bridge_class = match class_result {
            Ok(value) => JClass::from(value.l()?),
            Err(error) => return Err(jni_failure(env, method, error)),
        };

        let request = env.new_string(&encoded)?;
        let request_object = JObject::from(request);
        let response_result = env.call_static_method(
            bridge_class,
            method,
            BRIDGE_SIGNATURE,
            &[JValue::Object(&context), JValue::Object(&request_object)],
        );
        let response = match response_result {
            Ok(value) => value.l()?,
            Err(error) => return Err(jni_failure(env, method, error)),
        };
        if response.is_null() {
            return Err(AppError::State(format!(
                "Android VPN bridge method `{method}` returned no response"
            )));
        }

        let response = JString::from(response);
        let response: String = env.get_string(&response)?.into();
        serde_json::from_str(&response).map_err(|error| {
            AppError::State(format!(
                "Android VPN bridge method `{method}` returned invalid JSON: {error}"
            ))
        })
    })
}

fn permission_required(status: &VpnStatus) -> bool {
    status
        .status
        .trim()
        .eq_ignore_ascii_case("permission_required")
        || status
            .error
            .as_deref()
            .is_some_and(|error| error.trim().eq_ignore_ascii_case("permission_required"))
}

fn validate_status(status: VpnStatus) -> AppResult<VpnStatus> {
    if permission_required(&status) {
        return Err(AppError::Command(
            "Android VPN permission is required. Approve the system VPN connection request in Noland Connect, then retry the tunnel operation."
                .into(),
        ));
    }
    if status.ok {
        Ok(status)
    } else {
        Err(AppError::Command(status.error.clone().unwrap_or_else(
            || format!("Android managed VPN operation failed ({})", status.status),
        )))
    }
}

fn validate_runtime(runtime: Runtime) -> AppResult<Runtime> {
    if runtime
        .error
        .as_deref()
        .is_some_and(|error| error.trim().eq_ignore_ascii_case("permission_required"))
    {
        return Err(AppError::Command(
            "Android VPN permission is required. Approve the system VPN connection request in Noland Connect, then retry the tunnel operation."
                .into(),
        ));
    }
    if runtime.ok {
        Ok(runtime)
    } else {
        Err(AppError::State(runtime.error.clone().unwrap_or_else(
            || "Android VpnService runtime is unavailable".into(),
        )))
    }
}

pub fn instance_id(config_path: &Path) -> AppResult<u64> {
    config_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| {
            AppError::State(format!(
                "WireGuard path does not contain an instance identity: {}",
                config_path.display()
            ))
        })
}

pub fn reference(config_path: &Path) -> AppResult<String> {
    Ok(format!("{REFERENCE_PREFIX}{}", instance_id(config_path)?))
}

pub fn install(
    config_path: &Path,
    configuration: &str,
    launch_id: &str,
    fingerprint: &str,
) -> AppResult<VpnStatus> {
    let reference = reference(config_path)?;
    validate_status(call(
        "installAndStart",
        &InstallRequest {
            configuration_reference: &reference,
            configuration,
            operation_id: &uuid::Uuid::new_v4().to_string(),
            instance_id: instance_id(config_path)?,
            launch_id,
            config_fingerprint: fingerprint,
        },
    )?)
}

pub fn reconnect(config_path: &Path) -> AppResult<VpnStatus> {
    let reference = reference(config_path)?;
    validate_status(call(
        "reconnect",
        &ReferenceRequest {
            configuration_reference: &reference,
        },
    )?)
}

pub fn stop(config_path: &Path) -> AppResult<VpnStatus> {
    let reference = reference(config_path)?;
    validate_status(call(
        "stop",
        &ReferenceRequest {
            configuration_reference: &reference,
        },
    )?)
}

pub fn remove(config_path: &Path) -> AppResult<VpnStatus> {
    let reference = reference(config_path)?;
    validate_status(call(
        "remove",
        &ReferenceRequest {
            configuration_reference: &reference,
        },
    )?)
}

pub fn status(config_path: &Path) -> AppResult<VpnStatus> {
    let reference = reference(config_path)?;
    validate_status(call(
        "status",
        &ReferenceRequest {
            configuration_reference: &reference,
        },
    )?)
}

pub fn update(
    config_path: &Path,
    configuration: &str,
    expected: &str,
    next: &str,
    transition_id: uuid::Uuid,
) -> AppResult<Runtime> {
    let reference = reference(config_path)?;
    validate_runtime(call(
        "update",
        &UpdateRequest {
            configuration_reference: &reference,
            configuration,
            expected_config_fingerprint: expected,
            config_fingerprint: next,
            transition_id,
        },
    )?)
}

pub fn runtime(config_path: &Path) -> AppResult<Runtime> {
    let reference = reference(config_path)?;
    validate_runtime(call(
        "runtime",
        &ReferenceRequest {
            configuration_reference: &reference,
        },
    )?)
}

pub fn configuration(config_path: &Path) -> AppResult<String> {
    let reference = reference(config_path)?;
    let response: ConfigurationResponse = call(
        "configuration",
        &ReferenceRequest {
            configuration_reference: &reference,
        },
    )?;
    if !response.ok {
        return Err(AppError::State(response.error.unwrap_or_else(|| {
            "The Android WireGuard configuration is unavailable".into()
        })));
    }
    response
        .configuration
        .ok_or_else(|| AppError::State("The Android WireGuard configuration is empty".into()))
}

#[cfg(test)]
mod tests {
    use super::{validate_status, VpnStatus};

    #[test]
    fn permission_required_is_never_accepted_as_success() {
        let result = validate_status(VpnStatus {
            ok: true,
            status: "permission_required".into(),
            configuration_reference: None,
            error: None,
        });
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Approve the system VPN connection request"));
    }
}
