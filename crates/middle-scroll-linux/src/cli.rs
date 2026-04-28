use std::path::PathBuf;

use clap::{ArgAction, Parser};

#[derive(Debug, Parser)]
#[command(
    name = "wayland-wheeltani",
    version,
    about = "Progressive middle-button autoscroll daemon for Wayland.",
    long_about = "Holds the middle button to enter a progressive autoscroll mode whose speed \
                  follows the mouse offset from the press position. A short middle click \
                  (release inside the deadzone) is forwarded as a normal middle click."
)]
#[allow(clippy::struct_excessive_bools)]
pub struct Args {
    #[arg(long, value_name = "PATH")]
    pub device: Option<PathBuf>,

    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Interactively choose a mouse and save it to the config file; combine with --install-service to enable the user service"
    )]
    pub setup: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = [
            "list_devices",
            "remove_service",
            "remove_udev_rule",
            "start",
            "stop",
            "restart",
            "dry_run",
            "no_grab",
            "safety_timeout_seconds"
        ],
        help = "Install, enable, and start the systemd user service; combine with --setup for first-time setup"
    )]
    pub install_service: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = [
            "setup",
            "install_service",
            "install_udev_rule",
            "remove_udev_rule",
            "start",
            "stop",
            "restart",
            "list_devices",
            "device",
            "config",
            "dry_run",
            "no_grab",
            "safety_timeout_seconds"
        ],
        help = "Stop, disable, and remove the installed systemd user service"
    )]
    pub remove_service: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = [
            "install_service",
            "remove_service",
            "remove_udev_rule",
            "start",
            "stop",
            "restart",
            "list_devices",
            "dry_run",
            "no_grab",
            "safety_timeout_seconds"
        ],
        help = "Install a targeted udev rule for the selected mouse and /dev/uinput; run with sudo"
    )]
    pub install_udev_rule: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = [
            "setup",
            "install_service",
            "install_udev_rule",
            "remove_service",
            "start",
            "stop",
            "restart",
            "list_devices",
            "device",
            "config",
            "dry_run",
            "no_grab",
            "safety_timeout_seconds"
        ],
        help = "Remove the installed Wayland-Wheeltani udev rule; run with sudo"
    )]
    pub remove_udev_rule: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = [
            "setup",
            "install_service",
            "install_udev_rule",
            "remove_service",
            "remove_udev_rule",
            "stop",
            "restart",
            "list_devices",
            "device",
            "config",
            "dry_run",
            "no_grab",
            "safety_timeout_seconds"
        ],
        help = "Start the installed systemd user service"
    )]
    pub start: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = [
            "setup",
            "install_service",
            "install_udev_rule",
            "remove_service",
            "remove_udev_rule",
            "start",
            "restart",
            "list_devices",
            "device",
            "config",
            "dry_run",
            "no_grab",
            "safety_timeout_seconds"
        ],
        help = "Stop the installed systemd user service"
    )]
    pub stop: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = [
            "setup",
            "install_service",
            "install_udev_rule",
            "remove_service",
            "remove_udev_rule",
            "start",
            "stop",
            "list_devices",
            "device",
            "config",
            "dry_run",
            "no_grab",
            "safety_timeout_seconds"
        ],
        help = "Restart the installed systemd user service"
    )]
    pub restart: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        conflicts_with_all = ["device", "dry_run", "no_grab", "safety_timeout_seconds"],
    )]
    pub list_devices: bool,

    #[arg(long = "no-grab", action = ArgAction::SetTrue)]
    pub no_grab: bool,

    #[arg(long, action = ArgAction::SetTrue)]
    pub dry_run: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Never prompt interactively; fail if no device is configured"
    )]
    pub no_interactive: bool,

    #[arg(short, long, action = ArgAction::Count)]
    pub verbose: u8,

    #[arg(long, value_name = "SECONDS")]
    pub safety_timeout_seconds: Option<u64>,
}

impl Args {
    pub fn parsed() -> Self {
        Self::parse()
    }

    pub const fn log_directive(&self) -> &'static str {
        match self.verbose {
            0 => "wayland_wheeltani=info,middle_scroll_core=warn",
            1 => "wayland_wheeltani=debug,middle_scroll_core=info",
            _ => "wayland_wheeltani=trace,middle_scroll_core=trace",
        }
    }
}
