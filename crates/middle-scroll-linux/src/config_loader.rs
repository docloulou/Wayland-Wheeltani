use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use middle_scroll_core::CoreConfig;
use nix::unistd::{chown, Gid, Uid, User};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::cli::Args;
use crate::errors::DaemonError;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonFileConfig {
    pub device: Option<PathBuf>,
    pub grab: Option<bool>,
    pub dry_run: Option<bool>,
    pub safety_timeout_seconds: Option<u64>,

    #[serde(flatten)]
    pub core: CoreFileConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CoreFileConfig {
    pub mode: Option<middle_scroll_core::Mode>,

    pub deadzone_units: Option<i32>,
    pub full_speed_units: Option<i32>,
    pub max_offset_units: Option<i32>,

    pub min_speed_detents_per_second: Option<f64>,
    pub max_speed_detents_per_second: Option<f64>,
    pub acceleration_exponent: Option<f64>,
    pub scroll_speed_steps: Option<Vec<middle_scroll_core::SpeedStep>>,

    pub tick_hz: Option<u32>,

    pub invert_vertical: Option<bool>,
    pub invert_horizontal: Option<bool>,

    pub suppress_motion_while_pending: Option<bool>,
    pub suppress_motion_while_scrolling: Option<bool>,
    pub replay_pending_motion_on_click: Option<bool>,

    pub emit_hires_wheel: Option<bool>,
    pub emit_legacy_wheel: Option<bool>,
    pub min_hires_units_per_event: Option<i32>,

    pub horizontal_scroll: Option<bool>,
    pub max_detents_per_tick: Option<i32>,
}

impl CoreFileConfig {
    #[allow(clippy::missing_const_for_fn)]
    pub fn into_core(self, mut base: CoreConfig) -> CoreConfig {
        if let Some(v) = self.mode {
            base.mode = v;
        }
        if let Some(v) = self.deadzone_units {
            base.deadzone_units = v;
        }
        if let Some(v) = self.full_speed_units {
            base.full_speed_units = v;
        }
        if let Some(v) = self.max_offset_units {
            base.max_offset_units = v;
        }
        if let Some(v) = self.min_speed_detents_per_second {
            base.min_speed_detents_per_second = v;
        }
        if let Some(v) = self.max_speed_detents_per_second {
            base.max_speed_detents_per_second = v;
        }
        if let Some(v) = self.acceleration_exponent {
            base.acceleration_exponent = v;
        }
        if let Some(v) = self.scroll_speed_steps {
            base.scroll_speed_steps = v;
        }
        if let Some(v) = self.tick_hz {
            base.tick_hz = v;
        }
        if let Some(v) = self.invert_vertical {
            base.invert_vertical = v;
        }
        if let Some(v) = self.invert_horizontal {
            base.invert_horizontal = v;
        }
        if let Some(v) = self.suppress_motion_while_pending {
            base.suppress_motion_while_pending = v;
        }
        if let Some(v) = self.suppress_motion_while_scrolling {
            base.suppress_motion_while_scrolling = v;
        }
        if let Some(v) = self.replay_pending_motion_on_click {
            base.replay_pending_motion_on_click = v;
        }
        if let Some(v) = self.emit_hires_wheel {
            base.emit_hires_wheel = v;
        }
        if let Some(v) = self.emit_legacy_wheel {
            base.emit_legacy_wheel = v;
        }
        if let Some(v) = self.min_hires_units_per_event {
            base.min_hires_units_per_event = v;
        }
        if let Some(v) = self.horizontal_scroll {
            base.horizontal_scroll = v;
        }
        if let Some(v) = self.max_detents_per_tick {
            base.max_detents_per_tick = v;
        }
        base
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub core: CoreConfig,
    pub device: Option<PathBuf>,
    pub grab: bool,
    pub dry_run: bool,
    pub safety_timeout_seconds: Option<u64>,
}

pub fn default_config_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "Wayland-Wheeltani").map(|p| p.config_dir().join("config.toml"))
}

