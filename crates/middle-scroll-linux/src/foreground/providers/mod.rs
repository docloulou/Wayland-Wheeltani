//! Foreground providers and the `auto` selection logic.
//!
//! Every threaded provider keeps a [`SharedSnapshot`] up to date on a
//! background thread; `snapshot()` is just a non-blocking read.

mod command;
mod gnome;
mod hyprland;
mod kde;
mod none;
mod sway;

use std::process::{Command, Output, Stdio};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use tracing::{info, warn};

use super::config::{ForegroundConfig, ForegroundProviderKind};
use super::filter::{ForegroundApp, ForegroundProvider, ForegroundSnapshot, ForegroundSourceKind};

pub use command::CommandProvider;
pub use gnome::GnomeProvider;
pub use hyprland::HyprlandProvider;
pub use kde::KdeProvider;
pub use none::NoneProvider;
pub use sway::SwayProvider;

/// Shared, thread-updated snapshot storage.
pub(super) type SharedSnapshot = Arc<RwLock<ForegroundSnapshot>>;

/// Reads the current snapshot, recovering gracefully from a poisoned lock.
pub(super) fn read_snapshot(shared: &SharedSnapshot) -> ForegroundSnapshot {
    match shared.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Stores a new snapshot, ignoring a poisoned lock (we simply overwrite).
pub(super) fn store(shared: &SharedSnapshot, snapshot: ForegroundSnapshot) {
    match shared.write() {
        Ok(mut guard) => *guard = snapshot,
        Err(poisoned) => *poisoned.into_inner() = snapshot,
    }
}

/// How long provider helper subprocesses (`gdbus`, `kdotool`, the user's
/// `command`) are allowed to run before being killed. Prevents a hung helper
/// from freezing a provider thread forever with a stale snapshot.
pub(super) const HELPER_TIMEOUT: Duration = Duration::from_secs(5);

/// Interval between `try_wait` polls in [`run_with_timeout`].
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Runs `cmd` to completion, killing it if it exceeds `timeout`.
///
/// Returns `Ok(None)` when the child was killed on timeout. The child's
/// stdout/stderr are piped and collected after exit; this is safe here because
/// every helper prints at most a few hundred bytes (well under the pipe
/// buffer), so the child can never block on a full pipe before exiting.
pub(super) fn run_with_timeout(
    cmd: &mut Command,
    timeout: Duration,
) -> std::io::Result<Option<Output>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(_) => return child.wait_with_output().map(Some),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(None);
            }
            None => std::thread::sleep(WAIT_POLL_INTERVAL),
        }
    }
}

/// True when `name` currently has an owner on the session bus. Shells out to
/// `gdbus` (no D-Bus crate, per the dependency policy); a missing `gdbus`, a
/// failed call, or a timeout is treated as "not owned". Shared by the GNOME
/// and KDE providers.
pub(super) fn dbus_name_has_owner(name: &str) -> bool {
    run_with_timeout(
        Command::new("gdbus").args([
            "call",
            "--session",
            "--dest",
            "org.freedesktop.DBus",
            "--object-path",
            "/org/freedesktop/DBus",
            "--method",
            "org.freedesktop.DBus.NameHasOwner",
            name,
        ]),
        HELPER_TIMEOUT,
    )
    .ok()
    .flatten()
    .is_some_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).contains("true"))
}

/// Builds a [`ForegroundApp`] from a JSON object shared by the `command` and
/// `gnome` providers: `{app_id, class, resource_class, title, pid}`.
pub(super) fn json_to_app(v: &Value, source: ForegroundSourceKind) -> ForegroundApp {
    let s = |key: &str| {
        v.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let pid = v
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p).ok());
    ForegroundApp {
        app_id: s("app_id"),
        class: s("class"),
        resource_class: s("resource_class"),
        title: s("title"),
        pid,
        source,
    }
}

/// Builds the provider for the resolved config, applying auto-detection when
/// `provider = "auto"`.
#[must_use]
pub fn select_provider(cfg: &ForegroundConfig) -> Box<dyn ForegroundProvider> {
    match cfg.provider {
        ForegroundProviderKind::None => {
            info!("foreground provider selected: none");
            Box::new(NoneProvider::new("provider set to none"))
        }
        ForegroundProviderKind::Hyprland => {
            info!("foreground provider selected: hyprland");
            Box::new(HyprlandProvider::start())
        }
        ForegroundProviderKind::Sway => {
            info!("foreground provider selected: sway");
            Box::new(SwayProvider::start())
        }
        ForegroundProviderKind::Gnome => {
            info!("foreground provider selected: gnome");
            Box::new(GnomeProvider::start())
        }
        ForegroundProviderKind::Kde => {
            info!("foreground provider selected: kde");
            Box::new(KdeProvider::start(cfg.command_refresh_ms))
        }
        ForegroundProviderKind::Command => {
            info!("foreground provider selected: command");
            Box::new(CommandProvider::start(
                cfg.command.clone(),
                cfg.command_refresh_ms,
            ))
        }
        ForegroundProviderKind::Auto => auto_select(cfg),
    }
}

fn auto_select(cfg: &ForegroundConfig) -> Box<dyn ForegroundProvider> {
    if hyprland::is_available() {
        info!("foreground provider selected: hyprland (auto)");
        return Box::new(HyprlandProvider::start());
    }
    if sway::is_available() {
        info!("foreground provider selected: sway (auto)");
        return Box::new(SwayProvider::start());
    }
    if gnome::is_available() {
        info!("foreground provider selected: gnome (auto)");
        return Box::new(GnomeProvider::start());
    }
    if kde::is_available() {
        info!("foreground provider selected: kde (auto)");
        return Box::new(KdeProvider::start(cfg.command_refresh_ms));
    }
    if !cfg.command.is_empty() {
        info!("foreground provider selected: command (auto)");
        return Box::new(CommandProvider::start(
            cfg.command.clone(),
            cfg.command_refresh_ms,
        ));
    }
    warn!("foreground provider unsupported: no compositor environment detected");
    Box::new(NoneProvider::new("no compositor environment detected"))
}
