//! Foreground application model, the provider trait, and the decision gate.

use std::fmt;

use super::config::{ForegroundConfig, UnknownPolicy};
use super::matcher;

/// A snapshot of the currently focused application, as reported by a provider.
///
/// All identity fields are optional because different compositors expose
/// different subsets (Hyprland exposes a window class, Sway can expose an
/// `app_id` for native Wayland and a WM class for `XWayland`, etc.). The matcher
/// compares the configured lists against every populated field.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForegroundApp {
    pub app_id: Option<String>,
    pub class: Option<String>,
    pub resource_class: Option<String>,
    pub title: Option<String>,
    pub pid: Option<u32>,
    pub source: ForegroundSourceKind,
}

/// Where a [`ForegroundApp`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForegroundSourceKind {
    #[default]
    None,
    Command,
    Hyprland,
    Sway,
    Gnome,
    /// KDE Plasma (`KWin`), resolved through the `kdotool` helper.
    Kde,
    // Part of the documented source taxonomy; reserved for providers that report
    // an indeterminate origin.
    #[allow(dead_code)]
    Unknown,
}

/// The result of querying a provider for the active window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForegroundSnapshot {
    /// The focused application is known.
    Known(ForegroundApp),
    /// The provider is running but cannot currently tell which app is focused.
    Unknown { reason: String },
    /// No usable provider for this session (wrong compositor, missing helper).
    Unsupported { reason: String },
}

/// A foreground provider keeps the latest known window up to date in the
/// background and exposes it through a cheap, non-blocking [`snapshot`].
///
/// Implementations MUST NOT block in `snapshot()`: the input hot path calls it
/// on every middle-button press. All I/O (sockets, subprocesses) must happen on
/// a background thread that updates shared state.
///
/// [`snapshot`]: ForegroundProvider::snapshot
pub trait ForegroundProvider: Send {
    fn snapshot(&self) -> ForegroundSnapshot;
}

/// Whether the autoscroll engine should handle a gesture, or whether its
/// events must be passed straight through to the virtual device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoscrollDecision {
    Enabled,
    Disabled,
}

/// The applications a `[[profiles]]` entry applies to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProfileRule {
    pub apps: Vec<String>,
    pub match_title: bool,
}

/// What the gate decided for one trigger gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GateDecision {
    pub decision: AutoscrollDecision,
    /// Index of the first profile whose `apps` match the focused application.
    pub profile: Option<usize>,
}

impl GateDecision {
    const DEFAULT: Self = Self {
        decision: AutoscrollDecision::Enabled,
        profile: None,
    };
}

/// Decision layer between the event router and the autoscroll engine.
///
/// At the start of each trigger gesture it reads the focused application
/// once, decides whether autoscroll is allowed there (`[foreground]`) and
/// which profile applies (`[[profiles]]`), and latches that answer until the
/// gesture ends so a focus change mid-gesture never changes the rules.
pub struct ForegroundGate {
    cfg: ForegroundConfig,
    rules: Vec<ProfileRule>,
    /// Started lazily: only when the filter is enabled or profiles exist.
    provider: Option<Box<dyn ForegroundProvider>>,
    latched: Option<GateDecision>,
}

impl fmt::Debug for ForegroundGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForegroundGate")
            .field("cfg", &self.cfg)
            .field("rules", &self.rules)
            .field("provider_started", &self.provider.is_some())
            .field("latched", &self.latched)
            .finish()
    }
}

impl ForegroundGate {
    /// Builds a gate for `cfg` and the profile `rules`.
    ///
    /// A provider is only started when something needs it: with the filter
    /// disabled and no profiles the gate always answers `Enabled` without a
    /// snapshot, so provider threads (and their `gdbus`/`kdotool`
    /// subprocesses) would only cost CPU and memory.
    #[must_use]
    pub fn new(cfg: ForegroundConfig, rules: Vec<ProfileRule>) -> Self {
        let mut gate = Self {
            cfg,
            rules,
            provider: None,
            latched: None,
        };
        gate.start_provider_if_needed();
        tracing::debug!(
            enabled = gate.cfg.enabled,
            profiles = gate.rules.len(),
            provider = ?gate.cfg.provider,
            mode = ?gate.cfg.mode,
            "foreground gate created"
        );
        gate
    }

    /// Builds a gate with an explicit provider. Used by tests.
    #[cfg(test)]
    #[must_use]
    pub fn with_provider(
        cfg: ForegroundConfig,
        rules: Vec<ProfileRule>,
        provider: Box<dyn ForegroundProvider>,
    ) -> Self {
        Self {
            cfg,
            rules,
            provider: Some(provider),
            latched: None,
        }
    }

    fn needs_provider(&self) -> bool {
        self.cfg.enabled || !self.rules.is_empty()
    }

    fn start_provider_if_needed(&mut self) {
        if self.provider.is_none() && self.needs_provider() {
            self.provider = Some(super::providers::select_provider(&self.cfg));
        }
    }

