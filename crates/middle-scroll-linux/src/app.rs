//! Command-line front end: one-shot commands (setup, service, udev rule,
//! control socket, completions...) and, by default, the daemon itself.

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{bail, Context};
use tracing_subscriber::EnvFilter;

use crate::cli::Args;
use crate::config_loader::{self, ResolvedConfig};
use crate::device_discovery::{self, DeviceInfo};
use crate::errors::DaemonError;
use crate::foreground::config::{ForegroundConfig, ForegroundProviderKind};
use crate::foreground::filter::{ForegroundApp, ForegroundSnapshot};
use crate::foreground::matcher::normalize;
use crate::{control, daemon, service, tune, udev_rule};

/// Countdown shown by `--detect-foreground` so the user can focus the window
/// they want to identify before the snapshot is read.
const DETECT_COUNTDOWN_SECS: u64 = 3;

pub fn run(bin_name: &'static str) -> anyhow::Result<()> {
    let args = Args::parse_for(bin_name);

    if let Some(shell) = args.completions {
        let mut command = Args::command_for(bin_name);
        clap_complete::generate(shell, &mut command, bin_name, &mut io::stdout());
        return Ok(());
    }
    if args.man {
        clap_mangen::Man::new(Args::command_for(bin_name))
            .render(&mut io::stdout())
            .context("failed to write the man page")?;
        return Ok(());
    }

    init_tracing(&args);

    if args.tune {
        if !tune::run()? {
            std::process::exit(1);
        }
        return Ok(());
    }

    if let Some(request) = args.control_request() {
        return run_control(request);
    }

    if let Some(action) = service::requested_action(&args) {
        return service::run_action(action);
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

    let mut resolved = config_loader::resolve(&args)?;

    if args.detect_foreground {
        detect_foreground(&resolved.foreground);
        return Ok(());
    }

    if args.add_device {
        add_device(&args)?;
        if !args.install_udev_rule && !args.install_service {
            return Ok(());
        }
        resolved = config_loader::resolve(&args)?;
    } else if should_save_device(&args, &resolved) {
        let selected = match args.device.clone() {
            Some(device) => device,
            None => select_device(&args, &[])?,
        };
        let saved = config_loader::save_device_to_config(&selected, &args, false)?;
        println!(
            "Saved device {} to {}",
            selected.display(),
            saved.path.display()
        );
        if args.setup && !args.install_service && !args.install_udev_rule {
            return Ok(());
        }
        resolved = config_loader::resolve(&args)?;
    }

    if args.install_udev_rule {
        let ids = configured_usb_ids(&resolved)?;
        udev_rule::install_for_devices(&ids)?;
        return Ok(());
    }

    if args.install_service {
        service::install_user_service(&args)?;
        return Ok(());
    }

    daemon::run(&args, resolved)
}

/// The mouse is (re)chosen with `--setup`, on first run, and when
/// `--install-service`/`--install-udev-rule` come with an explicit `--device`.
fn should_save_device(args: &Args, resolved: &ResolvedConfig) -> bool {
    args.setup
        || !resolved.has_configured_device()
        || ((args.install_service || args.install_udev_rule) && args.device.is_some())
}

/// `--status`, `--pause`...: one request to the running daemon.
fn run_control(request: &str) -> anyhow::Result<()> {
    let reply = control::send(request)?;
    let (ok, message) = control::split_reply(&reply);
    if !ok {
        bail!("{message}");
    }
    if !message.is_empty() {
        println!("{message}");
    }
    Ok(())
}

/// `--add-device`: adds a mouse next to the configured ones, then asks a
/// running daemon to pick it up.
fn add_device(args: &Args) -> anyhow::Result<()> {
    let selected = if let Some(device) = args.device.clone() {
        device
    } else {
        let resolved = config_loader::resolve(args)?;
        select_device(args, &configured_paths(&resolved))?
    };
    let saved = config_loader::save_device_to_config(&selected, args, true)?;
    if !saved.changed {
        println!(
            "{} is already in {}",
            selected.display(),
            saved.path.display()
        );
        return Ok(());
    }
    println!("Added {} to {}", selected.display(), saved.path.display());
    match control::send("reload") {
        Ok(reply) => match control::split_reply(&reply) {
            (true, _) => println!("The running daemon now handles it too."),
            (false, message) => eprintln!("warning: the running daemon did not reload: {message}"),
        },
        Err(control::ClientError::NotRunning(_)) => {}
        Err(err) => eprintln!("warning: could not ask the running daemon to reload: {err}"),
    }
    Ok(())
}

/// Event nodes of the mice the config already names, so `--add-device` does
/// not offer them again.
fn configured_paths(resolved: &ResolvedConfig) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = resolved
        .device_matches
        .iter()
        .filter_map(|m| device_discovery::find_match(&m.as_criteria().without_phys(), &[]))
        .map(|info| info.path)
        .collect();
    paths.extend(resolved.device.iter().cloned());
    paths
}

