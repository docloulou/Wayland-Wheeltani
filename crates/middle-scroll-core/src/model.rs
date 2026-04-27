//! Cross-platform input/output types for the scroll engine.
//!
//! These types deliberately avoid any reference to evdev / uinput / Linux input
//! event codes so that the core can be unit-tested on any host (notably macOS).

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
    Side,
    Extra,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoreInputEvent {
    MiddleDown,
    MiddleUp,
    LeftDown,
    LeftUp,
    RightDown,
    RightUp,
    Motion {
        dx: i32,
        dy: i32,
    },
    Wheel {
        vertical: i32,
        horizontal: i32,
    },
    WheelHiRes {
        vertical_units: i32,
        horizontal_units: i32,
    },
    Tick {
        dt_micros: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreAction {
    ForwardMouseButton {
        button: MouseButton,
        pressed: bool,
    },
    ForwardMotion {
        dx: i32,
        dy: i32,
    },
    ForwardWheel {
        vertical: i32,
        horizontal: i32,
    },
    EmitWheelDetents {
        vertical: i32,
        horizontal: i32,
    },
    EmitWheelHiRes {
        vertical_units: i32,
        horizontal_units: i32,
    },
    EmitMiddleClick,
    Suppress,
    EnterScrollMode,
    ExitScrollMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineState {
    Idle,
    MiddlePending,
    Scrolling,
}
