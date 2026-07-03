//! Hyprland provider: subscribes to the `.socket2.sock` event stream and tracks
//! the active window from `activewindow` lines.
//!
//! Socket discovery works even without `HYPRLAND_INSTANCE_SIGNATURE`: that
//! variable is set inside Hyprland's own children but is usually absent from a
//! `systemd --user` service environment, which previously made the provider
//! (and `auto` detection) fail under the recommended service setup. When the
//! variable is missing, the runtime directories are scanned for a live
//! `.socket2.sock` instead.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use tracing::debug;

use crate::foreground::filter::{
    ForegroundApp, ForegroundProvider, ForegroundSnapshot, ForegroundSourceKind,
};

use super::{read_snapshot, store, SharedSnapshot};

const BACKOFF_START: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(2);

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
        if best.as_ref().map_or(true, |(t, _)| modified > *t) {
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
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            // Only a stream that actually delivers data counts
                            // as healthy; resetting on connect alone could
                            // spawn-storm against a socket that dies instantly.
                            backoff = BACKOFF_START;
                            if let Some(app) = parse_event_line(&line) {
                                store(shared, ForegroundSnapshot::Known(app));
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

/// Parses a single `.socket2.sock` line. Returns an app only for `activewindow`
/// events, whose data is `WINDOWCLASS,WINDOWTITLE` (the title may itself
/// contain commas, so only the first comma is treated as the separator).
#[must_use]
pub fn parse_event_line(line: &str) -> Option<ForegroundApp> {
    let (event, data) = line.split_once(">>")?;
    if event != "activewindow" {
        return None;
    }
    let (class, title) = match data.split_once(',') {
        Some((c, t)) => (c.trim(), t.trim()),
        None => (data.trim(), ""),
    };
    let class = (!class.is_empty()).then(|| class.to_owned());
    let title = (!title.is_empty()).then(|| title.to_owned());
    Some(ForegroundApp {
        app_id: None,
        class,
        resource_class: None,
        title,
        pid: None,
        source: ForegroundSourceKind::Hyprland,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_activewindow_class_and_title() {
        let app = parse_event_line("activewindow>>firefox,Mozilla Firefox").expect("parsed");
        assert_eq!(app.class.as_deref(), Some("firefox"));
        assert_eq!(app.title.as_deref(), Some("Mozilla Firefox"));
        assert_eq!(app.source, ForegroundSourceKind::Hyprland);
    }

    #[test]
    fn parses_empty_activewindow() {
        let app = parse_event_line("activewindow>>,").expect("parsed");
        assert!(app.class.is_none());
        assert!(app.title.is_none());
    }

    #[test]
    fn title_with_commas_is_kept_whole() {
        let app = parse_event_line("activewindow>>code,main.rs, project, vim").expect("parsed");
        assert_eq!(app.class.as_deref(), Some("code"));
        assert_eq!(app.title.as_deref(), Some("main.rs, project, vim"));
    }

    #[test]
    fn ignores_other_events() {
        assert!(parse_event_line("windowtitlev2>>0x123,Title").is_none());
        assert!(parse_event_line("workspace>>1").is_none());
    }
}
