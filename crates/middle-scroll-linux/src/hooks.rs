//! Indicator hooks: user commands run when autoscroll starts or stops, and
//! when the daemon is paused or resumed.
//!
//! A built-in on-screen indicator is not portable on Wayland: there is no
//! protocol to read the global pointer position, and `wlr-layer-shell` (needed
//! for an overlay) is missing on GNOME. Running user commands instead lets each
//! desktop use what it has: `notify-send`, a cursor-theme switch, an `eww` or
//! `waybar` update, a sound...

use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// `[hooks]` table: each entry is an argv (program then arguments), run
/// without a shell. An empty list disables the hook.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HooksConfig {
    /// Run when autoscroll starts (the pointer left the deadzone, or toggle
    /// mode latched).
    pub on_enter: Vec<String>,
    /// Run when autoscroll stops (including the end of an inertia coast).
    pub on_exit: Vec<String>,
    /// Run when the daemon is paused (`--pause`, `--toggle`).
    pub on_pause: Vec<String>,
    /// Run when the daemon is resumed (`--resume`, `--toggle`).
    pub on_resume: Vec<String>,
}

/// What happened; also exported to the hook as `WHEELTANI_STATE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEvent {
    Enter,
    Exit,
    Pause,
    Resume,
}

impl HookEvent {
    /// Value of `WHEELTANI_STATE` for this event.
    pub const fn state(self) -> &'static str {
        match self {
            Self::Enter => "scrolling",
            Self::Exit => "idle",
            Self::Pause => "paused",
            Self::Resume => "active",
        }
    }
}

/// Runs the configured hook commands. Commands are spawned in the background
/// and never block the input path; a reaper thread collects each child.
#[derive(Debug, Default)]
pub struct Hooks {
    cfg: HooksConfig,
}

impl Hooks {
    pub const fn new(cfg: HooksConfig) -> Self {
        Self { cfg }
    }

    pub fn set_config(&mut self, cfg: HooksConfig) {
        self.cfg = cfg;
    }

    pub fn fire(&self, event: HookEvent) {
        let argv = match event {
            HookEvent::Enter => &self.cfg.on_enter,
            HookEvent::Exit => &self.cfg.on_exit,
            HookEvent::Pause => &self.cfg.on_pause,
            HookEvent::Resume => &self.cfg.on_resume,
        };
        let Some((program, args)) = argv.split_first() else {
            return;
        };
        let spawned = Command::new(program)
            .args(args)
            .env("WHEELTANI_STATE", event.state())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                debug!(?event, program, pid = child.id(), "hook started");
                let reaper = std::thread::Builder::new()
                    .name("hook-reaper".to_owned())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                if let Err(err) = reaper {
                    warn!(?err, "could not start the hook reaper thread");
                }
            }
            Err(err) => warn!(?event, program, %err, "failed to run hook"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_are_stable() {
        assert_eq!(HookEvent::Enter.state(), "scrolling");
        assert_eq!(HookEvent::Exit.state(), "idle");
        assert_eq!(HookEvent::Pause.state(), "paused");
        assert_eq!(HookEvent::Resume.state(), "active");
    }

    #[test]
    fn hook_receives_state_in_environment() {
        let dir = std::env::temp_dir().join(format!("wheeltani-hook-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("state");
        let hooks = Hooks::new(HooksConfig {
            on_pause: vec![
                "sh".to_owned(),
                "-c".to_owned(),
                format!("printf %s \"$WHEELTANI_STATE\" > '{}'", out.display()),
            ],
            ..HooksConfig::default()
        });
        hooks.fire(HookEvent::Pause);
        // Empty hooks are a no-op.
        hooks.fire(HookEvent::Enter);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let content = loop {
            match std::fs::read_to_string(&out) {
                Ok(s) if !s.is_empty() => break s,
                _ if std::time::Instant::now() >= deadline => panic!("hook did not run"),
                _ => std::thread::sleep(std::time::Duration::from_millis(20)),
            }
        };
        assert_eq!(content, "paused");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
