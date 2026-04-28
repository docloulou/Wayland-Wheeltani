use thiserror::Error;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Mode {
    #[default]
    HoldProgressive,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
pub struct SpeedStep {
    pub distance_units: i32,
    pub speed_detents_per_second: f64,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(default, deny_unknown_fields))]
#[allow(clippy::struct_excessive_bools)]
pub struct CoreConfig {
    pub mode: Mode,

    pub deadzone_units: i32,
    pub full_speed_units: i32,
    pub max_offset_units: i32,

    pub min_speed_detents_per_second: f64,
    pub max_speed_detents_per_second: f64,
    pub acceleration_exponent: f64,
    pub scroll_speed_steps: Vec<SpeedStep>,

    pub tick_hz: u32,

    pub invert_vertical: bool,
    pub invert_horizontal: bool,

    pub suppress_motion_while_pending: bool,
    pub suppress_motion_while_scrolling: bool,
    pub replay_pending_motion_on_click: bool,

    pub emit_hires_wheel: bool,
    pub emit_legacy_wheel: bool,
    pub min_hires_units_per_event: i32,

    pub horizontal_scroll: bool,
    pub max_detents_per_tick: i32,
}

impl Default for CoreConfig {
    fn default() -> Self {
        Self {
            mode: Mode::HoldProgressive,

            deadzone_units: 10,
            full_speed_units: 120,
            max_offset_units: 240,

            min_speed_detents_per_second: 1.5,
            max_speed_detents_per_second: 32.0,
            acceleration_exponent: 1.6,
            scroll_speed_steps: default_scroll_speed_steps(),

            tick_hz: 120,

            invert_vertical: false,
            invert_horizontal: false,

            suppress_motion_while_pending: true,
            suppress_motion_while_scrolling: true,
            replay_pending_motion_on_click: false,

            emit_hires_wheel: true,
            emit_legacy_wheel: true,
            min_hires_units_per_event: 15,

            horizontal_scroll: true,
            max_detents_per_tick: 4,
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum ConfigError {
    #[error("deadzone_units must be >= 0, got {0}")]
    NegativeDeadzone(i32),
    #[error("full_speed_units must be > 0, got {0}")]
    NonPositiveFullSpeed(i32),
    #[error("max_offset_units ({max}) must be > deadzone_units ({deadzone})")]
    MaxOffsetTooSmall { max: i32, deadzone: i32 },
    #[error("min_speed_detents_per_second must be > 0, got {0}")]
    NonPositiveMinSpeed(f64),
    #[error(
        "max_speed_detents_per_second ({max}) must be >= min_speed_detents_per_second ({min})"
    )]
    MaxSpeedLessThanMin { max: f64, min: f64 },
    #[error("acceleration_exponent must be > 0 and finite, got {0}")]
    BadExponent(f64),
    #[error("tick_hz must be in 1..=10000, got {0}")]
    BadTickHz(u32),
    #[error("max_detents_per_tick must be >= 1, got {0}")]
    BadMaxDetents(i32),
    #[error("min_hires_units_per_event must be in 1..=120, got {0}")]
    BadMinHiResUnits(i32),
    #[error("scroll_speed_steps[{index}].distance_units must be > deadzone_units ({deadzone}), got {distance}")]
    BadSpeedStepDistance {
        index: usize,
        deadzone: i32,
        distance: i32,
    },
    #[error("scroll_speed_steps[{index}].distance_units ({distance}) must be <= max_offset_units ({max})")]
    SpeedStepBeyondMaxOffset {
        index: usize,
        distance: i32,
        max: i32,
    },
    #[error("scroll_speed_steps must be strictly sorted by distance_units: step {index} has {current} after {previous}")]
    SpeedStepsNotSorted {
        index: usize,
        previous: i32,
        current: i32,
    },
    #[error(
        "scroll_speed_steps[{index}].speed_detents_per_second must be > 0 and finite, got {speed}"
    )]
    BadSpeedStepSpeed { index: usize, speed: f64 },
}

impl CoreConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.deadzone_units < 0 {
            return Err(ConfigError::NegativeDeadzone(self.deadzone_units));
        }
        if self.full_speed_units <= 0 {
            return Err(ConfigError::NonPositiveFullSpeed(self.full_speed_units));
        }
        if self.max_offset_units <= self.deadzone_units {
            return Err(ConfigError::MaxOffsetTooSmall {
                max: self.max_offset_units,
                deadzone: self.deadzone_units,
            });
        }
        if !self.min_speed_detents_per_second.is_finite()
            || self.min_speed_detents_per_second <= 0.0
        {
            return Err(ConfigError::NonPositiveMinSpeed(
                self.min_speed_detents_per_second,
            ));
        }
        if !self.max_speed_detents_per_second.is_finite()
            || self.max_speed_detents_per_second < self.min_speed_detents_per_second
        {
            return Err(ConfigError::MaxSpeedLessThanMin {
                max: self.max_speed_detents_per_second,
                min: self.min_speed_detents_per_second,
            });
        }
        if !self.acceleration_exponent.is_finite() || self.acceleration_exponent <= 0.0 {
            return Err(ConfigError::BadExponent(self.acceleration_exponent));
        }
        if self.tick_hz == 0 || self.tick_hz > 10_000 {
            return Err(ConfigError::BadTickHz(self.tick_hz));
        }
        if self.max_detents_per_tick < 1 {
            return Err(ConfigError::BadMaxDetents(self.max_detents_per_tick));
        }
        if !(1..=120).contains(&self.min_hires_units_per_event) {
            return Err(ConfigError::BadMinHiResUnits(
                self.min_hires_units_per_event,
            ));
        }
        let mut previous_distance = None;
        for (index, step) in self.scroll_speed_steps.iter().enumerate() {
            if step.distance_units <= self.deadzone_units {
                return Err(ConfigError::BadSpeedStepDistance {
                    index,
                    deadzone: self.deadzone_units,
                    distance: step.distance_units,
                });
            }
            if step.distance_units > self.max_offset_units {
                return Err(ConfigError::SpeedStepBeyondMaxOffset {
                    index,
                    distance: step.distance_units,
                    max: self.max_offset_units,
                });
            }
            if let Some(previous) = previous_distance {
                if step.distance_units <= previous {
                    return Err(ConfigError::SpeedStepsNotSorted {
                        index,
                        previous,
                        current: step.distance_units,
                    });
                }
            }
            if !step.speed_detents_per_second.is_finite() || step.speed_detents_per_second <= 0.0 {
                return Err(ConfigError::BadSpeedStepSpeed {
                    index,
                    speed: step.speed_detents_per_second,
                });
            }
            previous_distance = Some(step.distance_units);
        }
        Ok(())
    }
}

fn default_scroll_speed_steps() -> Vec<SpeedStep> {
    vec![
        SpeedStep {
            distance_units: 11,
            speed_detents_per_second: 1.5,
        },
        SpeedStep {
            distance_units: 40,
            speed_detents_per_second: 4.0,
        },
        SpeedStep {
            distance_units: 80,
            speed_detents_per_second: 10.0,
        },
        SpeedStep {
            distance_units: 140,
            speed_detents_per_second: 18.0,
        },
        SpeedStep {
            distance_units: 220,
            speed_detents_per_second: 32.0,
        },
    ]
}
