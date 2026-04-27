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
