//! The daemon loop.
//!
//! One thread reads every physical mouse, feeds the autoscroll engine and
//! writes the virtual mouse. Between input frames it serves the control
//! socket, reloads the config (SIGHUP or a change of the file), reconnects
//! unplugged mice and ticks the engine while it scrolls. When idle it sleeps
//! in `poll` without a timeout, so it costs no CPU until something happens.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, Read};
use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Context;
use evdev::{EventType, InputEvent, KeyCode, RelativeAxisCode};
use middle_scroll_core::{CoreAction, CoreConfig, CoreInputEvent, Engine, EngineState};
use nix::errno::Errno;
use nix::poll::{PollFd, PollFlags, PollTimeout};
use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};
use serde_json::json;
use tracing::{debug, info, warn};

use crate::cli::Args;
use crate::config_loader::{self, CoreFileConfig, ParsedDeviceMatch, Profile, ResolvedConfig};
use crate::control::{self, Command, ControlServer};
use crate::device_discovery::{self, MatchCriteria};
use crate::errors::DaemonError;
use crate::event_router::{self, Routed, Router};
use crate::foreground::{AutoscrollDecision, ForegroundGate, ProfileRule};
use crate::hooks::{HookEvent, Hooks};
use crate::physical_mouse::PhysicalMouse;
use crate::virtual_mouse::{VirtualMouse, BASE_AXES, BASE_KEYS};

/// Time given to the compositor to pick up a new virtual device before
/// events are written to it.
const COMPOSITOR_SETTLE_DELAY: Duration = Duration::from_millis(200);
/// How often a missing mouse is looked for.
const DEVICE_RETRY_INTERVAL: Duration = Duration::from_millis(500);
/// How long an interactive start waits for the configured mouse.
const STARTUP_DEVICE_TIMEOUT: Duration = Duration::from_secs(10);
/// Editors often write a file in several steps; wait for the last one.
const RELOAD_DEBOUNCE: Duration = Duration::from_millis(300);

/// Runs the daemon until a termination signal or the safety timeout.
pub fn run(args: &Args, cfg: ResolvedConfig) -> anyhow::Result<()> {
    let signals = Signals::install().context("failed to install the signal handlers")?;

    let specs = slot_specs(&cfg);
    if specs.is_empty() {
        return Err(DaemonError::NoDevice.into());
    }
    let grab = cfg.grab && !cfg.dry_run;
    if cfg.grab && cfg.dry_run {
        warn!("--dry-run skips grab to avoid silently capturing the mouse");
    } else if !cfg.grab {
        warn!("running without grab; the compositor will see both physical and virtual events");
    }
    info!(
        devices = specs.len(),
        grab,
        dry_run = cfg.dry_run,
        trigger = ?cfg.trigger_button,
        profiles = cfg.profiles.len(),
        "starting daemon"
    );

    // Mice are opened without grabbing first: the virtual mouse needs their
    // capabilities, and grabbing before it exists would leave the pointer
    // dead while the compositor picks it up.
    let mut slots: Vec<Slot> = specs.into_iter().map(Slot::new).collect();
    connect_missing(&mut slots, false, true)?;
    if slots.iter().any(Slot::missing) {
        if args.no_interactive {
            for slot in slots.iter().filter(|s| s.missing()) {
                warn!(device = %slot.label(), "mouse not found; it will be used as soon as it is plugged in");
            }
        } else {
            info!("waiting for the configured mouse to appear");
            let deadline = Instant::now() + STARTUP_DEVICE_TIMEOUT;
            while slots.iter().any(Slot::missing) && Instant::now() < deadline {
                if signals.term.load(Ordering::Relaxed) {
                    return Ok(());
                }
                std::thread::sleep(DEVICE_RETRY_INTERVAL);
                connect_missing(&mut slots, false, true)?;
            }
            if slots.iter().all(Slot::missing) {
                return Err(match &slots[0].spec {
                    SlotSpec::Match(m) => DaemonError::DeviceMatchNotFound {
                        vendor_id: m.vendor_id,
                        product_id: m.product_id,
                    },
                    SlotSpec::Path(path) => DaemonError::DeviceNotFound(path.clone()),
                }
                .into());
            }
            for slot in slots.iter().filter(|s| s.missing()) {
                warn!(device = %slot.label(), "mouse not found; it will be used as soon as it is plugged in");
            }
        }
    }

    let (keys, axes) = capabilities(&slots);
    let output = Output::new(cfg.dry_run, cfg.trigger_button.key_code(), keys, axes)?;
    if grab {
        for mouse in slots.iter_mut().filter_map(|s| s.mouse.as_mut()) {
            mouse.grab()?;
            debug!(path = %mouse.path().display(), "grabbed mouse");
        }
    }

    let gate = ForegroundGate::new(cfg.foreground.clone(), profile_rules(&cfg.profiles));
    let mut daemon = Daemon::new(args, cfg, grab, slots, output, signals, gate);
    daemon.sync_services();
    daemon.run_loop()
}

// ---------------------------------------------------------------------------
// Signals

/// Termination and reload requests. Signal handlers only set a flag and write
/// a byte to the wake pipe, which interrupts `poll`.
#[derive(Debug)]
struct Signals {
    term: Arc<AtomicBool>,
    hup: Arc<AtomicBool>,
    wake_rx: UnixStream,
    wake_tx: UnixStream,
}

impl Signals {
    fn install() -> io::Result<Self> {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};

        let (wake_rx, wake_tx) = UnixStream::pair()?;
        wake_rx.set_nonblocking(true)?;
        wake_tx.set_nonblocking(true)?;
        let term = Arc::new(AtomicBool::new(false));
        let hup = Arc::new(AtomicBool::new(false));
        for signal in [SIGTERM, SIGINT, SIGQUIT] {
            // A second signal while the first is being handled kills the
            // process, so a stuck daemon can always be stopped.
            signal_hook::flag::register_conditional_shutdown(signal, 1, Arc::clone(&term))?;
            signal_hook::flag::register(signal, Arc::clone(&term))?;
            signal_hook::low_level::pipe::register(signal, wake_tx.try_clone()?)?;
        }
        signal_hook::flag::register(SIGHUP, Arc::clone(&hup))?;
        signal_hook::low_level::pipe::register(SIGHUP, wake_tx.try_clone()?)?;
        Ok(Self {
            term,
            hup,
            wake_rx,
            wake_tx,
        })
    }

    #[cfg(test)]
    fn for_test() -> Self {
        let (wake_rx, wake_tx) = UnixStream::pair().expect("socket pair");
        wake_rx.set_nonblocking(true).expect("non-blocking");
        wake_tx.set_nonblocking(true).expect("non-blocking");
        Self {
            term: Arc::new(AtomicBool::new(false)),
            hup: Arc::new(AtomicBool::new(false)),
            wake_rx,
            wake_tx,
        }
    }

    /// Empties the wake pipe.
    fn drain(&self) {
        let mut buf = [0_u8; 64];
        while matches!((&self.wake_rx).read(&mut buf), Ok(n) if n > 0) {}
    }
}

// ---------------------------------------------------------------------------
// Physical mice

/// How a configured mouse is found again after it was unplugged.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReconnectMatch {
    vendor_id: u16,
    product_id: u16,
    name: Option<String>,
    phys: Option<String>,
}

