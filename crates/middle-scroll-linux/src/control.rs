//! Control socket: lets `--status`, `--pause`, `--resume`, `--toggle`,
//! `--reload` and `--tune` talk to the running daemon.
//!
//! The protocol is one line of text per connection: the client writes a
//! command, the daemon answers with a reply starting with `ok` or `error` and
//! closes the connection. A background thread accepts connections and hands
//! each command to the daemon loop through a channel, waking the loop with a
//! byte on its wake pipe, so commands are handled between input frames and
//! never race with the engine.

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Duration;

use nix::unistd::{geteuid, Uid};
use tracing::{debug, warn};

/// Socket directory and file names, under `$XDG_RUNTIME_DIR` (or `/run` for
/// root). The systemd unit sets `RuntimeDirectory=` to the same directory.
const SOCKET_DIR: &str = "wayland-wheeltani";
const SOCKET_NAME: &str = "control.sock";

/// Longest command line accepted from a client.
const MAX_REQUEST_BYTES: u64 = 4096;
/// How long a client may take to send its command.
const READ_TIMEOUT: Duration = Duration::from_secs(2);
/// How long the accept thread waits for the daemon loop to answer.
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a client waits for the daemon's answer.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// One command received on the socket, with the channel to answer it on.
#[derive(Debug)]
pub struct Request {
    pub line: String,
    reply: mpsc::Sender<String>,
}

impl Request {
    /// Sends the reply. The client may have gone away; that is not an error.
    pub fn respond(self, reply: String) {
        let _ = self.reply.send(reply);
    }
}

/// A command understood by the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Ping,
    Status,
    Pause,
    Resume,
    Toggle,
    Reload,
    /// Print the live engine settings, or one of them.
    Get(Option<String>),
    /// Change one engine setting live (`--tune`).
    Set {
        key: String,
        value: String,
    },
    /// Write the settings changed with `set` to the config file.
    Save,
}

impl Command {
    /// Parses a command line. `set` accepts `set key value`, `set key = value`
    /// and `set key=value`.
    pub fn parse(line: &str) -> Result<Self, String> {
        let line = line.trim();
        let (word, rest) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(w, r)| (w, r.trim()));
        let no_argument = |cmd: Self| {
            if rest.is_empty() {
                Ok(cmd)
            } else {
                Err(format!("`{word}` takes no argument"))
            }
        };
        match word {
            "ping" => no_argument(Self::Ping),
            "status" => no_argument(Self::Status),
            "pause" => no_argument(Self::Pause),
            "resume" => no_argument(Self::Resume),
            "toggle" => no_argument(Self::Toggle),
            "reload" => no_argument(Self::Reload),
            "save" => no_argument(Self::Save),
            "get" => Ok(Self::Get((!rest.is_empty()).then(|| rest.to_owned()))),
            "set" => {
                let (key, value) = rest
                    .split_once('=')
                    .or_else(|| rest.split_once(char::is_whitespace))
                    .map(|(k, v)| (k.trim(), v.trim()))
                    .filter(|(k, v)| !k.is_empty() && !v.is_empty())
                    .ok_or_else(|| "usage: set <key> <value>".to_owned())?;
                Ok(Self::Set {
                    key: key.to_owned(),
                    value: value.to_owned(),
                })
            }
            "" => Err("empty command".to_owned()),
            other => Err(format!("unknown command `{other}`")),
        }
    }
}

/// Path of the control socket for the current user.
pub fn socket_path() -> Option<PathBuf> {
    socket_path_for(
        geteuid(),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
    )
}

fn socket_path_for(uid: Uid, runtime_dir: Option<PathBuf>) -> Option<PathBuf> {
    let base = if uid.is_root() {
        PathBuf::from("/run")
    } else {
        runtime_dir.filter(|dir| dir.is_absolute()).or_else(|| {
            let dir = PathBuf::from(format!("/run/user/{uid}"));
            dir.is_dir().then_some(dir)
        })?
    };
    Some(base.join(SOCKET_DIR).join(SOCKET_NAME))
}

/// Sockets a client should try, in order. Under `sudo` the daemon usually
/// runs as the invoking user, so that user's socket is tried first.
fn client_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::with_capacity(2);
    if geteuid().is_root() {
        if let Some(uid) = std::env::var("SUDO_UID")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
        {
            if let Some(path) = socket_path_for(Uid::from_raw(uid), None) {
                candidates.push(path);
            }
        }
    }
    if let Some(path) = socket_path() {
        if !candidates.contains(&path) {
            candidates.push(path);
        }
    }
    candidates
}

