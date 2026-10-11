use noland_ssh::{
    keys::{Algorithm, PrivateKey, PublicKey},
    Connection, Error, Message, TerminalText,
};
use russh::{
    server::{self, ChannelOpenHandle, Msg, Session},
    Channel, ChannelId, Pty,
};
use russh_sftp::protocol::{Attrs, FileAttributes, Handle, OpenFlags, Status, StatusCode, Version};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

#[derive(Default)]
struct Observed {
    pty: Option<(String, u32, u32)>,
    resized: Option<(u32, u32)>,
    exec_count: usize,
    files: HashMap<String, Vec<u8>>,
    directories: Vec<String>,
}

struct Handler {
    expected_key: PublicKey,
    observed: Arc<Mutex<Observed>>,
    channels: HashMap<ChannelId, Channel<Msg>>,
    command: Vec<u8>,
    allow_sftp: bool,
}

impl server::Handler for Handler {
    type Error = russh::Error;
    async fn auth_publickey(
        &mut self,
        _: &str,
        key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if key == &self.expected_key {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }
    async fn exec_request(
        &mut self,
        id: ChannelId,
        command: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.observed.lock().unwrap().exec_count += 1;
        self.command = command.to_vec();
        session.channel_success(id)?;
        if command.starts_with(b"scp -t") {
            let channel = self.channels.remove(&id).unwrap();
            let observed = self.observed.clone();
            tokio::spawn(async move {
                receive_scp(
                    channel.into_stream(),
                    Path::new("/fallback-target"),
                    observed,
                )
                .await;
            });
        } else {
            // Handler callbacks consume ordinary exec data; subsystem protocols
            // need the channel reader and remove it themselves.
            self.channels.remove(&id);
        }
        Ok(())
    }
    async fn data(
        &mut self,
        id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.command.starts_with(b"scp -t") {
            return Ok(());
        }
        session.data(id, data.to_vec())?;
        Ok(())
    }
    async fn channel_eof(
        &mut self,
        id: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.command == b"hang" {
            return Ok(());
        }
        session.extended_data(id, 1, b"stderr preserved".to_vec())?;
        session.eof(id)?;
        session.exit_status_request(id, 7)?;
        session.close(id)?;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        id: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.observed.lock().unwrap().pty = Some((term.to_owned(), cols, rows));
        session.channel_success(id)?;
        Ok(())
    }
    async fn shell_request(
        &mut self,
        id: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&id);
        session.channel_success(id)?;
        Ok(())
    }
    async fn window_change_request(
        &mut self,
        id: ChannelId,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.observed.lock().unwrap().resized = Some((cols, rows));
        session.data(id, b"resized".to_vec())?;
        Ok(())
    }
    async fn subsystem_request(
        &mut self,
        id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        assert_eq!(name, "sftp");
        if !self.allow_sftp {
            session.channel_failure(id)?;
            return Ok(());
        }
        session.channel_success(id)?;
        let channel = self.channels.remove(&id).unwrap();
        let observed = self.observed.clone();
        tokio::spawn(async move {
            russh_sftp::server::run(channel.into_stream(), Sftp(observed)).await;
        });
        Ok(())
    }
}

struct Sftp(Arc<Mutex<Observed>>);
fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: String::new(),
        language_tag: String::new(),
    }
}
impl russh_sftp::server::Handler for Sftp {
    type Error = StatusCode;
    fn unimplemented(&self) -> Self::Error {
        StatusCode::OpUnsupported
    }
    async fn init(&mut self, _: u32, _: HashMap<String, String>) -> Result<Version, Self::Error> {
        Ok(Version::new())
    }
    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, Self::Error> {
        if !self.0.lock().unwrap().directories.contains(&path) {
            return Err(StatusCode::NoSuchFile);
        }
        let mut attrs = FileAttributes::empty();
        attrs.permissions = Some(0o040755);
        Ok(Attrs { id, attrs })
    }
    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _: FileAttributes,
    ) -> Result<Status, Self::Error> {
        self.0.lock().unwrap().directories.push(path);
        Ok(ok(id))
    }
    async fn open(
        &mut self,
        id: u32,
        filename: String,
        flags: OpenFlags,
        _: FileAttributes,
    ) -> Result<Handle, Self::Error> {
        assert!(flags.contains(OpenFlags::WRITE | OpenFlags::CREATE | OpenFlags::TRUNCATE));
        self.0
            .lock()
            .unwrap()
            .files
            .insert(filename.clone(), Vec::new());
        Ok(Handle {
            id,
            handle: filename,
        })
    }
    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, Self::Error> {
        let mut observed = self.0.lock().unwrap();
        let file = observed.files.get_mut(&handle).unwrap();
        let start = offset as usize;
        file.resize(file.len().max(start + data.len()), 0);
        file[start..start + data.len()].copy_from_slice(&data);
        Ok(ok(id))
    }
    async fn close(&mut self, id: u32, _: String) -> Result<Status, Self::Error> {
        Ok(ok(id))
    }
}

