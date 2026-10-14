//! Hyprland provider: subscribes to the `.socket2.sock` event stream and tracks
//! the active window from `activewindow` lines. Hyprland repeats that event
//! when the focused window's title changes, so titles stay current too.
//!
//! Each (re)connection first asks the request socket (`.socket.sock`) for the
//! current window with `j/activewindow`, so the snapshot is right before the
//! first focus change instead of staying "starting" until then.
//!
//! Socket discovery works even without `HYPRLAND_INSTANCE_SIGNATURE`: that
//! variable is set inside Hyprland's own children but is usually absent from a
//! `systemd --user` service environment, which previously made the provider
//! (and `auto` detection) fail under the recommended service setup. When the
//! variable is missing, the runtime directories are scanned for a live
//! `.socket2.sock` instead.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use serde_json::Value;
use tracing::debug;

use crate::foreground::filter::{
    ForegroundApp, ForegroundProvider, ForegroundSnapshot, ForegroundSourceKind,
};

use super::{read_snapshot, store, SharedSnapshot};

const BACKOFF_START: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(2);
/// Bounds the `j/activewindow` request so a stuck compositor cannot hold up
/// the event stream.
const QUERY_TIMEOUT: Duration = Duration::from_secs(1);
/// `activewindow` replies are a few hundred bytes; this only guards against a
/// runaway reply.
const MAX_REPLY_BYTES: u64 = 1024 * 1024;

#[derive(Debug)]
pub struct HyprlandProvider {
    shared: SharedSnapshot,
}

/// True when a Hyprland event socket can be located (via the instance
/// signature or by scanning the runtime directories).
#[must_use]
pub fn is_available() -> bool {
    socket2_path().is_some_and(|p| p.exists())
}

fn instance_signature() -> Option<String> {
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|s| !s.is_empty())
}

const SOCKET2_NAME: &str = ".socket2.sock";
const REQUEST_SOCKET_NAME: &str = ".socket.sock";

fn hypr_base_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::with_capacity(2);
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime.is_empty() {
            dirs.push(PathBuf::from(runtime).join("hypr"));
        }
    }
    // Legacy location used by older Hyprland releases.
    dirs.push(PathBuf::from("/tmp/hypr"));
    dirs
}

fn socket2_path() -> Option<PathBuf> {
    let bases = hypr_base_dirs();

    // Fast path: the instance signature pins the exact instance directory.
    if let Some(his) = instance_signature() {
        for base in &bases {
            let p = base.join(&his).join(SOCKET2_NAME);
            if p.exists() {
                return Some(p);
            }
        }
    }

    // Fallback for environments where the signature is not exported (typical
    // for systemd --user services): scan the instance directories and pick the
    // most recently created socket, which corresponds to the live instance.
    bases.iter().find_map(|base| newest_socket_under(base))
}

fn newest_socket_under(base: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let candidate = entry.path().join(SOCKET2_NAME);
        let Ok(meta) = candidate.symlink_metadata() else {
            continue;
        };
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().is_none_or(|(t, _)| modified > *t) {
            best = Some((modified, candidate));
        }
    }
    best.map(|(_, p)| p)
}

impl HyprlandProvider {
    pub fn start() -> Self {
        let shared: SharedSnapshot = Arc::new(RwLock::new(ForegroundSnapshot::Unknown {
            reason: "hyprland provider starting".to_owned(),
        }));
        let shared_bg = shared.clone();
        let spawned = thread::Builder::new()
            .name("wheeltani-fg-hyprland".to_owned())
            .spawn(move || event_loop(&shared_bg));
        if let Err(err) = spawned {
            store(
                &shared,
                ForegroundSnapshot::Unsupported {
                    reason: format!("failed to spawn hyprland thread: {err}"),
                },
            );
        }
        Self { shared }
    }
}

impl ForegroundProvider for HyprlandProvider {
    fn snapshot(&self) -> ForegroundSnapshot {
        read_snapshot(&self.shared)
    }
}

fn event_loop(shared: &SharedSnapshot) {
    let mut backoff = BACKOFF_START;
    loop {
        let Some(path) = socket2_path() else {
            store(
                shared,
                ForegroundSnapshot::Unsupported {
                    reason: "no hyprland event socket found (HYPRLAND_INSTANCE_SIGNATURE unset \
                             and no live instance in the runtime dir)"
                        .to_owned(),
                },
            );
            thread::sleep(BACKOFF_MAX);
            continue;
        };

        match UnixStream::connect(&path) {
            Ok(stream) => {
                // Subscribed first, so a focus change racing with the query
                // still arrives as an event afterwards.
                match query_active_window(&path) {
                    Ok(snapshot) => store(shared, snapshot),
                    Err(err) => debug!(?err, "hyprland activewindow query failed"),
                }
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            // Only a stream that actually delivers data counts
                            // as healthy; resetting on connect alone could
                            // spawn-storm against a socket that dies instantly.
                            backoff = BACKOFF_START;
                            if let Some(snapshot) = parse_event_line(&line) {
                                store(shared, snapshot);
                            }
                        }
                        Err(_) => break,
                    }
                }
                debug!("hyprland socket2 closed; reconnecting");
            }
            Err(err) => debug!(?err, "failed to connect to hyprland socket2; retrying"),
        }

        thread::sleep(backoff);
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

