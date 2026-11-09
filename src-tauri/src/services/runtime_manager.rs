//! Optional remote Linux runtime. Uses the platform RemoteExec adapter, never
//! a phone-side process/PTY or an on-device compatibility runtime.
use super::remote_exec::RemoteExec;
use crate::errors::{AppError, AppResult};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

const BINARY: &str = "/usr/local/bin/noland-runtime";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub enabled: bool,
    pub release_configured: bool,
    pub details: Option<serde_json::Value>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeApp {
    pub id: String,
    pub name: String,
    pub file: String,
    pub profile: String,
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
fn setting(name: &str, built: Option<&str>) -> Option<String> {
    std::env::var(name)
        .ok()
        .or_else(|| built.map(str::to_owned))
        .filter(|v| !v.trim().is_empty())
}
fn release_settings() -> AppResult<(String, String, String, String)> {
    let missing = || {
        AppError::InvalidInput("No signed Runtime release is configured in this mobile build. Configure the pinned Linux companion URL/SHA256, manifest URL and Ed25519 public key from the no_land runtime-artifacts release; see docs/runtime.md.".into())
    };
    let url = setting(
        "NOLAND_RUNTIME_AGENT_URL",
        option_env!("NOLAND_RUNTIME_AGENT_URL"),
    )
    .ok_or_else(missing)?;
    let digest = setting(
        "NOLAND_RUNTIME_AGENT_SHA256",
        option_env!("NOLAND_RUNTIME_AGENT_SHA256"),
    )
    .ok_or_else(missing)?;
    let manifest = setting(
        "NOLAND_RUNTIME_MANIFEST_URL",
        option_env!("NOLAND_RUNTIME_MANIFEST_URL"),
    )
    .ok_or_else(missing)?;
    let key = setting(
        "NOLAND_RUNTIME_PUBLIC_KEY",
        option_env!("NOLAND_RUNTIME_PUBLIC_KEY"),
    )
    .ok_or_else(missing)?;
    if !url.starts_with("https://")
        || !manifest.starts_with("https://")
        || hex::decode(&digest).map(|v| v.len()).ok() != Some(32)
        || hex::decode(&key).map(|v| v.len()).ok() != Some(32)
    {
        return Err(missing());
    }
    Ok((url, digest.to_lowercase(), manifest, key))
}

pub async fn run(remote: &RemoteExec, user: &str, args: &[&str]) -> AppResult<serde_json::Value> {
    let command = format!(
        "sudo -n -i -u {} -- {} {}",
        quote(user),
        BINARY,
        args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ")
    );
    let remote = remote.clone();
    let output =
        tokio::task::spawn_blocking(move || remote.ssh(&command, Duration::from_secs(240)))
            .await
            .map_err(|e| AppError::Command(e.to_string()))??;
    if output.status_code != 0 {
        return Err(AppError::Command(format!(
            "Runtime companion: {}",
            output.stderr.trim()
        )));
    }
    serde_json::from_str(output.stdout.trim())
        .map_err(|e| AppError::Serialization(format!("Invalid runtime response: {e}")))
}
pub async fn status(remote: &RemoteExec, user: &str) -> AppResult<RuntimeStatus> {
    let probe = remote.clone();
    let output = tokio::task::spawn_blocking(move || {
        probe.ssh(&format!("test -x {BINARY}"), Duration::from_secs(15))
    })
    .await
    .map_err(|e| AppError::Command(e.to_string()))??;
    let enabled = output.status_code == 0;
    Ok(RuntimeStatus {
        enabled,
        release_configured: release_settings().is_ok(),
        details: if enabled {
            Some(run(remote, user, &["status", "--json"]).await?)
        } else {
            None
        },
    })
}
pub async fn apps(remote: &RemoteExec, user: &str) -> AppResult<Vec<RuntimeApp>> {
    let status = status(remote, user).await?;
    serde_json::from_value(
        status
            .details
            .and_then(|d| d.get("apps").cloned())
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .map_err(|e| AppError::Serialization(e.to_string()))
}

pub async fn enable(remote: &RemoteExec, user: &str) -> AppResult<()> {
    if user.is_empty()
        || !user
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
    {
        return Err(AppError::InvalidInput("Invalid desktop user".into()));
    }
    let (url, digest, manifest, key) = release_settings()?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut url = reqwest::Url::parse(&url).map_err(|e| AppError::InvalidInput(e.to_string()))?;
    let mut response = None;
    for _ in 0..6 {
        if url.scheme() != "https" {
            return Err(AppError::InvalidInput(
                "Runtime downloads require HTTPS".into(),
            ));
        }
        let current = client.get(url.clone()).send().await?;
        if current.status().is_redirection() {
            let location = current
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| AppError::Command("Invalid runtime download redirect".into()))?;
            url = url
                .join(location)
                .map_err(|e| AppError::InvalidInput(e.to_string()))?;
        } else {
            response = Some(current.error_for_status()?);
            break;
        }
    }
    let mut response =
        response.ok_or_else(|| AppError::Command("Too many runtime download redirects".into()))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 64 * 1024 * 1024 {
            return Err(AppError::Command(
                "Runtime companion exceeds download size limit".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    if hex::encode(Sha256::digest(&bytes)) != digest {
        return Err(AppError::Command(
            "Runtime companion SHA256 mismatch; remote installation was not changed".into(),
        ));
    }
    let config = STANDARD.encode(
        serde_json::to_vec(&serde_json::json!({"manifestUrl":manifest,"publicKey":key}))
            .map_err(|e| AppError::Serialization(e.to_string()))?,
    );
    let script = format!(
        r#"set -eu
test "$(uname -m)" = x86_64 || {{ echo 'Only x86-64 runtime parents are supported' >&2; exit 1; }}
for command in zenity bwrap xauth curl sudo visudo timeout ldconfig ldd; do command -v "$command" >/dev/null || {{ echo "Missing parent prerequisite: $command (see runtime documentation)" >&2; exit 1; }}; done
test "$(id -u {user})" != 0
work=$(mktemp -d /var/tmp/noland-runtime.XXXXXXXX)
trap 'rm -rf -- "$work"' EXIT
cat > "$work/noland-runtime"
printf '%s  %s\n' {digest} "$work/noland-runtime" | sha256sum --check --status
chmod 755 "$work/noland-runtime"
"$work/noland-runtime" --version >/dev/null
install -d -m 755 /usr/local/libexec /etc/noland-runtime
cat > "$work/session" <<'HELPER'
#!/bin/sh
set -eu
[ "$#" = 0 ] || exit 64
exec /usr/bin/xauth -f /etc/X11/.Xauthority-noland extract - :0
HELPER
chmod 755 "$work/session"
printf '%s ALL=(root) NOPASSWD: /usr/local/libexec/noland-runtime-session ""\n' {user} > "$work/sudoers"
chmod 440 "$work/sudoers"
visudo -cf "$work/sudoers" >/dev/null
printf '%s' {config} | base64 -d > "$work/release.json"
install -o root -g root -m 755 "$work/session" /usr/local/libexec/noland-runtime-session
install -o root -g root -m 440 "$work/sudoers" /etc/sudoers.d/noland-runtime
install -o root -g root -m 644 "$work/release.json" /etc/noland-runtime/release.json
install -o root -g root -m 755 "$work/noland-runtime" /usr/local/bin/noland-runtime.new
mv -f /usr/local/bin/noland-runtime.new /usr/local/bin/noland-runtime
"#,
        user = quote(user),
        digest = quote(&digest),
        config = quote(&config)
    );
    // Do not replace binary stdin with a password pipe. Require root/passwordless SSH.
    let command = format!(
        "{}sh -c {}",
        if remote.is_root() { "" } else { "sudo -n " },
        quote(&script)
    );
    let remote = remote.clone();
    let output = tokio::task::spawn_blocking(move || {
        remote.ssh_with_stdin(&command, bytes, Duration::from_secs(90))
    })
    .await
    .map_err(|e| AppError::Command(e.to_string()))??;
    if output.status_code != 0 {
        return Err(AppError::Command(format!(
            "Runtime enable failed: {}",
            output.stderr
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn quotes_remote_arguments() {
        assert_eq!(
            super::quote("/tmp/a'; $(touch nope)"),
            "'/tmp/a'\"'\"'; $(touch nope)'"
        );
    }
}
