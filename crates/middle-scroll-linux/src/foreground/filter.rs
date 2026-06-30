//! Foreground application model, the provider trait, and the decision gate.

use std::fmt;

use middle_scroll_core::{CoreInputEvent, EngineState};

use crate::event_router::RoutedEvent;

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
    // Part of the documented source taxonomy; reserved for a future native KWin
    // bridge and for providers that report an indeterminate origin.
    #[allow(dead_code)]
    KwinBridge,
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

/// Whether the autoscroll engine should handle an event, or whether it must be
/// passed straight through to the virtual device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoscrollDecision {
    Enabled,
    Disabled,
}

/// Runtime decision layer sitting between `event_router::classify` and the
/// autoscroll engine. It evaluates the active provider and latches a single
/// decision for the duration of a middle-button gesture.
pub struct ForegroundGate {
    cfg: ForegroundConfig,
    provider: Box<dyn ForegroundProvider>,
    latched_middle_decision: Option<AutoscrollDecision>,
}

impl fmt::Debug for ForegroundGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ForegroundGate")
            .field("cfg", &self.cfg)
            .field("latched_middle_decision", &self.latched_middle_decision)
            .finish_non_exhaustive()
    }
}

impl ForegroundGate {
    /// Builds a gate for `cfg`, selecting and starting the appropriate provider.
    #[must_use]
    pub fn new(cfg: ForegroundConfig) -> Self {
        let provider = super::providers::select_provider(&cfg);
        tracing::debug!(
            enabled = cfg.enabled,
            provider = ?cfg.provider,
            mode = ?cfg.mode,
            "foreground gate created"
        );
        Self {
            cfg,
            provider,
            latched_middle_decision: None,
        }
    }

    /// Builds a gate with an explicit provider. Used by tests.
    #[cfg(test)]
    #[must_use]
    pub fn with_provider(cfg: ForegroundConfig, provider: Box<dyn ForegroundProvider>) -> Self {
        Self {
            cfg,
            provider,
            latched_middle_decision: None,
        }
    }

    /// Clears any latched gesture decision. Called after a physical-device
    /// reconnect so a gesture interrupted by an unplug cannot leave a stale
    /// latch behind.
    pub fn reset_latch(&mut self) {
        self.latched_middle_decision = None;
    }

    /// Returns whether the engine should process `routed`, or whether it must be
    /// passed through. The decision is taken once at the start of a middle
    /// gesture and held until `MiddleUp`.
    pub fn decision_for_event(
        &mut self,
        routed: &RoutedEvent,
        engine_state: EngineState,
    ) -> AutoscrollDecision {
        // Filter disabled: behave exactly like the historical daemon.
        if !self.cfg.enabled {
            return AutoscrollDecision::Enabled;
        }

        // A gesture in flight keeps the decision taken at its `MiddleDown`.
        if let Some(latched) = self.latched_middle_decision {
            return latched;
        }

        // Start of a new middle gesture: evaluate once and latch.
        if engine_state == EngineState::Idle
            && matches!(routed, RoutedEvent::Core(CoreInputEvent::MiddleDown))
        {
            let decision = self.evaluate();
            self.latched_middle_decision = Some(decision);
            return decision;
        }

        // Outside a gesture the engine is already transparent for non-middle
        // events (it only acts on the middle-button gesture), so we route them
        // through the engine exactly as the unfiltered daemon does. This keeps
        // behaviour byte-identical outside disabled gestures and avoids
        // evaluating the provider on every motion event.
        AutoscrollDecision::Enabled
    }

    /// Releases the gesture latch once the middle button is back up. Kept as a
    /// separate hook so the decision stays stable while the button is held even
    /// though, in the pass-through case, the engine never leaves `Idle`.
    pub fn after_event(&mut self, routed: &RoutedEvent, _engine_state_after: EngineState) {
        if matches!(routed, RoutedEvent::Core(CoreInputEvent::MiddleUp)) {
            self.latched_middle_decision = None;
        }
    }

