//! In-process SSH transport for sandboxed clients. No subprocesses or local PTYs.
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use russh::{
    client,
    keys::{known_hosts, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
    Channel, ChannelMsg, Disconnect,
};
use russh_sftp::{
    client::SftpSession,
    protocol::{FileAttributes, OpenFlags},
};
use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use zeroize::Zeroizing;

pub use russh::keys;
pub type Session = client::Handle<PinnedHost>;
pub type SessionChannel = Channel<client::Msg>;
pub use russh::ChannelMsg as Message;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SSH operation timed out")]
    Timeout,
    #[error("SSH authentication was rejected")]
    Authentication,
    #[error("SSH failure: {0}")]
    Ssh(#[from] russh::Error),
    #[error("SSH key failure: {0}")]
    Key(#[from] russh::keys::Error),
    #[error("File transfer failure: {0}")]
    Sftp(#[from] russh_sftp::client::error::Error),
    #[error("I/O failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("SSH server rejected the {0} channel request")]
    ChannelRejected(&'static str),
    #[error("{0}")]
    InvalidInput(String),
}

#[derive(Clone)]
pub struct Connection {
    pub user: String,
    pub host: String,
    pub port: u16,
    pub private_key: Arc<keys::PrivateKey>,
    pub known_hosts: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecOutput {
    pub command: String,
    pub status_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u128,
}

static HOST_KEY_GATE: Mutex<()> = Mutex::new(());

pub struct PinnedHost {
    host: String,
    port: u16,
    path: PathBuf,
}

impl client::Handler for PinnedHost {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key_or_certificate: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = key_or_certificate.public_key();
        // Serialize TOFU enrollment, including concurrent provisioning sessions.
        let _guard = HOST_KEY_GATE
            .lock()
            .map_err(|_| russh::Error::Inconsistent)?;
        match known_hosts::check_known_hosts_path(&self.host, self.port, &key, &self.path) {
            Ok(true) => Ok(true),
            Ok(false) => {
                known_hosts::learn_known_hosts_path(&self.host, self.port, &key, &self.path)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600))?;
                }
                Ok(true)
            }
            Err(error) => Err(error.into()),
        }
    }
}

impl Connection {
    pub async fn connect(&self) -> Result<Session, Error> {
        tokio::time::timeout(Duration::from_secs(10), async {
            let config = Arc::new(client::Config {
                inactivity_timeout: None,
                keepalive_interval: Some(Duration::from_secs(30)),
                keepalive_max: 3,
                nodelay: true,
                ..Default::default()
            });
            if let Some(parent) = self.known_hosts.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            let mut session = client::connect(
                config,
                (self.host.as_str(), self.port),
                PinnedHost {
                    host: self.host.clone(),
                    port: self.port,
                    path: self.known_hosts.clone(),
                },
            )
            .await?;
            let hash = session.best_supported_rsa_hash().await?.flatten();
            let auth = session
                .authenticate_publickey(
                    &self.user,
                    PrivateKeyWithHashAlg::new(self.private_key.clone(), hash),
                )
                .await?;
            if !auth.success() {
                return Err(Error::Authentication);
            }
            Ok(session)
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    /// Apply one deadline to connect, authenticate, stdin, and command completion.
    /// Never replay a command after dispatch: a dropped reply may follow a committed mutation.
    pub async fn exec(
        &self,
        command: &str,
        stdin: Option<Vec<u8>>,
        limit: Option<Duration>,
    ) -> Result<ExecOutput, Error> {
        let started = Instant::now();
        let operation = async {
            let session = self.connect().await?;
            let result = async {
                let mut channel = session.channel_open_session().await?;
                channel.exec(true, command).await?;
                expect_success(&mut channel, "command execution").await?;
                let (mut reader, writer) = channel.split();
                // Read output while writing input: commands may fill their output
                // window before consuming all stdin (clipboard and storage payloads).
                let write_input = async {
                    if let Some(bytes) = stdin {
                        let bytes = Zeroizing::new(bytes);
                        writer.data(&bytes[..]).await?;
                    }
                    writer.eof().await?;
                    Ok::<_, Error>(())
                };
                let read_output = async {
                    let mut stdout = Vec::new();
                    let mut stderr = Vec::new();
                    let mut status = -1;
                    while let Some(message) = reader.wait().await {
                        match message {
                            ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                            ChannelMsg::ExtendedData { data, .. } => {
                                stderr.extend_from_slice(&data)
                            }
                            ChannelMsg::ExitStatus { exit_status } => status = exit_status as i32,
                            ChannelMsg::ExitSignal { .. } => status = 255,
                            ChannelMsg::Close => break,
                            // EOF can precede ExitStatus. Continue draining.
                            _ => {}
                        }
                    }
                    Ok::<_, Error>((status, stdout, stderr))
                };
                let (_, (status_code, stdout, stderr)) =
                    tokio::try_join!(write_input, read_output)?;
                Ok(ExecOutput {
                    command: format!(
                        "ssh {}@{}:{} <remote-command>",
                        self.user, self.host, self.port
                    ),
                    status_code,
                    stdout: String::from_utf8_lossy(&stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr).into_owned(),
                    duration_ms: started.elapsed().as_millis(),
                })
            }
            .await;
            let _ = session
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
            result
        };
        match limit {
            Some(limit) => tokio::time::timeout(limit, operation)
                .await
                .map_err(|_| Error::Timeout)?,
            None => operation.await,
        }
    }

    pub async fn terminal(&self, rows: u16, cols: u16) -> Result<(Session, SessionChannel), Error> {
        tokio::time::timeout(Duration::from_secs(15), async {
            let session = self.connect().await?;
            let mut channel = session.channel_open_session().await?;
            channel
                .request_pty(
                    true,
                    "xterm-256color",
                    cols.max(1).into(),
                    rows.max(1).into(),
                    0,
                    0,
                    &[],
                )
                .await?;
            expect_success(&mut channel, "PTY").await?;
            channel.request_shell(true).await?;
            expect_success(&mut channel, "interactive shell").await?;
            Ok((session, channel))
        })
        .await
        .map_err(|_| Error::Timeout)?
    }

    /// The destination is the exact target path, matching the existing upload API.
    pub async fn upload(
        &self,
        source: &Path,
        destination: &str,
        recursive: bool,
        limit: Duration,
    ) -> Result<ExecOutput, Error> {
        let started = Instant::now();
        tokio::time::timeout(limit, async {
            let session = self.connect().await?;
            let result = async {
                let mut channel = session.channel_open_session().await?;
                channel.request_subsystem(true, "sftp").await?;
                match expect_success(&mut channel, "SFTP subsystem").await {
                    Ok(()) => {
                        let sftp = SftpSession::new(channel.into_stream()).await?;
                        let result = upload_tree(&sftp, source, destination, recursive).await;
                        let close_result = sftp.close().await;
                        result?;
                        close_result?;
                        Ok(ExecOutput {
                            command: format!(
                                "sftp {}@{}:{} <upload>",
                                self.user, self.host, self.port
                            ),
                            status_code: 0,
                            stdout: String::new(),
                            stderr: String::new(),
                            duration_ms: started.elapsed().as_millis(),
                        })
                    }
                    Err(Error::ChannelRejected("SFTP subsystem")) => {
                        warn!(
                            host = %self.host,
                            port = self.port,
                            "SSH server rejected SFTP; retrying upload with streaming SCP"
                        );
                        upload_with_scp(&session, source, destination, recursive, started, self)
                            .await
                    }
                    Err(error) => Err(error),
                }
            }
            .await;
            let _ = session
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
            result
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

async fn expect_success(channel: &mut SessionChannel, request: &'static str) -> Result<(), Error> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Failure) => return Err(Error::ChannelRejected(request)),
            Some(ChannelMsg::Close) | None => {
                return Err(Error::InvalidInput(format!(
                    "SSH channel closed before the {request} request was acknowledged"
                )))
            }
            _ => {}
        }
    }
}

enum ScpWork {
    Path {
        local: PathBuf,
        ancestors: Vec<PathBuf>,
    },
    EndDirectory,
}

struct ScpSink {
    channel: SessionChannel,
    pending: VecDeque<u8>,
    stderr: Vec<u8>,
    exit_status: Option<i32>,
}

impl ScpSink {
    fn new(channel: SessionChannel) -> Self {
        Self {
            channel,
            pending: VecDeque::new(),
            stderr: Vec::new(),
            exit_status: None,
        }
    }

    async fn send(&self, bytes: &[u8]) -> Result<(), Error> {
        self.channel.data(bytes).await?;
        Ok(())
    }

    async fn next_byte(&mut self) -> Result<u8, Error> {
        loop {
            if let Some(byte) = self.pending.pop_front() {
                return Ok(byte);
            }
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => self.pending.extend(data.iter().copied()),
                Some(ChannelMsg::ExtendedData { data, .. }) => self.stderr.extend_from_slice(&data),
                Some(ChannelMsg::ExitStatus { exit_status }) => {
                    self.exit_status = Some(exit_status as i32)
                }
                Some(ChannelMsg::ExitSignal { .. }) => self.exit_status = Some(255),
                Some(ChannelMsg::Close) | None => {
                    return Err(Error::InvalidInput(format!(
                        "Remote SCP process closed before acknowledging the upload{}",
                        scp_stderr_suffix(&self.stderr)
                    )))
                }
                _ => {}
            }
        }
    }

    async fn read_ack(&mut self) -> Result<(), Error> {
        let status = self.next_byte().await?;
        if status == 0 {
            return Ok(());
        }
        let mut message = Vec::new();
        loop {
            let byte = self.next_byte().await?;
            if byte == b'\n' {
                break;
            }
            message.push(byte);
            if message.len() >= 16 * 1024 {
                break;
            }
        }
        let message = String::from_utf8_lossy(&message);
        Err(Error::InvalidInput(format!(
            "Remote SCP rejected the upload (status {status}): {message}{}",
            scp_stderr_suffix(&self.stderr)
        )))
    }
}

async fn upload_with_scp(
    session: &Session,
    source: &Path,
    destination: &str,
    recursive: bool,
    started: Instant,
    connection: &Connection,
) -> Result<ExecOutput, Error> {
    if destination.starts_with('-') || destination.contains('\0') || destination.contains('\n') {
        return Err(Error::InvalidInput(
            "Remote upload destination is not safe for the SCP protocol".into(),
        ));
    }
    let channel = session.channel_open_session().await?;
    let recursive_flag = if recursive { " -r" } else { "" };
    let command = format!("scp -t{recursive_flag} {}", shell_quote(destination));
    // The first SCP protocol acknowledgement is sufficient proof that the
    // remote command started. Avoid requesting a separate SSH acknowledgement:
    // some servers can emit SCP's initial NUL before the channel-success packet,
    // which would otherwise be consumed while waiting for channel success.
    channel.exec(false, command).await?;
    let mut sink = ScpSink::new(channel);
    sink.read_ack().await?;
    info!(
        host = %connection.host,
        port = connection.port,
        recursive,
        "Remote SCP sink accepted the upload"
    );

    let mut pending = vec![ScpWork::Path {
        local: source.to_path_buf(),
        ancestors: Vec::new(),
    }];
    while let Some(work) = pending.pop() {
        match work {
            ScpWork::EndDirectory => {
                sink.send(b"E\n").await?;
                sink.read_ack().await?;
            }
            ScpWork::Path {
                local,
                mut ancestors,
            } => {
                let canonical = tokio::fs::canonicalize(&local).await?;
                let metadata = tokio::fs::metadata(&local).await?;
                let name = scp_file_name(&local)?;
                #[cfg(unix)]
                let mode = {
                    use std::os::unix::fs::PermissionsExt;
                    metadata.permissions().mode() & 0o777
                };
                #[cfg(not(unix))]
                let mode = if metadata.is_dir() { 0o755 } else { 0o644 };

                if metadata.is_dir() {
                    if !recursive {
                        return Err(Error::InvalidInput(
                            "Directory upload requires recursive mode".into(),
                        ));
                    }
                    if ancestors.contains(&canonical) {
                        return Err(Error::InvalidInput(
                            "Upload contains a directory symlink cycle".into(),
                        ));
                    }
                    ancestors.push(canonical);
                    sink.send(format!("D{mode:04o} 0 {name}\n").as_bytes())
                        .await?;
                    sink.read_ack().await?;
                    pending.push(ScpWork::EndDirectory);
                    let mut children = tokio::fs::read_dir(&local).await?;
                    let mut entries = Vec::new();
                    while let Some(child) = children.next_entry().await? {
                        entries.push(child.path());
                    }
                    for child in entries.into_iter().rev() {
                        pending.push(ScpWork::Path {
                            local: child,
                            ancestors: ancestors.clone(),
                        });
                    }
                } else if metadata.is_file() {
                    sink.send(format!("C{mode:04o} {} {name}\n", metadata.len()).as_bytes())
                        .await?;
                    sink.read_ack().await?;
                    let mut input = tokio::fs::File::open(&local).await?;
                    let mut buffer = vec![0u8; 64 * 1024];
                    loop {
                        use tokio::io::AsyncReadExt;
                        let count = input.read(&mut buffer).await?;
                        if count == 0 {
                            break;
                        }
                        sink.send(&buffer[..count]).await?;
                    }
                    sink.send(&[0]).await?;
                    sink.read_ack().await?;
                } else {
                    return Err(Error::InvalidInput(
                        "Upload source is not a regular file or directory".into(),
                    ));
                }
            }
        }
    }

    sink.channel.eof().await?;
    let mut stdout = Vec::new();
    while let Some(message) = sink.channel.wait().await {
        match message {
            ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
            ChannelMsg::ExtendedData { data, .. } => sink.stderr.extend_from_slice(&data),
            ChannelMsg::ExitStatus { exit_status } => sink.exit_status = Some(exit_status as i32),
            ChannelMsg::ExitSignal { .. } => sink.exit_status = Some(255),
            ChannelMsg::Close => break,
            _ => {}
        }
    }
    let status_code = sink.exit_status.unwrap_or(0);
    info!(
        host = %connection.host,
        port = connection.port,
        status_code,
        duration_ms = started.elapsed().as_millis(),
        "Streaming SCP upload finished"
    );
    Ok(ExecOutput {
        command: format!(
            "scp-protocol {}@{}:{} <upload>",
            connection.user, connection.host, connection.port
        ),
        status_code,
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&sink.stderr).into_owned(),
        duration_ms: started.elapsed().as_millis(),
    })
}

fn scp_file_name(path: &Path) -> Result<String, Error> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::InvalidInput("Upload filename is not valid UTF-8".into()))?;
    if name.is_empty() || name.contains('\0') || name.contains('\n') || name.contains('/') {
        return Err(Error::InvalidInput(
            "Upload filename is not safe for the SCP protocol".into(),
        ));
    }
    Ok(name.to_string())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn scp_stderr_suffix(stderr: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!("; stderr: {stderr}")
    }
}

async fn upload_tree(
    sftp: &SftpSession,
    source: &Path,
    destination: &str,
    recursive: bool,
) -> Result<(), Error> {
    let mut pending = vec![(
        source.to_path_buf(),
        destination.to_owned(),
        Vec::<PathBuf>::new(),
    )];
    while let Some((local, remote, mut ancestors)) = pending.pop() {
        // OpenSSH scp follows local symlinks. Detect ancestor cycles without
        // rejecting independent links to the same file/directory.
        let canonical = tokio::fs::canonicalize(&local).await?;
        let metadata = tokio::fs::metadata(&local).await?;
        if metadata.is_dir() {
            if !recursive {
                return Err(Error::InvalidInput(
                    "Directory upload requires recursive mode".into(),
                ));
            }
            if ancestors.contains(&canonical) {
                return Err(Error::InvalidInput(
                    "Upload contains a directory symlink cycle".into(),
                ));
            }
            ancestors.push(canonical);
            match sftp.metadata(&remote).await {
                Ok(attrs) if attrs.is_dir() => {}
                Ok(_) => {
                    return Err(Error::InvalidInput(
                        "Remote directory target is a file".into(),
                    ))
                }
                Err(russh_sftp::client::error::Error::Status(status))
                    if status.status_code == russh_sftp::protocol::StatusCode::NoSuchFile =>
                {
                    sftp.create_dir(&remote).await?
                }
                Err(error) => return Err(error.into()),
            }
            let mut children = tokio::fs::read_dir(&local).await?;
            while let Some(child) = children.next_entry().await? {
                let name = child
                    .file_name()
                    .into_string()
                    .map_err(|_| Error::InvalidInput("Upload filename is not UTF-8".into()))?;
                pending.push((
                    child.path(),
                    format!("{}/{name}", remote.trim_end_matches('/')),
                    ancestors.clone(),
                ));
            }
        } else if metadata.is_file() {
            let mut input = tokio::fs::File::open(&local).await?;
            let mut attrs = FileAttributes::empty();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                attrs.permissions = Some(metadata.permissions().mode() & 0o777);
            }
            let mut output = sftp
                .open_with_flags_and_attributes(
                    &remote,
                    OpenFlags::CREATE | OpenFlags::TRUNCATE | OpenFlags::WRITE,
                    attrs,
                )
                .await?;
            tokio::io::copy(&mut input, &mut output).await?;
            output.shutdown().await?;
        } else {
            return Err(Error::InvalidInput(
                "Upload source is not a regular file or directory".into(),
            ));
        }
    }
    Ok(())
}

/// Streaming UTF-8 decoder: preserve characters split across SSH packets.
#[derive(Default)]
pub struct TerminalText {
    pending: Vec<u8>,
}

impl TerminalText {
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut output = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(text) => {
                    output.push_str(text);
                    self.pending.clear();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    output.push_str(
                        std::str::from_utf8(&self.pending[..valid]).expect("validated prefix"),
                    );
                    self.pending.drain(..valid);
                    match error.error_len() {
                        Some(length) => {
                            output.push('\u{fffd}');
                            self.pending.drain(..length);
                        }
                        None => break,
                    }
                }
            }
        }
        output
    }

    pub fn finish(&mut self) -> String {
        let tail = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        tail
    }
}
