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
mod indicator;
#[cfg(target_os = "linux")]
mod physical_mouse;
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
    use crate::config_loader::{self, ResolvedConfig};
    use crate::device_discovery;
    use crate::errors::DaemonError;
    use crate::event_router::{self, RoutedEvent};
    use crate::indicator::{Indicator, NoopIndicator};
    use crate::physical_mouse::PhysicalMouse;
    use crate::virtual_mouse::VirtualMouse;

    const COMPOSITOR_SETTLE_DELAY: Duration = Duration::from_millis(200);

    pub fn run() -> anyhow::Result<()> {
        let args = Args::parsed();
        init_tracing(&args);

        if args.list_devices {
            let devices = device_discovery::enumerate_mice();
            device_discovery::print_listing(io::stdout().lock(), &devices)?;
            return Ok(());
        }

        let resolved = config_loader::resolve(&args)?;
        let device_path = if args.setup || resolved.device.is_none() {
            let selected = select_device(&args)?;
            let config_path = config_loader::save_device_to_config(&selected, &args)?;
            println!(
                "Saved device {} to {}",
                selected.display(),
                config_path.display()
            );
            if args.setup {
                return Ok(());
            }
            selected
        } else {
            resolved.device.clone().ok_or(DaemonError::NoDevice)?
        };

        run_daemon(&device_path, &resolved)
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

    fn prompt_device_selection(
        devices: &[device_discovery::DeviceInfo],
    ) -> anyhow::Result<PathBuf> {
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

        let mut physical = PhysicalMouse::open(device_path)?;
        info!(name = physical.name(), "opened physical device");

        let mut virtual_mouse = if cfg.dry_run {
            None
        } else {
            let v = VirtualMouse::new()?;
            std::thread::sleep(COMPOSITOR_SETTLE_DELAY);
            Some(v)
        };

        if cfg.grab && !cfg.dry_run {
            physical
                .grab()
                .with_context(|| format!("failed to grab device {}", device_path.display()))?;
            info!("grabbed physical device exclusively");
        } else if cfg.grab && cfg.dry_run {
            warn!("--dry-run skips grab to avoid silently capturing the mouse");
        } else {
            warn!("running without grab; the compositor will see both physical and virtual events");
        }

        let started = Instant::now();
        let safety_timeout = cfg.safety_timeout_seconds.map(Duration::from_secs);
        let tick_period = Duration::from_micros(1_000_000 / u64::from(cfg.core.tick_hz));

        let mut engine = Engine::new(cfg.core.clone());
        let mut last_tick = Instant::now();
        let mut last_state = engine.state();

        let result = run_loop(
            &mut physical,
            virtual_mouse.as_mut(),
            &mut engine,
            &mut indicator,
            &shutdown,
            tick_period,
            safety_timeout,
            started,
            cfg.dry_run,
            &mut last_tick,
            &mut last_state,
        );

        info!("shutting down");
        physical.ungrab();

        result
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
        shutdown: &Arc<AtomicBool>,
        tick_period: Duration,
        safety_timeout: Option<Duration>,
        started: Instant,
        dry_run: bool,
        last_tick: &mut Instant,
        last_state: &mut EngineState,
    ) -> anyhow::Result<()> {
        loop {
            if shutdown.load(Ordering::SeqCst) {
                info!("shutdown signal received");
                break;
            }
            if let Some(limit) = safety_timeout {
                if started.elapsed() >= limit {
                    warn!(?limit, "safety timeout reached, exiting");
                    break;
                }
            }

            let now = Instant::now();
            let until_next_tick = tick_period.saturating_sub(now.duration_since(*last_tick));
            let timeout_ms = poll_timeout_millis(until_next_tick);

            let readable = {
                let fd = physical.as_fd();
                let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
                match nix::poll::poll(&mut fds, PollTimeout::from(timeout_ms)) {
                    Ok(0) => false,
                    Ok(_) => fds[0]
                        .revents()
                        .is_some_and(|r| r.contains(PollFlags::POLLIN)),
                    Err(nix::errno::Errno::EINTR) => continue,
                    Err(err) => {
                        error!(?err, "poll failed");
                        return Err(err.into());
                    }
                }
            };

            if readable {
                process_pending_events(
                    physical,
                    virtual_mouse.as_deref_mut(),
                    engine,
                    indicator,
                    dry_run,
                    last_state,
                )?;
            }

            let now = Instant::now();
            if now.duration_since(*last_tick) >= tick_period {
                let dt = now.duration_since(*last_tick);
                *last_tick = now;
                let dt_us = dt.as_micros().min(u128::from(u64::MAX)) as u64;
                let actions = engine.process(CoreInputEvent::Tick { dt_micros: dt_us });
                emit_actions(
                    &actions,
                    virtual_mouse.as_deref_mut(),
                    dry_run,
                    indicator,
                    engine,
                    last_state,
                )?;
            }
        }

        Ok(())
    }

    fn process_pending_events(
        physical: &mut PhysicalMouse,
        mut virtual_mouse: Option<&mut VirtualMouse>,
        engine: &mut Engine,
        indicator: &mut dyn Indicator,
        dry_run: bool,
        last_state: &mut EngineState,
    ) -> anyhow::Result<()> {
        let events: Vec<evdev::InputEvent> = match physical.fetch_events() {
            Ok(iter) => iter.collect(),
            Err(err) => {
                error!(
                    ?err,
                    "fetch_events failed; device may have been disconnected"
                );
                return Err(err.into());
            }
        };

        for ev in events {
            match event_router::classify(&ev) {
                RoutedEvent::Core(core_event) => {
                    let actions = engine.process(core_event);
                    emit_actions(
                        &actions,
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
                        virtual_mouse.as_deref_mut(),
                        dry_run,
                        indicator,
                        engine,
                        last_state,
                    )?;
                }
                RoutedEvent::Ignore => {}
            }
        }
        Ok(())
    }

    #[allow(clippy::unnecessary_wraps)]
    fn emit_actions(
        actions: &[CoreAction],
        mut virtual_mouse: Option<&mut VirtualMouse>,
        dry_run: bool,
        indicator: &mut dyn Indicator,
        engine: &Engine,
        last_state: &mut EngineState,
    ) -> anyhow::Result<()> {
        let mut batch: Vec<CoreAction> = Vec::with_capacity(actions.len());

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
                    flush_batch(&mut batch, virtual_mouse.as_deref_mut(), dry_run);
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

        flush_batch(&mut batch, virtual_mouse, dry_run);

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
    }
}
