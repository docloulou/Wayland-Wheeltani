//! KDE Plasma (`KWin`) provider.
//!
//! `KWin` (Wayland) exposes no readable D-Bus API for the focused window; the
//! only reliable route is its scripting API (inject a script, run it, scrape the
//! result back). Rather than ship and maintain an untested `KWin`-script +
//! `dbus-monitor` bridge, this provider shells out to
//! [`kdotool`](https://github.com/jinliu/kdotool) — a small, well-maintained
//! `xdotool`-like helper for Plasma 6 that performs the load/run/unload
//! `KWin`-script dance internally. It is polled on a background thread, exactly
//! like the `command` provider, so the input hot path never blocks.
//!
//! Install `kdotool` with your distribution's package or `cargo install kdotool`.

use std::process::Command;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use tracing::debug;

use crate::foreground::filter::{
    ForegroundApp, ForegroundProvider, ForegroundSnapshot, ForegroundSourceKind,
};

use super::{
    dbus_name_has_owner, read_snapshot, run_with_timeout, store, SharedSnapshot, HELPER_TIMEOUT,
};

/// `KWin`'s well-known session-bus name; present whenever a Plasma/`KWin`
/// session is running and reachable from this process.
const KWIN_BUS_NAME: &str = "org.kde.KWin";

/// Floor for the poll interval to avoid a busy loop on a misconfigured `0`.
const MIN_REFRESH: Duration = Duration::from_millis(100);

#[derive(Debug)]
pub struct KdeProvider {
    shared: SharedSnapshot,
}

/// True when `KWin` is on the session bus **and** `kdotool` is available — both
/// are required for this provider to resolve anything, so `auto` only selects KDE
/// when it can actually work (otherwise it falls through to `command`/`none`).
///
/// `KWin` presence is probed through `gdbus` (no D-Bus crate, per the dependency
/// policy). If `gdbus` is absent on an otherwise-KDE box, auto-detection will
/// skip KDE; users can still force it with `provider = "kde"`.
#[must_use]
pub fn is_available() -> bool {
    if !dbus_name_has_owner(KWIN_BUS_NAME) {
        debug!(
            bus = KWIN_BUS_NAME,
            "kde: KWin not owned on the session bus (not a KDE session, or gdbus missing)"
        );
        return false;
    }
    if !kdotool_available() {
        debug!("kde: kdotool not found in PATH (install kdotool for KDE support)");
        return false;
    }
    true
}

fn kdotool_available() -> bool {
    // We only need to know the binary can be launched; `--help` exits 0 but we
    // accept any successful run regardless of status (spawning only errors when
    // the binary is missing or cannot be executed).
    run_with_timeout(Command::new("kdotool").arg("--help"), HELPER_TIMEOUT)
        .is_ok_and(|out| out.is_some())
}

impl KdeProvider {
    pub fn start(refresh_ms: u64) -> Self {
        let shared: SharedSnapshot = Arc::new(RwLock::new(ForegroundSnapshot::Unknown {
            reason: "kde provider starting".to_owned(),
        }));

        if !kdotool_available() {
            store(
                &shared,
                ForegroundSnapshot::Unsupported {
                    reason: "kdotool not found (install kdotool for KDE support)".to_owned(),
                },
            );
            return Self { shared };
        }

        // Poll on a background thread. The first iteration runs immediately (no
        // initial sleep), so the snapshot is populated within one kdotool call of
        // startup without blocking the daemon's main thread on a possibly-slow
        // (or hanging) kdotool invocation.
        let bg = shared.clone();
        let refresh = Duration::from_millis(refresh_ms).max(MIN_REFRESH);
        let spawned = thread::Builder::new()
            .name("wheeltani-fg-kde".to_owned())
            .spawn(move || poll_loop(refresh, &bg));
        if let Err(err) = spawned {
            store(
                &shared,
                ForegroundSnapshot::Unsupported {
                    reason: format!("failed to spawn kde thread: {err}"),
                },
            );
        }

        Self { shared }
    }
}

impl ForegroundProvider for KdeProvider {
    fn snapshot(&self) -> ForegroundSnapshot {
        read_snapshot(&self.shared)
    }
}

fn poll_loop(refresh: Duration, shared: &SharedSnapshot) {
    loop {
        store(shared, query_once());
        thread::sleep(refresh);
    }
}

fn query_once() -> ForegroundSnapshot {
    match run_kdotool_class() {
        Ok(Some(class)) => ForegroundSnapshot::Known(app_from_class(&class)),
        Ok(None) => ForegroundSnapshot::Unknown {
            reason: "kdotool reported no active window".to_owned(),
        },
        Err(reason) => ForegroundSnapshot::Unknown { reason },
    }
}

