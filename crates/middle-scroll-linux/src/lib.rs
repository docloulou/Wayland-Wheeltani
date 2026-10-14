//! Linux backend of Wayland-Wheeltani: evdev input, uinput output, the daemon
//! loop and the command-line front end shared by the `wayland-wheeltani` and
//! `wlw` binaries.
//!
//! Everything lives in this library so both binaries are one-line wrappers
//! around [`run`], and so integration tests can drive the virtual mouse.

#[cfg(target_os = "linux")]
mod app;
#[cfg(target_os = "linux")]
pub mod cli;
#[cfg(target_os = "linux")]
pub mod config_loader;
#[cfg(target_os = "linux")]
pub mod control;
#[cfg(target_os = "linux")]
mod daemon;
#[cfg(target_os = "linux")]
pub mod device_discovery;
#[cfg(target_os = "linux")]
pub mod errors;
#[cfg(target_os = "linux")]
pub mod event_router;
#[cfg(target_os = "linux")]
pub mod foreground;
#[cfg(target_os = "linux")]
pub mod hooks;
#[cfg(target_os = "linux")]
pub mod physical_mouse;
#[cfg(target_os = "linux")]
pub mod service;
#[cfg(target_os = "linux")]
mod tune;
#[cfg(target_os = "linux")]
pub mod udev_rule;
#[cfg(target_os = "linux")]
pub mod virtual_mouse;

/// Entry point shared by both binaries. `bin_name` is the name the binary was
/// built as; it is used in `--help`, completions and the man page.
#[cfg(target_os = "linux")]
pub fn run(bin_name: &'static str) -> anyhow::Result<()> {
    app::run(bin_name)
}

/// Entry point shared by both binaries. The daemon needs evdev and uinput, so
/// on other systems it only explains that.
#[cfg(not(target_os = "linux"))]
pub fn run(bin_name: &'static str) -> anyhow::Result<()> {
    eprintln!(
        "{bin_name}: this binary only runs on Linux (evdev/uinput).\n\
         Cross-compile or build inside a Linux VM."
    );
    std::process::exit(2);
}
