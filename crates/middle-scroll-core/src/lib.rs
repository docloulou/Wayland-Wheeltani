//! Cross-platform core for the Wayland-Wheeltani daemon.
//!
//! This crate is intentionally free of any Linux input dependency (no `evdev`,
//! no `uinput`, no `libc`). It exposes:
//!
//! - [`CoreInputEvent`] / [`CoreAction`]: abstract event types translated from
//!   real OS events by a backend.
//! - [`CoreConfig`]: tunable parameters (deadzone, speed curve, accumulators).
//! - [`Engine`]: the state machine that turns input events + tick events into
//!   actions (forward, suppress, emit wheel, emit middle click...).
//!
//! See `SPEC.md` at the workspace root for the algorithm rationale.

pub mod config;
pub mod engine;
pub mod model;

pub use config::{ConfigError, CoreConfig, Mode, SpeedStep};
pub use engine::Engine;
pub use model::{CoreAction, CoreInputEvent, EngineState, MouseButton};

#[cfg(test)]
mod tests;
