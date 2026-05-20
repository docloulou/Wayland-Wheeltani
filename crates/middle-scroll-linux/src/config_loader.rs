use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use middle_scroll_core::CoreConfig;
use nix::unistd::{chown, Gid, Uid, User};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::cli::Args;
use crate::device_discovery::{DeviceInfo, MatchCriteria};
use crate::errors::DaemonError;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonFileConfig {
    pub device: Option<PathBuf>,
    pub device_match: Option<DeviceMatchConfig>,
    pub grab: Option<bool>,
    pub dry_run: Option<bool>,
    pub safety_timeout_seconds: Option<u64>,

    #[serde(flatten)]
    pub core: CoreFileConfig,
}

/// On-disk representation of a stable mouse identifier. Survives reboots and
/// USB renumbering because it does not embed `/dev/input/eventXX`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DeviceMatchConfig {
    /// USB vendor id as a 4-digit lowercase hexadecimal string (e.g. "046d").
    pub vendor_id: String,
    /// USB product id as a 4-digit lowercase hexadecimal string (e.g. "c539").
    pub product_id: String,
    /// Optional device name reported by the kernel; use to disambiguate two
    /// identical mice plugged into the same machine.
    pub name: Option<String>,
    /// Optional physical path (`EVIOCGPHYS`) to pin the match to a specific
    /// USB port topology.
    pub phys: Option<String>,
}

impl DeviceMatchConfig {
    pub fn from_device(device: &DeviceInfo) -> Self {
        Self {
            vendor_id: device.vendor_hex(),
            product_id: device.product_hex(),
            name: if device.name.is_empty() {
                None
            } else {
                Some(device.name.clone())
            },
            phys: device.phys.clone(),
        }
    }