async fn fixture() -> (
    Connection,
    Arc<Mutex<Observed>>,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    fixture_with_sftp(true).await
}

async fn fixture_with_sftp(
    allow_sftp: bool,
) -> (
    Connection,
    Arc<Mutex<Observed>>,
    tempfile::TempDir,
    tokio::task::JoinHandle<()>,
) {
    let root = tempfile::tempdir().unwrap();
    let private_key = Arc::new(PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap());
    let expected_key = private_key.public_key().clone();
    let observed = Arc::new(Mutex::new(Observed::default()));
    let server_observed = observed.clone();
    let config = Arc::new(server::Config {
        keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let handler = Handler {
                expected_key: expected_key.clone(),
                observed: server_observed.clone(),
                channels: HashMap::new(),
                command: Vec::new(),
                allow_sftp,
            };
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(session) = server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    (
        Connection {
            user: "noland".into(),
            host: "127.0.0.1".into(),
            port,
            private_key,
            known_hosts: root.path().join("known_hosts"),
        },
        observed,
        root,
        task,
    )
}

async fn receive_scp<S>(mut stream: S, target: &Path, observed: Arc<Mutex<Observed>>)
where
    S: AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    stream.write_all(&[0]).await.unwrap();
    let mut directories = Vec::<String>::new();
    let mut first_entry = true;
    loop {
        let Some(line) = read_protocol_line(&mut stream).await else {
            break;
        };
        if line == "E" {
            directories.pop();
            stream.write_all(&[0]).await.unwrap();
            continue;
        }
        let kind = line.as_bytes()[0];
        let mut fields = line[1..].splitn(3, ' ');
        let _mode = fields.next().unwrap();
        let size = fields.next().unwrap().parse::<usize>().unwrap();
        let name = fields.next().unwrap();
        let path = if first_entry {
            first_entry = false;
            target.to_string_lossy().into_owned()
        } else {
            format!("{}/{}", directories.last().unwrap(), name)
        };
        match kind {
            b'D' => {
                observed.lock().unwrap().directories.push(path.clone());
                directories.push(path);
                stream.write_all(&[0]).await.unwrap();
            }
            b'C' => {
                stream.write_all(&[0]).await.unwrap();
                let mut contents = vec![0; size];
                stream.read_exact(&mut contents).await.unwrap();
                let mut terminator = [0];
                stream.read_exact(&mut terminator).await.unwrap();
                assert_eq!(terminator, [0]);
                observed.lock().unwrap().files.insert(path, contents);
                stream.write_all(&[0]).await.unwrap();
            }
            _ => panic!("unexpected SCP command: {line}"),
        }
    }
}

async fn read_protocol_line<S: AsyncRead + Unpin>(stream: &mut S) -> Option<String> {
    let mut line = Vec::new();
    loop {
        let mut byte = [0];
        match stream.read_exact(&mut byte).await {
            Ok(_) if byte[0] == b'\n' => return Some(String::from_utf8(line).unwrap()),
            Ok(_) => line.push(byte[0]),
            Err(_) => return None,
        }
    }
}

#[tokio::test]
async fn duplex_stdin_stdout_and_exit_status_after_eof() {
    let (connection, observed, _root, server) = fixture().await;
    let payload = "input 'quoted' 🐕\n".repeat(160_000); // exceeds an SSH channel window
    let output = connection
        .exec(
            "echo",
            Some(payload.clone().into_bytes()),
            Some(Duration::from_secs(15)),
        )
        .await
        .unwrap();
    assert_eq!(output.stdout, payload);
    assert_eq!(output.stderr, "stderr preserved");
    assert_eq!(output.status_code, 7);
    assert_eq!(observed.lock().unwrap().exec_count, 1);
    assert!(connection.known_hosts.exists());
    server.abort();
}

#[tokio::test]
async fn timeout_does_not_replay_command() {
    let (connection, observed, _root, server) = fixture().await;
    let output = connection
        .exec("hang", None, Some(Duration::from_millis(500)))
        .await;
    assert!(matches!(output, Err(Error::Timeout)));
    assert_eq!(observed.lock().unwrap().exec_count, 1);
    server.abort();
}

#[tokio::test]
async fn rejects_changed_host_key_and_wrong_client_key() {
    let (mut connection, _, _root, server) = fixture().await;
    let session = connection.connect().await.unwrap();
    drop(session);
    connection.private_key =
        Arc::new(PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap());
    assert!(matches!(
        connection.connect().await,
        Err(Error::Authentication)
    ));
    std::fs::remove_file(&connection.known_hosts).unwrap();
    let wrong_host = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    noland_ssh::keys::known_hosts::learn_known_hosts_path(
        &connection.host,
        connection.port,
        wrong_host.public_key(),
        &connection.known_hosts,
    )
    .unwrap();
    assert!(matches!(connection.connect().await, Err(Error::Ssh(_))));
    server.abort();
}

#[tokio::test]
async fn remote_pty_input_resize_and_exit() {
    let (connection, observed, _root, server) = fixture().await;
    let (_session, mut channel) = connection.terminal(32, 120).await.unwrap();
    assert_eq!(
        observed.lock().unwrap().pty,
        Some(("xterm-256color".into(), 120, 32))
    );
    channel.data(&b"hello\n"[..]).await.unwrap();
    assert!(
        matches!(channel.wait().await, Some(Message::Data { data }) if &data[..] == b"hello\n")
    );
    channel.window_change(80, 24, 0, 0).await.unwrap();
    assert!(
        matches!(channel.wait().await, Some(Message::Data { data }) if &data[..] == b"resized")
    );
    assert_eq!(observed.lock().unwrap().resized, Some((80, 24)));
    channel.eof().await.unwrap();
    let mut exit = None;
    while let Some(message) = channel.wait().await {
        match message {
            Message::ExitStatus { exit_status } => exit = Some(exit_status),
            Message::Close => break,
            _ => {}
        }
    }
    assert_eq!(exit, Some(7));
    server.abort();
}

#[tokio::test]
async fn recursive_upload_preserves_contents_and_exact_target_paths() {
    let (connection, observed, root, server) = fixture().await;
    let source = root.path().join("source");
    std::fs::create_dir_all(source.join("nested empty")).unwrap();
    std::fs::write(source.join("quote ' 🐕.txt"), b"unchanged bytes\0\xff").unwrap();
    let large = vec![123; 200_000];
    std::fs::write(source.join("large"), &large).unwrap();
    connection
        .upload(&source, "/target", true, Duration::from_secs(10))
        .await
        .unwrap();
    let state = observed.lock().unwrap();
    assert!(state.directories.contains(&"/target/nested empty".into()));
    assert_eq!(
        state.files["/target/quote ' 🐕.txt"],
        b"unchanged bytes\0\xff"
    );
    assert_eq!(state.files["/target/large"], large);
    server.abort();
}

#[tokio::test]
async fn falls_back_to_streaming_scp_when_sftp_is_rejected() {
    let (connection, observed, root, server) = fixture_with_sftp(false).await;
    let source = root.path().join("source");
    std::fs::create_dir_all(source.join("nested")).unwrap();
    std::fs::write(source.join("root.txt"), b"root bytes").unwrap();
    std::fs::write(source.join("nested/child.bin"), b"child\0bytes").unwrap();

    let result = connection
        .upload(&source, "/fallback-target", true, Duration::from_secs(10))
        .await
        .unwrap();

    assert!(result.command.starts_with("scp-protocol "));
    let state = observed.lock().unwrap();
    assert!(state.directories.contains(&"/fallback-target".into()));
    assert!(state
        .directories
        .contains(&"/fallback-target/nested".into()));
    assert_eq!(state.files["/fallback-target/root.txt"], b"root bytes");
    assert_eq!(
        state.files["/fallback-target/nested/child.bin"],
        b"child\0bytes"
    );
    server.abort();
}

#[test]
fn terminal_utf8_survives_every_packet_boundary_and_invalid_bytes() {
    let text = "hello 🐕 日本語\r\n";
    for split in 0..=text.len() {
        let mut decoder = TerminalText::default();
        let result = decoder.push(&text.as_bytes()[..split])
            + &decoder.push(&text.as_bytes()[split..])
            + &decoder.finish();
        assert_eq!(result, text);
    }
    let mut decoder = TerminalText::default();
    assert_eq!(decoder.push(b"\xffa\xe2"), "�a");
    assert_eq!(decoder.finish(), "�");
}
