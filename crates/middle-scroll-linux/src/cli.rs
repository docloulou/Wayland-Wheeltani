use std::path::PathBuf;

use clap::{ArgAction, ArgGroup, CommandFactory, FromArgMatches, Parser};
use clap_complete::Shell;

/// Arguments that start or configure the daemon itself. Commands that only
/// talk to a running daemon (`--status`, `--pause`...) conflict with them.
const DAEMON_ARGS: [&str; 16] = [
    "setup",
    "add_device",
    "install_service",
    "remove_service",
    "install_udev_rule",
    "remove_udev_rule",
    "start",
    "stop",
    "restart",
    "list_devices",
    "device",
    "dry_run",
    "no_grab",
    "safety_timeout_seconds",
    "detect_foreground",
    "pin_port",
];

#[derive(Debug, Parser)]
#[command(
    // Replaced at runtime by the name of the binary actually invoked
    // (`wayland-wheeltani` or its `wlw` alias), see `Args::parse_for`.
    name = "wayland-wheeltani",
    version,
    about = "Progressive middle-button autoscroll daemon for Wayland.",
    long_about = "Holds the middle button to enter a progressive autoscroll mode whose speed \
                  follows the mouse offset from the press position. A short middle click \
                  (release inside the deadzone) is forwarded as a normal middle click.\n\n\
                  A running daemon can be controlled with --status, --pause, --resume, \
                  --toggle and --reload, and tuned live with --tune."
)]
#[command(group(
    ArgGroup::new("control")
        .args(["status", "pause", "resume", "toggle", "reload", "tune"])
        .multiple(false)
        .conflicts_with_all(DAEMON_ARGS)
))]
#[allow(clippy::struct_excessive_bools)]
pub struct Args {
    #[arg(
        long,
        value_name = "PATH",
        help = "Mouse event node to use (e.g. /dev/input/event5) instead of the configured mice"
    )]
    pub device: Option<PathBuf>,

    #[arg(
        long,
        value_name = "FILE",
        help = "Config file to read and write (default: ~/.config/wayland-wheeltani/config.toml)"
    )]
    pub config: Option<PathBuf>,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Interactively choose a mouse and save it to the config file; combine with --install-service to enable the user service"
    )]
    pub setup: bool,

    #[arg(
        long = "pin-port",
        action = ArgAction::SetTrue,
        help = "Pin the saved config to the current USB port (writes `phys`); by default the mouse is matched by USB id only and works on any port"
    )]
    pub pin_port: bool,

    #[arg(
        long = "add-device",
        action = ArgAction::SetTrue,
        conflicts_with_all = [
            "setup",
            "remove_service",
            "remove_udev_rule",
            "start",
            "stop",
            "restart",
            "list_devices",
            "dry_run",
            "no_grab",
            "safety_timeout_seconds",
            "detect_foreground"
        ],
        help = "Choose another mouse and add it to the config next to the ones already saved (the daemon handles all of them)"
    )]
    pub add_device: bool,

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
        help = "List the connected mice with their USB ids and event nodes"
    )]
    pub list_devices: bool,

    #[arg(
        long = "no-grab",
        action = ArgAction::SetTrue,
        help = "Do not take exclusive access to the mouse; the desktop then sees both the physical and the virtual events (debugging only)"
    )]
    pub no_grab: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Log what would be emitted instead of creating the virtual mouse; implies --no-grab"
    )]
    pub dry_run: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Never prompt interactively; fail if no device is configured"
    )]
    pub no_interactive: bool,

    #[arg(
        short,
        long,
        action = ArgAction::Count,
        help = "More logging: -v for debug, -vv for trace (RUST_LOG overrides it)"
    )]
    pub verbose: u8,

    #[arg(
        long,
        value_name = "SECONDS",
        help = "Exit after this many seconds, a safety net while trying new settings"
    )]
    pub safety_timeout_seconds: Option<u64>,

    #[arg(
        long = "detect-foreground",
        action = ArgAction::SetTrue,
        conflicts_with_all = [
            "setup",
            "install_service",
            "install_udev_rule",
            "remove_service",
            "remove_udev_rule",
            "start",
            "stop",
            "restart",
            "list_devices",
            "device",
            "dry_run",
            "no_grab",
            "safety_timeout_seconds"
        ],
        help = "Print the focused window's identity (after a short delay to focus the target app) and the exact string to put in [foreground] deny_apps/allow_apps; works with every provider"
    )]
    pub detect_foreground: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Show whether the running daemon is active or paused, its devices and the active profile"
    )]
    pub status: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Pause autoscroll in the running daemon; the mouse keeps working normally"
    )]
    pub pause: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Resume autoscroll in the running daemon"
    )]
    pub resume: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Pause the running daemon if active, resume it if paused (handy for a keyboard shortcut)"
    )]
    pub toggle: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Ask the running daemon to reload its config file (same as SIGHUP)"
    )]
    pub reload: bool,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        help = "Interactively tune the running daemon: change speed, deadzone... live, then save to the config file"
    )]
    pub tune: bool,

    #[arg(
        long,
        value_name = "SHELL",
        exclusive = true,
        help = "Print a shell completion script (bash, zsh, fish, elvish, powershell) to stdout"
    )]
    pub completions: Option<Shell>,

    #[arg(
        long,
        action = ArgAction::SetTrue,
        exclusive = true,
        help = "Print the man page (roff) to stdout"
    )]
    pub man: bool,
}

