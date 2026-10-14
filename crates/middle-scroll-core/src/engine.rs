use crate::config::{AxisLock, CoreConfig, Mode};
use crate::model::{CoreAction, CoreInputEvent, EngineState, MouseButton};

/// Coasting stops once an axis decays below this speed (detents per second).
const INERTIA_STOP_DETENTS_PER_SECOND: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Vertical,
    Horizontal,
}

impl Axis {
    const fn detents(self, n: i32) -> CoreAction {
        match self {
            Self::Vertical => CoreAction::EmitWheelDetents {
                vertical: n,
                horizontal: 0,
            },
            Self::Horizontal => CoreAction::EmitWheelDetents {
                vertical: 0,
                horizontal: n,
            },
        }
    }

    const fn hires(self, n: i32) -> CoreAction {
        match self {
            Self::Vertical => CoreAction::EmitWheelHiRes {
                vertical_units: n,
                horizontal_units: 0,
            },
            Self::Horizontal => CoreAction::EmitWheelHiRes {
                vertical_units: 0,
                horizontal_units: n,
            },
        }
    }
}

/// Per-axis scroll state.
#[derive(Debug, Clone, Copy)]
struct AxisState {
    /// Pointer displacement from the press point, clamped to
    /// `±max_offset_units`.
    offset: i32,
    /// Fractional legacy detents not emitted yet.
    detents: f64,
    /// Fractional hi-res units not emitted yet.
    hires: f64,
    /// Wheel direction (after inversion) used on the previous tick: -1, 0 or 1.
    /// The accumulators are cleared whenever it changes, so a reversal never
    /// has to "pay back" the remainder of the opposite direction first.
    last_sign: i32,
    /// Signed wheel speed of the last tick, in detents per second. Seeds the
    /// coasting speed when inertia is enabled.
    velocity: f64,
}

impl AxisState {
    const ZERO: Self = Self {
        offset: 0,
        detents: 0.0,
        hires: 0.0,
        last_sign: 0,
        velocity: 0.0,
    };

    fn clear_motion(&mut self) {
        self.detents = 0.0;
        self.hires = 0.0;
        self.last_sign = 0;
        self.velocity = 0.0;
    }
}

#[derive(Debug)]
pub struct Engine {
    config: CoreConfig,
    /// Configuration received while a gesture was in progress; applied as soon
    /// as the engine is back to idle so the rules never change mid-gesture.
    pending_config: Option<CoreConfig>,
    state: EngineState,
    y: AxisState,
    x: AxisState,
    /// Axis chosen by [`AxisLock::First`] for the current gesture.
    locked_axis: Option<Axis>,
    /// Scroll mode was latched by a click in toggle mode (the trigger is not
    /// held).
    latched: bool,
    /// The press that ended a latched scroll (or caught a coast) was
    /// swallowed; its matching release must be swallowed too.
    swallow_release: Option<MouseButton>,
    /// The current press interrupted a coast: releasing it inside the deadzone
    /// only stops scrolling instead of emitting a click.
    caught_coast: bool,
    /// Net pointer displacement accumulated while `MiddlePending`, replayed as
    /// a single motion before a short click when
    /// `replay_pending_motion_on_click` is set. Kept as a compact sum (not a
    /// list of events) so a long press can never grow memory unboundedly.
    pending_motion: (i32, i32),
}

impl Engine {
    pub const fn new(config: CoreConfig) -> Self {
        Self {
            config,
            pending_config: None,
            state: EngineState::Idle,
            y: AxisState::ZERO,
            x: AxisState::ZERO,
            locked_axis: None,
            latched: false,
            swallow_release: None,
            caught_coast: false,
            pending_motion: (0, 0),
        }
    }

    pub const fn config(&self) -> &CoreConfig {
        &self.config
    }

    pub const fn state(&self) -> EngineState {
        self.state
    }

    /// True while scroll mode was latched by a click (toggle mode).
    pub const fn is_latched(&self) -> bool {
        self.latched
    }

    pub const fn offset_y_units(&self) -> i32 {
        self.y.offset
    }

    pub const fn offset_x_units(&self) -> i32 {
        self.x.offset
    }