impl ReconnectMatch {
    fn from_match(m: &ParsedDeviceMatch) -> Self {
        Self {
            vendor_id: m.vendor_id,
            product_id: m.product_id,
            name: m.name.clone(),
            phys: m.phys.clone(),
        }
    }

    /// Criteria learnt from an opened device, so a mouse configured by path
    /// is found again on another port. `phys` is left out on purpose.
    fn from_device(mouse: &PhysicalMouse) -> Self {
        Self {
            vendor_id: mouse.vendor_id(),
            product_id: mouse.product_id(),
            name: Some(mouse.name().to_owned()).filter(|name| name != "(unknown)"),
            phys: None,
        }
    }

    fn criteria(&self) -> MatchCriteria<'_> {
        MatchCriteria {
            vendor_id: self.vendor_id,
            product_id: self.product_id,
            name: self.name.as_deref(),
            phys: self.phys.as_deref(),
        }
    }

    fn human_id(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }
}

/// Tries a strict match first (honouring `phys` when configured), then one
/// that ignores `phys`, so a mouse moved to another USB port is still found.
fn try_match(m: &ReconnectMatch, exclude: &[PathBuf]) -> Option<PathBuf> {
    let criteria = m.criteria();
    if let Some(found) = device_discovery::find_match(&criteria, exclude) {
        return Some(found.path);
    }
    if criteria.phys.is_some() {
        if let Some(found) = device_discovery::find_match(&criteria.without_phys(), exclude) {
            warn!(
                usb_id = %m.human_id(),
                event = %found.path.display(),
                found_phys = ?found.phys,
                "mouse found on a different USB port than the one pinned in config; matching by USB id instead (re-run `wayland-wheeltani --setup` to clear the pinned port)"
            );
            return Some(found.path);
        }
    }
    None
}

/// One configured mouse.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SlotSpec {
    /// `--device` or the legacy `device =` key.
    Path(PathBuf),
    /// A `[[device_match]]` entry.
    Match(ParsedDeviceMatch),
}

#[derive(Debug)]
struct Slot {
    spec: SlotSpec,
    reconnect: Option<ReconnectMatch>,
    mouse: Option<PhysicalMouse>,
    /// Last failure to open the mouse, so a retry loop logs each new error
    /// once instead of every half second.
    last_error: Option<String>,
}

impl Slot {
    fn new(spec: SlotSpec) -> Self {
        let reconnect = match &spec {
            SlotSpec::Match(m) => Some(ReconnectMatch::from_match(m)),
            SlotSpec::Path(_) => None,
        };
        Self {
            spec,
            reconnect,
            mouse: None,
            last_error: None,
        }
    }

    fn label(&self) -> String {
        match &self.spec {
            SlotSpec::Path(path) => path.display().to_string(),
            SlotSpec::Match(m) => m.human_id(),
        }
    }

    /// Slots pinned to a port are matched first, so an unpinned slot for the
    /// same model cannot take their mouse.
    const fn pinned(&self) -> bool {
        matches!(
            self.spec,
            SlotSpec::Match(ParsedDeviceMatch { phys: Some(_), .. })
        )
    }

    const fn missing(&self) -> bool {
        self.mouse.is_none()
    }

    fn locate(&self, exclude: &[PathBuf]) -> Option<PathBuf> {
        if let Some(m) = &self.reconnect {
            return try_match(m, exclude);
        }
        match &self.spec {
            SlotSpec::Path(path) if path.exists() && !exclude.contains(path) => Some(path.clone()),
            _ => None,
        }
    }
}

fn slot_specs(cfg: &ResolvedConfig) -> Vec<SlotSpec> {
    if cfg.device_matches.is_empty() {
        cfg.device.iter().cloned().map(SlotSpec::Path).collect()
    } else {
        cfg.device_matches
            .iter()
            .cloned()
            .map(SlotSpec::Match)
            .collect()
    }
}

fn open_mouse(path: &Path, grab: bool) -> Result<PhysicalMouse, DaemonError> {
    let mut mouse = PhysicalMouse::open(path)?;
    if grab {
        mouse.grab()?;
    }
    Ok(mouse)
}

/// Opens every missing mouse that can be found. Returns the indices of the
/// slots that were connected. With `fatal`, the first error is returned;
/// otherwise it is logged and the slot is retried later.
fn connect_missing(slots: &mut [Slot], grab: bool, fatal: bool) -> Result<Vec<usize>, DaemonError> {
    let mut order: Vec<usize> = (0..slots.len()).filter(|&i| slots[i].missing()).collect();
    order.sort_by_key(|&i| !slots[i].pinned());
    let mut connected = Vec::new();
    for index in order {
        let open: Vec<PathBuf> = slots
            .iter()
            .filter_map(|s| s.mouse.as_ref().map(|m| m.path().to_path_buf()))
            .collect();
        let slot = &mut slots[index];
        let Some(path) = slot.locate(&open) else {
            continue;
        };
        match open_mouse(&path, grab) {
            Ok(mouse) => {
                if slot.reconnect.is_none() {
                    slot.reconnect = Some(ReconnectMatch::from_device(&mouse));
                }
                info!(
                    device = %slot.label(),
                    path = %path.display(),
                    name = mouse.name(),
                    "opened mouse"
                );
                slot.mouse = Some(mouse);
                slot.last_error = None;
                connected.push(index);
            }
            Err(err) if fatal => return Err(err),
            Err(err) => {
                let message = err.to_string();
                if slot.last_error.as_deref() != Some(message.as_str()) {
                    warn!(device = %slot.label(), error = %message, "cannot open the mouse yet; retrying");
                    slot.last_error = Some(message);
                }
            }
        }
    }
    Ok(connected)
}

/// Buttons and axes of the open mice that the virtual mouse does not have
/// by default.
fn capabilities(slots: &[Slot]) -> (Vec<KeyCode>, Vec<RelativeAxisCode>) {
    let mut keys = Vec::new();
    let mut axes = Vec::new();
    for mouse in slots.iter().filter_map(|s| s.mouse.as_ref()) {
        for key in mouse.supported_keys() {
            if !BASE_KEYS.contains(&key) && !keys.contains(&key) {
                keys.push(key);
            }
        }
        for axis in mouse.supported_axes() {
            if !BASE_AXES.contains(&axis) && !axes.contains(&axis) {
                axes.push(axis);
            }
        }
    }
    (keys, axes)
}

// ---------------------------------------------------------------------------
// Output

#[derive(Debug)]
enum Sink {
    Virtual(VirtualMouse),
    /// `--dry-run`: actions are logged instead of emitted.
    DryRun,
    #[cfg(test)]
    Recorder(Vec<Recorded>),
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum Recorded {
    Action(CoreAction),
    Raw { kind: u16, code: u16, value: i32 },
}

/// The virtual mouse, plus what is needed to rebuild it when a mouse with
/// more buttons is plugged in.
#[derive(Debug)]
struct Output {
    sink: Sink,
    trigger: KeyCode,
    synthesize_detents: bool,
    /// Extra keys and axes advertised on top of the base set.
    keys: Vec<KeyCode>,
    axes: Vec<RelativeAxisCode>,
    write_failures: u64,
}

impl Output {
    fn new(
        dry_run: bool,
        trigger: KeyCode,
        keys: Vec<KeyCode>,
        axes: Vec<RelativeAxisCode>,
    ) -> Result<Self, DaemonError> {
        let sink = if dry_run {
            Sink::DryRun
        } else {
            let mouse = VirtualMouse::new(trigger, &keys, &axes)?;
            std::thread::sleep(COMPOSITOR_SETTLE_DELAY);
            Sink::Virtual(mouse)
        };
        Ok(Self {
            sink,
            trigger,
            synthesize_detents: false,
            keys,
            axes,
            write_failures: 0,
        })
    }

