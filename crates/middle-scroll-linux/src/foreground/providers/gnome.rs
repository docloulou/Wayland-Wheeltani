//! GNOME Shell provider.
//!
//! GNOME (Wayland) deliberately exposes no portable API for the focused window,
//! so this provider talks to the bundled GNOME Shell extension over the session
//! bus. To avoid pulling in a D-Bus crate (per the project's dependency policy)
//! it shells out to `gdbus` (shipped with glib) on a background thread:
//!
//! - a low-latency push path runs `gdbus monitor` and parses `FocusedChanged`
//!   signals;
//! - an authoritative resync path periodically calls `GetFocused`, which also
//!   detects the extension going away (so `unknown_policy` can take over).
//!
//! The daemon runs as a systemd `--user` service, so it shares the user's
//! session bus and these calls reach GNOME Shell.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use tracing::debug;

use crate::foreground::filter::{
    ForegroundApp, ForegroundProvider, ForegroundSnapshot, ForegroundSourceKind,
};

use super::{
    dbus_name_has_owner, json_to_app, read_snapshot, run_with_timeout, store, SharedSnapshot,
    HELPER_TIMEOUT,
};

const BUS_NAME: &str = "org.docloulou.WheeltaniForeground";
const OBJECT_PATH: &str = "/org/docloulou/WheeltaniForeground";
const INTERFACE: &str = "org.docloulou.WheeltaniForeground";

/// Push updates arrive through `gdbus monitor`, so the resync poll only needs
/// to catch a missed signal or the extension going away; 5s keeps that
/// liveness check while spawning far fewer subprocesses than a tight poll.
const RESYNC_INTERVAL: Duration = Duration::from_secs(5);
const BACKOFF_START: Duration = Duration::from_millis(250);
const BACKOFF_MAX: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct GnomeProvider {
    shared: SharedSnapshot,
}

/// True when `gdbus` is available and the extension's bus name is currently
/// owned on the session bus. This is the authoritative signal that GNOME and
/// the bundled extension are both running and reachable from THIS process.
///
/// We deliberately do not gate on desktop-environment variables such as
/// `XDG_CURRENT_DESKTOP`: those are frequently absent from the daemon's
/// `systemd --user` environment even though the session bus is perfectly
/// reachable, which previously made GNOME auto-detection fail.
#[must_use]
pub fn is_available() -> bool {
    if !gdbus_available() {
        debug!("gnome: gdbus binary not found in PATH");
        return false;
    }
    if !dbus_name_has_owner(BUS_NAME) {
        debug!(
            bus = BUS_NAME,
            "gnome: D-Bus name has no owner (extension not enabled, or the session bus is \
             unreachable from this process)"
        );
        return false;
    }
    true
}

fn gdbus_available() -> bool {
    // `gdbus` has no `--version`/`version` subcommand (those exit non-zero with
    // "Unknown command"). We only need to know the binary can be executed, so we
    // spawn `gdbus help` (exit 0) and treat any successful run as available,
    // regardless of the exit status. Spawning only errors when the binary is
    // missing or cannot be launched.
    run_with_timeout(Command::new("gdbus").arg("help"), HELPER_TIMEOUT)
        .is_ok_and(|out| out.is_some())
}

impl GnomeProvider {
    pub fn start() -> Self {
        let shared: SharedSnapshot = Arc::new(RwLock::new(ForegroundSnapshot::Unknown {
            reason: "gnome provider starting".to_owned(),
        }));

        if !gdbus_available() {
            store(
                &shared,
                ForegroundSnapshot::Unsupported {
                    reason: "gdbus not found (install glib2 / gdbus)".to_owned(),
                },
            );
            return Self { shared };
        }

        // Synchronous initial snapshot so the very first gesture already has the
        // correct decision instead of briefly falling back to unknown_policy.
        match call_get_focused() {
            Ok(Some(app)) => {
                debug!(app = ?app, "gnome: initial focused app");
                store(&shared, ForegroundSnapshot::Known(app));
            }
            Ok(None) => debug!("gnome: initial GetFocused returned no window"),
            Err(reason) => {
                debug!(%reason, "gnome: initial GetFocused failed (extension not ready?)");
            }
        }

        // Push path: low-latency updates from the extension's signal.
        let push = shared.clone();
        thread::Builder::new()
            .name("wheeltani-fg-gnome-mon".to_owned())
            .spawn(move || monitor_loop(&push))
            .ok();

        // Resync path: authoritative state + extension liveness.
        let poll = shared.clone();
        thread::Builder::new()
            .name("wheeltani-fg-gnome-poll".to_owned())
            .spawn(move || resync_loop(&poll))
            .ok();

        Self { shared }
    }
}

impl ForegroundProvider for GnomeProvider {
    fn snapshot(&self) -> ForegroundSnapshot {
        read_snapshot(&self.shared)
    }
}