pub fn effective_config_path(args: &Args) -> Option<PathBuf> {
    args.config
        .clone()
        .or_else(sudo_user_config_path)
        .or_else(default_config_path)
}

pub fn resolve(args: &Args) -> Result<ResolvedConfig, DaemonError> {
    let chosen_path = effective_config_path(args);

    let file_cfg = match chosen_path.as_deref() {
        Some(p) if p.exists() => Some(load_file(p)?),
        _ => None,
    };

    let mut core = CoreConfig::default();
    let mut device = None;
    let mut grab = true;
    let mut dry_run = false;
    let mut safety_timeout_seconds = None;

    if let Some(file) = file_cfg {
        core = file.core.into_core(core);
        if let Some(v) = file.device {
            device = Some(v);
        }
        if let Some(v) = file.grab {
            grab = v;
        }
        if let Some(v) = file.dry_run {
            dry_run = v;
        }
        if let Some(v) = file.safety_timeout_seconds {
            safety_timeout_seconds = Some(v);
        }
    }

    if let Some(v) = args.device.clone() {
        device = Some(v);
    }
    if args.no_grab {
        grab = false;
    }
    if args.dry_run {
        dry_run = true;
    }
    if let Some(v) = args.safety_timeout_seconds {
        safety_timeout_seconds = Some(v);
    }

    core.validate()?;

    Ok(ResolvedConfig {
        core,
        device,
        grab,
        dry_run,
        safety_timeout_seconds,
    })
}

