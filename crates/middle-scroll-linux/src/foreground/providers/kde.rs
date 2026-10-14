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
//! One chained call (`getactivewindow getwindowclassname getwindowname`)
//! returns both the class and the title, so `match_title` rules work on KDE
//! too. A `kdotool` that rejects the chain falls back to the class alone.
//!
//! A push-based `KWin` script (`workspace.windowActivated` calling back into
//! the daemon) would avoid polling, but receiving its calls needs a D-Bus
//! service, which the no-D-Bus-crate dependency policy rules out.
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
    let mut with_title = true;
    loop {
        let snapshot = match query_once(with_title) {
            Err(reason) if with_title => {
                // An older kdotool may not chain both getters; keep the class.
                debug!(%reason, "kdotool class+title query failed; trying the class alone");
                let fallback = query_once(false);
                if fallback.is_ok() {
                    with_title = false;
                }
                fallback
            }
            other => other,
        };
        store(
            shared,
            snapshot.unwrap_or_else(|reason| ForegroundSnapshot::Unknown { reason }),
        );
        thread::sleep(refresh);
    }
}

fn query_once(with_title: bool) -> Result<ForegroundSnapshot, String> {
    let args: &[&str] = if with_title {
        &["getactivewindow", "getwindowclassname", "getwindowname"]
    } else {
        &["getactivewindow", "getwindowclassname"]
    };
    let out = run_with_timeout(Command::new("kdotool").args(args), HELPER_TIMEOUT)
        .map_err(|e| format!("failed to run kdotool: {e}"))?
        .ok_or_else(|| "kdotool timed out (killed)".to_owned())?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        debug!(status = %out.status, stderr = %stderr.trim(), "kdotool exited unsuccessfully");
        return Err(format!("kdotool exited with {}", out.status));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let parsed = if with_title {
        parse_window_output(&stdout)
    } else {
        parse_class_output(&stdout).map(|class| (class, None))
    };
    Ok(parsed.map_or_else(
        || ForegroundSnapshot::Unknown {
            reason: "kdotool reported no active window".to_owned(),
        },
        |(class, title)| ForegroundSnapshot::Known(app_from_window(&class, title)),
    ))
}

/// Builds a [`ForegroundApp`] from a `KWin` resource class and caption. The
/// class is used for both `class` and `resource_class` (`kdotool` returns the
/// WM resource class, which for native KDE apps is typically the desktop id,
/// e.g. `org.kde.dolphin`, and the WM class for X/`XWayland` apps, e.g.
/// `firefox`). `app_id` is left unset because `kdotool` does not expose the
/// native Wayland `app_id` distinctly.
fn app_from_window(class: &str, title: Option<String>) -> ForegroundApp {
    ForegroundApp {
        app_id: None,
        class: Some(class.to_owned()),
        resource_class: Some(class.to_owned()),
        title,
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

/// Parses `kdotool getactivewindow getwindowclassname getwindowname` stdout:
/// the class line, then the title line (possibly empty). Window-id lines are
/// skipped wherever they appear. Returns `None` when there is no class.
#[must_use]
pub fn parse_window_output(stdout: &str) -> Option<(String, Option<String>)> {
    let mut lines = stdout.lines().map(str::trim).filter(|l| !is_window_id(l));
    let class = lines.find(|l| !l.is_empty())?.to_owned();
    let title = lines.next().filter(|t| !t.is_empty()).map(str::to_owned);
    Some((class, title))
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
        let app = app_from_window("org.kde.dolphin", None);
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
    fn parses_class_and_title() {
        let (class, title) =
            parse_window_output("org.kde.dolphin\nHome — Dolphin\n").expect("parsed");
        assert_eq!(class, "org.kde.dolphin");
        assert_eq!(title.as_deref(), Some("Home — Dolphin"));
    }

    #[test]
    fn empty_title_is_none() {
        let (class, title) = parse_window_output("konsole\n\n").expect("parsed");
        assert_eq!(class, "konsole");
        assert!(title.is_none());
        let (_, title) = parse_window_output("konsole\n").expect("parsed");
        assert!(title.is_none());
    }

    #[test]
    fn window_id_lines_are_skipped_before_class_and_title() {
        let out = "{2f4a1c0e-0000}\nfirefox\n{2f4a1c0e-0000}\nMozilla Firefox\r\n";
        let (class, title) = parse_window_output(out).expect("parsed");
        assert_eq!(class, "firefox");
        assert_eq!(title.as_deref(), Some("Mozilla Firefox"));
    }

    #[test]
    fn no_class_means_no_window() {
        assert!(parse_window_output("").is_none());
        assert!(parse_window_output("{2f4a1c0e-0000}\n\n").is_none());
    }

    #[test]
    fn app_from_window_keeps_title() {
        let app = app_from_window("code", Some("main.rs".to_owned()));
        assert_eq!(app.title.as_deref(), Some("main.rs"));
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
        let app = app_from_window("firefox", None);
        assert_eq!(app.class.as_deref(), Some("firefox"));
        assert_eq!(app.resource_class.as_deref(), Some("firefox"));
        assert!(app.title.is_none());
        assert!(app.pid.is_none());
    }
}