/// Error of [`send`].
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error(
        "no running daemon found (no control socket at {0}); start it with \
         `wayland-wheeltani --start` or check `control_socket` in the config"
    )]
    NotRunning(String),
    #[error("failed to talk to the daemon on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Sends `command` to the running daemon and returns its reply.
pub fn send(command: &str) -> Result<String, ClientError> {
    let candidates = client_candidates();
    let mut last_missing = None;
    for path in &candidates {
        match UnixStream::connect(path) {
            Ok(stream) => {
                return exchange(stream, command).map_err(|source| ClientError::Io {
                    path: path.clone(),
                    source,
                })
            }
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                last_missing = Some(path.display().to_string());
            }
            Err(source) => {
                return Err(ClientError::Io {
                    path: path.clone(),
                    source,
                })
            }
        }
    }
    Err(ClientError::NotRunning(last_missing.unwrap_or_else(|| {
        "$XDG_RUNTIME_DIR/wayland-wheeltani".to_owned()
    })))
}

fn exchange(mut stream: UnixStream, command: &str) -> io::Result<String> {
    stream.set_read_timeout(Some(CLIENT_TIMEOUT))?;
    stream.set_write_timeout(Some(CLIENT_TIMEOUT))?;
    stream.write_all(command.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply.trim_end().to_owned())
}

/// Splits a reply into success and message.
pub fn split_reply(reply: &str) -> (bool, &str) {
    if let Some(rest) = reply.strip_prefix("ok") {
        return (true, rest.trim_start());
    }
    let message = reply
        .strip_prefix("error")
        .map_or(reply, |rest| rest.trim_start_matches(':').trim_start());
    (false, message)
}

/// Error of [`ControlServer::start`].
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("another instance is already listening on {0}")]
    AlreadyRunning(PathBuf),
    #[error("control socket directory {0} is not private to this user")]
    UnsafeDirectory(PathBuf),
    #[error("cannot create the control socket {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// The listening side, owned by the daemon loop.
#[derive(Debug)]
pub struct ControlServer {
    path: PathBuf,
    requests: mpsc::Receiver<Request>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ControlServer {
    /// Binds the socket at `path` and starts the accept thread. Each request
    /// writes one byte to `waker` so the daemon loop wakes up to handle it.
    pub fn start(path: PathBuf, waker: UnixStream) -> Result<Self, ServerError> {
        let io_err = |source| ServerError::Io {
            path: path.clone(),
            source,
        };
        let dir = path
            .parent()
            .ok_or_else(|| io_err(io::ErrorKind::InvalidInput.into()))?;
        prepare_directory(dir)?;

        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_socket() => {
                if UnixStream::connect(&path).is_ok() {
                    return Err(ServerError::AlreadyRunning(path));
                }
                debug!(path = %path.display(), "removing stale control socket");
                std::fs::remove_file(&path).map_err(io_err)?;
            }
            Ok(_) => {
                return Err(io_err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "a file that is not a socket is in the way",
                )))
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(io_err(err)),
        }

        let listener = UnixListener::bind(&path).map_err(io_err)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(io_err)?;

        let (tx, requests) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("control".to_owned())
            .spawn(move || accept_loop(&listener, &tx, &waker, &thread_stop))
            .map_err(io_err)?;