    #[cfg(test)]
    const fn recorder(trigger: KeyCode) -> Self {
        Self {
            sink: Sink::Recorder(Vec::new()),
            trigger,
            synthesize_detents: false,
            keys: Vec::new(),
            axes: Vec::new(),
            write_failures: 0,
        }
    }

    #[cfg(test)]
    fn recorded(&self) -> &[Recorded] {
        match &self.sink {
            Sink::Recorder(log) => log,
            _ => &[],
        }
    }

    fn push_action(&mut self, action: &CoreAction) {
        let result = match &mut self.sink {
            Sink::Virtual(mouse) => mouse.push_action(action),
            Sink::DryRun => {
                if let Some(line) = event_router::dry_run_describe(action) {
                    if matches!(
                        action,
                        CoreAction::ForwardMotion { .. }
                            | CoreAction::ForwardWheel { .. }
                            | CoreAction::ForwardMouseButton { .. }
                    ) {
                        debug!(action = %line, "dry-run");
                    } else {
                        info!(action = %line, "dry-run");
                    }
                }
                Ok(())
            }
            #[cfg(test)]
            Sink::Recorder(log) => {
                log.push(Recorded::Action(action.clone()));
                Ok(())
            }
        };
        self.report(result);
    }

    fn push_raw(&mut self, event: InputEvent) {
        let result = match &mut self.sink {
            Sink::Virtual(mouse) => mouse.push_raw(event),
            Sink::DryRun => {
                debug!(
                    kind = event.event_type().0,
                    code = event.code(),
                    "dry-run: forward raw event"
                );
                Ok(())
            }
            #[cfg(test)]
            Sink::Recorder(log) => {
                log.push(Recorded::Raw {
                    kind: event.event_type().0,
                    code: event.code(),
                    value: event.value(),
                });
                Ok(())
            }
        };
        self.report(result);
    }

    fn flush(&mut self) {
        if let Sink::Virtual(mouse) = &mut self.sink {
            let result = mouse.flush();
            self.report(result);
        }
    }

    fn release_all_buttons(&mut self) {
        if let Sink::Virtual(mouse) = &mut self.sink {
            let result = mouse.release_all_buttons();
            self.report(result);
        }
    }

    const fn set_trigger(&mut self, trigger: KeyCode) {
        self.trigger = trigger;
        if let Sink::Virtual(mouse) = &mut self.sink {
            mouse.set_trigger(trigger);
        }
    }

    const fn set_synthesize_detents(&mut self, on: bool) {
        if self.synthesize_detents == on {
            return;
        }
        self.synthesize_detents = on;
        if let Sink::Virtual(mouse) = &mut self.sink {
            mouse.set_synthesize_detents(on);
        }
    }

    /// Adds the keys and axes the virtual mouse is missing. The device has
    /// to be recreated for that; returns true when it was.
    fn extend_capabilities(
        &mut self,
        keys: &[KeyCode],
        axes: &[RelativeAxisCode],
    ) -> Result<bool, DaemonError> {
        let new_keys: Vec<KeyCode> = keys
            .iter()
            .copied()
            .filter(|k| !BASE_KEYS.contains(k) && !self.keys.contains(k))
            .collect();
        let new_axes: Vec<RelativeAxisCode> = axes
            .iter()
            .copied()
            .filter(|a| !BASE_AXES.contains(a) && !self.axes.contains(a))
            .collect();
        if new_keys.is_empty() && new_axes.is_empty() {
            return Ok(false);
        }
        let mut all_keys = self.keys.clone();
        all_keys.extend(new_keys);
        let mut all_axes = self.axes.clone();
        all_axes.extend(new_axes);
        let recreated = if matches!(self.sink, Sink::Virtual(_)) {
            let mut mouse = VirtualMouse::new(self.trigger, &all_keys, &all_axes)?;
            mouse.set_synthesize_detents(self.synthesize_detents);
            // Dropping the old device makes the kernel release whatever it
            // still had pressed.
            self.sink = Sink::Virtual(mouse);
            std::thread::sleep(COMPOSITOR_SETTLE_DELAY);
            true
        } else {
            false
        };
        self.keys = all_keys;
        self.axes = all_axes;
        Ok(recreated)
    }

    /// Logs write failures without flooding the journal: the first one, then
    /// every thousandth.
    fn report(&mut self, result: io::Result<()>) {
        if let Err(err) = result {
            self.write_failures += 1;
            if self.write_failures == 1 || self.write_failures % 1000 == 0 {
                warn!(%err, failures = self.write_failures, "failed to write to the virtual mouse");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Config file watcher

/// Watches the directory of the config file: editors usually replace the
/// file (rename) rather than write it in place, which a watch on the file
/// itself would miss.
#[derive(Debug)]
struct ConfigWatcher {
    inotify: Inotify,
    file_name: OsString,
}

impl ConfigWatcher {
    fn new(path: &Path) -> Result<Self, String> {
        let file_name = path
            .file_name()
            .ok_or_else(|| format!("{} has no file name", path.display()))?
            .to_owned();
        let dir = match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        };
        let inotify = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC)
            .map_err(|err| err.to_string())?;
        inotify
            .add_watch(
                dir,
                AddWatchFlags::IN_CLOSE_WRITE | AddWatchFlags::IN_MOVED_TO,
            )
            .map_err(|err| format!("{}: {err}", dir.display()))?;
        Ok(Self { inotify, file_name })
    }

    /// Consumes the pending events; true when one concerns the config file.
    fn config_changed(&self) -> bool {
        let mut changed = false;
        loop {
            match self.inotify.read_events() {
                Ok(events) if !events.is_empty() => {
                    changed |= events.iter().any(|event| {
                        event.mask.contains(AddWatchFlags::IN_Q_OVERFLOW)
                            || event.name.as_deref() == Some(self.file_name.as_os_str())
                    });
                }
                Ok(_) | Err(Errno::EAGAIN) => break,
                Err(err) => {
                    warn!(%err, "failed to read config file events");
                    break;
                }
            }
        }
        changed
    }
}

// ---------------------------------------------------------------------------
// The daemon

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Wake,
    Watcher,
    Mouse(usize),
}

fn profile_rules(profiles: &[Profile]) -> Vec<ProfileRule> {
    profiles
        .iter()
        .map(|p| ProfileRule {
            apps: p.apps.clone(),
            match_title: p.match_title,
        })
        .collect()
}

const fn engine_state_name(state: EngineState) -> &'static str {
    match state {
        EngineState::Idle => "idle",
        EngineState::MiddlePending => "pending",
        EngineState::Scrolling => "scrolling",
        EngineState::Coasting => "coasting",
    }
}

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)]
struct Daemon<'a> {
    args: &'a Args,
    cfg: ResolvedConfig,
    /// Whether the mice are grabbed; fixed for the life of the process.
    grab: bool,
    started: Instant,

    engine: Engine,
    router: Router,
    gate: ForegroundGate,
    hooks: Hooks,
    output: Output,
    slots: Vec<Slot>,

    /// Index in `cfg.profiles` of the profile the engine runs with.
    active_profile: Option<usize>,
    /// Engine keys changed with `set` and not saved yet.
    tuned_keys: Vec<String>,
    paused: bool,
    /// The trigger's release must be dropped: its press was swallowed by a
    /// gesture that was aborted while the button was held.
    swallow_trigger_up: bool,
    /// A button whose press ended latched scrolling: its release is dropped.
    swallow_key_up: Option<u16>,

    last_state: EngineState,
    last_tick: Instant,
    next_retry: Instant,
    reload_at: Option<Instant>,
    actions: Vec<CoreAction>,
    events: Vec<InputEvent>,

    signals: Signals,
    control: Option<ControlServer>,
    watcher: Option<ConfigWatcher>,
    /// The control socket or the watcher must be started or stopped. Done
    /// from the loop, never while answering a control request: stopping the
    /// server waits for the thread that is waiting for that answer.
    services_dirty: bool,
}

