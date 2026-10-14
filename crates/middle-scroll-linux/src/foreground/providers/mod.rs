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

use std::io::{self, Read};
use std::os::unix::process::CommandExt;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use serde_json::Value;
use tracing::{debug, info, warn};

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

/// How much of a helper's stdout or stderr is kept. Helpers print a line or a
/// small JSON object; the rest of a runaway output is read and dropped.
const MAX_HELPER_OUTPUT: u64 = 1024 * 1024;

/// Runs `cmd` to completion, killing it if it exceeds `timeout`.
///
/// Returns `Ok(None)` when the child timed out. Both pipes are drained on
/// their own threads while the child runs, so a helper printing more than the
/// pipe buffer cannot block on write and hang until the timeout. The child
/// gets its own process group and the whole group is killed on timeout: a
/// shell-script helper's own children (`qdbus`, `journalctl`, `sleep`) would
/// otherwise survive it and keep the pipes open.
pub(super) fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> io::Result<Option<Output>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()?;
    let stdout = child.stdout.take().map(spawn_drain);
    let stderr = child.stderr.take().map(spawn_drain);
    let deadline = Instant::now() + timeout;
    let kill = |child: &mut std::process::Child| {
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    };
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            kill(&mut child);
            return Ok(None);
        }
        thread::sleep(WAIT_POLL_INTERVAL);
    };
    // A background grandchild can still hold a pipe open after the helper
    // exits; the output is only waited for until the same deadline.
    let readers = [&stdout, &stderr];
    while readers
        .iter()
        .any(|r| r.as_ref().is_some_and(|h| !h.is_finished()))
    {
        if Instant::now() >= deadline {
            kill(&mut child);
            return Ok(None);
        }
        thread::sleep(WAIT_POLL_INTERVAL);
    }
    let collect = |handle: Option<thread::JoinHandle<Vec<u8>>>| {
        handle.and_then(|h| h.join().ok()).unwrap_or_default()
    };
    Ok(Some(Output {
        status,
        stdout: collect(stdout),
        stderr: collect(stderr),
    }))
}

/// Reads `pipe` to the end on a new thread, keeping the first
/// [`MAX_HELPER_OUTPUT`] bytes.
fn spawn_drain(pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut kept = Vec::new();
        let mut limited = pipe.take(MAX_HELPER_OUTPUT);
        let _ = limited.read_to_end(&mut kept);
        let _ = io::copy(&mut limited.into_inner(), &mut io::sink());
        kept
    })
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

/// First and longest delay between two `auto` detection attempts.
const DETECT_RETRY_START: Duration = Duration::from_millis(500);
const DETECT_RETRY_MAX: Duration = Duration::from_secs(10);

fn auto_select(cfg: &ForegroundConfig) -> Box<dyn ForegroundProvider> {
    if let Some(provider) = detect(cfg) {
        return provider;
    }
    if !cfg.command.is_empty() {
        info!("foreground provider selected: command (auto)");
        return Box::new(CommandProvider::start(
            cfg.command.clone(),
            cfg.command_refresh_ms,
        ));
    }
    warn!("no compositor detected yet; retrying in the background");
    Box::new(AutoProvider::start(cfg.clone()))
}

/// Tries every compositor provider in order; `None` when none is reachable.
fn detect(cfg: &ForegroundConfig) -> Option<Box<dyn ForegroundProvider>> {
    if hyprland::is_available() {
        info!("foreground provider selected: hyprland (auto)");
        return Some(Box::new(HyprlandProvider::start()));
    }
    if sway::is_available() {
        info!("foreground provider selected: sway (auto)");
        return Some(Box::new(SwayProvider::start()));
    }
    if gnome::is_available() {
        info!("foreground provider selected: gnome (auto)");
        return Some(Box::new(GnomeProvider::start()));
    }
    if kde::is_available() {
        info!("foreground provider selected: kde (auto)");
        return Some(Box::new(KdeProvider::start(cfg.command_refresh_ms)));
    }
    None
}

/// `provider = "auto"` when nothing was detected at startup: keeps detecting
/// on a background thread, with a growing delay, and hands every snapshot to
/// the provider it finds.
///
/// The systemd unit is wanted by `default.target`, which is reached before
/// Hyprland or Sway start (without uwsm) and before GNOME Shell loads its
/// extensions. Detecting only once would leave the filter without a provider
/// for the whole session.
struct AutoProvider {
    found: Arc<Mutex<Option<Box<dyn ForegroundProvider>>>>,
}

impl std::fmt::Debug for AutoProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let found = self.found.lock().map_or(true, |found| found.is_some());
        f.debug_struct("AutoProvider")
            .field("found", &found)
            .finish()
    }
}

impl AutoProvider {
    fn start(cfg: ForegroundConfig) -> Self {
        let found = Arc::new(Mutex::new(None));
        let slot = found.clone();
        let spawned = thread::Builder::new()
            .name("wheeltani-fg-auto".to_owned())
            .spawn(move || {
                let mut delay = DETECT_RETRY_START;
                loop {
                    thread::sleep(delay);
                    if let Some(provider) = detect(&cfg) {
                        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(provider);
                        return;
                    }
                    debug!(?delay, "foreground auto-detection: still no compositor");
                    delay = (delay * 2).min(DETECT_RETRY_MAX);
                }
            });
        if let Err(err) = spawned {
            warn!(%err, "failed to spawn the foreground auto-detection thread");
        }
        Self { found }
    }
}

impl ForegroundProvider for AutoProvider {
    fn snapshot(&self) -> ForegroundSnapshot {
        // The lock is only ever taken by this call and by the single store
        // of the detected provider, so it never waits on provider start-up.
        let found = self.found.lock().unwrap_or_else(PoisonError::into_inner);
        found.as_ref().map_or_else(
            || ForegroundSnapshot::Unsupported {
                reason: "no compositor detected yet (still looking)".to_owned(),
            },
            |provider| provider.snapshot(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", script]);
        cmd
    }

    #[test]
    fn large_output_does_not_block_the_helper() {
        // 4 MiB is well past the 64 KiB pipe buffer: without draining while
        // the child runs, it would block on write until the timeout.
        let started = Instant::now();
        let out = run_with_timeout(
            &mut sh("head -c 4194304 /dev/zero; echo done >&2"),
            Duration::from_secs(10),
        )
        .expect("spawned")
        .expect("finished before the timeout");
        assert!(out.status.success());
        assert_eq!(out.stdout.len() as u64, MAX_HELPER_OUTPUT);
        assert_eq!(out.stderr, b"done\n");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn timeout_kills_the_helper_and_its_children() {
        let started = Instant::now();
        let out = run_with_timeout(&mut sh("sleep 30 & sleep 30"), Duration::from_millis(200))
            .expect("spawned");
        assert!(out.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn background_child_holding_the_pipe_times_out() {
        // The helper exits at once but leaves a child with its stdout open.
        let started = Instant::now();
        let out = run_with_timeout(&mut sh("echo hi; sleep 30 &"), Duration::from_millis(300))
            .expect("spawned");
        assert!(out.is_none());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn short_output_is_returned() {
        let out = run_with_timeout(&mut sh("echo firefox"), Duration::from_secs(5))
            .expect("spawned")
            .expect("finished");
        assert_eq!(out.stdout, b"firefox\n");
    }
}