    /// Replaces the configuration (e.g. on reload or when the foreground
    /// application selects another profile). The new configuration is applied
    /// immediately when idle, otherwise when the current gesture ends.
    ///
    /// The caller is responsible for passing a validated configuration.
    pub fn set_config(&mut self, config: CoreConfig) {
        if self.state == EngineState::Idle {
            self.config = config;
            self.pending_config = None;
        } else {
            self.pending_config = Some(config);
        }
    }

    /// Cancels whatever gesture is in progress and returns to idle without
    /// emitting a click. Emits [`CoreAction::ExitScrollMode`] when scroll mode
    /// was active. Used when pausing, when the device disappears, or when the
    /// foreground filter turns autoscroll off.
    ///
    /// If the trigger is still physically held, its release is swallowed so
    /// the compositor never sees a release without a press.
    pub fn abort(&mut self, out: &mut Vec<CoreAction>) {
        let trigger_held = match self.state {
            EngineState::MiddlePending => true,
            EngineState::Scrolling => !self.latched,
            EngineState::Idle | EngineState::Coasting => false,
        };
        if self.state.needs_ticks() {
            out.push(CoreAction::ExitScrollMode);
        }
        self.reset_to_idle();
        self.swallow_release = trigger_held.then_some(MouseButton::Middle);
    }

    /// Convenience wrapper around [`process_into`] that returns a fresh `Vec`.
    /// Prefer `process_into` with a reused buffer on hot paths.
    ///
    /// [`process_into`]: Engine::process_into
    pub fn process(&mut self, event: CoreInputEvent) -> Vec<CoreAction> {
        let mut out = Vec::new();
        self.process_into(event, &mut out);
        out
    }

    /// Processes one input event, appending the resulting actions to `out`.
    /// The buffer is not cleared, so callers can batch several events; on hot
    /// paths this avoids one heap allocation per input event.
    pub fn process_into(&mut self, event: CoreInputEvent, out: &mut Vec<CoreAction>) {
        match self.state {
            EngineState::Idle => self.process_idle(event, out),
            EngineState::MiddlePending => self.process_pending(event, out),
            EngineState::Scrolling => self.process_scrolling(event, out),
            EngineState::Coasting => self.process_coasting(event, out),
        }
    }

    fn process_idle(&mut self, event: CoreInputEvent, out: &mut Vec<CoreAction>) {
        if let Some(button) = self.swallow_release {
            match button_transition(event) {
                Some((b, false)) if b == button => {
                    self.swallow_release = None;
                    out.push(CoreAction::Suppress);
                    return;
                }
                // A new press of the same button means its release was lost
                // (e.g. the device was unplugged); stop waiting for it.
                Some((b, true)) if b == button => self.swallow_release = None,
                _ => {}
            }
        }
        match event {
            CoreInputEvent::MiddleDown => {
                self.reset_to_idle();
                self.state = EngineState::MiddlePending;
                out.push(CoreAction::Suppress);
            }
            CoreInputEvent::Tick { .. } => {}
            other => forward(other, out),
        }
    }

    fn process_pending(&mut self, event: CoreInputEvent, out: &mut Vec<CoreAction>) {
        match event {
            CoreInputEvent::MiddleDown | CoreInputEvent::Tick { .. } => {}
            CoreInputEvent::MiddleUp => {
                if self.caught_coast {
                    // The press only stopped a coast.
                    out.push(CoreAction::Suppress);
                    self.reset_to_idle();
                } else if self.config.mode == Mode::ToggleProgressive {
                    out.push(CoreAction::Suppress);
                    self.state = EngineState::Scrolling;
                    self.latched = true;
                    self.pending_motion = (0, 0);
                    out.push(CoreAction::EnterScrollMode);
                } else {
                    if self.config.replay_pending_motion_on_click {
                        let (dx, dy) = self.pending_motion;
                        if dx != 0 || dy != 0 {
                            out.push(CoreAction::ForwardMotion { dx, dy });
                        }
                    }
                    out.push(CoreAction::EmitMiddleClick);
                    self.reset_to_idle();
                }
            }
            CoreInputEvent::Motion { dx, dy } => {
                self.accumulate_offset(dx, dy);

                let suppressed = self.config.suppress_motion_while_pending;
                if self.config.replay_pending_motion_on_click && suppressed {
                    self.pending_motion.0 = self.pending_motion.0.saturating_add(dx);
                    self.pending_motion.1 = self.pending_motion.1.saturating_add(dy);
                }

                if suppressed {
                    out.push(CoreAction::Suppress);
                } else {
                    out.push(CoreAction::ForwardMotion { dx, dy });
                }

                if self.crossed_deadzone() {
                    self.state = EngineState::Scrolling;
                    self.pending_motion = (0, 0);
                    out.push(CoreAction::EnterScrollMode);
                }
            }
            other => forward(other, out),
        }
    }

