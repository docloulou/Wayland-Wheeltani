//! The uinput device every event is re-emitted on.
//!
//! Events are assembled into frames by [`FrameBuilder`] and written with one
//! `SYN_REPORT` per frame, mirroring the frames of the physical mouse: motion
//! and wheel deltas that arrived together stay together, so libinput's pointer
//! acceleration sees the same motion as without the daemon.

use std::io;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, EventType, InputEvent, InputId, KeyCode, RelativeAxisCode};
use middle_scroll_core::{CoreAction, MouseButton};

use crate::errors::DaemonError;

pub const VIRTUAL_MOUSE_NAME: &str = "Wayland-Wheeltani virtual mouse";

const VENDOR_ID: u16 = 0x1209;
const PRODUCT_ID: u16 = 0x7771;
const VERSION: u16 = 0x0001;

/// One wheel detent in hi-res units (see the kernel's `REL_WHEEL_HI_RES`).
const HIRES_PER_DETENT: i32 = 120;

/// Buttons the virtual mouse always supports: `BTN_LEFT` up to `BTN_TASK`.
pub const BASE_KEYS: [KeyCode; 8] = [
    KeyCode::BTN_LEFT,
    KeyCode::BTN_RIGHT,
    KeyCode::BTN_MIDDLE,
    KeyCode::BTN_SIDE,
    KeyCode::BTN_EXTRA,
    KeyCode::BTN_FORWARD,
    KeyCode::BTN_BACK,
    KeyCode::BTN_TASK,
];

/// Axes the virtual mouse always supports.
///
/// Advertising the hi-res wheels makes libinput ignore the legacy ones, which
/// is why [`FrameBuilder`] adds hi-res events for wheels that only produce
/// legacy clicks.
pub const BASE_AXES: [RelativeAxisCode; 6] = [
    RelativeAxisCode::REL_X,
    RelativeAxisCode::REL_Y,
    RelativeAxisCode::REL_WHEEL,
    RelativeAxisCode::REL_HWHEEL,
    RelativeAxisCode::REL_WHEEL_HI_RES,
    RelativeAxisCode::REL_HWHEEL_HI_RES,
];

const V: usize = 0;
const H: usize = 1;

/// Collects the events of one output frame.
///
/// Motion and wheel deltas are summed per axis; buttons and other raw events
/// keep their order. When the frame is finished, a wheel axis that received
/// legacy clicks but no hi-res units gets the matching hi-res event, because
/// the virtual device advertises hi-res wheels and libinput would otherwise
/// drop the legacy clicks of an older mouse.
#[derive(Debug)]
pub struct FrameBuilder {
    trigger: KeyCode,
    /// Also add hi-res units for the engine's own legacy detents. Only right
    /// when the engine emits no hi-res events itself (`emit_hires_wheel =
    /// false`); otherwise both streams already exist.
    synthesize_detents: bool,
    ordered: Vec<InputEvent>,
    motion: [i32; 2],
    legacy: [i32; 2],
    /// Legacy clicks that still need hi-res units, per axis.
    legacy_without_hires: [i32; 2],
    hires: [i32; 2],
    hires_seen: [bool; 2],
    out: Vec<InputEvent>,
}

impl FrameBuilder {
    pub fn new(trigger: KeyCode) -> Self {
        Self {
            trigger,
            synthesize_detents: false,
            ordered: Vec::with_capacity(8),
            motion: [0; 2],
            legacy: [0; 2],
            legacy_without_hires: [0; 2],
            hires: [0; 2],
            hires_seen: [false; 2],
            out: Vec::with_capacity(16),
        }
    }

    /// The key emitted for [`MouseButton::Middle`] actions, i.e. the trigger.
    pub const fn set_trigger(&mut self, trigger: KeyCode) {
        self.trigger = trigger;
    }

    pub const fn set_synthesize_detents(&mut self, on: bool) {
        self.synthesize_detents = on;
    }

    pub fn is_empty(&self) -> bool {
        self.ordered.is_empty()
            && self.motion == [0; 2]
            && self.legacy == [0; 2]
            && self.hires == [0; 2]
    }

