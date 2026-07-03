#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    eprintln!(
        "wayland-wheeltani: this binary only runs on Linux (evdev/uinput).\n\
         Cross-compile or build inside an Ubuntu VM."
    );
    std::process::exit(2);
}

#[cfg(target_os = "linux")]
mod cli;
#[cfg(target_os = "linux")]
mod config_loader;
#[cfg(target_os = "linux")]
mod device_discovery;
#[cfg(target_os = "linux")]
mod errors;
#[cfg(target_os = "linux")]
mod event_router;
#[cfg(target_os = "linux")]
mod foreground;
#[cfg(target_os = "linux")]
mod indicator;
#[cfg(target_os = "linux")]
mod physical_mouse;
#[cfg(target_os = "linux")]
mod service;
#[cfg(target_os = "linux")]
mod udev_rule;
#[cfg(target_os = "linux")]
mod virtual_mouse;

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    linux::run()
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io::{self, IsTerminal, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use anyhow::Context;
    use middle_scroll_core::{CoreAction, CoreInputEvent, Engine, EngineState};
    use nix::poll::{PollFd, PollFlags, PollTimeout};
    use tracing::{debug, error, info, warn};
    use tracing_subscriber::EnvFilter;

    use crate::cli::Args;
    use crate::config_loader::{self, ParsedDeviceMatch, ResolvedConfig};
    use crate::device_discovery::{self, DeviceInfo, MatchCriteria};
    use crate::errors::DaemonError;
    use crate::event_router::{self, RoutedEvent};
    use crate::foreground::{AutoscrollDecision, ForegroundGate};
    use crate::indicator::{Indicator, NoopIndicator};
    use crate::physical_mouse::PhysicalMouse;
    use crate::service;
    use crate::udev_rule;
    use crate::virtual_mouse::VirtualMouse;

    const COMPOSITOR_SETTLE_DELAY: Duration = Duration::from_millis(200);
    /// Total time spent waiting for a `device_match` to appear. Useful when
    /// the daemon is launched by systemd before USB enumeration completes.
    const DEVICE_MATCH_RETRY_TIMEOUT: Duration = Duration::from_secs(10);
    /// Polling interval while waiting for a `device_match`, both at startup and
    /// while waiting for the mouse to be plugged back in at runtime.
    const DEVICE_MATCH_RETRY_INTERVAL: Duration = Duration::from_millis(500);

    /// Countdown shown by `--detect-foreground` so the user can focus the window
    /// they want to identify before the snapshot is read.
    const DETECT_COUNTDOWN_SECS: u64 = 3;

    /// Reusable buffers for the hot event path. Both are cleared and refilled
    /// for every input event / tick, so keeping them alive across iterations
    /// avoids two heap allocations per event.
    struct Scratch {
        actions: Vec<CoreAction>,
        batch: Vec<CoreAction>,
    }

    impl Scratch {
        fn new() -> Self {
            Self {
                actions: Vec::with_capacity(16),
                batch: Vec::with_capacity(16),
            }
        }
    }

    /// Why the inner event loop returned. The supervisor in `run_daemon` uses
    /// this to decide between shutting down and re-acquiring the device.
    enum LoopOutcome {
        /// A shutdown signal or the safety timeout fired; stop the daemon.
        Shutdown,
        /// The physical device went away (unplugged). Try to re-open it,
        /// possibly on a different `/dev/input/eventXX` node or USB port.
        DeviceLost,
    }

    /// Owned criteria used to re-find the mouse after it is unplugged and
    /// plugged back in, possibly on a different USB port. Prefers the configured
    /// `[device_match]`; otherwise it is derived from the live device so that
    /// `--device`/legacy `device =` configs also reconnect across ports.
    struct ReconnectMatch {
        vendor_id: u16,
        product_id: u16,
        name: Option<String>,
        phys: Option<String>,
    }

    impl ReconnectMatch {
        /// Builds the reconnect criteria from a configured `[device_match]`, if
        /// any. Returns `None` for legacy `device =`/`--device` configs that have
        /// no stable USB id to match on.
        fn from_config(cfg: &ResolvedConfig) -> Option<Self> {
            cfg.device_match.as_ref().map(|m| Self {
                vendor_id: m.vendor_id,
                product_id: m.product_id,
                name: m.name.clone(),
                phys: m.phys.clone(),
            })
        }

        /// Derives reconnect criteria from a live device. Used so that
        /// `--device`/legacy configs also reconnect across ports: once the mouse
        /// has been opened, its USB id is known. `phys` is intentionally left
        /// unset so the match does not depend on the current port.
        fn from_device(physical: &PhysicalMouse) -> Self {
            Self {
                vendor_id: physical.vendor_id(),
                product_id: physical.product_id(),
                name: Some(physical.name().to_owned()),
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

    pub fn run() -> anyhow::Result<()> {
        let args = Args::parsed();
        init_tracing(&args);

        if let Some(action) = service::requested_action(&args) {
            service::run_action(action)?;
            return Ok(());
        }

        if args.remove_udev_rule {
            udev_rule::remove_rule()?;
            return Ok(());
        }

        if args.list_devices {
            let devices = device_discovery::enumerate_mice();
            device_discovery::print_listing(io::stdout().lock(), &devices)?;
            return Ok(());
        }

        if args.install_udev_rule {
            udev_rule::ensure_root()?;
        }

        let resolved = config_loader::resolve(&args)?;

        if args.detect_foreground {
            detect_foreground(&resolved.foreground);
            return Ok(());
        }

        let has_configured_device = resolved.device.is_some() || resolved.device_match.is_some();
        let should_save_device = args.setup
            || !has_configured_device
            || (args.install_service && args.device.is_some())
            || (args.install_udev_rule && args.device.is_some());
        let device_path = if should_save_device {
            let selected = if let Some(device) = args.device.clone() {
                device
            } else {
                select_device(&args)?
            };
            let config_path = config_loader::save_device_to_config(&selected, &args)?;
            println!(
                "Saved device {} to {}",
                selected.display(),
                config_path.display()
            );
            if args.setup && !args.install_service && !args.install_udev_rule {
                return Ok(());
            }
            selected
        } else {
            resolve_device_path(&args, &resolved)?
        };

        if args.install_udev_rule {
            udev_rule::install_for_device(&device_path)?;
            return Ok(());
        }

        if args.install_service {
            service::install_user_service(&args)?;
            return Ok(());
        }

        run_daemon(&device_path, &resolved)
    }

    /// Resolves the runtime device path from CLI arg, then `[device_match]`,
    /// then the legacy `device =` path. Returning here means the config is
    /// usable but the daemon might still fail later when opening the device.
    fn resolve_device_path(args: &Args, resolved: &ResolvedConfig) -> anyhow::Result<PathBuf> {
        if let Some(device) = args.device.clone() {
            return Ok(device);
        }

        if let Some(match_cfg) = &resolved.device_match {
            // A service (`--no-interactive`) may be started before the mouse is
            // plugged in, so wait indefinitely there instead of crash-looping.
            // Interactive runs keep the short timeout for snappy CLI feedback.
            return resolve_device_match(match_cfg, args.no_interactive);
        }

        if let Some(device) = resolved.device.clone() {
            warn!(
                device = %device.display(),
                "config uses legacy `device =` which is not stable across reboots; re-run `wayland-wheeltani --setup` to migrate to `[device_match]`"
            );
            return Ok(device);
        }

        Err(DaemonError::NoDevice.into())
    }

    /// Resolves a `[device_match]` to a `/dev/input/eventXX` node, waiting for
    /// the mouse to be plugged in. When `wait_forever` is set (service mode) it
    /// never gives up; otherwise it errors after `DEVICE_MATCH_RETRY_TIMEOUT` so
    /// interactive invocations fail fast.
    fn resolve_device_match(
        match_cfg: &ParsedDeviceMatch,
        wait_forever: bool,
    ) -> anyhow::Result<PathBuf> {
        let id = match_cfg.human_id();
        let criteria = match_cfg.as_criteria();

        if let Some(found) = try_match(&criteria, &id) {
            info!(
                usb_id = %id,
                event = %found.display(),
                "resolved device_match"
            );
            return Ok(found);
        }

        if wait_forever {
            warn!(
                usb_id = %id,
                "no input device currently matches; waiting for the mouse to be plugged in (any port works)"
            );
        } else {
            warn!(
                usb_id = %id,
                "no input device currently matches; waiting up to {:?} for it to appear",
                DEVICE_MATCH_RETRY_TIMEOUT
            );
        }

        let started = Instant::now();
        loop {
            std::thread::sleep(DEVICE_MATCH_RETRY_INTERVAL);
            if let Some(found) = try_match(&criteria, &id) {
                info!(
                    usb_id = %id,
                    event = %found.display(),
                    elapsed = ?started.elapsed(),
                    "resolved device_match after retry"
                );
                return Ok(found);
            }
            if !wait_forever && started.elapsed() >= DEVICE_MATCH_RETRY_TIMEOUT {
                return Err(DaemonError::DeviceMatchNotFound {
                    vendor_id: match_cfg.vendor_id,
                    product_id: match_cfg.product_id,
                }
                .into());
            }
        }
    }

    /// Tries a strict match first (honouring `phys` when configured), then a
    /// relaxed match that ignores `phys`. The relaxed pass lets legacy configs
    /// that pinned a USB port keep working when the mouse is moved to another
    /// port: the device is still found by USB id, just on a different node.
    fn try_match(criteria: &MatchCriteria<'_>, id: &str) -> Option<PathBuf> {
        if let Some(found) = device_discovery::find_match(criteria) {
            return Some(found.path);
        }

        if criteria.phys.is_some() {
            if let Some(found) = device_discovery::find_match(&criteria.without_phys()) {
                warn!(
                    usb_id = %id,
                    event = %found.path.display(),
                    found_phys = ?found.phys,
                    "mouse found on a different USB port than the one pinned in config; matching by USB id instead (re-run `wayland-wheeltani --setup` to clear the pinned port)"
                );
                return Some(found.path);
            }
        }

        None
    }

    fn select_device(args: &Args) -> anyhow::Result<PathBuf> {
        if args.no_interactive {
            return Err(DaemonError::NoDevice.into());
        }

        let devices = device_discovery::enumerate_mice();
        match devices.len() {
            0 => Err(DaemonError::NoMiceFound.into()),
            1 => Ok(devices[0].path.clone()),
            _ if !io::stdin().is_terminal() => Err(DaemonError::NonInteractiveDeviceChoice.into()),
            _ => prompt_device_selection(&devices),
        }
    }

    fn prompt_device_selection(devices: &[DeviceInfo]) -> anyhow::Result<PathBuf> {
        println!("No input device is configured yet. Candidate mice:\n");
        device_discovery::print_listing(io::stdout().lock(), devices)?;
        loop {
            print!("Select a device [1-{}]: ", devices.len());
            io::stdout().flush()?;

            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            if let Ok(choice) = line.trim().parse::<usize>() {
                if (1..=devices.len()).contains(&choice) {
                    return Ok(devices[choice - 1].path.clone());
                }
            }
            eprintln!(
                "Invalid selection; enter a number between 1 and {}.",
                devices.len()
            );
        }
    }

    fn init_tracing(args: &Args) {
        let directive = args.log_directive();
        let filter =
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(directive));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_target(true)
            .compact()
            .init();
    }

    /// `--detect-foreground`: start the configured foreground provider, give the
    /// user a moment to focus the target window, then print the focused app's
    /// identity and the exact string to drop into `deny_apps`/`allow_apps`. This
    /// shares the daemon's provider-selection logic, so it works with every
    /// provider (auto/hyprland/sway/gnome/kde/command). It never opens the mouse.
    fn detect_foreground(cfg: &crate::foreground::ForegroundConfig) {
        use crate::foreground::config::ForegroundProviderKind;
        use crate::foreground::filter::ForegroundSnapshot;

        // Discovery should be useful even if the user left the filter on `none`:
        // fall back to auto-detection in that case.
        let mut cfg = cfg.clone();
        if matches!(cfg.provider, ForegroundProviderKind::None) {
            cfg.provider = ForegroundProviderKind::Auto;
        }

        let provider = crate::foreground::providers::select_provider(&cfg);

        println!(
            "Focus the window you want to identify; reading the focused app in {DETECT_COUNTDOWN_SECS}s..."
        );
        for remaining in (1..=DETECT_COUNTDOWN_SECS).rev() {
            print!("\r  {remaining}... ");
            io::stdout().flush().ok();
            std::thread::sleep(Duration::from_secs(1));
        }
        print!("\r                 \r");
        io::stdout().flush().ok();

        // Threaded providers (hyprland/sway/command) may still be warming up:
        // briefly poll for a Known snapshot before giving up.
        let deadline = Instant::now() + Duration::from_secs(2);
        let snapshot = loop {
            let snapshot = provider.snapshot();
            if matches!(snapshot, ForegroundSnapshot::Known(_)) || Instant::now() >= deadline {
                break snapshot;
            }
            std::thread::sleep(Duration::from_millis(100));
        };

        match snapshot {
            ForegroundSnapshot::Known(app) => print_foreground_detection(&app),
            ForegroundSnapshot::Unknown { reason } => {
                println!("Could not determine the focused application ({reason}).");
                println!(
                    "The provider is running but reported no focused window. Try again and make \
                     sure the target window is actually focused during the countdown."
                );
            }
            ForegroundSnapshot::Unsupported { reason } => {
                println!("Foreground detection is not available ({reason}).");
                println!(
                    "Set `provider` under [foreground] (auto | hyprland | sway | gnome | kde | \
                     command). On GNOME, install the bundled Shell extension (see \
                     integrations/gnome/); on KDE Plasma, install `kdotool`."
                );
            }
        }
    }

    /// Prints the detected window's identity fields and the candidate strings to
    /// use in `deny_apps`/`allow_apps` (the same fields the matcher compares).
    fn print_foreground_detection(app: &crate::foreground::filter::ForegroundApp) {
        use crate::foreground::matcher::normalize;

        let show = |o: &Option<String>| o.as_deref().unwrap_or("(none)").to_owned();
        println!(
            "Detected foreground application (source: {:?}):",
            app.source
        );
        println!("  app_id         : {}", show(&app.app_id));
        println!("  class          : {}", show(&app.class));
        println!("  resource_class : {}", show(&app.resource_class));
        println!("  title          : {}", show(&app.title));
        println!(
            "  pid            : {}",
            app.pid
                .map_or_else(|| "(none)".to_owned(), |p| p.to_string())
        );
        println!();

        let mut candidates: Vec<String> = Vec::new();
        for value in [&app.app_id, &app.class, &app.resource_class]
            .into_iter()
            .flatten()
        {
            let normalized = normalize(value);
            if !normalized.is_empty() && !candidates.contains(&normalized) {
                candidates.push(normalized);
            }
        }

        if candidates.is_empty() {
            println!(
                "No stable identifier (app_id/class) was reported. Enable `match_title = true` to \
                 match on the window title instead."
            );
            return;
        }

        println!("Use one of these identifiers (case-insensitive, `.desktop` ignored):");
        for candidate in &candidates {
            println!("    {candidate}");
        }
        println!();
        println!("Example — block autoscroll in this app:");
        println!("  [foreground]");
        println!("  enabled = true");
        println!("  provider = \"auto\"");
        println!("  mode = \"denylist\"");
        println!("  deny_apps = [\"{}\"]", candidates[0]);
    }

    fn run_daemon(device_path: &Path, cfg: &ResolvedConfig) -> anyhow::Result<()> {
        info!(
            device = %device_path.display(),
            grab = cfg.grab,
            dry_run = cfg.dry_run,
            tick_hz = cfg.core.tick_hz,
            "starting daemon"
        );

        let shutdown = install_signal_handler()?;
        let mut indicator = NoopIndicator;

        // The virtual mouse is created once and kept alive across physical
        // reconnections, so the compositor never sees it disappear.
        let mut virtual_mouse = if cfg.dry_run {
            None
        } else {
            let v = VirtualMouse::new()?;
            std::thread::sleep(COMPOSITOR_SETTLE_DELAY);
            Some(v)
        };

        if cfg.grab && cfg.dry_run {
            warn!("--dry-run skips grab to avoid silently capturing the mouse");
        } else if !cfg.grab {
            warn!("running without grab; the compositor will see both physical and virtual events");
        }

        let started = Instant::now();
        let safety_timeout = cfg.safety_timeout_seconds.map(Duration::from_secs);
        let tick_period = Duration::from_micros(1_000_000 / u64::from(cfg.core.tick_hz));

        let mut engine = Engine::new(cfg.core.clone());
        let mut last_state = engine.state();
        let mut foreground_gate = ForegroundGate::new(cfg.foreground.clone());
        let mut scratch = Scratch::new();

        // Initial open is fatal: `run()` already waited for a `device_match` to
        // appear, so a failure here is a genuine setup problem.
        let mut physical = open_and_grab(device_path, cfg)?;
        info!(name = physical.name(), "opened physical device");

        let reconnect = ReconnectMatch::from_config(cfg)
            .unwrap_or_else(|| ReconnectMatch::from_device(&physical));

        let result = loop {
            let mut last_tick = Instant::now();
            let outcome = run_loop(
                &mut physical,
                virtual_mouse.as_mut(),
                &mut engine,
                &mut indicator,
                &mut foreground_gate,
                &shutdown,
                tick_period,
                safety_timeout,
                started,
                cfg.dry_run,
                &mut last_tick,
                &mut last_state,
                &mut scratch,
            );

            match outcome {
                Ok(LoopOutcome::Shutdown) => break Ok(()),
                Err(err) => break Err(err),
                Ok(LoopOutcome::DeviceLost) => {
                    warn!("physical device disconnected; waiting to reconnect");
                    drop(physical);

                    // Clear in-flight gesture state so the gap cannot leave a
                    // scroll detent pending or a forwarded button stuck down.
                    if let Some(v) = virtual_mouse.as_mut() {
                        if let Err(err) = v.release_all_buttons() {
                            warn!(?err, "failed to release virtual buttons after disconnect");
                        }
                    }
                    engine = Engine::new(cfg.core.clone());
                    last_state = engine.state();
                    // Drop any latched gesture decision so the gap cannot leave
                    // a stale foreground decision applied after reconnect.
                    foreground_gate.reset_latch();

                    match wait_for_device(&reconnect, cfg, &shutdown) {
                        Some(p) => {
                            physical = p;
                            info!(name = physical.name(), "reconnected physical device");
                        }
                        None => break Ok(()),
                    }
                }
            }
        };

        info!("shutting down");
        result
    }

    /// Opens the device at `path` and grabs it exclusively when configured.
    /// Used both for the initial open and for every runtime reconnection.
    fn open_and_grab(path: &Path, cfg: &ResolvedConfig) -> anyhow::Result<PhysicalMouse> {
        let mut physical = PhysicalMouse::open(path)?;
        if cfg.grab && !cfg.dry_run {
            physical
                .grab()
                .with_context(|| format!("failed to grab device {}", path.display()))?;
            info!("grabbed physical device exclusively");
        }
        Ok(physical)
    }

    /// Blocks until the mouse described by `reconnect` is plugged back in and can
    /// be opened/grabbed, then returns it ready to use. Returns `None` if a
    /// shutdown is requested while waiting. Matching is by USB id, so the mouse
    /// is found again even on a different `/dev/input/eventXX` node or USB port.
    fn wait_for_device(
        reconnect: &ReconnectMatch,
        cfg: &ResolvedConfig,
        shutdown: &Arc<AtomicBool>,
    ) -> Option<PhysicalMouse> {
        let id = reconnect.human_id();
        info!(
            usb_id = %id,
            "waiting for the mouse to be plugged back in (any port works)"
        );

        loop {
            if shutdown.load(Ordering::SeqCst) {
                return None;
            }

            if let Some(path) = try_match(&reconnect.criteria(), &id) {
                match open_and_grab(&path, cfg) {
                    Ok(physical) => {
                        info!(usb_id = %id, event = %path.display(), "mouse reconnected");
                        return Some(physical);
                    }
                    Err(err) => {
                        // The node reappeared but is not usable yet (udev still
                        // applying permissions, or a previous grab not released).
                        debug!(?err, event = %path.display(), "device not ready yet; retrying");
                    }
                }
            }

            std::thread::sleep(DEVICE_MATCH_RETRY_INTERVAL);
        }
    }

    fn install_signal_handler() -> anyhow::Result<Arc<AtomicBool>> {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        ctrlc::set_handler(move || {
            flag_clone.store(true, Ordering::SeqCst);
        })
        .context("failed to install signal handler")?;
        Ok(flag)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_loop(
        physical: &mut PhysicalMouse,
        mut virtual_mouse: Option<&mut VirtualMouse>,
        engine: &mut Engine,
        indicator: &mut dyn Indicator,
        foreground_gate: &mut ForegroundGate,
        shutdown: &Arc<AtomicBool>,
        tick_period: Duration,
        safety_timeout: Option<Duration>,
        started: Instant,
        dry_run: bool,
        last_tick: &mut Instant,
        last_state: &mut EngineState,
        scratch: &mut Scratch,
    ) -> anyhow::Result<LoopOutcome> {
        loop {
            if shutdown.load(Ordering::SeqCst) {
                info!("shutdown signal received");
                return Ok(LoopOutcome::Shutdown);
            }
            if let Some(limit) = safety_timeout {
                if started.elapsed() >= limit {
                    warn!(?limit, "safety timeout reached, exiting");
                    return Ok(LoopOutcome::Shutdown);
                }
            }

            let now = Instant::now();
            let until_next_tick = tick_period.saturating_sub(now.duration_since(*last_tick));
            let timeout_ms = poll_timeout_millis(until_next_tick);

            // `revents` is copied out so the borrow on `physical` ends before we
            // touch it mutably below.
            let revents = {
                let fd = physical.as_fd();
                let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
                match nix::poll::poll(&mut fds, PollTimeout::from(timeout_ms)) {
                    Ok(0) => None,
                    Ok(_) => fds[0].revents(),
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(err) => {
                        error!(?err, "poll failed");
                        return Err(err.into());
                    }
                }
            };

            if let Some(revents) = revents {
                // A hangup/error on the fd means the mouse was unplugged.
                if revents.intersects(PollFlags::POLLHUP | PollFlags::POLLERR | PollFlags::POLLNVAL)
                {
                    warn!(?revents, "physical device hung up; treating as disconnect");
                    return Ok(LoopOutcome::DeviceLost);
                }

                if revents.contains(PollFlags::POLLIN)
                    && !process_pending_events(
                        physical,
                        virtual_mouse.as_deref_mut(),
                        engine,
                        indicator,
                        foreground_gate,
                        dry_run,
                        last_state,
                        scratch,
                    )?
                {
                    return Ok(LoopOutcome::DeviceLost);
                }
            }

            let now = Instant::now();
            if now.duration_since(*last_tick) >= tick_period {
                let dt = now.duration_since(*last_tick);
                *last_tick = now;
                let dt_us = dt.as_micros().min(u128::from(u64::MAX)) as u64;
                scratch.actions.clear();
                engine.process_into(
                    CoreInputEvent::Tick { dt_micros: dt_us },
                    &mut scratch.actions,
                );
                emit_actions(
                    &scratch.actions,
                    &mut scratch.batch,
                    virtual_mouse.as_deref_mut(),
                    dry_run,
                    indicator,
                    engine,
                    last_state,
                )?;
            }
        }
    }

    /// Drains and dispatches pending input events. Returns `Ok(false)` when the
    /// read fails because the device disconnected, so the caller can switch to
    /// reconnecting instead of aborting the whole daemon.
    #[allow(clippy::too_many_arguments)]
    fn process_pending_events(
        physical: &mut PhysicalMouse,
        mut virtual_mouse: Option<&mut VirtualMouse>,
        engine: &mut Engine,
        indicator: &mut dyn Indicator,
        foreground_gate: &mut ForegroundGate,
        dry_run: bool,
        last_state: &mut EngineState,
        scratch: &mut Scratch,
    ) -> anyhow::Result<bool> {
        // Iterate the kernel buffer directly instead of collecting into a Vec:
        // this path wakes up for every burst of motion events, so avoiding a
        // heap allocation per wakeup matters.
        let events = match physical.fetch_events() {
            Ok(iter) => iter,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => return Ok(true),
            Err(err) => {
                warn!(?err, "fetch_events failed; treating device as disconnected");
                return Ok(false);
            }
        };

        for ev in events {
            let routed = event_router::classify(&ev);
            let decision = foreground_gate.decision_for_event(&routed, engine.state());

            match decision {
                AutoscrollDecision::Enabled => match routed {
                    RoutedEvent::Core(core_event) => {
                        scratch.actions.clear();
                        engine.process_into(core_event, &mut scratch.actions);
                        emit_actions(
                            &scratch.actions,
                            &mut scratch.batch,
                            virtual_mouse.as_deref_mut(),
                            dry_run,
                            indicator,
                            engine,
                            last_state,
                        )?;
                    }
                    RoutedEvent::DirectButton { button, pressed } => {
                        let action = CoreAction::ForwardMouseButton { button, pressed };
                        emit_actions(
                            std::slice::from_ref(&action),
                            &mut scratch.batch,
                            virtual_mouse.as_deref_mut(),
                            dry_run,
                            indicator,
                            engine,
                            last_state,
                        )?;
                    }
                    RoutedEvent::Ignore => {}
                },
                AutoscrollDecision::Disabled => {
                    let actions = event_router::passthrough_actions(&routed);
                    emit_actions(
                        &actions,
                        &mut scratch.batch,
                        virtual_mouse.as_deref_mut(),
                        dry_run,
                        indicator,
                        engine,
                        last_state,
                    )?;
                }
            }

            foreground_gate.after_event(&routed, engine.state());
        }
        Ok(true)
    }

    #[allow(clippy::unnecessary_wraps, clippy::too_many_arguments)]
    fn emit_actions(
        actions: &[CoreAction],
        batch: &mut Vec<CoreAction>,
        mut virtual_mouse: Option<&mut VirtualMouse>,
        dry_run: bool,
        indicator: &mut dyn Indicator,
        engine: &Engine,
        last_state: &mut EngineState,
    ) -> anyhow::Result<()> {
        batch.clear();

        for action in actions {
            match action {
                CoreAction::EnterScrollMode => {
                    debug!("EnterScrollMode");
                    indicator.enter_scroll();
                }
                CoreAction::ExitScrollMode => {
                    debug!("ExitScrollMode");
                    indicator.exit_scroll();
                }
                CoreAction::EmitMiddleClick => {
                    flush_batch(batch, virtual_mouse.as_deref_mut(), dry_run);
                    debug!("EmitMiddleClick");
                    if dry_run {
                        info!("DRY-RUN action: EmitMiddleClick");
                    } else if let Some(v) = virtual_mouse.as_deref_mut() {
                        if let Err(err) = v.emit_middle_click() {
                            warn!(?err, "failed to emit middle click");
                        }
                    }
                }
                CoreAction::Suppress => {}
                _ => {
                    if dry_run {
                        if let Some(desc) = event_router::dry_run_describe(action) {
                            info!("DRY-RUN action: {desc}");
                        }
                    } else {
                        batch.push(action.clone());
                    }
                }
            }
        }

        flush_batch(batch, virtual_mouse, dry_run);

        let new_state = engine.state();
        if new_state != *last_state {
            debug!(from = ?*last_state, to = ?new_state, "state transition");
            *last_state = new_state;
        }
        Ok(())
    }

    fn flush_batch(batch: &mut Vec<CoreAction>, vm: Option<&mut VirtualMouse>, dry_run: bool) {
        if batch.is_empty() || dry_run {
            batch.clear();
            return;
        }
        if let Some(v) = vm {
            if let Err(err) = v.apply_batch(batch) {
                warn!(?err, "failed to emit batch");
            }
        }
        batch.clear();
    }

    fn poll_timeout_millis(until_next_tick: Duration) -> u16 {
        until_next_tick
            .as_micros()
            .div_ceil(1_000)
            .min(u128::from(u16::MAX))
            .try_into()
            .unwrap_or(u16::MAX)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn resolved_with_match(device_match: Option<ParsedDeviceMatch>) -> ResolvedConfig {
            ResolvedConfig {
                core: middle_scroll_core::CoreConfig::default(),
                device: None,
                device_match,
                grab: true,
                dry_run: false,
                safety_timeout_seconds: None,
                foreground: crate::foreground::ForegroundConfig::default(),
            }
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
        fn reconnect_match_from_config_copies_device_match() {
            let cfg = resolved_with_match(Some(ParsedDeviceMatch {
                vendor_id: 0x046d,
                product_id: 0xc539,
                name: Some("Logitech USB Receiver".to_owned()),
                phys: Some("usb-0000:00:14.0-2/input0".to_owned()),
            }));

            let rm = ReconnectMatch::from_config(&cfg).expect("device_match present");
            assert_eq!(rm.vendor_id, 0x046d);
            assert_eq!(rm.product_id, 0xc539);
            assert_eq!(rm.human_id(), "046d:c539");

            let criteria = rm.criteria();
            assert_eq!(criteria.vendor_id, 0x046d);
            assert_eq!(criteria.product_id, 0xc539);
            assert_eq!(criteria.name, Some("Logitech USB Receiver"));
            assert_eq!(criteria.phys, Some("usb-0000:00:14.0-2/input0"));
        }

        #[test]
        fn reconnect_match_from_config_is_none_for_legacy_path() {
            let cfg = resolved_with_match(None);
            assert!(ReconnectMatch::from_config(&cfg).is_none());
        }

        #[test]
        fn reconnect_match_human_id_is_zero_padded() {
            let cfg = resolved_with_match(Some(ParsedDeviceMatch {
                vendor_id: 0x1,
                product_id: 0x2bc,
                name: None,
                phys: None,
            }));

            let rm = ReconnectMatch::from_config(&cfg).expect("device_match present");
            assert_eq!(rm.human_id(), "0001:02bc");
            assert_eq!(rm.criteria().name, None);
            assert_eq!(rm.criteria().phys, None);
        }
    }
}