    pub fn parse(&self) -> Result<ParsedDeviceMatch, DaemonError> {
        let vendor_id = parse_hex_id(&self.vendor_id).ok_or_else(|| {
            DaemonError::DeviceMatchInvalid {
                field: "vendor_id".to_owned(),
                value: self.vendor_id.clone(),
            }
        })?;
        let product_id = parse_hex_id(&self.product_id).ok_or_else(|| {
            DaemonError::DeviceMatchInvalid {
                field: "product_id".to_owned(),
                value: self.product_id.clone(),
            }
        })?;
        Ok(ParsedDeviceMatch {
            vendor_id,
            product_id,
            name: self.name.clone(),
            phys: self.phys.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ParsedDeviceMatch {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: Option<String>,
    pub phys: Option<String>,
}

impl ParsedDeviceMatch {
    pub fn as_criteria(&self) -> MatchCriteria<'_> {
        MatchCriteria {
            vendor_id: self.vendor_id,
            product_id: self.product_id,
            name: self.name.as_deref(),
            phys: self.phys.as_deref(),
        }
    }

    pub fn human_id(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }
}

fn parse_hex_id(raw: &str) -> Option<u16> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    if stripped.is_empty() || stripped.len() > 4 {
        return None;
    }
    u16::from_str_radix(stripped, 16).ok()
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
    pub device_match: Option<ParsedDeviceMatch>,
    pub grab: bool,
    pub dry_run: bool,
    pub safety_timeout_seconds: Option<u64>,
}

/// XDG-compliant config directory name. Lowercase by convention (and because
/// `directories` 5.x silently lowercases application names on Linux anyway).
pub const APP_DIR: &str = "wayland-wheeltani";
/// Pre-1.1.3 capitalised directory, kept for one-shot migration.
const LEGACY_APP_DIR: &str = "Wayland-Wheeltani";

pub fn default_config_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", APP_DIR).map(|p| p.config_dir().join("config.toml"))
}

pub fn effective_config_path(args: &Args) -> Option<PathBuf> {
    args.config
        .clone()
        .or_else(sudo_user_config_path)
        .or_else(default_config_path)
        .map(migrate_legacy_path)
}

/// If only the legacy `~/.config/Wayland-Wheeltani/` exists, use that path as
/// fallback so existing users do not silently lose their config. The next
/// `save_device_to_config` call writes to the new location and warns about
/// the duplicate.
fn migrate_legacy_path(preferred: PathBuf) -> PathBuf {
    if preferred.exists() {
        return preferred;
    }
    let legacy = legacy_sibling(&preferred);
    if legacy.as_ref().is_some_and(|p| p.exists()) {
        let legacy = legacy.expect("checked is_some_and");
        warn!(
            preferred = %preferred.display(),
            legacy = %legacy.display(),
            "loading config from legacy capitalised directory; re-run `wayland-wheeltani --setup` to migrate"
        );
        return legacy;
    }
    preferred
}

fn legacy_sibling(path: &Path) -> Option<PathBuf> {
    let parent = path.parent()?;
    if parent.file_name().and_then(|s| s.to_str()) != Some(APP_DIR) {
        return None;
    }
    let grandparent = parent.parent()?;
    Some(grandparent.join(LEGACY_APP_DIR).join(path.file_name()?))
}

pub fn resolve(args: &Args) -> Result<ResolvedConfig, DaemonError> {
    let chosen_path = effective_config_path(args);

    let file_cfg = match chosen_path.as_deref() {
        Some(p) if p.exists() => Some(load_file(p)?),
        _ => None,
    };

    let mut core = CoreConfig::default();
    let mut device = None;
    let mut device_match = None;
    let mut grab = true;
    let mut dry_run = false;
    let mut safety_timeout_seconds = None;

    if let Some(file) = file_cfg {
        core = file.core.into_core(core);
        if let Some(v) = file.device_match {
            device_match = Some(v.parse()?);
        }
        if let Some(v) = file.device {
            if device_match.is_some() {
                warn!(
                    legacy = %v.display(),
                    "config defines both `device` and `[device_match]`; using `[device_match]` for stability"
                );
            } else {
                device = Some(v);
            }
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
        device_match = None;
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
        device_match,
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

    let probed = crate::device_discovery::probe(device);
    if let Some(info) = probed.as_ref() {
        file_cfg.device_match = Some(DeviceMatchConfig::from_device(info));
        file_cfg.device = None;
    } else {
        warn!(
            device = %device.display(),
            "could not read USB ids; falling back to legacy `device` path (may break on reboot)"
        );
        file_cfg.device = Some(device.to_path_buf());
        file_cfg.device_match = None;
    }

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

    if let Some(info) = probed {
        info!(
            config = %path.display(),
            device = %device.display(),
            usb_id = %format!("{}:{}", info.vendor_hex(), info.product_hex()),
            "saved device_match to config"
        );
    } else {
        info!(
            config = %path.display(),
            device = %device.display(),
            "saved legacy device path to config"
        );
    }
    warn_about_legacy_duplicate(&path);
    Ok(path)
}

/// Emits a warning when both the new and the legacy capitalised config
/// directories exist side by side, so the user is nudged to clean up.
fn warn_about_legacy_duplicate(written: &Path) {
    if let Some(legacy) = legacy_sibling(written) {
        if legacy.exists() && legacy != written {
            warn!(
                kept = %written.display(),
                legacy = %legacy.display(),
                "found duplicate legacy config directory; safe to delete after verifying its contents"
            );
        }
    }
}

fn sudo_user_config_path() -> Option<PathBuf> {
    let sudo_user = std::env::var("SUDO_USER").ok()?;
    if sudo_user.is_empty() || sudo_user == "root" {
        return None;
    }
    let user = User::from_name(&sudo_user).ok().flatten()?;
    Some(user.dir.join(".config").join(APP_DIR).join("config.toml"))
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

    /// Path very unlikely to exist on any real Linux host, so `probe()` will
    /// always return `None` and `save_device_to_config` will fall back to the
    /// legacy `device = "..."` form (which is what the next two tests assert).
    const BOGUS_DEVICE: &str = "/dev/input/event-wheeltani-test-bogus";

    #[test]
    fn save_device_to_config_writes_minimal_toml() {
        let path = temp_config_path("minimal");
        let args = args_with_config(path.clone());

        let written = save_device_to_config(Path::new(BOGUS_DEVICE), &args).unwrap();
        assert_eq!(written, path);

        let raw = std::fs::read_to_string(&written).unwrap();
        assert!(
            raw.contains(&format!("device = \"{BOGUS_DEVICE}\"")),
            "expected legacy fallback in {raw}"
        );
        assert!(
            !raw.contains("[device_match]"),
            "no `[device_match]` should be written when probe fails: {raw}"
        );

        let loaded = load_file(&written).unwrap();
        assert_eq!(loaded.device, Some(PathBuf::from(BOGUS_DEVICE)));
        assert!(loaded.device_match.is_none());
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

        save_device_to_config(Path::new(BOGUS_DEVICE), &args).unwrap();
        let loaded = load_file(&path).unwrap();

        assert_eq!(loaded.device, Some(PathBuf::from(BOGUS_DEVICE)));
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
    fn resolve_parses_device_match_block() {
        let path = temp_config_path("device-match");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "[device_match]\nvendor_id = \"046D\"\nproduct_id = \"C539\"\nname = \"Logitech USB Receiver\"\n",
        )
        .unwrap();

        let resolved = resolve(&args_with_config(path.clone())).unwrap();
        let m = resolved.device_match.expect("device_match should parse");
        assert_eq!(m.vendor_id, 0x046d);
        assert_eq!(m.product_id, 0xc539);
        assert_eq!(m.name.as_deref(), Some("Logitech USB Receiver"));
        assert!(resolved.device.is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn resolve_rejects_invalid_device_match_hex() {
        let path = temp_config_path("device-match-bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "[device_match]\nvendor_id = \"not-hex\"\nproduct_id = \"c539\"\n",
        )
        .unwrap();

        let err = resolve(&args_with_config(path.clone())).unwrap_err();
        match err {
            DaemonError::DeviceMatchInvalid { field, value } => {
                assert_eq!(field, "vendor_id");
                assert_eq!(value, "not-hex");
            }
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn cli_device_arg_overrides_device_match() {
        let path = temp_config_path("device-match-override");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "[device_match]\nvendor_id = \"046d\"\nproduct_id = \"c539\"\n",
        )
        .unwrap();
        let mut args = args_with_config(path.clone());
        args.device = Some(PathBuf::from(BOGUS_DEVICE));

        let resolved = resolve(&args).unwrap();
        assert!(resolved.device_match.is_none());
        assert_eq!(resolved.device, Some(PathBuf::from(BOGUS_DEVICE)));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn parse_hex_id_accepts_common_forms() {
        assert_eq!(parse_hex_id("046d"), Some(0x046d));
        assert_eq!(parse_hex_id("046D"), Some(0x046d));
        assert_eq!(parse_hex_id("0x046d"), Some(0x046d));
        assert_eq!(parse_hex_id("  c539  "), Some(0xc539));
        assert_eq!(parse_hex_id("1"), Some(0x0001));
    }

    #[test]
    fn parse_hex_id_rejects_garbage() {
        assert_eq!(parse_hex_id(""), None);
        assert_eq!(parse_hex_id("zzz"), None);
        assert_eq!(parse_hex_id("12345"), None);
    }

    #[test]
    fn explicit_config_path_wins() {
        let path = temp_config_path("explicit");
        let args = args_with_config(path.clone());
        assert_eq!(effective_config_path(&args), Some(path));
    }

    #[test]
    fn legacy_sibling_swaps_directory_case() {
        let preferred = PathBuf::from("/home/user/.config/wayland-wheeltani/config.toml");
        let legacy = legacy_sibling(&preferred).unwrap();
        assert_eq!(
            legacy,
            PathBuf::from("/home/user/.config/Wayland-Wheeltani/config.toml")
        );
    }

    #[test]
    fn legacy_sibling_returns_none_for_unrelated_path() {
        let preferred = PathBuf::from("/etc/something/else/config.toml");
        assert!(legacy_sibling(&preferred).is_none());
    }

    #[test]
    fn migrate_legacy_path_prefers_existing_legacy_when_new_missing() {
        let base = std::env::temp_dir().join(format!(
            "wheeltani-migrate-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let legacy_dir = base.join(LEGACY_APP_DIR);
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("config.toml"), "device = \"/dev/null\"\n").unwrap();

        let preferred = base.join(APP_DIR).join("config.toml");
        let resolved = migrate_legacy_path(preferred);
        assert_eq!(resolved, legacy_dir.join("config.toml"));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn migrate_legacy_path_keeps_preferred_when_new_exists() {
        let base = std::env::temp_dir().join(format!(
            "wheeltani-keep-new-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let new_dir = base.join(APP_DIR);
        std::fs::create_dir_all(&new_dir).unwrap();
        std::fs::write(new_dir.join("config.toml"), "device = \"/dev/null\"\n").unwrap();
        std::fs::create_dir_all(base.join(LEGACY_APP_DIR)).unwrap();

        let preferred = new_dir.join("config.toml");
        let resolved = migrate_legacy_path(preferred.clone());
        assert_eq!(resolved, preferred);

        let _ = std::fs::remove_dir_all(&base);
    }
}