    /// True when the frame already holds an event for `code`; a second
    /// press or release of the same key must go in a new frame.
    pub fn has_key(&self, code: KeyCode) -> bool {
        self.ordered
            .iter()
            .any(|ev| ev.event_type() == EventType::KEY && ev.code() == code.0)
    }

    pub fn push_raw(&mut self, event: InputEvent) {
        if event.event_type() == EventType::RELATIVE {
            let code = RelativeAxisCode(event.code());
            let value = event.value();
            match code {
                RelativeAxisCode::REL_X => self.motion[0] += value,
                RelativeAxisCode::REL_Y => self.motion[1] += value,
                RelativeAxisCode::REL_WHEEL => self.add_legacy(V, value, true),
                RelativeAxisCode::REL_HWHEEL => self.add_legacy(H, value, true),
                RelativeAxisCode::REL_WHEEL_HI_RES => self.add_hires(V, value),
                RelativeAxisCode::REL_HWHEEL_HI_RES => self.add_hires(H, value),
                _ => self.ordered.push(event),
            }
        } else {
            self.ordered.push(event);
        }
    }

    /// Adds the events of `action`. [`CoreAction::EmitMiddleClick`] needs two
    /// frames and is handled by [`VirtualMouse::push_action`]; the actions
    /// that emit nothing are ignored.
    pub fn push_action(&mut self, action: &CoreAction) {
        match *action {
            CoreAction::ForwardMouseButton { button, pressed } => {
                let code = self.key_for(button);
                self.ordered.push(key_event(code, pressed));
            }
            CoreAction::ForwardMotion { dx, dy } => {
                self.motion[0] += dx;
                self.motion[1] += dy;
            }
            CoreAction::ForwardWheel {
                vertical,
                horizontal,
            } => {
                self.add_legacy(V, vertical, true);
                self.add_legacy(H, horizontal, true);
            }
            CoreAction::EmitWheelDetents {
                vertical,
                horizontal,
            } => {
                let synthesize = self.synthesize_detents;
                self.add_legacy(V, vertical, synthesize);
                self.add_legacy(H, horizontal, synthesize);
            }
            CoreAction::EmitWheelHiRes {
                vertical_units,
                horizontal_units,
            } => {
                self.add_hires(V, vertical_units);
                self.add_hires(H, horizontal_units);
            }
            CoreAction::EmitMiddleClick
            | CoreAction::Suppress
            | CoreAction::EnterScrollMode
            | CoreAction::ExitScrollMode => {}
        }
    }

    /// The key emitted for `button`. The engine calls the trigger "middle".
    pub const fn key_for(&self, button: MouseButton) -> KeyCode {
        match button {
            MouseButton::Left => KeyCode::BTN_LEFT,
            MouseButton::Right => KeyCode::BTN_RIGHT,
            MouseButton::Middle => self.trigger,
            MouseButton::Side => KeyCode::BTN_SIDE,
            MouseButton::Extra => KeyCode::BTN_EXTRA,
            MouseButton::Forward => KeyCode::BTN_FORWARD,
            MouseButton::Back => KeyCode::BTN_BACK,
        }
    }

    /// Completes the frame and returns its events (without the trailing
    /// `SYN_REPORT`, which `VirtualDevice::emit` adds), then resets the
    /// builder for the next frame.
    pub fn finish(&mut self) -> &[InputEvent] {
        for axis in [V, H] {
            if !self.hires_seen[axis] && self.legacy_without_hires[axis] != 0 {
                self.hires[axis] = self.legacy_without_hires[axis] * HIRES_PER_DETENT;
            }
        }

        self.out.clear();
        self.out.append(&mut self.ordered);
        let rel = [
            (RelativeAxisCode::REL_X, self.motion[0]),
            (RelativeAxisCode::REL_Y, self.motion[1]),
            (RelativeAxisCode::REL_WHEEL, self.legacy[V]),
            (RelativeAxisCode::REL_HWHEEL, self.legacy[H]),
            (RelativeAxisCode::REL_WHEEL_HI_RES, self.hires[V]),
            (RelativeAxisCode::REL_HWHEEL_HI_RES, self.hires[H]),
        ];
        for (code, value) in rel {
            if value != 0 {
                self.out.push(rel_event(code, value));
            }
        }

        self.motion = [0; 2];
        self.legacy = [0; 2];
        self.legacy_without_hires = [0; 2];
        self.hires = [0; 2];
        self.hires_seen = [false; 2];
        &self.out
    }