        Ok(Self {
            path,
            requests,
            stop,
            thread: Some(thread),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the next pending request without blocking.
    pub fn try_next(&self) -> Option<Request> {
        self.requests.try_recv().ok()
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        while let Ok(request) = self.requests.try_recv() {
            request.respond("error: the daemon is shutting down".to_owned());
        }
        // Unblock `accept()` so the thread sees the flag and exits.
        let _ = UnixStream::connect(&self.path);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Creates the socket directory with mode 0700, or checks that an existing
/// one belongs to this user and is not reachable by others.
fn prepare_directory(dir: &Path) -> Result<(), ServerError> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => return Ok(()),
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(source) => {
            return Err(ServerError::Io {
                path: dir.to_path_buf(),
                source,
            })
        }
    }
    let meta = std::fs::symlink_metadata(dir).map_err(|source| ServerError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    if !meta.is_dir() || meta.uid() != geteuid().as_raw() || meta.mode() & 0o022 != 0 {
        return Err(ServerError::UnsafeDirectory(dir.to_path_buf()));
    }
    Ok(())
}

fn accept_loop(
    listener: &UnixListener,
    requests: &mpsc::Sender<Request>,
    waker: &UnixStream,
    stop: &AtomicBool,
) {
    for stream in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        match stream {
            Ok(stream) => {
                if let Err(err) = serve(&stream, requests, waker) {
                    debug!(%err, "control client failed");
                }
            }
            Err(err) => {
                warn!(%err, "control socket accept failed");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn serve(
    stream: &UnixStream,
    requests: &mpsc::Sender<Request>,
    mut waker: &UnixStream,
) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    stream.set_write_timeout(Some(READ_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REQUEST_BYTES)).read_line(&mut line)?;
    if line.trim().is_empty() {
        // A bare connect, like the "is a daemon already running?" probe of a
        // second instance: nothing to answer.
        return Ok(());
    }

    let (reply_tx, reply_rx) = mpsc::channel();
    let request = Request {
        line: line.trim().to_owned(),
        reply: reply_tx,
    };
    let reply = if requests.send(request).is_ok() {
        // The pipe is non-blocking: if it is full the loop is already awake.
        let _ = waker.write(&[1]);
        reply_rx
            .recv_timeout(REPLY_TIMEOUT)
            .unwrap_or_else(|_| "error: the daemon did not answer".to_owned())
    } else {
        "error: the daemon is shutting down".to_owned()
    };
    let mut stream = stream;
    stream.write_all(reply.as_bytes())?;
    stream.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_parse() {
        assert_eq!(Command::parse(" status "), Ok(Command::Status));
        assert_eq!(Command::parse("get"), Ok(Command::Get(None)));
        assert_eq!(
            Command::parse("get deadzone_units"),
            Ok(Command::Get(Some("deadzone_units".to_owned())))
        );
        for line in ["set tick_hz 240", "set tick_hz = 240", "set tick_hz=240"] {
            assert_eq!(
                Command::parse(line),
                Ok(Command::Set {
                    key: "tick_hz".to_owned(),
                    value: "240".to_owned(),
                }),
                "{line}"
            );
        }
        assert!(Command::parse("set tick_hz").is_err());
        assert!(Command::parse("pause now").is_err());
        assert!(Command::parse("explode").is_err());
        assert!(Command::parse("").is_err());
    }

    #[test]
    fn replies_split_on_status() {
        assert_eq!(split_reply("ok"), (true, ""));
        assert_eq!(split_reply("ok paused"), (true, "paused"));
        assert_eq!(split_reply("error: nope"), (false, "nope"));
        assert_eq!(split_reply("garbage"), (false, "garbage"));
    }

    #[test]
    fn socket_path_uses_the_runtime_dir() {
        assert_eq!(
            socket_path_for(Uid::from_raw(1000), Some(PathBuf::from("/run/user/1000"))),
            Some(PathBuf::from(
                "/run/user/1000/wayland-wheeltani/control.sock"
            ))
        );
        assert_eq!(
            socket_path_for(Uid::from_raw(0), Some(PathBuf::from("/whatever"))),
            Some(PathBuf::from("/run/wayland-wheeltani/control.sock"))
        );
        // A relative runtime dir is ignored.
        assert_ne!(
            socket_path_for(Uid::from_raw(1000), Some(PathBuf::from("relative"))),
            Some(PathBuf::from("relative/wayland-wheeltani/control.sock"))
        );
    }

    #[test]
    fn server_answers_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("wheeltani-control-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("control.sock");
        let (wake_rx, wake_tx) = UnixStream::pair().unwrap();
        wake_tx.set_nonblocking(true).unwrap();
        let server = ControlServer::start(path.clone(), wake_tx).unwrap();
        assert_eq!(
            std::fs::metadata(&dir).unwrap().mode() & 0o777,
            0o700,
            "socket directory must be private"
        );

        // A second server on the same path refuses to start.
        let (_, other_waker) = UnixStream::pair().unwrap();
        assert!(matches!(
            ControlServer::start(path.clone(), other_waker),
            Err(ServerError::AlreadyRunning(_))
        ));

        let client = std::thread::spawn({
            let path = path.clone();
            move || exchange(UnixStream::connect(&path).unwrap(), "ping").unwrap()
        });
        let mut byte = [0u8; 1];
        (&wake_rx).read_exact(&mut byte).unwrap();
        let request = loop {
            if let Some(request) = server.try_next() {
                break request;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(request.line, "ping");
        request.respond("ok pong".to_owned());
        assert_eq!(client.join().unwrap(), "ok pong");

        drop(server);
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_socket_is_replaced() {
        let dir = std::env::temp_dir().join(format!("wheeltani-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        let path = dir.join("control.sock");
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let (_, waker) = UnixStream::pair().unwrap();
        let server = ControlServer::start(path, waker).unwrap();
        drop(server);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