impl Args {
    /// Parses the process arguments, naming the program after `bin_name` in
    /// help and error messages.
    pub fn parse_for(bin_name: &'static str) -> Self {
        let matches = Self::command_for(bin_name).get_matches();
        Self::from_arg_matches(&matches).unwrap_or_else(|err| err.exit())
    }

    /// The clap command named after `bin_name`; also used for shell
    /// completions and the man page.
    pub fn command_for(bin_name: &'static str) -> clap::Command {
        Self::command().name(bin_name).bin_name(bin_name)
    }

    /// The control-socket request selected on the command line, if any.
    pub const fn control_request(&self) -> Option<&'static str> {
        if self.status {
            Some("status")
        } else if self.pause {
            Some("pause")
        } else if self.resume {
            Some("resume")
        } else if self.toggle {
            Some("toggle")
        } else if self.reload {
            Some("reload")
        } else {
            None
        }
    }

    pub const fn log_directive(&self) -> &'static str {
        match self.verbose {
            0 => "wayland_wheeltani=info,middle_scroll_core=warn",
            1 => "wayland_wheeltani=debug,middle_scroll_core=info",
            _ => "wayland_wheeltani=trace,middle_scroll_core=trace",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_well_formed() {
        Args::command_for("wlw").debug_assert();
    }

    #[test]
    fn control_flags_map_to_requests() {
        let args = Args::parse_from(["wlw", "--toggle"]);
        assert_eq!(args.control_request(), Some("toggle"));
        let args = Args::parse_from(["wlw"]);
        assert_eq!(args.control_request(), None);
    }

    #[test]
    fn control_flags_are_exclusive_with_daemon_flags() {
        assert!(Args::try_parse_from(["wlw", "--pause", "--resume"]).is_err());
        assert!(Args::try_parse_from(["wlw", "--status", "--setup"]).is_err());
        assert!(Args::try_parse_from(["wlw", "--tune", "--dry-run"]).is_err());
        assert!(Args::try_parse_from(["wlw", "--status", "--config", "x.toml"]).is_ok());
    }

    #[test]
    fn completions_take_a_shell() {
        let args = Args::parse_from(["wlw", "--completions", "fish"]);
        assert_eq!(args.completions, Some(Shell::Fish));
        assert!(Args::try_parse_from(["wlw", "--completions", "bash", "-v"]).is_err());
    }
}