    /// Applies a reloaded configuration. A gesture in flight keeps its
    /// latched decision. The provider is started if it is now needed; a
    /// different provider or helper command only takes effect after a
    /// restart, because provider threads cannot be stopped cleanly.
    pub fn set_config(&mut self, cfg: ForegroundConfig, rules: Vec<ProfileRule>) {
        let provider_changed = self.cfg.provider != cfg.provider
            || self.cfg.command != cfg.command
            || self.cfg.command_refresh_ms != cfg.command_refresh_ms;
        if self.provider.is_some() && provider_changed {
            tracing::warn!(
                old = ?self.cfg.provider,
                new = ?cfg.provider,
                "the foreground provider settings changed; restart the daemon to apply them"
            );
            // Keep describing the provider that is actually running.
            let ForegroundConfig {
                provider,
                command,
                command_refresh_ms,
                ..
            } = std::mem::take(&mut self.cfg);
            self.cfg = ForegroundConfig {
                provider,
                command,
                command_refresh_ms,
                ..cfg
            };
        } else {
            self.cfg = cfg;
        }
        self.rules = rules;
        self.start_provider_if_needed();
    }

    /// The current snapshot of the focused application, if a provider runs.
    pub fn snapshot(&self) -> Option<ForegroundSnapshot> {
        self.provider.as_ref().map(|p| p.snapshot())
    }

    /// Decides for a new trigger gesture and latches the answer until
    /// [`end_gesture`](Self::end_gesture).
    pub fn begin_gesture(&mut self) -> GateDecision {
        let decision = self.evaluate();
        self.latched = Some(decision);
        decision
    }

    /// The decision of the gesture in flight, if any.
    pub const fn latched(&self) -> Option<GateDecision> {
        self.latched
    }

    /// Releases the latch once the trigger is back up.
    pub const fn end_gesture(&mut self) {
        self.latched = None;
    }

    /// Clears any latched decision. Called after a device disconnect so a
    /// gesture interrupted by an unplug cannot leave a stale latch behind.
    pub const fn reset_latch(&mut self) {
        self.latched = None;
    }

