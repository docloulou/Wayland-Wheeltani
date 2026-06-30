//! Optional foreground-application filter.
//!
//! This layer sits between `event_router::classify` and the autoscroll engine
//! and decides, per middle-button gesture, whether the engine should handle the
//! event (historical behaviour) or whether the event should be passed straight
//! through to the virtual device. It is disabled by default, so an unconfigured
//! daemon behaves exactly as before.
//!
//! See `SPEC.md` and the `[foreground]` table in `examples/config.toml`.

pub mod config;
pub mod filter;
pub mod matcher;
pub mod providers;

pub use config::{ForegroundConfig, ForegroundFileConfig};
pub use filter::{AutoscrollDecision, ForegroundGate};