fn monitor_loop(shared: &SharedSnapshot) {
    let mut backoff = BACKOFF_START;
    loop {
        match Command::new("gdbus")
            .args([
                "monitor",
                "--session",
                "--dest",
                BUS_NAME,
                "--object-path",
                OBJECT_PATH,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(stdout) = child.stdout.take() {
                    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                        // Only a monitor that actually produces output counts
                        // as healthy; resetting on spawn alone would defeat
                        // the backoff if gdbus exits immediately.
                        backoff = BACKOFF_START;
                        if let Some(app) = parse_monitor_line(&line) {
                            store(shared, ForegroundSnapshot::Known(app));
                        }
                    }
                }
                let _ = child.wait();
                debug!("gdbus monitor exited; restarting");
            }
            Err(err) => debug!(?err, "failed to spawn gdbus monitor; retrying"),
        }
        thread::sleep(backoff);
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

fn resync_loop(shared: &SharedSnapshot) {
    loop {
        match call_get_focused() {
            Ok(Some(app)) => store(shared, ForegroundSnapshot::Known(app)),
            Ok(None) => {}
            Err(reason) => store(shared, ForegroundSnapshot::Unknown { reason }),
        }
        thread::sleep(RESYNC_INTERVAL);
    }
}

fn call_get_focused() -> Result<Option<ForegroundApp>, String> {
    let out = run_with_timeout(
        Command::new("gdbus").args([
            "call",
            "--session",
            "--dest",
            BUS_NAME,
            "--object-path",
            OBJECT_PATH,
            "--method",
            &format!("{INTERFACE}.GetFocused"),
        ]),
        HELPER_TIMEOUT,
    )
    .map_err(|e| format!("failed to run gdbus: {e}"))?
    .ok_or_else(|| "gdbus call timed out".to_owned())?;
    if !out.status.success() {
        return Err(format!(
            "gdbus call failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(parse_call_output(&String::from_utf8_lossy(&out.stdout)))
}

/// Extracts and unescapes the first single-quoted `GVariant` string in `s`.
/// `gdbus` prints strings as `'...'` with `\'` and `\\` escapes.
fn extract_gvariant_string(s: &str) -> Option<String> {
    let mut chars = s.chars();
    let mut opened = false;
    for c in chars.by_ref() {
        if c == '\'' {
            opened = true;
            break;
        }
    }
    if !opened {
        return None;
    }
    let mut out = String::new();
    let mut escaped = false;
    for c in chars {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '\'' {
            return Some(out);
        } else {
            out.push(c);
        }
    }
    None
}

fn parse_focused_payload(json: &str) -> Option<ForegroundApp> {
    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "{}" {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let app = json_to_app(&value, ForegroundSourceKind::Gnome);
    if app.app_id.is_none() && app.class.is_none() && app.resource_class.is_none() {
        return None;
    }
    Some(app)
}

/// Parses the JSON payload returned by `GetFocused` (a `GVariant` `(s)` tuple).
#[must_use]
pub fn parse_call_output(output: &str) -> Option<ForegroundApp> {
    let json = extract_gvariant_string(output)?;
    parse_focused_payload(&json)
}

/// Parses a `gdbus monitor` line carrying a `FocusedChanged` signal.
#[must_use]
pub fn parse_monitor_line(line: &str) -> Option<ForegroundApp> {
    if !line.contains("FocusedChanged") {
        return None;
    }
    let json = extract_gvariant_string(line)?;
    parse_focused_payload(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get_focused_tuple() {
        let out = r#"('{"app_id":"org.mozilla.firefox","class":"firefox","title":"X","pid":7}',)"#;
        let app = parse_call_output(out).expect("parsed");
        assert_eq!(app.app_id.as_deref(), Some("org.mozilla.firefox"));
        assert_eq!(app.class.as_deref(), Some("firefox"));
        assert_eq!(app.pid, Some(7));
        assert_eq!(app.source, ForegroundSourceKind::Gnome);
    }

    #[test]
    fn parses_monitor_signal_line() {
        let line = "/org/docloulou/WheeltaniForeground: org.docloulou.WheeltaniForeground.FocusedChanged ('{\"class\":\"code\"}',)";
        let app = parse_monitor_line(line).expect("parsed");
        assert_eq!(app.class.as_deref(), Some("code"));
    }

    #[test]
    fn ignores_unrelated_monitor_lines() {
        let line = "/org/docloulou/WheeltaniForeground: org.freedesktop.DBus.NameAcquired ('x',)";
        assert!(parse_monitor_line(line).is_none());
    }

    #[test]
    fn empty_object_is_no_window() {
        let out = "('{}',)";
        assert!(parse_call_output(out).is_none());
    }

    #[test]
    fn handles_escaped_quotes_in_title() {
        let line = r#"x FocusedChanged ('{"class":"code","title":"it\'s fine"}',)"#;
        let app = parse_monitor_line(line).expect("parsed");
        assert_eq!(app.class.as_deref(), Some("code"));
        assert_eq!(app.title.as_deref(), Some("it's fine"));
    }
}