    fn process_scrolling(&mut self, event: CoreInputEvent, out: &mut Vec<CoreAction>) {
        if self.latched {
            if let Some((button, true)) = button_transition(event) {
                // Any click ends a latched scroll; the click itself is eaten.
                self.reset_to_idle();
                self.swallow_release = Some(button);
                out.push(CoreAction::Suppress);
                out.push(CoreAction::ExitScrollMode);
                return;
            }
        }
        match event {
            CoreInputEvent::MiddleDown => {}
            CoreInputEvent::MiddleUp => {
                if self.latched {
                    out.push(CoreAction::Suppress);
                } else if self.config.inertia && self.has_velocity() {
                    self.state = EngineState::Coasting;
                    out.push(CoreAction::Suppress);
                } else {
                    self.reset_to_idle();
                    out.push(CoreAction::ExitScrollMode);
                }
            }
            CoreInputEvent::Motion { dx, dy } => {
                self.accumulate_offset(dx, dy);
                if self.config.suppress_motion_while_scrolling {
                    out.push(CoreAction::Suppress);
                } else {
                    out.push(CoreAction::ForwardMotion { dx, dy });
                }
            }
            CoreInputEvent::Tick { dt_micros } => self.tick(dt_micros, out),
            other => forward(other, out),
        }
    }

    fn process_coasting(&mut self, event: CoreInputEvent, out: &mut Vec<CoreAction>) {
        match event {
            CoreInputEvent::MiddleDown => {
                // Catch the coast: a drag starts a new scroll right away, a
                // plain click only stops the current one.
                out.push(CoreAction::ExitScrollMode);
                self.reset_to_idle();
                self.process_idle(event, out);
                self.caught_coast = true;
            }
            CoreInputEvent::LeftDown
            | CoreInputEvent::RightDown
            | CoreInputEvent::Wheel { .. }
            | CoreInputEvent::WheelHiRes { .. } => {
                // Clicking or using the real wheel stops the coast and still
                // goes through.
                out.push(CoreAction::ExitScrollMode);
                self.reset_to_idle();
                forward(event, out);
            }
            CoreInputEvent::Tick { dt_micros } => self.coast(dt_micros, out),
            other => forward(other, out),
        }
    }

    fn reset_to_idle(&mut self) {
        self.state = EngineState::Idle;
        self.y = AxisState::ZERO;
        self.x = AxisState::ZERO;
        self.locked_axis = None;
        self.latched = false;
        self.caught_coast = false;
        self.pending_motion = (0, 0);
        if let Some(config) = self.pending_config.take() {
            self.config = config;
        }
    }

    fn accumulate_offset(&mut self, dx: i32, dy: i32) {
        let max = self.config.max_offset_units;
        self.y.offset = self.y.offset.saturating_add(dy).clamp(-max, max);
        if self.config.horizontal_scroll {
            self.x.offset = self.x.offset.saturating_add(dx).clamp(-max, max);
        }
    }

    const fn crossed_deadzone(&self) -> bool {
        self.y.offset.abs() > self.config.deadzone_units
            || (self.config.horizontal_scroll && self.x.offset.abs() > self.config.deadzone_units)
    }

    fn has_velocity(&self) -> bool {
        self.y.velocity != 0.0 || self.x.velocity != 0.0
    }

