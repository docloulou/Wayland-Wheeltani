use crate::config::CoreConfig;
use crate::model::{CoreAction, CoreInputEvent, EngineState, MouseButton};

#[derive(Debug)]
pub struct Engine {
    config: CoreConfig,
    state: EngineState,
    offset_y_units: i32,
    offset_x_units: i32,
    detent_accumulator_y: f64,
    hires_accumulator_y: f64,
    detent_accumulator_x: f64,
    hires_accumulator_x: f64,
    pending_motion: Vec<(i32, i32)>,
}

impl Engine {
    pub const fn new(config: CoreConfig) -> Self {
        Self {
            config,
            state: EngineState::Idle,
            offset_y_units: 0,
            offset_x_units: 0,
            detent_accumulator_y: 0.0,
            hires_accumulator_y: 0.0,
            detent_accumulator_x: 0.0,
            hires_accumulator_x: 0.0,
            pending_motion: Vec::new(),
        }
    }

    pub const fn config(&self) -> &CoreConfig {
        &self.config
    }

    pub const fn state(&self) -> EngineState {
        self.state
    }

    pub const fn offset_y_units(&self) -> i32 {
        self.offset_y_units
    }

    pub const fn offset_x_units(&self) -> i32 {
        self.offset_x_units
    }

    pub fn process(&mut self, event: CoreInputEvent) -> Vec<CoreAction> {
        match self.state {
            EngineState::Idle => self.process_idle(event),
            EngineState::MiddlePending => self.process_pending(event),
            EngineState::Scrolling => self.process_scrolling(event),
        }
    }

    fn process_idle(&mut self, event: CoreInputEvent) -> Vec<CoreAction> {
        match event {
            CoreInputEvent::MiddleDown => {
                self.state = EngineState::MiddlePending;
                self.reset_offsets_and_accumulators();
                self.pending_motion.clear();
                vec![CoreAction::Suppress]
            }
            CoreInputEvent::MiddleUp => {
                vec![CoreAction::ForwardMouseButton {
                    button: MouseButton::Middle,
                    pressed: false,
                }]
            }
            CoreInputEvent::LeftDown => Self::forward_btn(MouseButton::Left, true),
            CoreInputEvent::LeftUp => Self::forward_btn(MouseButton::Left, false),
            CoreInputEvent::RightDown => Self::forward_btn(MouseButton::Right, true),
            CoreInputEvent::RightUp => Self::forward_btn(MouseButton::Right, false),
            CoreInputEvent::Motion { dx, dy } => {
                vec![CoreAction::ForwardMotion { dx, dy }]
            }
            CoreInputEvent::Wheel {
                vertical,
                horizontal,
            } => {
                vec![CoreAction::ForwardWheel {
                    vertical,
                    horizontal,
                }]
            }
            CoreInputEvent::WheelHiRes {
                vertical_units,
                horizontal_units,
            } => {
                vec![CoreAction::EmitWheelHiRes {
                    vertical_units,
                    horizontal_units,
                }]
            }
            CoreInputEvent::Tick { .. } => Vec::new(),
        }
    }

    fn process_pending(&mut self, event: CoreInputEvent) -> Vec<CoreAction> {
        match event {
            CoreInputEvent::MiddleDown | CoreInputEvent::Tick { .. } => Vec::new(),
            CoreInputEvent::MiddleUp => {
                let mut actions = Vec::new();
                if self.config.replay_pending_motion_on_click {
                    for (dx, dy) in self.pending_motion.drain(..) {
                        actions.push(CoreAction::ForwardMotion { dx, dy });
                    }
                }
                actions.push(CoreAction::EmitMiddleClick);
                self.reset_to_idle();
                actions
            }
            CoreInputEvent::LeftDown => Self::forward_btn(MouseButton::Left, true),
            CoreInputEvent::LeftUp => Self::forward_btn(MouseButton::Left, false),
            CoreInputEvent::RightDown => Self::forward_btn(MouseButton::Right, true),
            CoreInputEvent::RightUp => Self::forward_btn(MouseButton::Right, false),
            CoreInputEvent::Motion { dx, dy } => {
                self.accumulate_offset(dx, dy);

                let suppressed = self.config.suppress_motion_while_pending;
                if self.config.replay_pending_motion_on_click && suppressed {
                    self.pending_motion.push((dx, dy));
                }

                let mut actions = Vec::new();
                if suppressed {
                    actions.push(CoreAction::Suppress);
                } else {
                    actions.push(CoreAction::ForwardMotion { dx, dy });
                }

                if self.crossed_deadzone() {
                    self.state = EngineState::Scrolling;
                    self.pending_motion.clear();
                    actions.push(CoreAction::EnterScrollMode);
                }
                actions
            }
            CoreInputEvent::Wheel {
                vertical,
                horizontal,
            } => {
                vec![CoreAction::ForwardWheel {
                    vertical,
                    horizontal,
                }]
            }
            CoreInputEvent::WheelHiRes {
                vertical_units,
                horizontal_units,
            } => {
                vec![CoreAction::EmitWheelHiRes {
                    vertical_units,
                    horizontal_units,
                }]
            }
        }
    }

