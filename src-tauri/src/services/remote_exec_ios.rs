//! Mobile adapter retaining the desktop command/event contract over in-process SSH.
use std::{
    collections::HashMap,
    future::Future,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

pub use noland_ssh::ExecOutput;
use noland_ssh::{Connection, Message, TerminalText};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, oneshot};

use crate::errors::{AppError, AppResult};

#[derive(Clone)]
pub struct RemoteExec {
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: u16,
    pub private_key_path: String,
    pub ssh_password: String,
}

// Never include credentials in Debug/log output.
impl std::fmt::Debug for RemoteExec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteExec")
            .field("ssh_user", &self.ssh_user)
            .field("ssh_host", &self.ssh_host)
            .field("ssh_port", &self.ssh_port)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSession {
    pub session_id: String,
    pub ssh_user: String,
    pub ssh_host: String,
    pub ssh_port: u16,
}

enum Control {
    Write(Vec<u8>),
    Resize(u16, u16),
    Close,
}
type Request = (Control, oneshot::Sender<AppResult<()>>);
static SESSIONS: OnceLock<Mutex<HashMap<String, mpsc::Sender<Request>>>> = OnceLock::new();
static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();

fn runtime() -> AppResult<&'static tokio::runtime::Runtime> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("noland-ssh")
                .enable_all()
                .build()
                .map_err(|error| error.to_string())
        })
        .as_ref()
        .map_err(|error| AppError::Command(format!("SSH runtime: {error}")))
}

// The existing orchestration API is synchronous and invoked from blocking tasks.
// Keep SSH I/O on an independent long-lived runtime, including terminal sessions.
fn wait<T: Send + 'static>(
    future: impl Future<Output = AppResult<T>> + Send + 'static,
) -> AppResult<T> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    runtime()?.spawn(async move {
        let _ = tx.send(future.await);
    });
    rx.recv()
        .map_err(|_| AppError::Command("SSH task stopped unexpectedly".into()))?
}

fn sessions() -> AppResult<std::sync::MutexGuard<'static, HashMap<String, mpsc::Sender<Request>>>> {
    SESSIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| AppError::Command("Terminal registry lock was poisoned".into()))
}