    /// Which axes may scroll on this tick, according to `axis_lock`.
    fn allowed_axes(&mut self) -> (bool, bool) {
        let deadzone = self.config.deadzone_units;
        let dist_y = self.y.offset.saturating_abs();
        let dist_x = if self.config.horizontal_scroll {
            self.x.offset.saturating_abs()
        } else {
            0
        };
        let y_active = dist_y > deadzone;
        let x_active = dist_x > deadzone;
        let dominant = if !y_active && !x_active {
            None
        } else if dist_y >= dist_x {
            Some(Axis::Vertical)
        } else {
            Some(Axis::Horizontal)
        };
        let only = |axis: Option<Axis>| match axis {
            Some(Axis::Vertical) => (y_active, false),
            Some(Axis::Horizontal) => (false, x_active),
            None => (false, false),
        };
        match self.config.axis_lock {
            AxisLock::Off => (y_active, x_active),
            AxisLock::Dominant => only(dominant),
            AxisLock::First => {
                if self.locked_axis.is_none() {
                    self.locked_axis = dominant;
                }
                only(self.locked_axis)
            }
        }
    }

    fn tick(&mut self, dt_micros: u64, out: &mut Vec<CoreAction>) {
        let dt_seconds = dt_micros as f64 / 1_000_000.0;
        let (allow_y, allow_x) = self.allowed_axes();
        let config = &self.config;
        scroll_axis(
            config,
            &mut self.y,
            Axis::Vertical,
            allow_y,
            dt_seconds,
            out,
        );
        if config.horizontal_scroll {
            scroll_axis(
                config,
                &mut self.x,
                Axis::Horizontal,
                allow_x,
                dt_seconds,
                out,
            );
        }
    }

    /// One coasting step: every axis keeps its last speed, halving every
    /// `inertia_half_life_ms`, until all of them are slow enough to stop.
    fn coast(&mut self, dt_micros: u64, out: &mut Vec<CoreAction>) {
        let dt_seconds = dt_micros as f64 / 1_000_000.0;
        let half_life = f64::from(self.config.inertia_half_life_ms) / 1000.0;
        let decay = (-dt_seconds / half_life * std::f64::consts::LN_2).exp();
        let config = &self.config;
        for (axis, state) in [
            (Axis::Vertical, &mut self.y),
            (Axis::Horizontal, &mut self.x),
        ] {
            if state.velocity == 0.0 {
                continue;
            }
            state.velocity *= decay;
            if !state.velocity.is_finite() || state.velocity.abs() < INERTIA_STOP_DETENTS_PER_SECOND
            {
                state.clear_motion();
                continue;
            }
            emit_axis(config, state, axis, state.velocity * dt_seconds, out);
        }
        if !self.has_velocity() {
            self.reset_to_idle();
            out.push(CoreAction::ExitScrollMode);
        }
    }
}

/// Returns `(button, pressed)` for button events handled by the engine.
const fn button_transition(event: CoreInputEvent) -> Option<(MouseButton, bool)> {
    match event {
        CoreInputEvent::MiddleDown => Some((MouseButton::Middle, true)),
        CoreInputEvent::MiddleUp => Some((MouseButton::Middle, false)),
        CoreInputEvent::LeftDown => Some((MouseButton::Left, true)),
        CoreInputEvent::LeftUp => Some((MouseButton::Left, false)),
        CoreInputEvent::RightDown => Some((MouseButton::Right, true)),
        CoreInputEvent::RightUp => Some((MouseButton::Right, false)),
        _ => None,
    }
}

/// Passes an event through unchanged.
fn forward(event: CoreInputEvent, out: &mut Vec<CoreAction>) {
    match event {
        CoreInputEvent::Motion { dx, dy } => out.push(CoreAction::ForwardMotion { dx, dy }),
        CoreInputEvent::Wheel {
            vertical,
            horizontal,
        } => out.push(CoreAction::ForwardWheel {
            vertical,
            horizontal,
        }),
        CoreInputEvent::WheelHiRes {
            vertical_units,
            horizontal_units,
        } => out.push(CoreAction::EmitWheelHiRes {
            vertical_units,
            horizontal_units,
        }),
        CoreInputEvent::Tick { .. } => {}
        button => {
            if let Some((button, pressed)) = button_transition(button) {
                out.push(CoreAction::ForwardMouseButton { button, pressed });
            }
        }
    }
}