    fn evaluate(&self) -> GateDecision {
        let Some(provider) = self.provider.as_ref().filter(|_| self.needs_provider()) else {
            return GateDecision::DEFAULT;
        };
        let snapshot = provider.snapshot();
        let decision = if self.cfg.enabled {
            match &snapshot {
                ForegroundSnapshot::Known(app) => matcher::decide(app, &self.cfg),
                ForegroundSnapshot::Unknown { .. } | ForegroundSnapshot::Unsupported { .. } => {
                    match self.cfg.unknown_policy {
                        UnknownPolicy::Enabled => AutoscrollDecision::Enabled,
                        UnknownPolicy::Disabled => AutoscrollDecision::Disabled,
                    }
                }
            }
        } else {
            AutoscrollDecision::Enabled
        };
        let profile = match &snapshot {
            ForegroundSnapshot::Known(app) => self
                .rules
                .iter()
                .position(|rule| matcher::app_matches(app, &rule.apps, rule.match_title)),
            _ => None,
        };
        tracing::debug!(?snapshot, ?decision, ?profile, "foreground decision");
        GateDecision { decision, profile }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{
        AutoscrollDecision, ForegroundApp, ForegroundGate, ForegroundProvider, ForegroundSnapshot,
        ForegroundSourceKind, GateDecision, ProfileRule,
    };
    use crate::foreground::config::{ForegroundConfig, ForegroundMode, UnknownPolicy};

    /// Provider whose snapshot can be swapped at runtime, to model focus changes.
    struct MutableProvider(Mutex<ForegroundSnapshot>);

    impl ForegroundProvider for Arc<MutableProvider> {
        fn snapshot(&self) -> ForegroundSnapshot {
            self.0.lock().expect("snapshot lock").clone()
        }
    }

    fn known(class: &str) -> ForegroundSnapshot {
        ForegroundSnapshot::Known(ForegroundApp {
            app_id: None,
            class: Some(class.to_owned()),
            resource_class: None,
            title: None,
            pid: None,
            source: ForegroundSourceKind::Unknown,
        })
    }

    fn denylist_cfg(entries: &[&str]) -> ForegroundConfig {
        ForegroundConfig {
            enabled: true,
            mode: ForegroundMode::Denylist,
            deny_apps: entries.iter().map(|s| (*s).to_owned()).collect(),
            ..ForegroundConfig::default()
        }
    }

    fn rule(apps: &[&str]) -> ProfileRule {
        ProfileRule {
            apps: apps.iter().map(|s| (*s).to_owned()).collect(),
            match_title: false,
        }
    }

    fn shared(snap: ForegroundSnapshot) -> Arc<MutableProvider> {
        Arc::new(MutableProvider(Mutex::new(snap)))
    }

    fn gate(
        cfg: ForegroundConfig,
        rules: Vec<ProfileRule>,
        snap: ForegroundSnapshot,
    ) -> ForegroundGate {
        ForegroundGate::with_provider(cfg, rules, Box::new(shared(snap)))
    }

    fn decision(g: &mut ForegroundGate) -> AutoscrollDecision {
        g.begin_gesture().decision
    }

    #[test]
    fn disabled_config_without_profiles_is_always_enabled() {
        let mut g = gate(ForegroundConfig::default(), Vec::new(), known("firefox"));
        assert_eq!(g.begin_gesture(), GateDecision::DEFAULT);
    }

    #[test]
    fn disabled_config_without_profiles_starts_no_provider() {
        let g = ForegroundGate::new(ForegroundConfig::default(), Vec::new());
        assert!(g.snapshot().is_none());
    }

    #[test]
    fn denylist_match_latches_disabled() {
        let mut g = gate(denylist_cfg(&["firefox"]), Vec::new(), known("firefox"));
        assert_eq!(decision(&mut g), AutoscrollDecision::Disabled);
        assert_eq!(
            g.latched().map(|d| d.decision),
            Some(AutoscrollDecision::Disabled)
        );
        g.end_gesture();
        assert_eq!(g.latched(), None);
    }

    #[test]
    fn focus_change_during_gesture_keeps_decision() {
        let provider = shared(known("code"));
        let mut g = ForegroundGate::with_provider(
            denylist_cfg(&["firefox"]),
            Vec::new(),
            Box::new(provider.clone()),
        );

        assert_eq!(decision(&mut g), AutoscrollDecision::Enabled);
        // Focus moves to a denied app mid-gesture: the latch holds.
        *provider.0.lock().unwrap() = known("firefox");
        assert_eq!(
            g.latched().map(|d| d.decision),
            Some(AutoscrollDecision::Enabled)
        );

        // The next gesture re-evaluates the (now denied) app.
        g.end_gesture();
        assert_eq!(decision(&mut g), AutoscrollDecision::Disabled);
    }

    #[test]
    fn unknown_policy_enabled_keeps_autoscroll() {
        let cfg = ForegroundConfig {
            enabled: true,
            unknown_policy: UnknownPolicy::Enabled,
            ..ForegroundConfig::default()
        };
        let mut g = gate(
            cfg,
            Vec::new(),
            ForegroundSnapshot::Unknown {
                reason: "no provider".to_owned(),
            },
        );
        assert_eq!(decision(&mut g), AutoscrollDecision::Enabled);
    }

    #[test]
    fn unknown_policy_disabled_passes_through() {
        let cfg = ForegroundConfig {
            enabled: true,
            unknown_policy: UnknownPolicy::Disabled,
            ..ForegroundConfig::default()
        };
        let mut g = gate(
            cfg,
            Vec::new(),
            ForegroundSnapshot::Unsupported {
                reason: "no provider".to_owned(),
            },
        );
        assert_eq!(decision(&mut g), AutoscrollDecision::Disabled);
    }

    #[test]
    fn first_matching_profile_wins() {
        let rules = vec![
            rule(&["code"]),
            rule(&["Firefox", "chromium"]),
            rule(&["firefox"]),
        ];
        let mut g = gate(ForegroundConfig::default(), rules, known("firefox"));
        assert_eq!(
            g.begin_gesture(),
            GateDecision {
                decision: AutoscrollDecision::Enabled,
                profile: Some(1),
            }
        );
    }

    #[test]
    fn no_profile_for_unknown_or_unmatched_apps() {
        let rules = vec![rule(&["code"])];
        let mut g = gate(ForegroundConfig::default(), rules.clone(), known("firefox"));
        assert_eq!(g.begin_gesture().profile, None);

        let mut g = gate(
            ForegroundConfig::default(),
            rules,
            ForegroundSnapshot::Unknown {
                reason: "starting".to_owned(),
            },
        );
        assert_eq!(g.begin_gesture().profile, None);
    }

    #[test]
    fn profile_title_matching_is_per_rule() {
        let app = ForegroundSnapshot::Known(ForegroundApp {
            class: Some("kitty".to_owned()),
            title: Some("nvim".to_owned()),
            ..ForegroundApp::default()
        });
        let mut by_title = rule(&["nvim"]);
        by_title.match_title = true;
        let mut g = gate(
            ForegroundConfig::default(),
            vec![rule(&["nvim"]), by_title],
            app,
        );
        assert_eq!(g.begin_gesture().profile, Some(1));
    }

    #[test]
    fn denied_app_still_reports_its_profile() {
        let mut g = gate(
            denylist_cfg(&["firefox"]),
            vec![rule(&["firefox"])],
            known("firefox"),
        );
        assert_eq!(
            g.begin_gesture(),
            GateDecision {
                decision: AutoscrollDecision::Disabled,
                profile: Some(0),
            }
        );
    }

    #[test]
    fn set_config_keeps_the_running_provider_settings() {
        let mut g = gate(denylist_cfg(&["firefox"]), Vec::new(), known("firefox"));
        let mut next = denylist_cfg(&["code"]);
        next.provider = crate::foreground::config::ForegroundProviderKind::Sway;
        g.set_config(next, vec![rule(&["firefox"])]);
        assert_eq!(
            g.cfg.provider,
            crate::foreground::config::ForegroundProviderKind::Auto
        );
        assert_eq!(g.cfg.deny_apps, vec!["code".to_owned()]);
        assert_eq!(
            g.begin_gesture(),
            GateDecision {
                decision: AutoscrollDecision::Enabled,
                profile: Some(0),
            }
        );
    }
}
