use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("device not specified: pass --device or set `device =` in the config file")]
    NoDevice,

    #[error("no mouse-like input devices found under /dev/input")]
    NoMiceFound,

    #[error("multiple mouse-like devices found; run `wayland-wheeltani --setup` or pass --device")]
    NonInteractiveDeviceChoice,

    #[error("device path does not exist: {0}")]
    DeviceNotFound(PathBuf),

    #[error("device {path} does not look like a mouse (missing BTN_LEFT or REL_X/REL_Y)")]
    NotAMouse { path: PathBuf },

    #[error("failed to open evdev device {path}: {source}")]
    OpenDevice {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to grab device {path}: {source} (is another grabber running?)")]
    GrabFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "failed to create /dev/uinput virtual mouse: {source}\n\
         hint: ensure /dev/uinput exists (`sudo modprobe uinput`) and that you have\n\
         write access to it (run as root, or add a udev rule)."
    )]
    UinputUnavailable {
        #[source]
        source: std::io::Error,
    },

    #[error("failed to load config from {path}: {source}")]
    ConfigRead {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse config from {path}: {source}")]
    ConfigParse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("failed to serialize config for {path}: {source}")]
    ConfigSerialize {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },

    #[error("failed to write config to {path}: {source}")]
    ConfigWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("refusing unsafe config path {path}: {reason}")]
    ConfigPathUnsafe { path: PathBuf, reason: String },

    #[error("failed to set config ownership for {path}: {source}")]
    ConfigOwnership {
        path: PathBuf,
        #[source]
        source: nix::Error,
    },

    #[error("invalid config: {0}")]
    ConfigInvalid(#[from] middle_scroll_core::ConfigError),
}