fn ssh_error(error: noland_ssh::Error) -> AppError {
    match error {
        noland_ssh::Error::Timeout => {
            AppError::Timeout("SSH operation exceeded its deadline".into())
        }
        other => AppError::Command(other.to_string()),
    }
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

impl RemoteExec {
    pub fn is_root(&self) -> bool {
        self.ssh_user == "root"
    }

    pub fn sudo_prefix(&self) -> String {
        if self.is_root() {
            String::new()
        } else if self.ssh_password.trim().is_empty() {
            "sudo ".into()
        } else {
            format!("printf %s {} | sudo -S -p '' ", quote(&self.ssh_password))
        }
    }

    pub fn sudo_as_user_prefix(&self, target_user: &str) -> String {
        if self.is_root() || self.ssh_password.trim().is_empty() {
            format!("sudo -u {} ", target_user)
        } else {
            format!(
                "printf %s {} | sudo -S -p '' -u {} ",
                quote(&self.ssh_password),
                target_user
            )
        }
    }

    fn connection(&self) -> AppResult<Connection> {
        let key = super::ssh_keys::mobile::load_private_key(Path::new(&self.private_key_path))?;
        let known_hosts = Path::new(&self.private_key_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("known_hosts");
        Ok(Connection {
            user: self.ssh_user.clone(),
            host: self.ssh_host.clone(),
            port: self.ssh_port,
            private_key: Arc::new(key),
            known_hosts,
        })
    }

    pub fn ssh(&self, command: &str, timeout: Duration) -> AppResult<ExecOutput> {
        self.execute(command, None, Some(timeout))
    }

    pub fn ssh_with_stdin(
        &self,
        command: &str,
        input: Vec<u8>,
        timeout: Duration,
    ) -> AppResult<ExecOutput> {
        self.execute(command, Some(input), Some(timeout))
    }

    pub fn ssh_until_complete(&self, command: &str) -> AppResult<ExecOutput> {
        self.execute(command, None, None)
    }

    fn execute(
        &self,
        command: &str,
        input: Option<Vec<u8>>,
        timeout: Option<Duration>,
    ) -> AppResult<ExecOutput> {
        let connection = self.connection()?;
        let command = command.to_owned();
        wait(async move {
            connection
                .exec(&command, input, timeout)
                .await
                .map_err(ssh_error)
        })
    }

    pub fn scp(&self, local: &Path, remote: &str, timeout: Duration) -> AppResult<ExecOutput> {
        self.scp_path(local, remote, false, timeout)
    }

    pub fn scp_path(
        &self,
        local: &Path,
        remote: &str,
        recursive: bool,
        timeout: Duration,
    ) -> AppResult<ExecOutput> {
        let connection = self.connection()?;
        let local = local.to_path_buf();
        let remote = remote.to_owned();
        wait(async move {
            connection
                .upload(&local, &remote, recursive, timeout)
                .await
                .map_err(ssh_error)
        })
    }

    pub fn open_terminal(&self, app: AppHandle) -> AppResult<TerminalSession> {
        let connection = self.connection()?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let id = session_id.clone();
        wait(async move {
            let (session, channel) = connection.terminal(32, 120).await.map_err(ssh_error)?;
            let (sender, mut controls) = mpsc::channel::<Request>(128);
            sessions()?.insert(id.clone(), sender);
            let (mut reader, writer) = channel.split();
            tokio::spawn(async move {
                let mut decoder = TerminalText::default();
                let mut exit_code = None;
                let mut reason = None;
                loop {
                    tokio::select! {
                        message = reader.wait() => match message {
                            Some(Message::Data { data }) | Some(Message::ExtendedData { data, .. }) => {
                                emit_output(&app, &id, decoder.push(&data));
                            }
                            Some(Message::ExitStatus { exit_status }) => exit_code = Some(exit_status),
                            Some(Message::ExitSignal { error_message, .. }) => {
                                exit_code = Some(255); reason = Some(error_message);
                            }
                            Some(Message::Close) | None => break,
                            _ => {}
                        },
                        control = controls.recv() => {
                            let Some((control, reply)) = control else { break; };
                            let closing = matches!(control, Control::Close);
                            let result = tokio::time::timeout(Duration::from_secs(10), async {
                                match control {
                                    Control::Write(bytes) => writer.data(&bytes[..]).await,
                                    Control::Resize(rows, cols) => writer.window_change(cols.max(1).into(), rows.max(1).into(), 0, 0).await,
                                    Control::Close => writer.close().await,
                                }
                            }).await.map_err(|_| AppError::Timeout("Terminal control timed out".into()))
                                .and_then(|result| result.map_err(|error| AppError::Command(error.to_string())));
                            let failed = result.is_err();
                            if let Err(error) = &result { reason = Some(error.to_string()); }
                            let _ = reply.send(result);
                            if closing || failed { break; }
                        }
                    }
                }
                emit_output(&app, &id, decoder.finish());
                if let Ok(mut registry) = sessions() {
                    registry.remove(&id);
                }
                drop(session);
                let _ = app.emit(
                    "remote-terminal-closed",
                    serde_json::json!({
                        "sessionId": id, "exitCode": exit_code, "reason": reason,
                    }),
                );
            });
            Ok(())
        })?;
        Ok(TerminalSession {
            session_id,
            ssh_user: self.ssh_user.clone(),
            ssh_host: self.ssh_host.clone(),
            ssh_port: self.ssh_port,
        })
    }

    pub fn write_terminal(id: &str, input: &str) -> AppResult<()> {
        control(id, Control::Write(input.as_bytes().to_vec()))
    }
    pub fn resize_terminal(id: &str, rows: u16, cols: u16) -> AppResult<()> {
        control(id, Control::Resize(rows, cols))
    }
    pub fn close_terminal(id: &str) -> AppResult<()> {
        control(id, Control::Close)
    }
}

fn emit_output(app: &AppHandle, id: &str, data: String) {
    if !data.is_empty() {
        let _ = app.emit(
            "remote-terminal-output",
            serde_json::json!({ "sessionId": id, "data": data }),
        );
    }
}

fn control(id: &str, control: Control) -> AppResult<()> {
    let closing = matches!(control, Control::Close);
    let sender = {
        let mut registry = sessions()?;
        if closing {
            registry.remove(id)
        } else {
            registry.get(id).cloned()
        }
    };
    let Some(sender) = sender else {
        return if closing {
            Ok(())
        } else {
            Err(AppError::InvalidInput(
                "Terminal session is no longer connected".into(),
            ))
        };
    };
    wait(async move {
        let (tx, rx) = oneshot::channel();
        sender
            .send((control, tx))
            .await
            .map_err(|_| AppError::Command("Terminal session closed".into()))?;
        rx.await
            .map_err(|_| AppError::Command("Terminal session closed".into()))?
    })
}