impl<'a> Daemon<'a> {
    fn new(
        args: &'a Args,
        cfg: ResolvedConfig,
        grab: bool,
        slots: Vec<Slot>,
        output: Output,
        signals: Signals,
        gate: ForegroundGate,
    ) -> Self {
        let now = Instant::now();
        let mut daemon = Self {
            args,
            grab,
            started: now,
            engine: Engine::new(cfg.core.clone()),
            router: Router::new(cfg.trigger_button),
            gate,
            hooks: Hooks::new(cfg.hooks.clone()),
            output,
            slots,
            active_profile: None,
            tuned_keys: Vec::new(),
            paused: false,
            swallow_trigger_up: false,
            swallow_key_up: None,
            last_state: EngineState::Idle,
            last_tick: now,
            next_retry: now,
            reload_at: None,
            actions: Vec::with_capacity(16),
            events: Vec::with_capacity(64),
            signals,
            control: None,
            watcher: None,
            services_dirty: false,
            cfg,
        };
        daemon.apply_core();
        daemon
    }

    // `drain` keeps the buffer's allocation for the next iteration.
    #[allow(clippy::iter_with_drain)]
    fn run_loop(&mut self) -> anyhow::Result<()> {
        let mut ready = Vec::new();
        loop {
            if self.signals.term.load(Ordering::Relaxed) {
                info!("termination signal received");
                break;
            }
            let now = Instant::now();
            if self
                .safety_deadline()
                .is_some_and(|deadline| now >= deadline)
            {
                warn!("safety timeout reached; exiting");
                break;
            }
            if self.signals.hup.swap(false, Ordering::Relaxed) {
                info!("SIGHUP received; reloading the config");
                self.reload_logged();
            }
            if self.reload_at.is_some_and(|at| now >= at) {
                self.reload_at = None;
                info!("config file changed; reloading");
                self.reload_logged();
            }
            self.serve_control();
            if self.services_dirty {
                self.sync_services();
            }
            if self.slots.iter().any(Slot::missing) && now >= self.next_retry {
                self.reconnect();
            }

            self.wait(&mut ready)?;
            for (source, revents) in ready.drain(..) {
                match source {
                    Source::Wake => self.signals.drain(),
                    Source::Watcher => {
                        if self
                            .watcher
                            .as_ref()
                            .is_some_and(ConfigWatcher::config_changed)
                        {
                            self.reload_at = Some(Instant::now() + RELOAD_DEBOUNCE);
                        }
                    }
                    Source::Mouse(index) => {
                        if revents.intersects(PollFlags::POLLIN) {
                            self.read_mouse(index);
                        }
                        if revents.intersects(
                            PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL,
                        ) {
                            self.disconnect(index, "device closed");
                        }
                    }
                }
            }
            self.tick_if_due();
        }
        self.shutdown();
        Ok(())
    }

    fn shutdown(&mut self) {
        self.abort_gesture();
        self.output.flush();
        // Removes the socket file.
        self.control = None;
        info!("shutting down");
    }

    fn safety_deadline(&self) -> Option<Instant> {
        self.cfg
            .safety_timeout_seconds
            .and_then(|secs| self.started.checked_add(Duration::from_secs(secs)))
    }

    fn tick_period(&self) -> Duration {
        Duration::from_micros(1_000_000 / u64::from(self.engine.config().tick_hz.max(1)))
    }

    /// How long `poll` may sleep: until the next engine tick, mouse retry,
    /// pending reload or the safety timeout, and forever when none applies.
    fn poll_timeout(&self, now: Instant) -> PollTimeout {
        let mut deadline: Option<Instant> = None;
        let mut consider = |at: Instant| {
            deadline = Some(deadline.map_or(at, |current| current.min(at)));
        };
        if self.engine.state().needs_ticks() {
            consider(self.last_tick + self.tick_period());
        }
        if self.slots.iter().any(Slot::missing) {
            consider(self.next_retry);
        }
        if let Some(at) = self.safety_deadline() {
            consider(at);
        }
        if let Some(at) = self.reload_at {
            consider(at);
        }
        deadline.map_or(PollTimeout::NONE, |at| {
            PollTimeout::from(poll_timeout_millis(at.saturating_duration_since(now)))
        })
    }

