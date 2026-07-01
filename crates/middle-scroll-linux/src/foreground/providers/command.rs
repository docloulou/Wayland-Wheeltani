//! An external-command provider: runs a user-configured command periodically on
//! a background thread and parses its stdout. The command is never run from the
//! input hot path. Intended as the escape hatch for `KWin` and other compositors.

use std::process::Command;
use std::thread;
use std::time::Duration;

use tracing::debug;

use crate::foreground::filter::{
    ForegroundApp, ForegroundProvider, ForegroundSnapshot, ForegroundSourceKind,
};

use super::{json_to_app, read_snapshot, store, SharedSnapshot};

/// Floor for the refresh interval to avoid a busy loop on a misconfigured `0`.
const MIN_REFRESH: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub struct CommandProvider {
    shared: SharedSnapshot,
}

impl CommandProvider {
    pub fn start(command: Vec<String>, refresh_ms: u64) -> Self {
        let initial = if command.is_empty() {
            ForegroundSnapshot::Unsupported {
                reason: "foreground.command is empty".to_owned(),
            }
        } else {
            ForegroundSnapshot::Unknown {
                reason: "command provider starting".to_owned(),
            }
        };
        let shared: SharedSnapshot = std::sync::Arc::new(std::sync::RwLock::new(initial));

        if !command.is_empty() {
            let shared_bg = shared.clone();
            let refresh = Duration::from_millis(refresh_ms).max(MIN_REFRESH);
            let spawned = thread::Builder::new()
                .name("wheeltani-fg-command".to_owned())
                .spawn(move || command_loop(&command, refresh, &shared_bg));
            if let Err(err) = spawned {
                store(
                    &shared,
                    ForegroundSnapshot::Unsupported {
                        reason: format!("failed to spawn command thread: {err}"),
                    },
                );
            }
        }

        Self { shared }
    }
}

impl ForegroundProvider for CommandProvider {
    fn snapshot(&self) -> ForegroundSnapshot {
        read_snapshot(&self.shared)
    }
}

fn command_loop(command: &[String], refresh: Duration, shared: &SharedSnapshot) {
    loop {
        store(shared, run_once(command));
        thread::sleep(refresh);
    }
}

fn run_once(command: &[String]) -> ForegroundSnapshot {
    let Some((bin, args)) = command.split_first() else {
        return ForegroundSnapshot::Unsupported {
            reason: "foreground.command is empty".to_owned(),
        };
    };
    match Command::new(bin).args(args).output() {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            parse_stdout(&stdout)
        }
        Ok(out) => {
            debug!(status = %out.status, "foreground command exited unsuccessfully");
            ForegroundSnapshot::Unknown {
                reason: format!("command exited with {}", out.status),
            }
        }
        Err(err) => ForegroundSnapshot::Unknown {
            reason: format!("command failed to run: {err}"),
        },
    }
}

/// Parses provider command stdout. A payload starting with `{` is parsed as the
/// shared JSON object; otherwise the first non-empty line is taken as a bare app
/// id.
#[must_use]
pub fn parse_stdout(stdout: &str) -> ForegroundSnapshot {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return ForegroundSnapshot::Unknown {
            reason: "command produced no output".to_owned(),
        };
    }
    if trimmed.starts_with('{') {
        return match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => ForegroundSnapshot::Known(json_to_app(&v, ForegroundSourceKind::Command)),
            Err(err) => ForegroundSnapshot::Unknown {
                reason: format!("invalid JSON from command: {err}"),
            },
        };
    }

    let id = trimmed.lines().next().unwrap_or("").trim();
    ForegroundSnapshot::Known(ForegroundApp {
        app_id: Some(id.to_owned()),
        class: Some(id.to_owned()),
        resource_class: None,
        title: None,
        pid: None,
        source: ForegroundSourceKind::Command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_app_id_and_class() {
        let snap = parse_stdout("org.mozilla.firefox\n");
        match snap {
            ForegroundSnapshot::Known(app) => {
                assert_eq!(app.app_id.as_deref(), Some("org.mozilla.firefox"));
                assert_eq!(app.class.as_deref(), Some("org.mozilla.firefox"));
                assert_eq!(app.source, ForegroundSourceKind::Command);
            }
            other => panic!("expected Known, got {other:?}"),
        }
    }

    #[test]
    fn full_json_is_parsed() {
        let snap = parse_stdout(
            r#"{"app_id":"org.mozilla.firefox","class":"firefox","title":"Mozilla Firefox","pid":12345}"#,
        );
        match snap {
            ForegroundSnapshot::Known(app) => {
                assert_eq!(app.app_id.as_deref(), Some("org.mozilla.firefox"));
                assert_eq!(app.class.as_deref(), Some("firefox"));
                assert_eq!(app.title.as_deref(), Some("Mozilla Firefox"));
                assert_eq!(app.pid, Some(12345));
            }
            other => panic!("expected Known, got {other:?}"),
        }
    }

    #[test]
    fn partial_json_keeps_present_fields() {
        let snap = parse_stdout(r#"{"class":"krita"}"#);
        match snap {
            ForegroundSnapshot::Known(app) => {
                assert_eq!(app.class.as_deref(), Some("krita"));
                assert!(app.app_id.is_none());
                assert!(app.pid.is_none());
            }
            other => panic!("expected Known, got {other:?}"),
        }
    }

    #[test]
    fn empty_stdout_is_unknown() {
        assert!(matches!(
            parse_stdout("   \n"),
            ForegroundSnapshot::Unknown { .. }
        ));
    }

    #[test]
    fn invalid_json_is_unknown() {
        assert!(matches!(
            parse_stdout("{not json"),
            ForegroundSnapshot::Unknown { .. }
        ));
    }
}