fn load_file(path: &Path) -> Result<DaemonFileConfig, DaemonError> {
    let raw = std::fs::read_to_string(path).map_err(|source| DaemonError::ConfigRead {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&raw).map_err(|source| DaemonError::ConfigParse {
        path: path.to_path_buf(),
        source,
    })
}

pub fn save_device_to_config(device: &Path, args: &Args) -> Result<PathBuf, DaemonError> {
    let Some(path) = effective_config_path(args) else {
        return Err(DaemonError::NoDevice);
    };
    reject_symlinked_existing_path(&path)?;

    let mut file_cfg = if path.exists() {
        load_file(&path)?
    } else {
        DaemonFileConfig::default()
    };
    file_cfg.device = Some(device.to_path_buf());

    let parent = path.parent().ok_or_else(|| DaemonError::ConfigPathUnsafe {
        path: path.clone(),
        reason: "config path has no parent directory".to_owned(),
    })?;
    let parent_existed = parent.exists();
    std::fs::create_dir_all(parent).map_err(|source| DaemonError::ConfigWrite {
        path: parent.to_path_buf(),
        source,
    })?;
    reject_symlinked_existing_path(parent)?;

    let raw = toml::to_string_pretty(&file_cfg).map_err(|source| DaemonError::ConfigSerialize {
        path: path.clone(),
        source,
    })?;
    let owner = if args.config.is_none() {
        sudo_owner()
    } else {
        None
    };
    atomic_write(&path, raw.as_bytes(), owner)?;

    if let Some((uid, gid)) = owner {
        if !parent_existed {
            chown(parent, Some(uid), Some(gid)).map_err(|source| DaemonError::ConfigOwnership {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }

    info!(config = %path.display(), device = %device.display(), "saved device to config");
    Ok(path)
}

fn sudo_user_config_path() -> Option<PathBuf> {
    let sudo_user = std::env::var("SUDO_USER").ok()?;
    if sudo_user.is_empty() || sudo_user == "root" {
        return None;
    }
    let user = User::from_name(&sudo_user).ok().flatten()?;
    Some(
        user.dir
            .join(".config")
            .join("Wayland-Wheeltani")
            .join("config.toml"),
    )
}

fn sudo_owner() -> Option<(Uid, Gid)> {
    let uid = std::env::var("SUDO_UID").ok()?.parse::<u32>().ok()?;
    let gid = std::env::var("SUDO_GID").ok()?.parse::<u32>().ok()?;
    Some((Uid::from_raw(uid), Gid::from_raw(gid)))
}

fn reject_symlinked_existing_path(path: &Path) -> Result<(), DaemonError> {
    for ancestor in path.ancestors() {
        if !ancestor.exists() {
            continue;
        }
        let meta =
            std::fs::symlink_metadata(ancestor).map_err(|source| DaemonError::ConfigWrite {
                path: ancestor.to_path_buf(),
                source,
            })?;
        if meta.file_type().is_symlink() {
            return Err(DaemonError::ConfigPathUnsafe {
                path: ancestor.to_path_buf(),
                reason: "path component is a symbolic link".to_owned(),
            });
        }
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8], owner: Option<(Uid, Gid)>) -> Result<(), DaemonError> {
    let parent = path.parent().ok_or_else(|| DaemonError::ConfigPathUnsafe {
        path: path.to_path_buf(),
        reason: "config path has no parent directory".to_owned(),
    })?;
    let file_name =
        path.file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| DaemonError::ConfigPathUnsafe {
                path: path.to_path_buf(),
                reason: "config path has no valid file name".to_owned(),
            })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let tmp = parent.join(format!(".{file_name}.{nonce}.tmp"));

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|source| DaemonError::ConfigWrite {
                path: tmp.clone(),
                source,
            })?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|source| DaemonError::ConfigWrite {
                path: tmp.clone(),
                source,
            })?;
        if let Some((uid, gid)) = owner {
            chown(&tmp, Some(uid), Some(gid)).map_err(|source| DaemonError::ConfigOwnership {
                path: tmp.clone(),
                source,
            })?;
        }
        std::fs::rename(&tmp, path).map_err(|source| DaemonError::ConfigWrite {
            path: path.to_path_buf(),
            source,
        })
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    const fn args_with_config(path: PathBuf) -> Args {
        Args {
            device: None,
            config: Some(path),
            setup: false,
            install_service: false,
            remove_service: false,
            install_udev_rule: false,
            remove_udev_rule: false,
            start: false,
            stop: false,
            restart: false,
            list_devices: false,
            no_grab: false,
            dry_run: false,
            no_interactive: false,
            verbose: 0,
            safety_timeout_seconds: None,
        }
    }

    fn temp_config_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("wayland-wheeltani-{name}-{nonce}"))
            .join("config.toml")
    }

    #[test]
    fn save_device_to_config_writes_minimal_toml() {
        let path = temp_config_path("minimal");
        let args = args_with_config(path.clone());

        let written = save_device_to_config(Path::new("/dev/input/event12"), &args).unwrap();
        assert_eq!(written, path);

        let raw = std::fs::read_to_string(&written).unwrap();
        assert!(raw.contains("device = \"/dev/input/event12\""));

        let loaded = load_file(&written).unwrap();
        assert_eq!(loaded.device, Some(PathBuf::from("/dev/input/event12")));
        let _ = std::fs::remove_dir_all(written.parent().unwrap());
    }

    #[test]
    fn save_device_to_config_preserves_existing_values() {
        let path = temp_config_path("preserve");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "grab = false\ndeadzone_units = 42\n\n[[scroll_speed_steps]]\ndistance_units = 50\nspeed_detents_per_second = 6.0\n",
        )
        .unwrap();
        let args = args_with_config(path.clone());

        save_device_to_config(Path::new("/dev/input/event7"), &args).unwrap();
        let loaded = load_file(&path).unwrap();

        assert_eq!(loaded.device, Some(PathBuf::from("/dev/input/event7")));
        assert_eq!(loaded.grab, Some(false));
        assert_eq!(loaded.core.deadzone_units, Some(42));
        assert_eq!(
            loaded.core.scroll_speed_steps.unwrap()[0].distance_units,
            50
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn resolve_loads_min_hires_units_per_event() {
        let path = temp_config_path("hires-threshold");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "min_hires_units_per_event = 30\n").unwrap();

        let resolved = resolve(&args_with_config(path.clone())).unwrap();
        assert_eq!(resolved.core.min_hires_units_per_event, 30);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn explicit_config_path_wins() {
        let path = temp_config_path("explicit");
        let args = args_with_config(path.clone());
        assert_eq!(effective_config_path(&args), Some(path));
    }
}
