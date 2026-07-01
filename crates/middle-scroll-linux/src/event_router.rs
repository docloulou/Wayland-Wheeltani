use evdev::{EventSummary, InputEvent, KeyCode, RelativeAxisCode};
use middle_scroll_core::{CoreAction, CoreInputEvent, MouseButton};

#[derive(Debug)]
pub enum RoutedEvent {
    Core(CoreInputEvent),
    DirectButton { button: MouseButton, pressed: bool },
    Ignore,
}

pub fn classify(event: &InputEvent) -> RoutedEvent {
    match event.destructure() {
        EventSummary::Key(_, KeyCode::BTN_MIDDLE, 1) => {
            RoutedEvent::Core(CoreInputEvent::MiddleDown)
        }
        EventSummary::Key(_, KeyCode::BTN_MIDDLE, 0) => RoutedEvent::Core(CoreInputEvent::MiddleUp),
        EventSummary::Key(_, KeyCode::BTN_LEFT, value) => {
            press_or_release(value, CoreInputEvent::LeftDown, CoreInputEvent::LeftUp)
        }
        EventSummary::Key(_, KeyCode::BTN_RIGHT, value) => {
            press_or_release(value, CoreInputEvent::RightDown, CoreInputEvent::RightUp)
        }
        EventSummary::Key(_, KeyCode::BTN_SIDE, value) => {
            button_passthrough(value, MouseButton::Side)
        }
        EventSummary::Key(_, KeyCode::BTN_EXTRA, value) => {
            button_passthrough(value, MouseButton::Extra)
        }
        EventSummary::Key(_, KeyCode::BTN_FORWARD, value) => {
            button_passthrough(value, MouseButton::Forward)
        }
        EventSummary::Key(_, KeyCode::BTN_BACK, value) => {
            button_passthrough(value, MouseButton::Back)
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_X, value) => {
            RoutedEvent::Core(CoreInputEvent::Motion { dx: value, dy: 0 })
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_Y, value) => {
            RoutedEvent::Core(CoreInputEvent::Motion { dx: 0, dy: value })
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_WHEEL, value) => {
            RoutedEvent::Core(CoreInputEvent::Wheel {
                vertical: value,
                horizontal: 0,
            })
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_HWHEEL, value) => {
            RoutedEvent::Core(CoreInputEvent::Wheel {
                vertical: 0,
                horizontal: value,
            })
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_WHEEL_HI_RES, value) => {
            RoutedEvent::Core(CoreInputEvent::WheelHiRes {
                vertical_units: value,
                horizontal_units: 0,
            })
        }
        EventSummary::RelativeAxis(_, RelativeAxisCode::REL_HWHEEL_HI_RES, value) => {
            RoutedEvent::Core(CoreInputEvent::WheelHiRes {
                vertical_units: 0,
                horizontal_units: value,
            })
        }
        _ => RoutedEvent::Ignore,
    }
}

const fn press_or_release(value: i32, down: CoreInputEvent, up: CoreInputEvent) -> RoutedEvent {
    match value {
        1 => RoutedEvent::Core(down),
        0 => RoutedEvent::Core(up),
        _ => RoutedEvent::Ignore,
    }
}

const fn button_passthrough(value: i32, button: MouseButton) -> RoutedEvent {
    match value {
        1 => RoutedEvent::DirectButton {
            button,
            pressed: true,
        },
        0 => RoutedEvent::DirectButton {
            button,
            pressed: false,
        },
        _ => RoutedEvent::Ignore,
    }
}

/// Converts a routed event into the `CoreAction`s that forward it untouched.
///
/// Used when the foreground filter disables autoscroll for the active app:
/// because the daemon grabs the physical device, simply dropping the events
/// would make the mouse appear dead, so every event is re-emitted verbatim on
/// the virtual device instead.
#[must_use]
pub fn passthrough_actions(routed: &RoutedEvent) -> Vec<CoreAction> {
    match routed {
        RoutedEvent::Core(event) => passthrough_core_event(*event),
        RoutedEvent::DirectButton { button, pressed } => vec![CoreAction::ForwardMouseButton {
            button: *button,
            pressed: *pressed,
        }],
        RoutedEvent::Ignore => Vec::new(),
    }
}

