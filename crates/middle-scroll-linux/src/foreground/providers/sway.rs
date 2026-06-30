//! Sway / i3 provider: subscribes to `window` events and resolves the focused
//! node from `GET_TREE`. The tree is more reliable than individual events for
//! reconstructing the current app across native-Wayland and `XWayland` windows.

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

const BACKOFF_START: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct SwayProvider {
    shared: SharedSnapshot,
}

/// True when a Sway/i3 IPC socket is advertised and present.
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
    None
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

    // Initialise from the current tree, then refresh on every window event.
    refresh(path, shared);
    loop {
        let _ = read_msg(&mut sub)?;
        refresh(path, shared);
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
