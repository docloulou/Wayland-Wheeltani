//! Configuration types for the optional foreground-application filter.
//!
//! The file-facing types (`ForegroundFileConfig` and the enums) are
//! `serde`-derived so they can live inside the daemon's TOML config under the
//! `[foreground]` table. `ForegroundConfig` is the fully-resolved runtime form
//! with all defaults applied; its `Default` reproduces the historical
//! behaviour (filter disabled), so an absent `[foreground]` table changes
//! nothing.

use serde::{Deserialize, Serialize};

/// Which backend resolves the currently focused application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundProviderKind {
    /// Pick the best available provider for the running session.
    #[default]
    Auto,
    /// Never resolve a foreground app (filter effectively inert).
    None,
    /// Hyprland `.socket2.sock` event stream.
    Hyprland,
    /// Sway / i3 IPC.
    Sway,
    /// GNOME Shell via the bundled extension and its D-Bus service.
    Gnome,
    /// KDE Plasma (`KWin`) via the `kdotool` helper.
    Kde,
    /// An external command that prints the focused app.
    Command,
}

/// How the allow/deny lists are interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundMode {
    /// Autoscroll everywhere except apps in `deny_apps`.
    #[default]
    Denylist,
    /// Autoscroll only in apps in `allow_apps`.
    Allowlist,
}

/// What to do when the foreground application cannot be determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnknownPolicy {
    /// Keep the historical behaviour (autoscroll stays enabled).
    #[default]
    Enabled,
    /// Pass mouse events straight through when the app is unknown.
    Disabled,
}

/// `[foreground]` table as parsed from the TOML config file. Every field is
/// optional, so a missing value falls back to [`ForegroundConfig`] defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ForegroundFileConfig {
    pub enabled: Option<bool>,
    pub provider: Option<ForegroundProviderKind>,
    pub mode: Option<ForegroundMode>,
    pub unknown_policy: Option<UnknownPolicy>,
    pub allow_apps: Option<Vec<String>>,
    pub deny_apps: Option<Vec<String>>,
    pub match_title: Option<bool>,
    pub command: Option<Vec<String>>,
    pub command_refresh_ms: Option<u64>,
}

/// Fully-resolved foreground filter configuration with all defaults applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForegroundConfig {
    pub enabled: bool,
    pub provider: ForegroundProviderKind,
    pub mode: ForegroundMode,
    pub unknown_policy: UnknownPolicy,
    pub allow_apps: Vec<String>,
    pub deny_apps: Vec<String>,
    pub match_title: bool,
    pub command: Vec<String>,
    pub command_refresh_ms: u64,
}

impl Default for ForegroundConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: ForegroundProviderKind::Auto,
            mode: ForegroundMode::Denylist,
            unknown_policy: UnknownPolicy::Enabled,
            allow_apps: Vec::new(),
            deny_apps: Vec::new(),
            match_title: false,
            command: Vec::new(),
            command_refresh_ms: 500,
        }
    }
}

impl ForegroundFileConfig {
    /// Applies the parsed file values on top of the built-in defaults.
    #[must_use]
    pub fn into_resolved(self) -> ForegroundConfig {
        let d = ForegroundConfig::default();
        ForegroundConfig {
            enabled: self.enabled.unwrap_or(d.enabled),
            provider: self.provider.unwrap_or(d.provider),
            mode: self.mode.unwrap_or(d.mode),
            unknown_policy: self.unknown_policy.unwrap_or(d.unknown_policy),
            allow_apps: self.allow_apps.unwrap_or(d.allow_apps),
            deny_apps: self.deny_apps.unwrap_or(d.deny_apps),
            match_title: self.match_title.unwrap_or(d.match_title),
            command: self.command.unwrap_or(d.command),
            command_refresh_ms: self.command_refresh_ms.unwrap_or(d.command_refresh_ms),
        }
    }
}
