//! Classifies raw evdev events for the daemon loop.
//!
//! Only the events the autoscroll engine understands (trigger button, left and
//! right buttons, motion and wheels) are translated to [`CoreInputEvent`]s.
//! Everything else the mouse reports (extra buttons, `REL_DIAL`, ...) is kept
//! as a raw event and re-emitted untouched, so grabbing the mouse never makes
//! one of its buttons or axes disappear.

use evdev::{EventSummary, InputEvent, KeyCode, RelativeAxisCode, SynchronizationCode};
use middle_scroll_core::{CoreAction, CoreInputEvent};
use serde::{Deserialize, Serialize};

/// Which physical button starts autoscroll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerButton {
    /// The middle button (wheel click).
    #[default]
    Middle,
    /// `BTN_SIDE`, usually the rear thumb button.
    Side,
    /// `BTN_EXTRA`, usually the front thumb button.
    Extra,
    /// `BTN_FORWARD`.
    Forward,
    /// `BTN_BACK`.
    Back,
    /// `BTN_TASK`.
    Task,
}

impl TriggerButton {
    pub const fn key_code(self) -> KeyCode {
        match self {
            Self::Middle => KeyCode::BTN_MIDDLE,
            Self::Side => KeyCode::BTN_SIDE,
            Self::Extra => KeyCode::BTN_EXTRA,
            Self::Forward => KeyCode::BTN_FORWARD,
            Self::Back => KeyCode::BTN_BACK,
            Self::Task => KeyCode::BTN_TASK,
        }
    }
}

/// What the daemon loop should do with one evdev event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routed {
    /// An event the autoscroll engine handles. The trigger button is always
    /// reported as `MiddleDown`/`MiddleUp`, whatever button it really is.
    Core(CoreInputEvent),
    /// A key or relative axis the engine does not know: forward it as is.
    Raw(InputEvent),
    /// `SYN_REPORT`: the end of one hardware frame.
    Sync,
    /// Autorepeat, `MSC_SCAN` and other events the virtual mouse never needs.
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Router {
    trigger: KeyCode,
}

impl Router {
    pub const fn new(trigger: TriggerButton) -> Self {
        Self {
            trigger: trigger.key_code(),
        }
    }

    pub const fn trigger(&self) -> KeyCode {
        self.trigger
    }

    pub fn classify(&self, event: &InputEvent) -> Routed {
        match event.destructure() {
            EventSummary::Synchronization(_, SynchronizationCode::SYN_REPORT, _) => Routed::Sync,
            // Autorepeat: mice do not repeat, and the virtual device would
            // only see duplicated presses.
            EventSummary::Key(_, _, 2) => Routed::Ignore,
            EventSummary::Key(_, code, value) => {
                let pressed = value != 0;
                if code == self.trigger {
                    return Routed::Core(if pressed {
                        CoreInputEvent::MiddleDown
                    } else {
                        CoreInputEvent::MiddleUp
                    });
                }
                match code {
                    KeyCode::BTN_LEFT => Routed::Core(if pressed {
                        CoreInputEvent::LeftDown
                    } else {
                        CoreInputEvent::LeftUp
                    }),
                    KeyCode::BTN_RIGHT => Routed::Core(if pressed {
                        CoreInputEvent::RightDown
                    } else {
                        CoreInputEvent::RightUp
                    }),
                    _ => Routed::Raw(*event),
                }
            }
            EventSummary::RelativeAxis(_, code, value) => match code {
                RelativeAxisCode::REL_X => {
                    Routed::Core(CoreInputEvent::Motion { dx: value, dy: 0 })
                }
                RelativeAxisCode::REL_Y => {
                    Routed::Core(CoreInputEvent::Motion { dx: 0, dy: value })
                }
                RelativeAxisCode::REL_WHEEL => Routed::Core(CoreInputEvent::Wheel {
                    vertical: value,
                    horizontal: 0,
                }),
                RelativeAxisCode::REL_HWHEEL => Routed::Core(CoreInputEvent::Wheel {
                    vertical: 0,
                    horizontal: value,
                }),
                RelativeAxisCode::REL_WHEEL_HI_RES => Routed::Core(CoreInputEvent::WheelHiRes {
                    vertical_units: value,
                    horizontal_units: 0,
                }),
                RelativeAxisCode::REL_HWHEEL_HI_RES => Routed::Core(CoreInputEvent::WheelHiRes {
                    vertical_units: 0,
                    horizontal_units: value,
                }),
                _ => Routed::Raw(*event),
            },
            _ => Routed::Ignore,
        }
    }
}

/// The action that forwards `event` untouched, used when autoscroll is
/// paused or disabled for the focused app. The daemon grabs the mouse, so
/// dropping the event would make the mouse look dead.
pub const fn passthrough_action(event: CoreInputEvent) -> Option<CoreAction> {
    use middle_scroll_core::MouseButton;

    let (button, pressed) = match event {
        CoreInputEvent::Motion { dx, dy } => return Some(CoreAction::ForwardMotion { dx, dy }),
        CoreInputEvent::Wheel {
            vertical,
            horizontal,
        } => {
            return Some(CoreAction::ForwardWheel {
                vertical,
                horizontal,
            })
        }
        CoreInputEvent::WheelHiRes {
            vertical_units,
            horizontal_units,
        } => {
            return Some(CoreAction::EmitWheelHiRes {
                vertical_units,
                horizontal_units,
            })
        }
        CoreInputEvent::Tick { .. } => return None,
        CoreInputEvent::MiddleDown => (MouseButton::Middle, true),
        CoreInputEvent::MiddleUp => (MouseButton::Middle, false),
        CoreInputEvent::LeftDown => (MouseButton::Left, true),
        CoreInputEvent::LeftUp => (MouseButton::Left, false),
        CoreInputEvent::RightDown => (MouseButton::Right, true),
        CoreInputEvent::RightUp => (MouseButton::Right, false),
    };
    Some(CoreAction::ForwardMouseButton { button, pressed })
}