/// Scrolls one axis for one tick, based on its offset from the press point.
fn scroll_axis(
    config: &CoreConfig,
    state: &mut AxisState,
    axis: Axis,
    allowed: bool,
    dt_seconds: f64,
    out: &mut Vec<CoreAction>,
) {
    let sign = if allowed {
        wheel_sign(config, axis, state.offset)
    } else {
        0
    };
    if sign != state.last_sign {
        state.detents = 0.0;
        state.hires = 0.0;
        state.last_sign = sign;
    }
    let speed = speed_detents_per_second(config, state.offset.saturating_abs());
    if sign == 0 || speed <= 0.0 {
        state.velocity = 0.0;
        return;
    }
    state.velocity = f64::from(sign) * speed;
    emit_axis(config, state, axis, state.velocity * dt_seconds, out);
}

/// Adds `delta_detents` to the axis accumulators and emits whatever whole
/// detents / hi-res chunks are ready.
fn emit_axis(
    config: &CoreConfig,
    state: &mut AxisState,
    axis: Axis,
    delta_detents: f64,
    out: &mut Vec<CoreAction>,
) {
    if !delta_detents.is_finite() {
        state.detents = 0.0;
        state.hires = 0.0;
        return;
    }
    if config.emit_legacy_wheel {
        state.detents += delta_detents;
        drain_legacy_axis(&mut state.detents, config.max_detents_per_tick, |n| {
            out.push(axis.detents(n));
        });
    }
    if config.emit_hires_wheel {
        state.hires = delta_detents.mul_add(HIRES_UNITS_PER_DETENT_F64, state.hires);
        let max_units = config
            .max_detents_per_tick
            .saturating_mul(HIRES_UNITS_PER_DETENT);
        if let Some(n) = drain_hires_axis(
            &mut state.hires,
            max_units,
            config.min_hires_units_per_event,
        ) {
            out.push(axis.hires(n));
        }
    }
}

/// Wheel direction for an offset: pointer DOWN (positive y offset) scrolls
/// the content down, i.e. a negative `REL_WHEEL`; pointer RIGHT (positive x
/// offset) is a positive `REL_HWHEEL`. Inside the deadzone the sign is 0.
const fn wheel_sign(config: &CoreConfig, axis: Axis, offset: i32) -> i32 {
    let deadzone = config.deadzone_units;
    let raw = if offset > deadzone {
        1
    } else if offset < -deadzone {
        -1
    } else {
        0
    };
    let (natural, invert) = match axis {
        Axis::Vertical => (-raw, config.invert_vertical),
        Axis::Horizontal => (raw, config.invert_horizontal),
    };
    if invert {
        -natural
    } else {
        natural
    }
}

/// Speed for the given absolute distance from the press point.
///
/// The default profile is the smooth progressive curve driven by
/// `min_speed_detents_per_second`, `max_speed_detents_per_second`,
/// `full_speed_units` and `acceleration_exponent`. When the user configures
/// `scroll_speed_steps`, the stepped profile takes over from the first step
/// on (the last reached step controls the speed); between the deadzone and
/// the first step the smooth curve still applies.
fn speed_detents_per_second(config: &CoreConfig, distance: i32) -> f64 {
    if distance <= config.deadzone_units {
        return 0.0;
    }
    if let Some(step) = config
        .scroll_speed_steps
        .iter()
        .rev()
        .find(|step| distance >= step.distance_units)
    {
        return step.speed_detents_per_second;
    }
    let active = f64::from(distance - config.deadzone_units);
    let full = f64::from(config.full_speed_units);
    let normalized = (active / full).clamp(0.0, 1.0);
    let min_s = config.min_speed_detents_per_second;
    let max_s = config.max_speed_detents_per_second;
    let exp = config.acceleration_exponent;
    (max_s - min_s).mul_add(normalized.powf(exp), min_s)
}

fn drain_legacy_axis(accumulator: &mut f64, max: i32, mut emit: impl FnMut(i32)) {
    loop {
        let raw = accumulator.trunc() as i32;
        if raw == 0 {
            break;
        }
        let n = raw.clamp(-max, max);
        emit(n);
        *accumulator -= f64::from(n);
        if n != raw {
            *accumulator = 0.0;
            break;
        }
    }
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

const HIRES_UNITS_PER_DETENT: i32 = 120;
const HIRES_UNITS_PER_DETENT_F64: f64 = 120.0;