fn passthrough_core_event(event: CoreInputEvent) -> Vec<CoreAction> {
    match event {
        CoreInputEvent::MiddleDown => vec![CoreAction::ForwardMouseButton {
            button: MouseButton::Middle,
            pressed: true,
        }],
        CoreInputEvent::MiddleUp => vec![CoreAction::ForwardMouseButton {
            button: MouseButton::Middle,
            pressed: false,
        }],
        CoreInputEvent::LeftDown => vec![CoreAction::ForwardMouseButton {
            button: MouseButton::Left,
            pressed: true,
        }],
        CoreInputEvent::LeftUp => vec![CoreAction::ForwardMouseButton {
            button: MouseButton::Left,
            pressed: false,
        }],
        CoreInputEvent::RightDown => vec![CoreAction::ForwardMouseButton {
            button: MouseButton::Right,
            pressed: true,
        }],
        CoreInputEvent::RightUp => vec![CoreAction::ForwardMouseButton {
            button: MouseButton::Right,
            pressed: false,
        }],
        CoreInputEvent::Motion { dx, dy } => vec![CoreAction::ForwardMotion { dx, dy }],
        CoreInputEvent::Wheel {
            vertical,
            horizontal,
        } => vec![CoreAction::ForwardWheel {
            vertical,
            horizontal,
        }],
        CoreInputEvent::WheelHiRes {
            vertical_units,
            horizontal_units,
        } => vec![CoreAction::EmitWheelHiRes {
            vertical_units,
            horizontal_units,
        }],
        CoreInputEvent::Tick { .. } => Vec::new(),
    }
}

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
    use super::{passthrough_actions, RoutedEvent};
    use middle_scroll_core::{CoreAction, CoreInputEvent, MouseButton};

    #[test]
    fn middledown_forwards_middle_press() {
        assert_eq!(
            passthrough_actions(&RoutedEvent::Core(CoreInputEvent::MiddleDown)),
            vec![CoreAction::ForwardMouseButton {
                button: MouseButton::Middle,
                pressed: true,
            }]
        );
    }

    #[test]
    fn middleup_forwards_middle_release() {
        assert_eq!(
            passthrough_actions(&RoutedEvent::Core(CoreInputEvent::MiddleUp)),
            vec![CoreAction::ForwardMouseButton {
                button: MouseButton::Middle,
                pressed: false,
            }]
        );
    }

    #[test]
    fn motion_is_forwarded() {
        assert_eq!(
            passthrough_actions(&RoutedEvent::Core(CoreInputEvent::Motion { dx: 3, dy: -4 })),
            vec![CoreAction::ForwardMotion { dx: 3, dy: -4 }]
        );
    }

    #[test]
    fn wheel_is_forwarded() {
        assert_eq!(
            passthrough_actions(&RoutedEvent::Core(CoreInputEvent::Wheel {
                vertical: 1,
                horizontal: 0,
            })),
            vec![CoreAction::ForwardWheel {
                vertical: 1,
                horizontal: 0,
            }]
        );
    }

    #[test]
    fn wheel_hi_res_is_emitted() {
        assert_eq!(
            passthrough_actions(&RoutedEvent::Core(CoreInputEvent::WheelHiRes {
                vertical_units: 120,
                horizontal_units: 0,
            })),
            vec![CoreAction::EmitWheelHiRes {
                vertical_units: 120,
                horizontal_units: 0,
            }]
        );
    }

    #[test]
    fn direct_button_is_forwarded() {
        assert_eq!(
            passthrough_actions(&RoutedEvent::DirectButton {
                button: MouseButton::Back,
                pressed: true,
            }),
            vec![CoreAction::ForwardMouseButton {
                button: MouseButton::Back,
                pressed: true,
            }]
        );
    }

    #[test]
    fn ignore_and_tick_produce_no_actions() {
        assert!(passthrough_actions(&RoutedEvent::Ignore).is_empty());
        assert!(
            passthrough_actions(&RoutedEvent::Core(CoreInputEvent::Tick { dt_micros: 1000 }))
                .is_empty()
        );
    }
}