    fn process_scrolling(&mut self, event: CoreInputEvent) -> Vec<CoreAction> {
        match event {
            CoreInputEvent::MiddleDown => Vec::new(),
            CoreInputEvent::MiddleUp => {
                self.reset_to_idle();
                vec![CoreAction::ExitScrollMode]
            }
            CoreInputEvent::LeftDown => Self::forward_btn(MouseButton::Left, true),
            CoreInputEvent::LeftUp => Self::forward_btn(MouseButton::Left, false),
            CoreInputEvent::RightDown => Self::forward_btn(MouseButton::Right, true),
            CoreInputEvent::RightUp => Self::forward_btn(MouseButton::Right, false),
            CoreInputEvent::Motion { dx, dy } => {
                self.accumulate_offset(dx, dy);
                if self.config.suppress_motion_while_scrolling {
                    vec![CoreAction::Suppress]
                } else {
                    vec![CoreAction::ForwardMotion { dx, dy }]
                }
            }
            CoreInputEvent::Wheel {
                vertical,
                horizontal,
            } => {
                vec![CoreAction::ForwardWheel {
                    vertical,
                    horizontal,
                }]
            }
            CoreInputEvent::WheelHiRes {
                vertical_units,
                horizontal_units,
            } => {
                vec![CoreAction::EmitWheelHiRes {
                    vertical_units,
                    horizontal_units,
                }]
            }
            CoreInputEvent::Tick { dt_micros } => self.tick(dt_micros),
        }
    }

    fn reset_to_idle(&mut self) {
        self.state = EngineState::Idle;
        self.reset_offsets_and_accumulators();
        self.pending_motion.clear();
    }

    fn reset_offsets_and_accumulators(&mut self) {
        self.offset_y_units = 0;
        self.offset_x_units = 0;
        self.detent_accumulator_y = 0.0;
        self.hires_accumulator_y = 0.0;
        self.detent_accumulator_x = 0.0;
        self.hires_accumulator_x = 0.0;
    }

    fn accumulate_offset(&mut self, dx: i32, dy: i32) {
        let max = self.config.max_offset_units;
        self.offset_y_units = self.offset_y_units.saturating_add(dy).clamp(-max, max);
        if self.config.horizontal_scroll {
            self.offset_x_units = self.offset_x_units.saturating_add(dx).clamp(-max, max);
        }
    }

    const fn crossed_deadzone(&self) -> bool {
        self.offset_y_units.abs() > self.config.deadzone_units
            || (self.config.horizontal_scroll
                && self.offset_x_units.abs() > self.config.deadzone_units)
    }

    fn forward_btn(button: MouseButton, pressed: bool) -> Vec<CoreAction> {
        vec![CoreAction::ForwardMouseButton { button, pressed }]
    }

    fn tick(&mut self, dt_micros: u64) -> Vec<CoreAction> {
        let dt_seconds = dt_micros as f64 / 1_000_000.0;
        let mut actions = Vec::new();
        self.tick_axis_vertical(dt_seconds, &mut actions);
        if self.config.horizontal_scroll {
            self.tick_axis_horizontal(dt_seconds, &mut actions);
        }
        actions
    }

    fn tick_axis_vertical(&mut self, dt_seconds: f64, actions: &mut Vec<CoreAction>) {
        let distance = self.offset_y_units.unsigned_abs() as i32;
        let speed = self.compute_speed_detents_per_second(distance);
        let sign = self.wheel_sign_y();
        if speed == 0.0 || sign == 0 {
            return;
        }
        let mut direction = f64::from(sign);
        if self.config.invert_vertical {
            direction = -direction;
        }
        let delta_detents = direction * speed * dt_seconds;
        if !delta_detents.is_finite() {
            self.detent_accumulator_y = 0.0;
            self.hires_accumulator_y = 0.0;
            return;
        }
        if self.config.emit_legacy_wheel {
            self.detent_accumulator_y += delta_detents;
            let max = self.config.max_detents_per_tick;
            for n in Self::drain_legacy_axis(&mut self.detent_accumulator_y, max) {
                actions.push(CoreAction::EmitWheelDetents {
                    vertical: n,
                    horizontal: 0,
                });
            }
        }
        if self.config.emit_hires_wheel {
            self.hires_accumulator_y =
                delta_detents.mul_add(HIRES_UNITS_PER_DETENT_F64, self.hires_accumulator_y);
            let max_units = self
                .config
                .max_detents_per_tick
                .saturating_mul(HIRES_UNITS_PER_DETENT);
            if let Some(n) = Self::drain_hires_axis(
                &mut self.hires_accumulator_y,
                max_units,
                self.config.min_hires_units_per_event,
            ) {
                actions.push(CoreAction::EmitWheelHiRes {
                    vertical_units: n,
                    horizontal_units: 0,
                });
            }
        }
    }