    fn evaluate(&self) -> AutoscrollDecision {
        let snapshot = self.provider.snapshot();
        let decision = match &snapshot {
            ForegroundSnapshot::Known(app) => matcher::decide(app, &self.cfg),
            ForegroundSnapshot::Unknown { .. } | ForegroundSnapshot::Unsupported { .. } => {
                match self.cfg.unknown_policy {
                    UnknownPolicy::Enabled => AutoscrollDecision::Enabled,
                    UnknownPolicy::Disabled => AutoscrollDecision::Disabled,
                }
            }
        };
        tracing::debug!(snapshot = ?snapshot, decision = ?decision, "foreground decision");
        decision
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use middle_scroll_core::{CoreInputEvent, EngineState};

    use super::{
        AutoscrollDecision, ForegroundApp, ForegroundGate, ForegroundProvider, ForegroundSnapshot,
        ForegroundSourceKind,
    };
    use crate::event_router::RoutedEvent;
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

    fn core(ev: CoreInputEvent) -> RoutedEvent {
        RoutedEvent::Core(ev)
    }

    fn gate(cfg: ForegroundConfig, snap: ForegroundSnapshot) -> ForegroundGate {
        let shared = Arc::new(MutableProvider(Mutex::new(snap)));
        ForegroundGate::with_provider(cfg, Box::new(shared))
    }

    #[test]
    fn disabled_config_is_always_enabled() {
        let mut g = gate(ForegroundConfig::default(), known("firefox"));
        assert_eq!(
            g.decision_for_event(&core(CoreInputEvent::MiddleDown), EngineState::Idle),
            AutoscrollDecision::Enabled
        );
    }

    #[test]
    fn denylist_match_latches_disabled() {
        let mut g = gate(denylist_cfg(&["firefox"]), known("firefox"));
        assert_eq!(
            g.decision_for_event(&core(CoreInputEvent::MiddleDown), EngineState::Idle),
            AutoscrollDecision::Disabled
        );
        // In pass-through the engine stays Idle, but the latch must hold.
        assert_eq!(
            g.decision_for_event(
                &core(CoreInputEvent::Motion { dx: 0, dy: 9 }),
                EngineState::Idle
            ),
            AutoscrollDecision::Disabled
        );
    }

    #[test]
    fn non_gesture_events_stay_enabled_even_for_denied_app() {
        // Outside a middle gesture the engine is transparent, so the gate must
        // route events through it (Enabled) rather than evaluating per event.
        let mut g = gate(denylist_cfg(&["firefox"]), known("firefox"));
        assert_eq!(
            g.decision_for_event(
                &core(CoreInputEvent::Motion { dx: 1, dy: 1 }),
                EngineState::Idle
            ),
            AutoscrollDecision::Enabled
        );
        assert_eq!(
            g.decision_for_event(
                &core(CoreInputEvent::Wheel {
                    vertical: 1,
                    horizontal: 0,
                }),
                EngineState::Idle
            ),
            AutoscrollDecision::Enabled
        );
    }

    #[test]
    fn focus_change_during_gesture_keeps_decision() {
        let shared = Arc::new(MutableProvider(Mutex::new(known("code"))));
        let mut g =
            ForegroundGate::with_provider(denylist_cfg(&["firefox"]), Box::new(shared.clone()));

        // Gesture starts in an allowed app.
        assert_eq!(
            g.decision_for_event(&core(CoreInputEvent::MiddleDown), EngineState::Idle),
            AutoscrollDecision::Enabled
        );
        // Focus moves to a denied app mid-gesture.
        *shared.0.lock().unwrap() = known("firefox");
        assert_eq!(
            g.decision_for_event(
                &core(CoreInputEvent::Motion { dx: 0, dy: 9 }),
                EngineState::MiddlePending
            ),
            AutoscrollDecision::Enabled
        );
    }

    #[test]
    fn middleup_releases_latch_for_next_gesture() {
        let shared = Arc::new(MutableProvider(Mutex::new(known("code"))));
        let mut g =
            ForegroundGate::with_provider(denylist_cfg(&["firefox"]), Box::new(shared.clone()));

        assert_eq!(
            g.decision_for_event(&core(CoreInputEvent::MiddleDown), EngineState::Idle),
            AutoscrollDecision::Enabled
        );
        g.after_event(&core(CoreInputEvent::MiddleUp), EngineState::Idle);

        // A new gesture re-evaluates the (now denied) app.
        *shared.0.lock().unwrap() = known("firefox");
        assert_eq!(
            g.decision_for_event(&core(CoreInputEvent::MiddleDown), EngineState::Idle),
            AutoscrollDecision::Disabled
        );
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
            ForegroundSnapshot::Unknown {
                reason: "no provider".to_owned(),
            },
        );
        assert_eq!(
            g.decision_for_event(&core(CoreInputEvent::MiddleDown), EngineState::Idle),
            AutoscrollDecision::Enabled
        );
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
            ForegroundSnapshot::Unsupported {
                reason: "no provider".to_owned(),
            },
        );
        assert_eq!(
            g.decision_for_event(&core(CoreInputEvent::MiddleDown), EngineState::Idle),
            AutoscrollDecision::Disabled
        );
    }
}