    fn wait(&self, ready: &mut Vec<(Source, PollFlags)>) -> anyhow::Result<()> {
        let timeout = self.poll_timeout(Instant::now());
        let mut sources = Vec::with_capacity(self.slots.len() + 2);
        let mut fds = Vec::with_capacity(self.slots.len() + 2);
        sources.push(Source::Wake);
        fds.push(PollFd::new(self.signals.wake_rx.as_fd(), PollFlags::POLLIN));
        if let Some(watcher) = &self.watcher {
            sources.push(Source::Watcher);
            fds.push(PollFd::new(watcher.inotify.as_fd(), PollFlags::POLLIN));
        }
        for (index, slot) in self.slots.iter().enumerate() {
            if let Some(mouse) = &slot.mouse {
                sources.push(Source::Mouse(index));
                fds.push(PollFd::new(mouse.as_fd(), PollFlags::POLLIN));
            }
        }
        match nix::poll::poll(&mut fds, timeout) {
            Ok(_) | Err(Errno::EINTR) => {}
            Err(err) => return Err(anyhow::Error::new(err).context("poll failed")),
        }
        for (fd, source) in fds.iter().zip(sources) {
            if let Some(revents) = fd.revents() {
                if !revents.is_empty() {
                    ready.push((source, revents));
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::iter_with_drain)]
    fn read_mouse(&mut self, index: usize) {
        let Some(mouse) = self.slots[index].mouse.as_mut() else {
            return;
        };
        let result = mouse
            .fetch_events()
            .map(|events| self.events.extend(events));
        let mut events = std::mem::take(&mut self.events);
        for event in events.drain(..) {
            self.handle_event(event);
        }
        self.events = events;
        self.output.flush();
        match result {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => self.disconnect(index, &err.to_string()),
        }
    }

    fn handle_event(&mut self, event: InputEvent) {
        match self.router.classify(&event) {
            Routed::Core(core) => self.handle_core(core),
            Routed::Raw(raw) => self.handle_raw(raw),
            Routed::Sync => self.output.flush(),
            Routed::Ignore => {}
        }
    }

    fn handle_core(&mut self, event: CoreInputEvent) {
        match event {
            CoreInputEvent::MiddleUp if self.swallow_trigger_up => {
                self.swallow_trigger_up = false;
                return;
            }
            CoreInputEvent::MiddleDown => self.swallow_trigger_up = false,
            _ => {}
        }
        if self.paused {
            self.passthrough(event);
            return;
        }
        if self
            .gate
            .latched()
            .is_some_and(|latched| latched.decision == AutoscrollDecision::Disabled)
        {
            self.passthrough(event);
            if event == CoreInputEvent::MiddleUp {
                self.gate.end_gesture();
            }
            return;
        }
        if event == CoreInputEvent::MiddleDown
            && !self.engine.is_latched()
            && matches!(
                self.engine.state(),
                EngineState::Idle | EngineState::Coasting
            )
        {
            let decision = self.gate.begin_gesture();
            self.select_profile(decision.profile);
            if decision.decision == AutoscrollDecision::Disabled {
                if self.engine.state() == EngineState::Coasting {
                    self.engine.abort(&mut self.actions);
                    self.emit_actions();
                }
                self.passthrough(event);
                return;
            }
        }
        self.engine.process_into(event, &mut self.actions);
        self.emit_actions();
        if event == CoreInputEvent::MiddleUp {
            self.gate.end_gesture();
        }
    }

    fn handle_raw(&mut self, event: InputEvent) {
        if event.event_type() == EventType::KEY {
            let code = event.code();
            let pressed = event.value() != 0;
            if self.swallow_key_up == Some(code) {
                self.swallow_key_up = None;
                if !pressed {
                    return;
                }
            }
            if pressed && !self.paused {
                if self.engine.is_latched() {
                    // In toggle mode any button ends scroll mode, and that
                    // press and its release never reach the application, like
                    // the left and right buttons in the engine.
                    self.abort_gesture();
                    self.swallow_key_up = Some(code);
                    return;
                }
                if self.engine.state() == EngineState::Coasting {
                    self.abort_gesture();
                }
            }
        }
        self.output.push_raw(event);
    }

    fn passthrough(&mut self, event: CoreInputEvent) {
        if let Some(action) = event_router::passthrough_action(event) {
            self.output.push_action(&action);
        }
    }

    /// Sends the engine's actions to the output and fires the hooks.
    fn emit_actions(&mut self) {
        self.output
            .set_synthesize_detents(!self.engine.config().emit_hires_wheel);
        for action in self.actions.drain(..) {
            match action {
                CoreAction::Suppress => continue,
                CoreAction::EnterScrollMode => self.hooks.fire(HookEvent::Enter),
                CoreAction::ExitScrollMode => self.hooks.fire(HookEvent::Exit),
                _ => {}
            }
            self.output.push_action(&action);
        }
        self.note_state();
    }

    fn note_state(&mut self) {
        let state = self.engine.state();
        if state == self.last_state {
            return;
        }
        debug!(from = ?self.last_state, to = ?state, "engine state");
        if state.needs_ticks() && !self.last_state.needs_ticks() {
            self.last_tick = Instant::now();
        }
        self.last_state = state;
    }

    fn tick_if_due(&mut self) {
        if !self.engine.state().needs_ticks() {
            return;
        }
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_tick);
        if elapsed < self.tick_period() {
            return;
        }
        self.last_tick = now;
        let dt_micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.engine
            .process_into(CoreInputEvent::Tick { dt_micros }, &mut self.actions);
        self.emit_actions();
        self.output.flush();
    }

    /// Cancels the gesture in progress. If the trigger is still held, its
    /// release is dropped later, so the compositor never sees a release
    /// without a press.
    fn abort_gesture(&mut self) {
        let trigger_held = match self.engine.state() {
            EngineState::MiddlePending => true,
            EngineState::Scrolling => !self.engine.is_latched(),
            EngineState::Idle | EngineState::Coasting => false,
        };
        self.engine.abort(&mut self.actions);
        self.emit_actions();
        if trigger_held {
            self.swallow_trigger_up = true;
        }
    }

    fn set_paused(&mut self, paused: bool) {
        if paused == self.paused {
            return;
        }
        if paused {
            self.abort_gesture();
            self.output.flush();
            self.gate.reset_latch();
            self.paused = true;
            self.hooks.fire(HookEvent::Pause);
            info!("autoscroll paused");
        } else {
            self.paused = false;
            self.hooks.fire(HookEvent::Resume);
            info!("autoscroll resumed");
        }
    }

    fn disconnect(&mut self, index: usize, reason: &str) {
        let Some(mouse) = self.slots[index].mouse.take() else {
            return;
        };
        warn!(
            device = %self.slots[index].label(),
            path = %mouse.path().display(),
            reason,
            "mouse disconnected; waiting for it to come back"
        );
        drop(mouse);
        // Nothing may stay half done across the gap: no scroll mode, no
        // pending swallow, no button held down on the virtual mouse.
        self.abort_gesture();
        self.swallow_trigger_up = false;
        self.swallow_key_up = None;
        self.gate.reset_latch();
        self.output.release_all_buttons();
        self.next_retry = Instant::now() + DEVICE_RETRY_INTERVAL;
    }

    fn reconnect(&mut self) {
        self.next_retry = Instant::now() + DEVICE_RETRY_INTERVAL;
        let connected = connect_missing(&mut self.slots, self.grab, false).unwrap_or_default();
        if !connected.is_empty() {
            self.refresh_capabilities();
        }
    }

    fn refresh_capabilities(&mut self) {
        let (keys, axes) = capabilities(&self.slots);
        match self.output.extend_capabilities(&keys, &axes) {
            Ok(true) => info!("recreated the virtual mouse for the extra buttons of the new mouse"),
            Ok(false) => {}
            Err(err) => {
                warn!(%err, "cannot extend the virtual mouse; the new mouse's extra buttons are ignored");
            }
        }
    }

    fn current_core(&self) -> &CoreConfig {
        self.active_profile
            .and_then(|index| self.cfg.profiles.get(index))
            .map_or(&self.cfg.core, |profile| &profile.core)
    }

    /// Hands the settings of the active profile (or the top-level ones) to
    /// the engine. A gesture in progress keeps its settings until it ends.
    fn apply_core(&mut self) {
        let core = self.current_core().clone();
        self.engine.set_config(core);
    }

    fn select_profile(&mut self, profile: Option<usize>) {
        let profile = profile.filter(|&index| index < self.cfg.profiles.len());
        if profile == self.active_profile {
            return;
        }
        if let Some(index) = profile {
            info!(profile = %self.cfg.profiles[index].name, "profile applied");
        } else {
            info!("default settings applied");
        }
        self.active_profile = profile;
        self.apply_core();
    }

    // -- services -----------------------------------------------------------

    fn sync_services(&mut self) {
        self.services_dirty = false;

        if self.cfg.control_socket {
            if self.control.is_none() {
                self.start_control();
            }
        } else if self.control.take().is_some() {
            info!("control socket closed");
        }

        let watch = self
            .cfg
            .config_path
            .as_deref()
            .filter(|_| self.cfg.watch_config);
        match (watch, self.watcher.is_some()) {
            (Some(path), false) => match ConfigWatcher::new(path) {
                Ok(watcher) => {
                    debug!(path = %path.display(), "watching the config file");
                    self.watcher = Some(watcher);
                }
                Err(err) => {
                    info!(%err, "not watching the config file; send SIGHUP or run --reload after editing it");
                }
            },
            (None, true) => self.watcher = None,
            _ => {}
        }
    }

    fn start_control(&mut self) {
        let Some(path) = control::socket_path() else {
            warn!("no runtime directory ($XDG_RUNTIME_DIR); the control socket is disabled");
            return;
        };
        let waker = match self.signals.wake_tx.try_clone() {
            Ok(waker) => waker,
            Err(err) => {
                warn!(%err, "cannot start the control socket");
                return;
            }
        };
        match ControlServer::start(path, waker) {
            Ok(server) => {
                info!(path = %server.path().display(), "control socket ready");
                self.control = Some(server);
            }
            Err(err) => {
                warn!(%err, "control socket unavailable; --status, --pause and --tune will not work");
            }
        }
    }

    fn serve_control(&mut self) {
        while let Some(request) = self.control.as_ref().and_then(ControlServer::try_next) {
            let reply = match Command::parse(&request.line) {
                Ok(command) => self.execute(command),
                Err(err) => format!("error: {err}"),
            };
            request.respond(reply);
        }
    }

    fn execute(&mut self, command: Command) -> String {
        match command {
            Command::Ping => "ok pong".to_owned(),
            Command::Status => match serde_json::to_string_pretty(&self.status()) {
                Ok(text) => format!("ok {text}"),
                Err(err) => format!("error: {err}"),
            },
            Command::Pause => {
                self.set_paused(true);
                "ok paused".to_owned()
            }
            Command::Resume => {
                self.set_paused(false);
                "ok active".to_owned()
            }
            Command::Toggle => {
                self.set_paused(!self.paused);
                if self.paused {
                    "ok paused"
                } else {
                    "ok active"
                }
                .to_owned()
            }
            Command::Reload => match self.reload() {
                Ok(()) => "ok reloaded".to_owned(),
                Err(err) => format!("error: {err}"),
            },
            Command::Get(key) => self.get(key.as_deref()),
            Command::Set { key, value } => match self.set(&key, &value) {
                Ok(reply) | Err(reply) => reply,
            },
            Command::Save => self.save(),
        }
    }

    fn status(&self) -> serde_json::Value {
        let devices: Vec<serde_json::Value> = self
            .slots
            .iter()
            .map(|slot| {
                json!({
                    "device": slot.label(),
                    "connected": slot.mouse.is_some(),
                    "path": slot.mouse.as_ref().map(|m| m.path().display().to_string()),
                    "name": slot.mouse.as_ref().map(|m| m.name().to_owned()),
                })
            })
            .collect();
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "state": if self.paused { "paused" } else { "active" },
            "engine": engine_state_name(self.engine.state()),
            "profile": self
                .active_profile
                .and_then(|index| self.cfg.profiles.get(index))
                .map(|profile| profile.name.clone()),
            "profiles": self.cfg.profiles.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
            "trigger_button": self.cfg.trigger_button,
            "mode": self.engine.config().mode,
            "foreground_filter": self.cfg.foreground.enabled,
            "grab": self.grab,
            "dry_run": self.cfg.dry_run,
            "devices": devices,
            "config": self.cfg.config_path.as_ref().map(|p| p.display().to_string()),
            "unsaved_settings": self.tuned_keys,
        })
    }

    fn get(&self, key: Option<&str>) -> String {
        let table = match toml::Table::try_from(&self.cfg.core) {
            Ok(table) => table,
            Err(err) => return format!("error: {err}"),
        };
        if let Some(key) = key {
            return table.get(key).map_or_else(
                || format!("error: unknown setting `{key}`"),
                |value| format!("ok {key} = {value}"),
            );
        }
        let mut text = match toml::to_string(&table) {
            Ok(text) => text,
            Err(err) => return format!("error: {err}"),
        };
        if let Some(profile) = self.active_profile.and_then(|i| self.cfg.profiles.get(i)) {
            let overridden = toml::Table::try_from(&profile.overrides)
                .map(|t| t.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
            let _ = writeln!(
                text,
                "# profile `{}` is active and overrides: {overridden}",
                profile.name
            );
        }
        format!("ok\n{}", text.trim_end())
    }

    /// Changes one engine setting live. Profiles are rebased on the new
    /// value; nothing changes when the value is invalid.
    fn set(&mut self, key: &str, value: &str) -> Result<String, String> {
        let fail = |reason: String| format!("error: {reason}");
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(fail(format!("invalid setting name `{key}`")));
        }
        let assignment = parse_assignment(key, value).map_err(fail)?;
        let overrides: CoreFileConfig = toml::Value::Table(assignment)
            .try_into()
            .map_err(|err: toml::de::Error| fail(err.message().to_owned()))?;
        let core = overrides.into_core(self.cfg.core.clone());
        core.validate().map_err(|err| fail(err.to_string()))?;
        let profiles = config_loader::rebase_profiles(&self.cfg.profiles, &core)
            .map_err(|err| fail(err.to_string()))?;
        self.cfg.core = core;
        self.cfg.profiles = profiles;
        if !self.tuned_keys.iter().any(|k| k == key) {
            self.tuned_keys.push(key.to_owned());
        }
        self.apply_core();
        let shown = toml::Table::try_from(&self.cfg.core)
            .ok()
            .and_then(|table| table.get(key).map(ToString::to_string))
            .unwrap_or_else(|| value.to_owned());
        Ok(format!("ok {key} = {shown}"))
    }

    fn save(&mut self) -> String {
        if self.tuned_keys.is_empty() {
            return "ok nothing to save".to_owned();
        }
        let Some(path) = self.cfg.config_path.clone() else {
            return "error: no config file path; pass --config".to_owned();
        };
        let owner = config_loader::config_owner(self.args);
        match config_loader::save_core_keys(&path, &self.tuned_keys, &self.cfg.core, owner) {
            Ok(()) => {
                let keys = self.tuned_keys.join(", ");
                self.tuned_keys.clear();
                info!(path = %path.display(), keys, "saved tuned settings");
                format!("ok saved {keys} to {}", path.display())
            }
            Err(err) => format!("error: {err}"),
        }
    }

    // -- reload -------------------------------------------------------------

    fn reload_logged(&mut self) {
        match self.reload() {
            Ok(()) => {}
            Err(err) => warn!(%err, "config reload failed; keeping the current settings"),
        }
    }

    /// Re-reads the config file and applies it. A gesture in progress keeps
    /// its settings until it ends. On error nothing changes.
    fn reload(&mut self) -> Result<(), DaemonError> {
        let mut cfg = config_loader::resolve(self.args)?;
        if cfg.grab != self.cfg.grab || cfg.dry_run != self.cfg.dry_run {
            warn!("`grab` and `dry_run` only change after a restart");
            cfg.grab = self.cfg.grab;
            cfg.dry_run = self.cfg.dry_run;
        }
        if cfg.trigger_button != self.cfg.trigger_button {
            self.abort_gesture();
            self.output.flush();
            // A fresh engine: nothing of the old button may be swallowed or
            // replayed as the new one.
            self.engine = Engine::new(self.engine.config().clone());
            self.last_state = EngineState::Idle;
            self.swallow_trigger_up = false;
            self.gate.reset_latch();
            self.router = Router::new(cfg.trigger_button);
            self.output.set_trigger(cfg.trigger_button.key_code());
            info!(trigger = ?cfg.trigger_button, "trigger button changed");
        }
        self.gate
            .set_config(cfg.foreground.clone(), profile_rules(&cfg.profiles));
        self.hooks.set_config(cfg.hooks.clone());
        let specs = slot_specs(&cfg);
        self.cfg = cfg;
        self.tuned_keys.clear();
        self.active_profile = None;
        self.apply_core();
        self.update_slots(specs);
        self.services_dirty = true;
        info!("configuration reloaded");
        Ok(())
    }

    /// Applies a new list of mice: kept ones stay open, new ones are looked
    /// for, removed ones are released.
    fn update_slots(&mut self, specs: Vec<SlotSpec>) {
        if specs.is_empty() {
            warn!("the reloaded config names no mouse; keeping the current ones");
            return;
        }
        if specs.iter().eq(self.slots.iter().map(|slot| &slot.spec)) {
            return;
        }
        let mut old = std::mem::take(&mut self.slots);
        for spec in specs {
            match old.iter().position(|slot| slot.spec == spec) {
                Some(position) => self.slots.push(old.swap_remove(position)),
                None => self.slots.push(Slot::new(spec)),
            }
        }
        let mut released = false;
        for slot in old {
            if let Some(mouse) = &slot.mouse {
                info!(device = %slot.label(), path = %mouse.path().display(), "mouse removed from the config; releasing it");
                released = true;
            }
        }
        if released {
            self.abort_gesture();
            self.swallow_trigger_up = false;
            self.swallow_key_up = None;
            self.gate.reset_latch();
            self.output.release_all_buttons();
        }
        self.next_retry = Instant::now();
    }
}