    fn tick_axis_horizontal(&mut self, dt_seconds: f64, actions: &mut Vec<CoreAction>) {
        let distance = self.offset_x_units.unsigned_abs() as i32;
        let speed = self.compute_speed_detents_per_second(distance);
        let sign = self.wheel_sign_x();
        if speed == 0.0 || sign == 0 {
            return;
        }
        let mut direction = f64::from(sign);
        if self.config.invert_horizontal {
            direction = -direction;
        }
        let delta_detents = direction * speed * dt_seconds;
        if !delta_detents.is_finite() {
            self.detent_accumulator_x = 0.0;
            self.hires_accumulator_x = 0.0;
            return;
        }
        if self.config.emit_legacy_wheel {
            self.detent_accumulator_x += delta_detents;
            let max = self.config.max_detents_per_tick;
            for n in Self::drain_legacy_axis(&mut self.detent_accumulator_x, max) {
                actions.push(CoreAction::EmitWheelDetents {
                    vertical: 0,
                    horizontal: n,
                });
            }
        }
        if self.config.emit_hires_wheel {
            self.hires_accumulator_x =
                delta_detents.mul_add(HIRES_UNITS_PER_DETENT_F64, self.hires_accumulator_x);
            let max_units = self
                .config
                .max_detents_per_tick
                .saturating_mul(HIRES_UNITS_PER_DETENT);
            if let Some(n) = Self::drain_hires_axis(
                &mut self.hires_accumulator_x,
                max_units,
                self.config.min_hires_units_per_event,
            ) {
                actions.push(CoreAction::EmitWheelHiRes {
                    vertical_units: 0,
                    horizontal_units: n,
                });
            }
        }
    }

    fn compute_speed_detents_per_second(&self, distance: i32) -> f64 {
        if distance <= self.config.deadzone_units {
            return 0.0;
        }
        if let Some(step) = self
            .config
            .scroll_speed_steps
            .iter()
            .rev()
            .find(|step| distance >= step.distance_units)
        {
            return step.speed_detents_per_second;
        }
        let active = f64::from(distance - self.config.deadzone_units);
        let full = f64::from(self.config.full_speed_units);
        let normalized = (active / full).clamp(0.0, 1.0);
        let min_s = self.config.min_speed_detents_per_second;
        let max_s = self.config.max_speed_detents_per_second;
        let exp = self.config.acceleration_exponent;
        (max_s - min_s).mul_add(normalized.powf(exp), min_s)
    }

    /// Vertical wheel sign: pointer moved DOWN (positive `offset_y`) maps to a
    /// negative `REL_WHEEL` value (scroll content downward). Pointer up
    /// returns +1.
    const fn wheel_sign_y(&self) -> i32 {
        if self.offset_y_units > self.config.deadzone_units {
            -1
        } else if self.offset_y_units < -self.config.deadzone_units {
            1
        } else {
            0
        }
    }

    /// Horizontal wheel sign: pointer moved RIGHT (positive `offset_x`) maps
    /// to a positive `REL_HWHEEL` value (scroll content rightward). Pointer
    /// left returns -1.
    const fn wheel_sign_x(&self) -> i32 {
        if self.offset_x_units > self.config.deadzone_units {
            1
        } else if self.offset_x_units < -self.config.deadzone_units {
            -1
        } else {
            0
        }
    }

    fn drain_legacy_axis(accumulator: &mut f64, max: i32) -> Vec<i32> {
        let mut out = Vec::new();
        loop {
            let raw = accumulator.trunc() as i32;
            if raw == 0 {
                break;
            }
            let n = raw.clamp(-max, max);
            out.push(n);
            *accumulator -= f64::from(n);
            if n != raw {
                *accumulator = 0.0;
                break;
            }
        }
        out
    }

    fn drain_hires_axis(accumulator: &mut f64, max_units: i32, min_units: i32) -> Option<i32> {
        let raw = accumulator.trunc() as i32;
        if raw.unsigned_abs() < min_units as u32 {
            return None;
        }
        let n = raw.clamp(-max_units, max_units);
        if n == raw {
            *accumulator -= f64::from(n);
        } else {
            *accumulator = 0.0;
        }
        Some(n)
    }
}

const HIRES_UNITS_PER_DETENT: i32 = 120;
const HIRES_UNITS_PER_DETENT_F64: f64 = 120.0;
