use crate::config::CoreConfig;
use crate::model::{CoreAction, CoreInputEvent, EngineState, MouseButton};

#[derive(Debug)]
pub struct Engine {
    config: CoreConfig,
    state: EngineState,
    offset_y_units: i32,
    detent_accumulator: f64,
    hires_accumulator: f64,
    pending_motion: Vec<(i32, i32)>,
}

impl Engine {
    pub const fn new(config: CoreConfig) -> Self {
        Self {
            config,
            state: EngineState::Idle,
            offset_y_units: 0,
            detent_accumulator: 0.0,
            hires_accumulator: 0.0,
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
                self.offset_y_units = 0;
                self.detent_accumulator = 0.0;
                self.hires_accumulator = 0.0;
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
                self.accumulate_offset(dy);

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

                if self.offset_y_units.abs() > self.config.deadzone_units {
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
                self.accumulate_offset(dy);
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
        self.offset_y_units = 0;
        self.detent_accumulator = 0.0;
        self.hires_accumulator = 0.0;
        self.pending_motion.clear();
    }

    fn accumulate_offset(&mut self, dy: i32) {
        let max = self.config.max_offset_units;
        self.offset_y_units = (self.offset_y_units.saturating_add(dy)).clamp(-max, max);
    }

    fn forward_btn(button: MouseButton, pressed: bool) -> Vec<CoreAction> {
        vec![CoreAction::ForwardMouseButton { button, pressed }]
    }

    fn tick(&mut self, dt_micros: u64) -> Vec<CoreAction> {
        let speed = self.compute_speed_detents_per_second();
        let sign = self.wheel_sign();
        if speed == 0.0 || sign == 0 {
            return Vec::new();
        }

        let mut direction = f64::from(sign);
        if self.config.invert_vertical {
            direction = -direction;
        }
        let dt_seconds = dt_micros as f64 / 1_000_000.0;
        let delta_detents = direction * speed * dt_seconds;
        if !delta_detents.is_finite() {
            self.detent_accumulator = 0.0;
            self.hires_accumulator = 0.0;
            return Vec::new();
        }

        let mut actions = Vec::new();

        if self.config.emit_legacy_wheel {
            self.detent_accumulator += delta_detents;
            self.drain_legacy_detents(&mut actions);
        }

        if self.config.emit_hires_wheel {
            self.hires_accumulator += delta_detents * HIRES_UNITS_PER_DETENT_F64;
            self.drain_hires(&mut actions);
        }

        actions
    }

    fn compute_speed_detents_per_second(&self) -> f64 {
        let distance = self.offset_y_units.unsigned_abs() as i32;
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

    const fn wheel_sign(&self) -> i32 {
        if self.offset_y_units > self.config.deadzone_units {
            -1
        } else if self.offset_y_units < -self.config.deadzone_units {
            1
        } else {
            0
        }
    }

    fn drain_legacy_detents(&mut self, actions: &mut Vec<CoreAction>) {
        loop {
            let raw = self.detent_accumulator.trunc() as i32;
            if raw == 0 {
                break;
            }
            let max = self.config.max_detents_per_tick;
            let n = raw.clamp(-max, max);
            actions.push(CoreAction::EmitWheelDetents {
                vertical: n,
                horizontal: 0,
            });
            self.detent_accumulator -= f64::from(n);
            if n != raw {
                self.detent_accumulator = 0.0;
                break;
            }
        }
    }

    fn drain_hires(&mut self, actions: &mut Vec<CoreAction>) {
        let raw = self.hires_accumulator.trunc() as i32;
        if raw == 0 {
            return;
        }
        let max_units = self
            .config
            .max_detents_per_tick
            .saturating_mul(HIRES_UNITS_PER_DETENT);
        let n = raw.clamp(-max_units, max_units);
        actions.push(CoreAction::EmitWheelHiRes {
            vertical_units: n,
            horizontal_units: 0,
        });
        if n == raw {
            self.hires_accumulator -= f64::from(n);
        } else {
            self.hires_accumulator = 0.0;
        }
    }
}

const HIRES_UNITS_PER_DETENT: i32 = 120;
const HIRES_UNITS_PER_DETENT_F64: f64 = 120.0;