/// Parses `key = value` as TOML. A bare word that is not valid TOML is taken
/// as a string, so `set mode toggle_progressive` works without quotes.
fn parse_assignment(key: &str, value: &str) -> Result<toml::Table, String> {
    let quoted = toml::Value::String(value.to_owned()).to_string();
    let table = toml::from_str::<toml::Table>(&format!("{key} = {value}"))
        .or_else(|_| toml::from_str::<toml::Table>(&format!("{key} = {quoted}")))
        .map_err(|err| err.message().to_owned())?;
    if table.len() != 1 || !table.contains_key(key) {
        return Err(format!("expected a single value for `{key}`"));
    }
    Ok(table)
}

/// Converts the time until the next deadline to a `poll` timeout in
/// milliseconds, rounding up so the loop never wakes just before it.
fn poll_timeout_millis(until: Duration) -> u16 {
    until
        .as_micros()
        .div_ceil(1_000)
        .min(u128::from(u16::MAX))
        .try_into()
        .unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foreground::filter::{
        ForegroundApp, ForegroundProvider, ForegroundSnapshot, ForegroundSourceKind,
    };
    use clap::Parser;
    use middle_scroll_core::{Mode, MouseButton};

    struct Fixed(ForegroundSnapshot);

    impl ForegroundProvider for Fixed {
        fn snapshot(&self) -> ForegroundSnapshot {
            self.0.clone()
        }
    }

    fn focused(app_id: &str) -> ForegroundSnapshot {
        ForegroundSnapshot::Known(ForegroundApp {
            app_id: Some(app_id.to_owned()),
            source: ForegroundSourceKind::Command,
            ..ForegroundApp::default()
        })
    }

    struct TempConfig {
        dir: PathBuf,
        path: PathBuf,
    }

    impl TempConfig {
        fn new(tag: &str, content: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("wheeltani-daemon-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.toml");
            std::fs::write(&path, content).unwrap();
            Self { dir, path }
        }

        fn args(&self) -> Args {
            Args::parse_from(["wlw", "--config", self.path.to_str().unwrap()])
        }
    }

    impl Drop for TempConfig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn daemon(args: &Args, snapshot: ForegroundSnapshot) -> Daemon<'_> {
        let cfg = config_loader::resolve(args).expect("config resolves");
        let gate = ForegroundGate::with_provider(
            cfg.foreground.clone(),
            profile_rules(&cfg.profiles),
            Box::new(Fixed(snapshot)),
        );
        let output = Output::recorder(cfg.trigger_button.key_code());
        Daemon::new(
            args,
            cfg,
            true,
            Vec::new(),
            output,
            Signals::for_test(),
            gate,
        )
    }

    fn key(code: KeyCode, pressed: bool) -> InputEvent {
        InputEvent::new(EventType::KEY.0, code.0, i32::from(pressed))
    }

    fn feed(d: &mut Daemon<'_>, events: &[InputEvent]) {
        for event in events {
            d.handle_event(*event);
        }
    }

    fn actions(d: &Daemon<'_>) -> Vec<CoreAction> {
        d.output
            .recorded()
            .iter()
            .filter_map(|r| match r {
                Recorded::Action(a) => Some(a.clone()),
                Recorded::Raw { .. } => None,
            })
            .collect()
    }

    fn middle(pressed: bool) -> CoreAction {
        CoreAction::ForwardMouseButton {
            button: MouseButton::Middle,
            pressed,
        }
    }

    #[test]
    fn paused_daemon_passes_the_trigger_through() {
        let cfg = TempConfig::new("paused", "");
        let args = cfg.args();
        let mut d = daemon(&args, focused("firefox"));
        assert_eq!(d.execute(Command::Pause), "ok paused");
        feed(
            &mut d,
            &[
                key(KeyCode::BTN_MIDDLE, true),
                key(KeyCode::BTN_MIDDLE, false),
            ],
        );
        assert_eq!(actions(&d), vec![middle(true), middle(false)]);
        assert_eq!(d.engine.state(), EngineState::Idle);
        assert_eq!(d.execute(Command::Toggle), "ok active");
    }

    #[test]
    fn pausing_mid_gesture_swallows_the_trigger_release() {
        let cfg = TempConfig::new("pause-mid", "");
        let args = cfg.args();
        let mut d = daemon(&args, focused("firefox"));
        feed(&mut d, &[key(KeyCode::BTN_MIDDLE, true)]);
        assert_eq!(d.engine.state(), EngineState::MiddlePending);
        d.set_paused(true);
        feed(&mut d, &[key(KeyCode::BTN_MIDDLE, false)]);
        assert!(actions(&d).is_empty(), "{:?}", actions(&d));
        // The next press is handled normally again.
        feed(&mut d, &[key(KeyCode::BTN_MIDDLE, true)]);
        assert_eq!(actions(&d), vec![middle(true)]);
    }

    #[test]
    fn denied_app_passes_through_and_releases_the_latch() {
        let cfg = TempConfig::new(
            "denied",
            "[foreground]\nenabled = true\nmode = \"denylist\"\ndeny_apps = [\"firefox\"]\n",
        );
        let args = cfg.args();
        let mut d = daemon(&args, focused("firefox"));
        feed(&mut d, &[key(KeyCode::BTN_MIDDLE, true)]);
        assert!(d.gate.latched().is_some());
        feed(&mut d, &[key(KeyCode::BTN_MIDDLE, false)]);
        assert_eq!(actions(&d), vec![middle(true), middle(false)]);
        assert!(d.gate.latched().is_none());
        assert_eq!(d.engine.state(), EngineState::Idle);
    }

    #[test]
    fn profile_enables_toggle_mode_and_a_raw_button_ends_it() {
        let cfg = TempConfig::new(
            "profile",
            "[[profiles]]\nname = \"browser\"\napps = [\"firefox\"]\nmode = \"toggle_progressive\"\n",
        );
        let args = cfg.args();
        let mut d = daemon(&args, focused("firefox"));
        feed(
            &mut d,
            &[
                key(KeyCode::BTN_MIDDLE, true),
                key(KeyCode::BTN_MIDDLE, false),
            ],
        );
        assert_eq!(d.active_profile, Some(0));
        assert_eq!(d.engine.config().mode, Mode::ToggleProgressive);
        assert!(d.engine.is_latched());

        // A thumb button ends scroll mode; neither its press nor its release
        // reaches the application.
        feed(
            &mut d,
            &[key(KeyCode::BTN_SIDE, true), key(KeyCode::BTN_SIDE, false)],
        );
        assert_eq!(d.engine.state(), EngineState::Idle);
        assert!(!d
            .output
            .recorded()
            .iter()
            .any(|r| matches!(r, Recorded::Raw { .. })));
        assert_eq!(
            actions(&d),
            vec![CoreAction::EnterScrollMode, CoreAction::ExitScrollMode]
        );

        // The next press of that button is forwarded again.
        feed(&mut d, &[key(KeyCode::BTN_SIDE, true)]);
        assert!(d.output.recorded().contains(&Recorded::Raw {
            kind: EventType::KEY.0,
            code: KeyCode::BTN_SIDE.0,
            value: 1,
        }));

        let status: serde_json::Value =
            serde_json::from_str(d.execute(Command::Status).strip_prefix("ok ").unwrap()).unwrap();
        assert_eq!(status["profile"], "browser");
        assert_eq!(status["mode"], "toggle_progressive");
        assert_eq!(status["state"], "active");
        assert_eq!(status["engine"], "idle");
    }

    #[test]
    fn set_get_and_save_round_trip() {
        let cfg = TempConfig::new("tune", "# my settings\ndeadzone_units = 6 # small\n");
        let args = cfg.args();
        let mut d = daemon(&args, focused("firefox"));
        assert_eq!(
            d.execute(Command::Get(Some("deadzone_units".into()))),
            "ok deadzone_units = 6"
        );
        let set = |key: &str, value: &str| Command::Set {
            key: key.to_owned(),
            value: value.to_owned(),
        };
        assert_eq!(
            d.execute(set("deadzone_units", "12")),
            "ok deadzone_units = 12"
        );
        assert_eq!(d.engine.config().deadzone_units, 12);
        assert_eq!(
            d.execute(set("mode", "toggle_progressive")),
            "ok mode = \"toggle_progressive\""
        );
        // `--tune` users type whole numbers for speeds.
        assert_eq!(
            d.execute(set("max_speed_detents_per_second", "40")),
            "ok max_speed_detents_per_second = 40.0"
        );

        assert!(d.execute(set("no_such_key", "1")).starts_with("error:"));
        assert!(d
            .execute(set("deadzone_units", "\"wide\""))
            .starts_with("error:"));
        assert!(d
            .execute(set("deadzone_units", "1\nmode = 2"))
            .starts_with("error:"));
        assert!(d.execute(set("grab", "false")).starts_with("error:"));
        assert_eq!(d.engine.config().deadzone_units, 12);

        let reply = d.execute(Command::Save);
        assert!(
            reply.starts_with("ok saved deadzone_units, mode"),
            "{reply}"
        );
        let saved = std::fs::read_to_string(&cfg.path).unwrap();
        assert!(saved.contains("# my settings"), "{saved}");
        assert!(saved.contains("deadzone_units = 12"), "{saved}");
        assert!(saved.contains("mode = \"toggle_progressive\""), "{saved}");
        assert_eq!(d.execute(Command::Save), "ok nothing to save");
    }

    #[test]
    fn reload_applies_the_new_file() {
        let cfg = TempConfig::new("reload", "deadzone_units = 5\n");
        let args = cfg.args();
        let mut d = daemon(&args, focused("firefox"));
        assert_eq!(d.engine.config().deadzone_units, 5);
        std::fs::write(&cfg.path, "deadzone_units = 9\ntrigger_button = \"side\"\n").unwrap();
        assert_eq!(d.execute(Command::Reload), "ok reloaded");
        assert_eq!(d.engine.config().deadzone_units, 9);
        assert_eq!(d.router.trigger(), KeyCode::BTN_SIDE);
        assert!(d.services_dirty);

        // A broken file keeps the current settings.
        std::fs::write(&cfg.path, "deadzone_units = \"nope\"\n").unwrap();
        assert!(d.execute(Command::Reload).starts_with("error:"));
        assert_eq!(d.engine.config().deadzone_units, 9);
    }

    #[test]
    fn idle_daemon_sleeps_without_timeout() {
        let cfg = TempConfig::new("idle", "");
        let args = cfg.args();
        let mut d = daemon(&args, focused("firefox"));
        assert_eq!(d.poll_timeout(Instant::now()), PollTimeout::NONE);
        feed(&mut d, &[key(KeyCode::BTN_MIDDLE, true)]);
        // Still no ticks while the press is pending.
        assert_eq!(d.poll_timeout(Instant::now()), PollTimeout::NONE);
        d.reload_at = Some(Instant::now() + Duration::from_millis(100));
        assert_ne!(d.poll_timeout(Instant::now()), PollTimeout::NONE);
    }

    #[test]
    fn poll_timeout_rounds_sub_millisecond_deadlines_up() {
        assert_eq!(poll_timeout_millis(Duration::ZERO), 0);
        assert_eq!(poll_timeout_millis(Duration::from_micros(1)), 1);
        assert_eq!(poll_timeout_millis(Duration::from_micros(999)), 1);
        assert_eq!(poll_timeout_millis(Duration::from_micros(1_001)), 2);
    }

    #[test]
    fn poll_timeout_saturates_at_poll_limit() {
        assert_eq!(poll_timeout_millis(Duration::from_secs(120)), u16::MAX);
    }

    #[test]
    fn reconnect_match_copies_device_match() {
        let m = ParsedDeviceMatch {
            vendor_id: 0x046d,
            product_id: 0xc539,
            name: Some("Logitech USB Receiver".to_owned()),
            phys: Some("usb-0000:00:14.0-2/input0".to_owned()),
        };
        let slot = Slot::new(SlotSpec::Match(m));
        assert!(slot.pinned());
        assert_eq!(slot.label(), "046d:c539");
        let rm = slot.reconnect.as_ref().unwrap();
        let criteria = rm.criteria();
        assert_eq!(criteria.vendor_id, 0x046d);
        assert_eq!(criteria.product_id, 0xc539);
        assert_eq!(criteria.name, Some("Logitech USB Receiver"));
        assert_eq!(criteria.phys, Some("usb-0000:00:14.0-2/input0"));
    }

    #[test]
    fn path_slots_have_no_reconnect_match_until_opened() {
        let slot = Slot::new(SlotSpec::Path(PathBuf::from(
            "/dev/input/event-does-not-exist",
        )));
        assert!(slot.reconnect.is_none());
        assert!(!slot.pinned());
        assert!(slot.locate(&[]).is_none());
    }

    #[test]
    fn parse_assignment_quotes_bare_words() {
        let table = parse_assignment("mode", "toggle_progressive").unwrap();
        assert_eq!(table["mode"].as_str(), Some("toggle_progressive"));
        let table = parse_assignment("deadzone_units", "7").unwrap();
        assert_eq!(table["deadzone_units"].as_integer(), Some(7));
        assert!(parse_assignment("a", "1\nb = 2").is_err());
    }
}