/// Log line for `--dry-run`. Motion and wheel amounts are redacted so a log
/// never records what the user was doing, only the kind of action.
pub fn dry_run_describe(action: &CoreAction) -> Option<String> {
    Some(match action {
        CoreAction::ForwardMouseButton { button, pressed } => {
            format!("ForwardMouseButton({button:?}, pressed={pressed})")
        }
        CoreAction::ForwardMotion { .. } => "ForwardMotion(<redacted>)".into(),
        CoreAction::ForwardWheel { .. } => "ForwardWheel(<redacted>)".into(),
        CoreAction::EmitWheelDetents { vertical, .. } => {
            format!("EmitWheelDetents(direction={})", sign(*vertical))
        }
        CoreAction::EmitWheelHiRes { vertical_units, .. } => {
            format!("EmitWheelHiRes(direction={})", sign(*vertical_units))
        }
        CoreAction::EmitMiddleClick => "EmitMiddleClick".into(),
        CoreAction::EnterScrollMode => "EnterScrollMode".into(),
        CoreAction::ExitScrollMode => "ExitScrollMode".into(),
        CoreAction::Suppress => return None,
    })
}

const fn sign(v: i32) -> &'static str {
    if v > 0 {
        "+"
    } else if v < 0 {
        "-"
    } else {
        "0"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::EventType;
    use middle_scroll_core::MouseButton;

    fn key(code: KeyCode, value: i32) -> InputEvent {
        InputEvent::new(EventType::KEY.0, code.0, value)
    }

    fn rel(code: RelativeAxisCode, value: i32) -> InputEvent {
        InputEvent::new(EventType::RELATIVE.0, code.0, value)
    }

    #[test]
    fn middle_is_the_default_trigger() {
        let router = Router::new(TriggerButton::default());
        assert_eq!(
            router.classify(&key(KeyCode::BTN_MIDDLE, 1)),
            Routed::Core(CoreInputEvent::MiddleDown)
        );
        assert_eq!(
            router.classify(&key(KeyCode::BTN_MIDDLE, 0)),
            Routed::Core(CoreInputEvent::MiddleUp)
        );
    }

    #[test]
    fn custom_trigger_replaces_middle() {
        let router = Router::new(TriggerButton::Side);
        assert_eq!(
            router.classify(&key(KeyCode::BTN_SIDE, 1)),
            Routed::Core(CoreInputEvent::MiddleDown)
        );
        // The real middle button is then an ordinary button.
        let middle = key(KeyCode::BTN_MIDDLE, 1);
        assert_eq!(router.classify(&middle), Routed::Raw(middle));
    }

    #[test]
    fn unknown_buttons_and_axes_are_kept_raw() {
        let router = Router::new(TriggerButton::Middle);
        for ev in [
            key(KeyCode::BTN_SIDE, 1),
            key(KeyCode::BTN_TASK, 0),
            key(KeyCode::KEY_VOLUMEUP, 1),
            rel(RelativeAxisCode::REL_DIAL, 3),
            rel(RelativeAxisCode::REL_MISC, -1),
        ] {
            assert_eq!(router.classify(&ev), Routed::Raw(ev));
        }
    }

    #[test]
    fn autorepeat_is_ignored_and_syn_report_is_a_frame_end() {
        let router = Router::new(TriggerButton::Middle);
        assert_eq!(router.classify(&key(KeyCode::BTN_LEFT, 2)), Routed::Ignore);
        let syn = InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0);
        assert_eq!(router.classify(&syn), Routed::Sync);
    }

    #[test]
    fn motion_and_wheels_go_to_the_engine() {
        let router = Router::new(TriggerButton::Middle);
        assert_eq!(
            router.classify(&rel(RelativeAxisCode::REL_Y, -4)),
            Routed::Core(CoreInputEvent::Motion { dx: 0, dy: -4 })
        );
        assert_eq!(
            router.classify(&rel(RelativeAxisCode::REL_HWHEEL_HI_RES, 60)),
            Routed::Core(CoreInputEvent::WheelHiRes {
                vertical_units: 0,
                horizontal_units: 60,
            })
        );
    }

    #[test]
    fn passthrough_forwards_every_core_event() {
        assert_eq!(
            passthrough_action(CoreInputEvent::MiddleDown),
            Some(CoreAction::ForwardMouseButton {
                button: MouseButton::Middle,
                pressed: true,
            })
        );
        assert_eq!(
            passthrough_action(CoreInputEvent::RightUp),
            Some(CoreAction::ForwardMouseButton {
                button: MouseButton::Right,
                pressed: false,
            })
        );
        assert_eq!(
            passthrough_action(CoreInputEvent::Motion { dx: 3, dy: -4 }),
            Some(CoreAction::ForwardMotion { dx: 3, dy: -4 })
        );
        assert_eq!(
            passthrough_action(CoreInputEvent::Wheel {
                vertical: 1,
                horizontal: 0,
            }),
            Some(CoreAction::ForwardWheel {
                vertical: 1,
                horizontal: 0,
            })
        );
        assert_eq!(
            passthrough_action(CoreInputEvent::WheelHiRes {
                vertical_units: 120,
                horizontal_units: 0,
            }),
            Some(CoreAction::EmitWheelHiRes {
                vertical_units: 120,
                horizontal_units: 0,
            })
        );
        assert_eq!(
            passthrough_action(CoreInputEvent::Tick { dt_micros: 1000 }),
            None
        );
    }
}
