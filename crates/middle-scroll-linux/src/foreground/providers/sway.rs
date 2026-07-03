//! Sway / i3 provider: subscribes to `window` events and tracks the focused
//! app from the container carried by each event, falling back to `GET_TREE`
//! only when an event cannot identify the focused window (initial sync and
//! window-close). This keeps the steady state at zero extra connections and
//! zero tree parses per focus change.
//!
//! Socket discovery works even without `SWAYSOCK`/`I3SOCK`: those variables
//! are set inside the compositor's children but are usually absent from a
//! `systemd --user` service environment, so the runtime directory is scanned
//! for a live `sway-ipc.*.sock` as a fallback.

use std::io::{self, Read, Write};
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

const MAGIC: &[u8] = b"i3-ipc";
const IPC_SUBSCRIBE: u32 = 2;
const IPC_GET_TREE: u32 = 4;

/// Upper bound on an IPC payload we are willing to buffer. Real `GET_TREE`
/// payloads are well under a few megabytes; anything larger indicates a
/// corrupt stream and must not translate into an unbounded allocation.
const MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

const BACKOFF_START: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct SwayProvider {
    shared: SharedSnapshot,
}

/// True when a Sway/i3 IPC socket is advertised (env var) or discoverable in
/// the runtime directory.
#[must_use]
pub fn is_available() -> bool {
    socket_path().is_some_and(|p| p.exists())
}

fn socket_path() -> Option<PathBuf> {
    for var in ["SWAYSOCK", "I3SOCK"] {
        if let Ok(p) = std::env::var(var) {
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
    }
    discover_sway_socket()
}

/// Scans `$XDG_RUNTIME_DIR` for `sway-ipc.<uid>.<pid>.sock` and returns the
/// most recently created one (the live compositor). Needed when the daemon
/// runs as a `systemd --user` service, where `SWAYSOCK` is not exported.
fn discover_sway_socket() -> Option<PathBuf> {
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok().filter(|s| !s.is_empty())?;
    let entries = std::fs::read_dir(runtime).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Sway hardcodes the lowercase `sway-ipc.<uid>.<pid>.sock` pattern.
        #[allow(clippy::case_sensitive_file_extension_comparisons)]
        if !(name.starts_with("sway-ipc.") && name.ends_with(".sock")) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().map_or(true, |(t, _)| modified > *t) {
            best = Some((modified, entry.path()));
        }
    }
    best.map(|(_, p)| p)
}

impl SwayProvider {
    pub fn start() -> Self {
        let shared: SharedSnapshot = Arc::new(RwLock::new(ForegroundSnapshot::Unknown {
            reason: "sway provider starting".to_owned(),
        }));
        let shared_bg = shared.clone();
        let spawned = thread::Builder::new()
            .name("wheeltani-fg-sway".to_owned())
            .spawn(move || event_loop(&shared_bg));
        if let Err(err) = spawned {
            store(
                &shared,
                ForegroundSnapshot::Unsupported {
                    reason: format!("failed to spawn sway thread: {err}"),
                },
            );
        }
        Self { shared }
    }
}

impl ForegroundProvider for SwayProvider {
    fn snapshot(&self) -> ForegroundSnapshot {
        read_snapshot(&self.shared)
    }
}