    const fn add_legacy(&mut self, axis: usize, value: i32, needs_hires: bool) {
        if value == 0 {
            return;
        }
        self.legacy[axis] += value;
        if needs_hires {
            self.legacy_without_hires[axis] += value;
        }
    }

    const fn add_hires(&mut self, axis: usize, value: i32) {
        if value == 0 {
            return;
        }
        self.hires[axis] += value;
        self.hires_seen[axis] = true;
    }
}

#[derive(Debug)]
pub struct VirtualMouse {
    device: VirtualDevice,
    frame: FrameBuilder,
    keys: Vec<KeyCode>,
}

impl VirtualMouse {
    /// Creates the virtual mouse with [`BASE_KEYS`] and [`BASE_AXES`] plus
    /// whatever else the physical mice report, so their extra buttons and
    /// axes can be forwarded.
    pub fn new(
        trigger: KeyCode,
        extra_keys: &[KeyCode],
        extra_axes: &[RelativeAxisCode],
    ) -> Result<Self, DaemonError> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for k in BASE_KEYS.iter().chain(extra_keys) {
            keys.insert(*k);
        }
        let mut axes = AttributeSet::<RelativeAxisCode>::new();
        for a in BASE_AXES.iter().chain(extra_axes) {
            axes.insert(*a);
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

        Ok(Self {
            device,
            frame: FrameBuilder::new(trigger),
            keys: keys.iter().collect(),
        })
    }

    pub const fn set_trigger(&mut self, trigger: KeyCode) {
        self.frame.set_trigger(trigger);
    }

    pub const fn set_synthesize_detents(&mut self, on: bool) {
        self.frame.set_synthesize_detents(on);
    }

    pub fn push_raw(&mut self, event: InputEvent) -> io::Result<()> {
        if event.event_type() == EventType::KEY && self.frame.has_key(KeyCode(event.code())) {
            self.flush()?;
        }
        self.frame.push_raw(event);
        Ok(())
    }

    pub fn push_action(&mut self, action: &CoreAction) -> io::Result<()> {
        match *action {
            CoreAction::EmitMiddleClick => {
                // Press and release go in two separate frames: in a single
                // frame some clients (and libinput's button debouncing) see
                // the release before the press was handled and drop the click.
                self.flush()?;
                let code = self.frame.key_for(MouseButton::Middle);
                self.device.emit(&[key_event(code, true)])?;
                self.device.emit(&[key_event(code, false)])
            }
            CoreAction::ForwardMouseButton { button, .. }
                if self.frame.has_key(self.frame.key_for(button)) =>
            {
                self.flush()?;
                self.frame.push_action(action);
                Ok(())
            }
            _ => {
                self.frame.push_action(action);
                Ok(())
            }
        }
    }

    /// Writes the pending frame, if any.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.frame.is_empty() {
            return Ok(());
        }
        let events = self.frame.finish();
        self.device.emit(events)
    }

    /// Releases every button the device supports. Used when a physical mouse
    /// disconnects mid-gesture so a forwarded press cannot stay stuck; the
    /// kernel drops releases of buttons that are not down.
    pub fn release_all_buttons(&mut self) -> io::Result<()> {
        self.flush()?;
        let ups: Vec<InputEvent> = self.keys.iter().map(|k| key_event(*k, false)).collect();
        self.device.emit(&ups)
    }
}

fn key_event(code: KeyCode, pressed: bool) -> InputEvent {
    InputEvent::new(EventType::KEY.0, code.0, i32::from(pressed))
}

fn rel_event(code: RelativeAxisCode, value: i32) -> InputEvent {
    InputEvent::new(EventType::RELATIVE.0, code.0, value)
}

