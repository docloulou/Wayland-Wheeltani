use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode, RelativeAxisCode};
use middle_scroll_core::{CoreAction, MouseButton};

use crate::errors::DaemonError;

pub const VIRTUAL_MOUSE_NAME: &str = "Wayland-Wheeltani virtual mouse";

const VENDOR_ID: u16 = 0x1209;
const PRODUCT_ID: u16 = 0x7771;
const VERSION: u16 = 0x0001;

#[derive(Debug)]
pub struct VirtualMouse {
    device: VirtualDevice,
}

impl VirtualMouse {
    pub fn new() -> Result<Self, DaemonError> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for k in [
            KeyCode::BTN_LEFT,
            KeyCode::BTN_RIGHT,
            KeyCode::BTN_MIDDLE,
            KeyCode::BTN_SIDE,
            KeyCode::BTN_EXTRA,
            KeyCode::BTN_FORWARD,
            KeyCode::BTN_BACK,
        ] {
            keys.insert(k);
        }

        let mut axes = AttributeSet::<RelativeAxisCode>::new();
        for a in [
            RelativeAxisCode::REL_X,
            RelativeAxisCode::REL_Y,
            RelativeAxisCode::REL_WHEEL,
            RelativeAxisCode::REL_HWHEEL,
            RelativeAxisCode::REL_WHEEL_HI_RES,
            RelativeAxisCode::REL_HWHEEL_HI_RES,
        ] {
            axes.insert(a);
        }

        let device = VirtualDevice::builder()
            .map_err(uinput_err)?
            .name(VIRTUAL_MOUSE_NAME)
            .input_id(InputId::new(
                BusType::BUS_VIRTUAL,
                VENDOR_ID,
                PRODUCT_ID,
                VERSION,
            ))
            .with_keys(&keys)
            .map_err(uinput_err)?
            .with_relative_axes(&axes)
            .map_err(uinput_err)?
            .build()
            .map_err(uinput_err)?;

        Ok(Self { device })
    }

    pub fn apply_batch(&mut self, actions: &[CoreAction]) -> io::Result<()> {
        let mut buf = Vec::with_capacity(actions.len() * 2);
        for a in actions {
            encode_action(a, &mut buf);
        }
        if buf.is_empty() {
            return Ok(());
        }
        self.device.emit(&buf)
    }

    pub fn emit_middle_click(&mut self) -> io::Result<()> {
        let down = key_event(KeyCode::BTN_MIDDLE, true);
        let up = key_event(KeyCode::BTN_MIDDLE, false);
        self.device.emit(&[down])?;
        self.device.emit(&[up])
    }
}

fn encode_action(action: &CoreAction, out: &mut Vec<InputEvent>) {
    match action {
        CoreAction::ForwardMouseButton { button, pressed } => {
            out.push(key_event(mouse_button_to_keycode(*button), *pressed));
        }
        CoreAction::ForwardMotion { dx, dy } => {
            if *dx != 0 {
                out.push(rel_event(RelativeAxisCode::REL_X, *dx));
            }
            if *dy != 0 {
                out.push(rel_event(RelativeAxisCode::REL_Y, *dy));
            }
        }
        CoreAction::ForwardWheel {
            vertical,
            horizontal,
        }
        | CoreAction::EmitWheelDetents {
            vertical,
            horizontal,
        } => {
            if *vertical != 0 {
                out.push(rel_event(RelativeAxisCode::REL_WHEEL, *vertical));
            }
            if *horizontal != 0 {
                out.push(rel_event(RelativeAxisCode::REL_HWHEEL, *horizontal));
            }
        }
        CoreAction::EmitWheelHiRes {
            vertical_units,
            horizontal_units,
        } => {
            if *vertical_units != 0 {
                out.push(rel_event(
                    RelativeAxisCode::REL_WHEEL_HI_RES,
                    *vertical_units,
                ));
            }
            if *horizontal_units != 0 {
                out.push(rel_event(
                    RelativeAxisCode::REL_HWHEEL_HI_RES,
                    *horizontal_units,
                ));
            }
        }
        CoreAction::EmitMiddleClick
        | CoreAction::Suppress
        | CoreAction::EnterScrollMode
        | CoreAction::ExitScrollMode => {}
    }
}

fn key_event(code: KeyCode, pressed: bool) -> InputEvent {
    InputEvent::new(EventType::KEY.0, code.0, i32::from(pressed))
}

fn rel_event(code: RelativeAxisCode, value: i32) -> InputEvent {
    InputEvent::new(EventType::RELATIVE.0, code.0, value)
}

const fn mouse_button_to_keycode(button: MouseButton) -> KeyCode {
    match button {
        MouseButton::Left => KeyCode::BTN_LEFT,
        MouseButton::Right => KeyCode::BTN_RIGHT,
        MouseButton::Middle => KeyCode::BTN_MIDDLE,
        MouseButton::Side => KeyCode::BTN_SIDE,
        MouseButton::Extra => KeyCode::BTN_EXTRA,
        MouseButton::Forward => KeyCode::BTN_FORWARD,
        MouseButton::Back => KeyCode::BTN_BACK,
    }
}

const fn uinput_err(source: io::Error) -> DaemonError {
    DaemonError::UinputUnavailable { source }
}