fn run_kdotool_class() -> Result<Option<String>, String> {
    let out = run_with_timeout(
        Command::new("kdotool").args(["getactivewindow", "getwindowclassname"]),
        HELPER_TIMEOUT,
    )
    .map_err(|e| format!("failed to run kdotool: {e}"))?
    .ok_or_else(|| "kdotool timed out (killed)".to_owned())?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        debug!(status = %out.status, stderr = %stderr.trim(), "kdotool exited unsuccessfully");
        return Err(format!("kdotool exited with {}", out.status));
    }
    Ok(parse_class_output(&String::from_utf8_lossy(&out.stdout)))
}

/// Builds a [`ForegroundApp`] from a `KWin` resource class. The same value is
/// used for `class` and `resource_class` (`kdotool` returns the WM resource
/// class, which for native KDE apps is typically the desktop id, e.g.
/// `org.kde.dolphin`, and the WM class for X/`XWayland` apps, e.g. `firefox`).
/// `app_id` is left unset because `kdotool` does not expose the native Wayland
/// `app_id` distinctly.
fn app_from_class(class: &str) -> ForegroundApp {
    ForegroundApp {
        app_id: None,
        class: Some(class.to_owned()),
        resource_class: Some(class.to_owned()),
        title: None,
        pid: None,
        source: ForegroundSourceKind::Kde,
    }
}

/// Parses `kdotool getactivewindow getwindowclassname` stdout. The class is the
/// last meaningful line; a window-id line (`{uuid}`) or empty output means no
/// resolvable active window.
#[must_use]
pub fn parse_class_output(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && !is_window_id(l))
        .map(str::to_owned)
}

/// `kdotool` prints `KWin` window ids as `{uuid}`; that is never a real class
/// name.
fn is_window_id(s: &str) -> bool {
    s.starts_with('{') && s.ends_with('}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_class() {
        assert_eq!(parse_class_output("firefox\n").as_deref(), Some("firefox"));
    }

    #[test]
    fn parses_desktop_id_class() {
        assert_eq!(
            parse_class_output("org.kde.dolphin\n").as_deref(),
            Some("org.kde.dolphin")
        );
    }

    #[test]
    fn ignores_leading_window_id_line() {
        // If kdotool ever echoes the window id before the class, take the class.
        let out = "{2f4a1c0e-0000-0000-0000-000000000000}\nkonsole\n";
        assert_eq!(parse_class_output(out).as_deref(), Some("konsole"));
    }

    #[test]
    fn empty_output_is_none() {
        assert!(parse_class_output("   \n").is_none());
    }

    #[test]
    fn only_window_id_is_none() {
        assert!(parse_class_output("{2f4a1c0e-aaaa}\n").is_none());
    }

    #[test]
    fn app_from_class_sets_class_and_resource_class() {
        let app = app_from_class("org.kde.dolphin");
        assert_eq!(app.class.as_deref(), Some("org.kde.dolphin"));
        assert_eq!(app.resource_class.as_deref(), Some("org.kde.dolphin"));
        assert!(app.app_id.is_none());
        assert_eq!(app.source, ForegroundSourceKind::Kde);
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            parse_class_output("   firefox  \n").as_deref(),
            Some("firefox")
        );
    }

    #[test]
    fn handles_crlf_line_endings() {
        assert_eq!(
            parse_class_output("org.kde.konsole\r\n").as_deref(),
            Some("org.kde.konsole")
        );
    }

    #[test]
    fn ignores_blank_lines_around_class() {
        assert_eq!(
            parse_class_output("\n\n  brave-browser \n\n").as_deref(),
            Some("brave-browser")
        );
    }

    #[test]
    fn picks_last_non_id_line() {
        // Defensive: if kdotool ever emitted several lines, the class is last.
        let out = "{2f4a1c0e-aaaa}\nfirst\nsecond\n";
        assert_eq!(parse_class_output(out).as_deref(), Some("second"));
    }

    #[test]
    fn window_id_among_classes_is_skipped() {
        let out = "konsole\n{2f4a1c0e-bbbb}\n";
        assert_eq!(parse_class_output(out).as_deref(), Some("konsole"));
    }

    #[test]
    fn preserves_dotted_class_unchanged() {
        // Matching/normalization happens later; the provider must not mangle it.
        assert_eq!(
            parse_class_output("org.mozilla.firefox\n").as_deref(),
            Some("org.mozilla.firefox")
        );
    }

    #[test]
    fn is_window_id_recognises_braced_uuid_only() {
        assert!(is_window_id("{2f4a1c0e-0000-0000-0000-000000000000}"));
        assert!(!is_window_id("firefox"));
        assert!(!is_window_id("org.kde.dolphin"));
        assert!(!is_window_id("{not-closed"));
        assert!(!is_window_id("not-opened}"));
    }

    #[test]
    fn app_from_x_app_class_keeps_wm_class() {
        let app = app_from_class("firefox");
        assert_eq!(app.class.as_deref(), Some("firefox"));
        assert_eq!(app.resource_class.as_deref(), Some("firefox"));
        assert!(app.title.is_none());
        assert!(app.pid.is_none());
    }
}
