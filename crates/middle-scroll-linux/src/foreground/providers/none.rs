//! A provider that never resolves a foreground app. Used for
//! `provider = "none"` and as the `auto` fallback when no session is detected.

use crate::foreground::filter::{ForegroundProvider, ForegroundSnapshot};

#[derive(Debug)]
pub struct NoneProvider {
    reason: String,
}

impl NoneProvider {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

impl ForegroundProvider for NoneProvider {
    fn snapshot(&self) -> ForegroundSnapshot {
        ForegroundSnapshot::Unsupported {
            reason: self.reason.clone(),
        }
    }
}