/// USB ids of every configured mouse, for `--install-udev-rule`.
fn configured_usb_ids(resolved: &ResolvedConfig) -> anyhow::Result<Vec<udev_rule::UsbId>> {
    let mut ids: Vec<udev_rule::UsbId> = resolved
        .device_matches
        .iter()
        .map(|m| udev_rule::UsbId {
            vendor: m.vendor_id,
            product: m.product_id,
        })
        .collect();
    if let Some(device) = &resolved.device {
        let info = device_discovery::probe(device)
            .ok_or_else(|| DaemonError::DeviceProbe(device.clone()))?;
        ids.push(udev_rule::UsbId {
            vendor: info.vendor_id,
            product: info.product_id,
        });
    }
    if ids.is_empty() {
        return Err(DaemonError::NoDevice.into());
    }
    Ok(ids)
}

/// Picks a mouse, prompting when several are plugged in. Mice whose event
/// node is in `exclude` are not offered.
fn select_device(args: &Args, exclude: &[PathBuf]) -> anyhow::Result<PathBuf> {
    if args.no_interactive {
        return Err(DaemonError::NoDevice.into());
    }

    let devices: Vec<DeviceInfo> = device_discovery::enumerate_mice()
        .into_iter()
        .filter(|d| !exclude.contains(&d.path))
        .collect();
    match devices.len() {
        0 => Err(DaemonError::NoMiceFound.into()),
        1 => Ok(devices[0].path.clone()),
        _ if !io::stdin().is_terminal() => Err(DaemonError::NonInteractiveDeviceChoice.into()),
        _ => prompt_device_selection(&devices),
    }
}

fn prompt_device_selection(devices: &[DeviceInfo]) -> anyhow::Result<PathBuf> {
    println!("Candidate mice:\n");
    device_discovery::print_listing(io::stdout().lock(), devices)?;
    loop {
        print!("Select a device [1-{}]: ", devices.len());
        io::stdout().flush()?;

        let mut line = String::new();
        if io::stdin().read_line(&mut line)? == 0 {
            bail!("no device selected (end of input)");
        }
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

/// Logs go to stderr so they never mix with the output of `--status`,
/// `--completions` or `--man`, and journald still collects them.
fn init_tracing(args: &Args) {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(args.log_directive()));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .with_writer(io::stderr)
        .compact()
        .init();
}

/// `--detect-foreground`: start the configured foreground provider, give the
/// user a moment to focus the target window, then print the focused app's
/// identity and the exact string to drop into `deny_apps`/`allow_apps`. This
/// shares the daemon's provider-selection logic, so it works with every
/// provider. It never opens the mouse.
fn detect_foreground(cfg: &ForegroundConfig) {
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

    // Threaded providers may still be warming up: briefly poll for a Known
    // snapshot before giving up.
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
                 integrations/gnome/); on KDE Plasma, install the bundled KWin script \
                 (see integrations/kde/) or `kdotool`."
            );
        }
    }
}

/// Prints the detected window's identity fields and the candidate strings to
/// use in `deny_apps`/`allow_apps` (the same fields the matcher compares).
fn print_foreground_detection(app: &ForegroundApp) {
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

    let candidates = identifier_candidates(app);
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
    println!();
    println!("Or give it its own settings:");
    println!("  [[profiles]]");
    println!("  apps = [\"{}\"]", candidates[0]);
    println!("  max_speed_detents_per_second = 40");
}

fn identifier_candidates(app: &ForegroundApp) -> Vec<String> {
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
    candidates
}