fn no_window() -> ForegroundSnapshot {
    ForegroundSnapshot::Unknown {
        reason: "no focused window".to_owned(),
    }
}

/// Builds the snapshot for a window's class and title; both empty means no
/// window has focus (an empty workspace), which is not a known app.
fn window_snapshot(class: &str, title: &str, pid: Option<u32>) -> ForegroundSnapshot {
    let class = class.trim();
    let title = title.trim();
    if class.is_empty() && title.is_empty() {
        return no_window();
    }
    ForegroundSnapshot::Known(ForegroundApp {
        app_id: None,
        class: (!class.is_empty()).then(|| class.to_owned()),
        resource_class: None,
        title: (!title.is_empty()).then(|| title.to_owned()),
        pid,
        source: ForegroundSourceKind::Hyprland,
    })
}

/// Parses a single `.socket2.sock` line. Returns a snapshot only for
/// `activewindow` events, whose data is `WINDOWCLASS,WINDOWTITLE` (the title
/// may itself contain commas, so only the first comma is treated as the
/// separator).
#[must_use]
pub fn parse_event_line(line: &str) -> Option<ForegroundSnapshot> {
    let (event, data) = line.split_once(">>")?;
    if event != "activewindow" {
        return None;
    }
    let (class, title) = data.split_once(',').unwrap_or((data, ""));
    Some(window_snapshot(class, title, None))
}

/// Asks the request socket next to `socket2` for the focused window.
fn query_active_window(socket2: &Path) -> std::io::Result<ForegroundSnapshot> {
    let mut stream = UnixStream::connect(socket2.with_file_name(REQUEST_SOCKET_NAME))?;
    stream.set_read_timeout(Some(QUERY_TIMEOUT))?;
    stream.set_write_timeout(Some(QUERY_TIMEOUT))?;
    stream.write_all(b"j/activewindow")?;
    let mut reply = Vec::new();
    stream.take(MAX_REPLY_BYTES).read_to_end(&mut reply)?;
    Ok(parse_active_window_reply(&reply))
}

/// Parses the JSON reply to `j/activewindow`; Hyprland answers `{}` when no
/// window has focus.
#[must_use]
pub fn parse_active_window_reply(reply: &[u8]) -> ForegroundSnapshot {
    let Ok(value) = serde_json::from_slice::<Value>(reply) else {
        return ForegroundSnapshot::Unknown {
            reason: "unreadable hyprland activewindow reply".to_owned(),
        };
    };
    let text = |key: &str| value.get(key).and_then(Value::as_str).unwrap_or("");
    let pid = value
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok());
    window_snapshot(text("class"), text("title"), pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(snapshot: Option<ForegroundSnapshot>) -> ForegroundApp {
        match snapshot {
            Some(ForegroundSnapshot::Known(app)) => app,
            other => panic!("expected a known app, got {other:?}"),
        }
    }

    #[test]
    fn parses_activewindow_class_and_title() {
        let app = known(parse_event_line("activewindow>>firefox,Mozilla Firefox"));
        assert_eq!(app.class.as_deref(), Some("firefox"));
        assert_eq!(app.title.as_deref(), Some("Mozilla Firefox"));
        assert_eq!(app.source, ForegroundSourceKind::Hyprland);
    }

    #[test]
    fn empty_activewindow_is_no_window() {
        assert!(matches!(
            parse_event_line("activewindow>>,"),
            Some(ForegroundSnapshot::Unknown { .. })
        ));
    }

    #[test]
    fn title_with_commas_is_kept_whole() {
        let app = known(parse_event_line("activewindow>>code,main.rs, project, vim"));
        assert_eq!(app.class.as_deref(), Some("code"));
        assert_eq!(app.title.as_deref(), Some("main.rs, project, vim"));
    }

    #[test]
    fn ignores_other_events() {
        assert!(parse_event_line("windowtitlev2>>0x123,Title").is_none());
        assert!(parse_event_line("workspace>>1").is_none());
    }

    #[test]
    fn parses_activewindow_reply() {
        let reply =
            br#"{"address":"0x5f","class":"kitty","title":"~","pid":4242,"initialClass":"kitty"}"#;
        match parse_active_window_reply(reply) {
            ForegroundSnapshot::Known(app) => {
                assert_eq!(app.class.as_deref(), Some("kitty"));
                assert_eq!(app.title.as_deref(), Some("~"));
                assert_eq!(app.pid, Some(4242));
            }
            other => panic!("expected a known app, got {other:?}"),
        }
    }

    #[test]
    fn empty_or_invalid_reply_is_unknown() {
        assert!(matches!(
            parse_active_window_reply(b"{}"),
            ForegroundSnapshot::Unknown { .. }
        ));
        assert!(matches!(
            parse_active_window_reply(b"Invalid"),
            ForegroundSnapshot::Unknown { .. }
        ));
    }
}