const fn uinput_err(source: io::Error) -> DaemonError {
    DaemonError::UinputUnavailable { source }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(events: &[InputEvent]) -> Vec<(u16, u16, i32)> {
        events
            .iter()
            .map(|e| (e.event_type().0, e.code(), e.value()))
            .collect()
    }

    fn rel(code: RelativeAxisCode, value: i32) -> (u16, u16, i32) {
        (EventType::RELATIVE.0, code.0, value)
    }

    fn key(code: KeyCode, value: i32) -> (u16, u16, i32) {
        (EventType::KEY.0, code.0, value)
    }

    #[test]
    fn motion_of_one_frame_is_merged() {
        let mut f = FrameBuilder::new(KeyCode::BTN_MIDDLE);
        f.push_action(&CoreAction::ForwardMotion { dx: 2, dy: 0 });
        f.push_action(&CoreAction::ForwardMotion { dx: 0, dy: -3 });
        f.push_action(&CoreAction::ForwardMotion { dx: 1, dy: 0 });
        assert_eq!(
            summary(f.finish()),
            vec![
                rel(RelativeAxisCode::REL_X, 3),
                rel(RelativeAxisCode::REL_Y, -3)
            ]
        );
        assert!(f.is_empty());
        assert!(f.finish().is_empty());
    }

    #[test]
    fn legacy_only_wheel_gets_hires_units() {
        let mut f = FrameBuilder::new(KeyCode::BTN_MIDDLE);
        f.push_action(&CoreAction::ForwardWheel {
            vertical: -1,
            horizontal: 2,
        });
        assert_eq!(
            summary(f.finish()),
            vec![
                rel(RelativeAxisCode::REL_WHEEL, -1),
                rel(RelativeAxisCode::REL_HWHEEL, 2),
                rel(RelativeAxisCode::REL_WHEEL_HI_RES, -120),
                rel(RelativeAxisCode::REL_HWHEEL_HI_RES, 240),
            ]
        );
    }

    #[test]
    fn hires_wheel_is_not_doubled() {
        let mut f = FrameBuilder::new(KeyCode::BTN_MIDDLE);
        f.push_action(&CoreAction::EmitWheelHiRes {
            vertical_units: 120,
            horizontal_units: 0,
        });
        f.push_action(&CoreAction::ForwardWheel {
            vertical: 1,
            horizontal: 0,
        });
        assert_eq!(
            summary(f.finish()),
            vec![
                rel(RelativeAxisCode::REL_WHEEL, 1),
                rel(RelativeAxisCode::REL_WHEEL_HI_RES, 120),
            ]
        );
    }

    #[test]
    fn engine_detents_get_hires_only_when_requested() {
        let detent = CoreAction::EmitWheelDetents {
            vertical: 1,
            horizontal: 0,
        };
        let mut f = FrameBuilder::new(KeyCode::BTN_MIDDLE);
        f.push_action(&detent);
        assert_eq!(
            summary(f.finish()),
            vec![rel(RelativeAxisCode::REL_WHEEL, 1)]
        );

        f.set_synthesize_detents(true);
        f.push_action(&detent);
        assert_eq!(
            summary(f.finish()),
            vec![
                rel(RelativeAxisCode::REL_WHEEL, 1),
                rel(RelativeAxisCode::REL_WHEEL_HI_RES, 120),
            ]
        );
    }

    #[test]
    fn middle_actions_use_the_trigger_key() {
        let mut f = FrameBuilder::new(KeyCode::BTN_SIDE);
        f.push_action(&CoreAction::ForwardMouseButton {
            button: MouseButton::Middle,
            pressed: true,
        });
        assert!(f.has_key(KeyCode::BTN_SIDE));
        assert_eq!(summary(f.finish()), vec![key(KeyCode::BTN_SIDE, 1)]);
    }

    #[test]
    fn raw_events_keep_their_order_before_motion() {
        let mut f = FrameBuilder::new(KeyCode::BTN_MIDDLE);
        f.push_action(&CoreAction::ForwardMotion { dx: 1, dy: 1 });
        f.push_raw(InputEvent::new(EventType::KEY.0, KeyCode::BTN_EXTRA.0, 1));
        f.push_raw(InputEvent::new(
            EventType::RELATIVE.0,
            RelativeAxisCode::REL_DIAL.0,
            -2,
        ));
        assert_eq!(
            summary(f.finish()),
            vec![
                key(KeyCode::BTN_EXTRA, 1),
                rel(RelativeAxisCode::REL_DIAL, -2),
                rel(RelativeAxisCode::REL_X, 1),
                rel(RelativeAxisCode::REL_Y, 1),
            ]
        );
    }
}
