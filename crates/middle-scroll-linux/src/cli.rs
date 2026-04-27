use std::path::PathBuf;

use clap::{ArgAction, Parser};

#[derive(Debug, Parser)]
#[command(
    name = "wayland-wheeltani",
    version,
    about = "Progressive middle-button autoscroll daemon for Wayland.",
    long_about = "Holds the middle button to enter a progressive autoscroll mode whose speed \
                  follows the vertical mouse offset from the press position. A short middle click \
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
        help = "Interactively choose a mouse and save it to the config file, then exit"
    )]
    pub setup: bool,

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