fn event_loop(shared: &SharedSnapshot) {
    let mut backoff = BACKOFF_START;
    loop {
        let Some(path) = socket_path() else {
            store(
                shared,
                ForegroundSnapshot::Unsupported {
                    reason: "SWAYSOCK/I3SOCK unset".to_owned(),
                },
            );
            thread::sleep(BACKOFF_MAX);
            continue;
        };

        match run_session(&path, shared, &mut backoff) {
            Ok(()) => {}
            Err(err) => debug!(?err, "sway ipc error; reconnecting"),
        }

        thread::sleep(backoff);
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

fn run_session(path: &Path, shared: &SharedSnapshot, backoff: &mut Duration) -> io::Result<()> {
    let mut sub = UnixStream::connect(path)?;
    send_msg(&mut sub, IPC_SUBSCRIBE, br#"["window"]"#)?;
    let _ = read_msg(&mut sub)?; // subscribe acknowledgement
    *backoff = BACKOFF_START;

    // Initialise from the current tree, then track focus from the container
    // carried by each event; the tree is only re-queried when an event cannot
    // tell us who is focused now (e.g. the focused window closed).
    refresh(path, shared);
    loop {
        let (_, payload) = read_msg(&mut sub)?;
        match handle_window_event(&payload) {
            EventOutcome::Update(app) => store(shared, ForegroundSnapshot::Known(app)),
            EventOutcome::NeedsTreeRefresh => refresh(path, shared),
            EventOutcome::Ignore => {}
        }
    }
}

/// What a `window` event tells us about the focused app.
#[derive(Debug)]
enum EventOutcome {
    /// The event identifies the newly focused (or retitled focused) window.
    Update(ForegroundApp),
    /// The event invalidates the snapshot without naming a successor; the
    /// tree must be queried (e.g. the focused window closed and focus may
    /// have moved to nothing / another workspace).
    NeedsTreeRefresh,
    /// The event does not affect the focused app.
    Ignore,
}

/// Interprets a `window` event payload (`{"change": ..., "container": ...}`).
fn handle_window_event(payload: &[u8]) -> EventOutcome {
    let Ok(event) = serde_json::from_slice::<Value>(payload) else {
        // Unparseable payload: resync from the tree rather than going stale.
        return EventOutcome::NeedsTreeRefresh;
    };
    let change = event.get("change").and_then(Value::as_str).unwrap_or("");
    let container = event.get("container");
    match change {
        "focus" => container.map_or(EventOutcome::NeedsTreeRefresh, |c| {
            EventOutcome::Update(node_to_app(c))
        }),
        // Title changes only matter for the currently focused window.
        "title" => match container {
            Some(c) if c.get("focused").and_then(Value::as_bool) == Some(true) => {
                EventOutcome::Update(node_to_app(c))
            }
            _ => EventOutcome::Ignore,
        },
        // Closing a window can move focus without a matching focus event
        // (e.g. the workspace becomes empty).
        "close" => EventOutcome::NeedsTreeRefresh,
        _ => EventOutcome::Ignore,
    }
}

fn refresh(path: &Path, shared: &SharedSnapshot) {
    match query_focused(path) {
        Ok(Some(app)) => store(shared, ForegroundSnapshot::Known(app)),
        Ok(None) => store(
            shared,
            ForegroundSnapshot::Unknown {
                reason: "no focused window in sway tree".to_owned(),
            },
        ),
        Err(err) => debug!(?err, "failed to query sway tree"),
    }
}

fn query_focused(path: &Path) -> io::Result<Option<ForegroundApp>> {
    let mut conn = UnixStream::connect(path)?;
    send_msg(&mut conn, IPC_GET_TREE, b"")?;
    let (_msg_type, payload) = read_msg(&mut conn)?;
    let tree: Value = serde_json::from_slice(&payload)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(find_focused(&tree))
}

fn send_msg(stream: &mut UnixStream, msg_type: u32, payload: &[u8]) -> io::Result<()> {
    let len = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "payload too large"))?;
    let mut buf = Vec::with_capacity(MAGIC.len() + 8 + payload.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&len.to_ne_bytes());
    buf.extend_from_slice(&msg_type.to_ne_bytes());
    buf.extend_from_slice(payload);
    stream.write_all(&buf)
}

fn read_msg(stream: &mut UnixStream) -> io::Result<(u32, Vec<u8>)> {
    let mut header = [0u8; 14];
    stream.read_exact(&mut header)?;
    if &header[0..6] != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "bad ipc magic"));
    }
    let len = u32::from_ne_bytes([header[6], header[7], header[8], header[9]]) as usize;
    let msg_type = u32::from_ne_bytes([header[10], header[11], header[12], header[13]]);
    if len > MAX_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "ipc payload too large",
        ));
    }
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload)?;
    Ok((msg_type, payload))
}

/// Recursively finds the node with `focused == true` and maps it to a
/// [`ForegroundApp`]. Searches both tiled (`nodes`) and `floating_nodes`.
#[must_use]
pub fn find_focused(node: &Value) -> Option<ForegroundApp> {
    if node.get("focused").and_then(Value::as_bool) == Some(true) {
        return Some(node_to_app(node));
    }
    for key in ["nodes", "floating_nodes"] {
        if let Some(children) = node.get(key).and_then(Value::as_array) {
            for child in children {
                if let Some(found) = find_focused(child) {
                    return Some(found);
                }
            }
        }
    }
    None
}

