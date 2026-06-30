//! Hyprland provider: subscribes to the `.socket2.sock` event stream and tracks
//! the active window from `activewindow` lines.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
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

/// True when a Hyprland instance signature is set and its event socket exists.
#[must_use]
pub fn is_available() -> bool {
    socket2_path().is_some_and(|p| p.exists())
}

fn instance_signature() -> Option<String> {
    std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|s| !s.is_empty())
}

fn socket2_path() -> Option<PathBuf> {
    let his = instance_signature()?;
    if let Ok(runtime) = std::env::var("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(runtime)
            .join("hypr")
            .join(&his)
            .join(".socket2.sock");
        if p.exists() {
            return Some(p);
        }
    }
    // Legacy location used by older Hyprland releases.
    Some(PathBuf::from("/tmp/hypr").join(&his).join(".socket2.sock"))
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
                    reason: "HYPRLAND_INSTANCE_SIGNATURE unset".to_owned(),
                },
            );
            thread::sleep(BACKOFF_MAX);
            continue;
        };

        match UnixStream::connect(&path) {
            Ok(stream) => {
                backoff = BACKOFF_START;
                let reader = BufReader::new(stream);
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
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