fn node_to_app(node: &Value) -> ForegroundApp {
    let str_field = |v: &Value, key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };

    let app_id = str_field(node, "app_id");
    let title = str_field(node, "name");
    let pid = node
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok());

    let props = node.get("window_properties");
    let class = props.and_then(|p| str_field(p, "class"));
    let instance = props.and_then(|p| str_field(p, "instance"));

    ForegroundApp {
        app_id,
        class,
        resource_class: instance,
        title,
        pid,
        source: ForegroundSourceKind::Sway,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_wayland_node_with_app_id() {
        let tree = serde_json::json!({
            "focused": false,
            "nodes": [
                {"focused": false, "nodes": [
                    {"focused": true, "app_id": "org.mozilla.firefox", "name": "Firefox", "pid": 42}
                ]}
            ]
        });
        let app = find_focused(&tree).expect("focused node");
        assert_eq!(app.app_id.as_deref(), Some("org.mozilla.firefox"));
        assert_eq!(app.title.as_deref(), Some("Firefox"));
        assert_eq!(app.pid, Some(42));
        assert_eq!(app.source, ForegroundSourceKind::Sway);
    }

    #[test]
    fn finds_xwayland_node_with_window_properties() {
        let tree = serde_json::json!({
            "focused": false,
            "nodes": [
                {"focused": true, "name": "Blender",
                 "window_properties": {"class": "Blender", "instance": "blender"}}
            ]
        });
        let app = find_focused(&tree).expect("focused node");
        assert_eq!(app.class.as_deref(), Some("Blender"));
        assert_eq!(app.resource_class.as_deref(), Some("blender"));
        assert!(app.app_id.is_none());
    }

    #[test]
    fn returns_none_without_focused_node() {
        let tree = serde_json::json!({
            "focused": false,
            "nodes": [{"focused": false, "nodes": []}]
        });
        assert!(find_focused(&tree).is_none());
    }

    #[test]
    fn focus_event_updates_from_container() {
        let payload = serde_json::json!({
            "change": "focus",
            "container": {"app_id": "org.mozilla.firefox", "name": "Firefox", "pid": 42}
        });
        match handle_window_event(payload.to_string().as_bytes()) {
            EventOutcome::Update(app) => {
                assert_eq!(app.app_id.as_deref(), Some("org.mozilla.firefox"));
                assert_eq!(app.pid, Some(42));
            }
            other => panic!("expected Update, got {other:?}"),
        }
    }

    #[test]
    fn title_event_only_updates_focused_container() {
        let focused = serde_json::json!({
            "change": "title",
            "container": {"focused": true, "app_id": "code", "name": "new title"}
        });
        assert!(matches!(
            handle_window_event(focused.to_string().as_bytes()),
            EventOutcome::Update(_)
        ));

        let unfocused = serde_json::json!({
            "change": "title",
            "container": {"focused": false, "app_id": "code", "name": "bg"}
        });
        assert!(matches!(
            handle_window_event(unfocused.to_string().as_bytes()),
            EventOutcome::Ignore
        ));
    }

    #[test]
    fn close_event_forces_tree_refresh() {
        let payload = serde_json::json!({"change": "close", "container": {"app_id": "mpv"}});
        assert!(matches!(
            handle_window_event(payload.to_string().as_bytes()),
            EventOutcome::NeedsTreeRefresh
        ));
    }

    #[test]
    fn unrelated_and_invalid_events() {
        let moved = serde_json::json!({"change": "move", "container": {}});
        assert!(matches!(
            handle_window_event(moved.to_string().as_bytes()),
            EventOutcome::Ignore
        ));
        assert!(matches!(
            handle_window_event(b"{not json"),
            EventOutcome::NeedsTreeRefresh
        ));
    }

    #[test]
    fn searches_floating_nodes() {
        let tree = serde_json::json!({
            "focused": false,
            "nodes": [],
            "floating_nodes": [
                {"focused": true, "app_id": "mpv", "name": "video"}
            ]
        });
        let app = find_focused(&tree).expect("focused floating node");
        assert_eq!(app.app_id.as_deref(), Some("mpv"));
    }
}
