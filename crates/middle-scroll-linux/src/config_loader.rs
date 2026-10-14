use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use directories::ProjectDirs;
use middle_scroll_core::CoreConfig;
use nix::unistd::{chown, geteuid, Gid, Uid, User};
use serde::{Deserialize, Deserializer, Serialize};
use toml_edit::{ArrayOfTables, DocumentMut, Item, Table, Value};
use tracing::{info, warn};

use crate::cli::Args;
use crate::device_discovery::{DeviceInfo, MatchCriteria};
use crate::errors::DaemonError;
use crate::event_router::TriggerButton;
use crate::foreground::{ForegroundConfig, ForegroundFileConfig};
use crate::hooks::HooksConfig;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DaemonFileConfig {
    pub device: Option<PathBuf>,
    /// One `[device_match]` table, or several `[[device_match]]` tables to
    /// drive more than one mouse.
    #[serde(deserialize_with = "one_or_many_matches")]
    pub device_match: Option<Vec<DeviceMatchConfig>>,
    pub grab: Option<bool>,
    pub dry_run: Option<bool>,
    pub safety_timeout_seconds: Option<u64>,
    pub trigger_button: Option<TriggerButton>,
    pub control_socket: Option<bool>,
    pub watch_config: Option<bool>,

    pub foreground: Option<ForegroundFileConfig>,
    pub hooks: Option<HooksConfig>,
    pub profiles: Option<Vec<ProfileFileConfig>>,

    #[serde(flatten)]
    pub core: CoreFileConfig,
}

/// Accepts both `[device_match]` (one mouse) and `[[device_match]]` (several).
/// Going through `toml::Value` gives clearer errors than an untagged enum.
fn one_or_many_matches<'de, D>(deserializer: D) -> Result<Option<Vec<DeviceMatchConfig>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    match toml::Value::deserialize(deserializer)? {
        value @ toml::Value::Table(_) => value
            .try_into::<DeviceMatchConfig>()
            .map(|m| Some(vec![m]))
            .map_err(D::Error::custom),
        value @ toml::Value::Array(_) => value
            .try_into::<Vec<DeviceMatchConfig>>()
            .map(Some)
            .map_err(D::Error::custom),
        other => Err(D::Error::custom(format!(
            "device_match must be a [device_match] table or [[device_match]] tables, found a {}",
            other.type_str()
        ))),
    }
}

/// On-disk representation of a stable mouse identifier. Survives reboots and
/// USB renumbering because it does not embed `/dev/input/eventXX`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Builds a stable match block from a probed device. `phys` (the USB port
    /// topology) is only written when `pin_port` is set, so by default the
    /// mouse is matched by USB id and keeps working on any port.
    pub fn from_device(device: &DeviceInfo, pin_port: bool) -> Self {
        Self {
            vendor_id: device.vendor_hex(),
            product_id: device.product_hex(),
            name: if device.name.is_empty() {
                None
            } else {
                Some(device.name.clone())
            },
            phys: if pin_port { device.phys.clone() } else { None },
        }
    }

    pub fn parse(&self) -> Result<ParsedDeviceMatch, DaemonError> {
        let vendor_id =
            parse_hex_id(&self.vendor_id).ok_or_else(|| DaemonError::DeviceMatchInvalid {
                field: "vendor_id".to_owned(),
                value: self.vendor_id.clone(),
            })?;
        let product_id =
            parse_hex_id(&self.product_id).ok_or_else(|| DaemonError::DeviceMatchInvalid {
                field: "product_id".to_owned(),
                value: self.product_id.clone(),
            })?;
        Ok(ParsedDeviceMatch {
            vendor_id,
            product_id,
            name: self.name.clone(),
            phys: self.phys.clone(),
        })
    }

    /// True when both entries select the same mouse, so `--add-device` does not
    /// add it twice.
    fn same_mouse(&self, other: &Self) -> bool {
        match (self.parse(), other.parse()) {
            (Ok(a), Ok(b)) => a == b,
            _ => self == other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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

    pub axis_lock: Option<middle_scroll_core::AxisLock>,
    pub inertia: Option<bool>,
    pub inertia_half_life_ms: Option<u32>,
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
        if let Some(v) = self.axis_lock {
            base.axis_lock = v;
        }
        if let Some(v) = self.inertia {
            base.inertia = v;
        }
        if let Some(v) = self.inertia_half_life_ms {
            base.inertia_half_life_ms = v;
        }
        base
    }
}

/// `[[profiles]]` entry: engine settings that apply while one of `apps` is
/// focused. Every key not set here comes from the top level of the file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfileFileConfig {
    pub name: Option<String>,
    pub apps: Vec<String>,
    pub match_title: Option<bool>,
    #[serde(flatten)]
    pub core: CoreFileConfig,
}

/// A resolved per-application profile.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub name: String,
    pub apps: Vec<String>,
    pub match_title: bool,
    /// Keys the profile overrides.
    pub overrides: CoreFileConfig,
    /// `overrides` applied on top of the top-level settings.
    pub core: CoreConfig,
}

impl Profile {
    fn from_file(
        index: usize,
        file: ProfileFileConfig,
        base: &CoreConfig,
        default_match_title: bool,
    ) -> Result<Self, DaemonError> {
        let name = file
            .name
            .clone()
            .or_else(|| file.apps.first().cloned())
            .unwrap_or_else(|| format!("profile #{}", index + 1));
        if file.apps.iter().all(|app| app.trim().is_empty()) {
            return Err(DaemonError::ProfileInvalid {
                name,
                reason: "`apps` must list at least one application".to_owned(),
            });
        }
        let mut profile = Self {
            name,
            apps: file.apps,
            match_title: file.match_title.unwrap_or(default_match_title),
            overrides: file.core,
            core: base.clone(),
        };
        profile.rebase(base)?;
        Ok(profile)
    }

    /// Re-applies the overrides on top of new top-level settings.
    pub fn rebase(&mut self, base: &CoreConfig) -> Result<(), DaemonError> {
        let core = self.overrides.clone().into_core(base.clone());
        core.validate().map_err(|err| DaemonError::ProfileInvalid {
            name: self.name.clone(),
            reason: err.to_string(),
        })?;
        self.core = core;
        Ok(())
    }
}

/// Rebases every profile on `base`; on error nothing is changed.
pub fn rebase_profiles(
    profiles: &[Profile],
    base: &CoreConfig,
) -> Result<Vec<Profile>, DaemonError> {
    profiles
        .iter()
        .cloned()
        .map(|mut profile| profile.rebase(base).map(|()| profile))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ResolvedConfig {
    pub core: CoreConfig,
    pub device: Option<PathBuf>,
    pub device_matches: Vec<ParsedDeviceMatch>,
    pub grab: bool,
    pub dry_run: bool,
    pub safety_timeout_seconds: Option<u64>,
    pub foreground: ForegroundConfig,
    pub trigger_button: TriggerButton,
    pub control_socket: bool,
    pub watch_config: bool,
    pub profiles: Vec<Profile>,
    pub hooks: HooksConfig,
    /// The file the settings were read from (it may not exist yet).
    pub config_path: Option<PathBuf>,
}

impl ResolvedConfig {
    /// Built-in defaults, used when there is no config file.
    pub fn defaults(config_path: Option<PathBuf>) -> Self {
        Self {
            core: CoreConfig::default(),
            device: None,
            device_matches: Vec::new(),
            grab: true,
            dry_run: false,
            safety_timeout_seconds: None,
            foreground: ForegroundConfig::default(),
            trigger_button: TriggerButton::default(),
            control_socket: true,
            watch_config: true,
            profiles: Vec::new(),
            hooks: HooksConfig::default(),
            config_path,
        }
    }

    pub fn has_configured_device(&self) -> bool {
        self.device.is_some() || !self.device_matches.is_empty()
    }
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

/// Owner to give files written for `args`: under `sudo`, the default config
/// belongs to the invoking user, not to root.
pub fn config_owner(args: &Args) -> Option<(Uid, Gid)> {
    if args.config.is_none() {
        sudo_owner()
    } else {
        None
    }
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
    let config_path = effective_config_path(args);

    let file_cfg = match config_path.as_deref() {
        Some(p) if p.exists() => Some(load_file(p)?),
        _ => None,
    };

    let mut resolved = ResolvedConfig::defaults(config_path);
    let profile_files = if let Some(file) = file_cfg {
        resolved.core = file.core.into_core(resolved.core);
        if let Some(list) = file.device_match {
            resolved.device_matches = list
                .iter()
                .map(DeviceMatchConfig::parse)
                .collect::<Result<_, _>>()?;
        }
        if let Some(v) = file.device {
            if resolved.device_matches.is_empty() {
                resolved.device = Some(v);
            } else {
                warn!(
                    legacy = %v.display(),
                    "config defines both `device` and `[device_match]`; using `[device_match]` for stability"
                );
            }
        }
        if let Some(v) = file.grab {
            resolved.grab = v;
        }
        if let Some(v) = file.dry_run {
            resolved.dry_run = v;
        }
        if let Some(v) = file.safety_timeout_seconds {
            resolved.safety_timeout_seconds = Some(v);
        }
        if let Some(v) = file.trigger_button {
            resolved.trigger_button = v;
        }
        if let Some(v) = file.control_socket {
            resolved.control_socket = v;
        }
        if let Some(v) = file.watch_config {
            resolved.watch_config = v;
        }
        if let Some(fg) = file.foreground {
            resolved.foreground = fg.into_resolved();
        }
        if let Some(hooks) = file.hooks {
            resolved.hooks = hooks;
        }
        file.profiles.unwrap_or_default()
    } else {
        Vec::new()
    };

    if let Some(v) = args.device.clone() {
        resolved.device = Some(v);
        resolved.device_matches.clear();
    }
    if args.no_grab {
        resolved.grab = false;
    }
    if args.dry_run {
        resolved.dry_run = true;
    }
    if let Some(v) = args.safety_timeout_seconds {
        resolved.safety_timeout_seconds = Some(v);
    }

    resolved.core.validate()?;
    resolved.profiles = profile_files
        .into_iter()
        .enumerate()
        .map(|(index, file)| {
            Profile::from_file(index, file, &resolved.core, resolved.foreground.match_title)
        })
        .collect::<Result<_, _>>()?;

    Ok(resolved)
}

fn read_config(path: &Path) -> Result<String, DaemonError> {
    std::fs::read_to_string(path).map_err(|source| DaemonError::ConfigRead {
        path: path.to_path_buf(),
        source,
    })
}

fn parse_config(path: &Path, raw: &str) -> Result<DaemonFileConfig, DaemonError> {
    toml::from_str(raw).map_err(|source| DaemonError::ConfigParse {
        path: path.to_path_buf(),
        source,
    })
}

fn load_file(path: &Path) -> Result<DaemonFileConfig, DaemonError> {
    parse_config(path, &read_config(path)?)
}

/// Loads `path` both as typed settings (so a broken file is reported instead
/// of being overwritten) and as an editable document that keeps comments and
/// formatting.
fn load_document(path: &Path) -> Result<(DocumentMut, DaemonFileConfig), DaemonError> {
    if !path.exists() {
        return Ok((DocumentMut::new(), DaemonFileConfig::default()));
    }
    let raw = read_config(path)?;
    let typed = parse_config(path, &raw)?;
    let doc = raw
        .parse::<DocumentMut>()
        .map_err(|source| DaemonError::ConfigEdit {
            path: path.to_path_buf(),
            source,
        })?;
    Ok((doc, typed))
}

/// Result of [`save_device_to_config`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedDevice {
    pub path: PathBuf,
    /// False when the mouse was already in the config (`--add-device`).
    pub changed: bool,
}

/// Saves `device` to the config file, keeping its comments and layout.
///
/// Without `append` (`--setup`) the selected mouse replaces the configured
/// one(s). With `append` (`--add-device`) it is added next to them, turning a
/// single `[device_match]` into `[[device_match]]` tables.
pub fn save_device_to_config(
    device: &Path,
    args: &Args,
    append: bool,
) -> Result<SavedDevice, DaemonError> {
    let Some(path) = effective_config_path(args) else {
        return Err(DaemonError::NoDevice);
    };
    reject_symlinked_existing_path(&path)?;
    let (mut doc, current) = load_document(&path)?;

    let probe = crate::device_discovery::probe;
    let changed = match probe(device) {
        Some(info) => {
            let entry = DeviceMatchConfig::from_device(&info, args.pin_port);
            let usb_id = format!("{}:{}", entry.vendor_id, entry.product_id);
            let changed = if append {
                let legacy = match (&current.device, &current.device_match) {
                    (Some(legacy), None) => Some(
                        probe(legacy)
                            .map(|i| DeviceMatchConfig::from_device(&i, false))
                            .ok_or_else(|| DaemonError::DeviceProbe(legacy.clone()))?,
                    ),
                    _ => None,
                };
                let existing = current.device_match.as_deref().unwrap_or_default();
                append_device_match(&mut doc, existing, legacy, &entry)
            } else {
                set_device_match(&mut doc, &entry);
                true
            };
            if changed {
                info!(config = %path.display(), device = %device.display(), %usb_id, "saved device_match to config");
            }
            changed
        }
        None if append => return Err(DaemonError::DeviceProbe(device.to_path_buf())),
        None => {
            warn!(
                device = %device.display(),
                "could not read USB ids; falling back to legacy `device` path (may break on reboot)"
            );
            set_legacy_device(&mut doc, device);
            info!(config = %path.display(), device = %device.display(), "saved legacy device path to config");
            true
        }
    };

    if changed {
        write_document(&path, &doc, config_owner(args))?;
        warn_about_legacy_duplicate(&path);
    }
    Ok(SavedDevice { path, changed })
}

/// Replaces the configured mouse (or mice) with `entry`. An existing single
/// `[device_match]` table is updated in place to keep its comments.
fn set_device_match(doc: &mut DocumentMut, entry: &DeviceMatchConfig) {
    doc.remove("device");
    if let Some(Item::Table(table)) = doc.get_mut("device_match") {
        fill_match_table(table, entry);
        return;
    }
    doc.remove("device_match");
    doc.insert("device_match", Item::Table(match_table(entry)));
}

/// Adds `entry` to the configured mice. `legacy` is the mouse of a legacy
/// `device = "/dev/input/..."` line, converted so it is not lost. Returns
/// false when the mouse is already configured.
fn append_device_match(
    doc: &mut DocumentMut,
    existing: &[DeviceMatchConfig],
    legacy: Option<DeviceMatchConfig>,
    entry: &DeviceMatchConfig,
) -> bool {
    if existing
        .iter()
        .chain(legacy.iter())
        .any(|m| m.same_mouse(entry))
    {
        return false;
    }
    doc.remove("device");
    let mut tables = take_match_tables(doc);
    if let Some(legacy) = legacy {
        tables.push(match_table(&legacy));
    }
    tables.push(match_table(entry));
    if tables.len() == 1 {
        doc.insert("device_match", Item::Table(tables.remove(0)));
    } else {
        let mut array = ArrayOfTables::new();
        for table in tables {
            array.push(table);
        }
        doc.insert("device_match", Item::ArrayOfTables(array));
    }
    true
}

/// Removes `device_match` from `doc`, whatever form it was written in, and
/// returns its tables.
fn take_match_tables(doc: &mut DocumentMut) -> Vec<Table> {
    match doc.remove("device_match") {
        Some(Item::Table(table)) => vec![table],
        Some(Item::ArrayOfTables(array)) => array.into_iter().collect(),
        Some(Item::Value(Value::InlineTable(table))) => vec![table.into_table()],
        Some(Item::Value(Value::Array(array))) => array
            .into_iter()
            .filter_map(|value| match value {
                Value::InlineTable(table) => Some(table.into_table()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn match_table(entry: &DeviceMatchConfig) -> Table {
    let mut table = Table::new();
    fill_match_table(&mut table, entry);
    table
}

fn fill_match_table(table: &mut Table, entry: &DeviceMatchConfig) {
    set_item(
        table,
        "vendor_id",
        toml_edit::value(entry.vendor_id.as_str()),
    );
    set_item(
        table,
        "product_id",
        toml_edit::value(entry.product_id.as_str()),
    );
    for (key, value) in [("name", &entry.name), ("phys", &entry.phys)] {
        match value {
            Some(v) => set_item(table, key, toml_edit::value(v.as_str())),
            None => {
                table.remove(key);
            }
        }
    }
}

fn set_legacy_device(doc: &mut DocumentMut, device: &Path) {
    doc.remove("device_match");
    let device = device.to_string_lossy();
    set_item(
        doc.as_table_mut(),
        "device",
        toml_edit::value(device.as_ref()),
    );
}

/// Sets `key` to `item`, keeping the comment and spacing around an existing
/// plain value.
fn set_item(table: &mut Table, key: &str, mut item: Item) {
    // Replacing in place keeps the key's own decor too, which holds the
    // comment lines written above it; `Table::insert` would reset it.
    let Some(existing) = table.get_mut(key) else {
        table.insert(key, item);
        return;
    };
    if let (Item::Value(old), Item::Value(new)) = (&*existing, &mut item) {
        *new.decor_mut() = old.decor().clone();
    }
    *existing = item;
}

/// Writes the given engine keys of `base` to the top level of the config file
/// at `path`, keeping everything else (comments included) untouched. Used by
/// `--tune` to persist live changes.
pub fn save_core_keys(
    path: &Path,
    keys: &[String],
    base: &CoreConfig,
    owner: Option<(Uid, Gid)>,
) -> Result<(), DaemonError> {
    reject_symlinked_existing_path(path)?;
    let (mut doc, _) = load_document(path)?;
    let serialize_err = |source| DaemonError::ConfigSerialize {
        path: path.to_path_buf(),
        source,
    };
    let rendered = toml::Table::try_from(base).map_err(serialize_err)?;
    for key in keys {
        let Some(value) = rendered.get(key) else {
            continue;
        };
        let mut single = toml::Table::new();
        single.insert(key.clone(), value.clone());
        let snippet = toml::to_string(&single).map_err(serialize_err)?;
        let mut parsed =
            snippet
                .parse::<DocumentMut>()
                .map_err(|source| DaemonError::ConfigEdit {
                    path: path.to_path_buf(),
                    source,
                })?;
        if let Some(item) = parsed.remove(key) {
            set_item(doc.as_table_mut(), key, item);
        }
    }
    write_document(path, &doc, owner)
}

fn write_document(
    path: &Path,
    doc: &DocumentMut,
    owner: Option<(Uid, Gid)>,
) -> Result<(), DaemonError> {
    let text = doc.to_string();
    // Never write a file the daemon could not load back.
    parse_config(path, &text)?;

    let parent = path.parent().ok_or_else(|| DaemonError::ConfigPathUnsafe {
        path: path.to_path_buf(),
        reason: "config path has no parent directory".to_owned(),
    })?;
    create_dir_all_owned(parent, owner)?;
    reject_symlinked_existing_path(parent)?;
    atomic_write(path, text.as_bytes(), owner)
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

/// `create_dir_all` that hands every directory it creates to `owner`, so
/// `sudo wayland-wheeltani --setup` cannot leave a root-owned `~/.config`
/// behind for the user.
fn create_dir_all_owned(dir: &Path, owner: Option<(Uid, Gid)>) -> Result<(), DaemonError> {
    let missing: Vec<PathBuf> = dir
        .ancestors()
        .take_while(|p| !p.as_os_str().is_empty() && !p.exists())
        .map(Path::to_path_buf)
        .collect();
    std::fs::create_dir_all(dir).map_err(|source| DaemonError::ConfigWrite {
        path: dir.to_path_buf(),
        source,
    })?;
    if let Some((uid, gid)) = owner {
        for created in missing.iter().rev() {
            chown(created, Some(uid), Some(gid)).map_err(|source| {
                DaemonError::ConfigOwnership {
                    path: created.clone(),
                    source,
                }
            })?;
        }
    }
    Ok(())
}

/// Replaces `path` atomically. An existing file keeps its permissions and,
/// when written by root, its owner.
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

    let existing = std::fs::metadata(path).ok();
    let owner = owner.or_else(|| {
        if geteuid().is_root() {
            existing
                .as_ref()
                .map(|m| (Uid::from_raw(m.uid()), Gid::from_raw(m.gid())))
        } else {
            None
        }
    });

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|source| DaemonError::ConfigWrite {
                path: tmp.clone(),
                source,
            })?;
        if let Some(meta) = &existing {
            file.set_permissions(meta.permissions()).map_err(|source| {
                DaemonError::ConfigWrite {
                    path: tmp.clone(),
                    source,
                }
            })?;
        }
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
    use clap::Parser;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn args_with_config(path: &Path) -> Args {
        Args::parse_from([
            std::ffi::OsStr::new("wayland-wheeltani"),
            std::ffi::OsStr::new("--config"),
            path.as_os_str(),
        ])
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

    fn write_config(name: &str, content: &str) -> PathBuf {
        let path = temp_config_path(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    fn cleanup(path: &Path) {
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Path very unlikely to exist on any real Linux host, so `probe()` will
    /// always return `None` and `save_device_to_config` will fall back to the
    /// legacy `device = "..."` form (which is what the next tests assert).
    const BOGUS_DEVICE: &str = "/dev/input/event-wheeltani-test-bogus";

    fn synthetic_device() -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from("/dev/input/event99"),
            name: "Test Mouse".to_owned(),
            phys: Some("usb-0000:00:14.0-5/input2".to_owned()),
            unique_name: None,
            vendor_id: 0x046d,
            product_id: 0xc539,
            bus_type: 3,
            keys: Vec::new(),
            axes: Vec::new(),
        }
    }

    fn entry(vendor: &str, product: &str, name: &str) -> DeviceMatchConfig {
        DeviceMatchConfig {
            vendor_id: vendor.to_owned(),
            product_id: product.to_owned(),
            name: Some(name.to_owned()),
            phys: None,
        }
    }

    #[test]
    fn from_device_omits_phys_by_default() {
        let cfg = DeviceMatchConfig::from_device(&synthetic_device(), false);
        assert_eq!(cfg.vendor_id, "046d");
        assert_eq!(cfg.product_id, "c539");
        assert_eq!(cfg.name.as_deref(), Some("Test Mouse"));
        assert!(
            cfg.phys.is_none(),
            "phys must not be written without --pin-port: {:?}",
            cfg.phys
        );
    }

    #[test]
    fn from_device_writes_phys_when_pinned() {
        let cfg = DeviceMatchConfig::from_device(&synthetic_device(), true);
        assert_eq!(cfg.phys.as_deref(), Some("usb-0000:00:14.0-5/input2"));
    }

    #[test]
    fn save_device_to_config_writes_minimal_toml() {
        let path = temp_config_path("minimal");
        let args = args_with_config(&path);

        let saved = save_device_to_config(Path::new(BOGUS_DEVICE), &args, false).unwrap();
        assert_eq!(saved.path, path);
        assert!(saved.changed);

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains(&format!("device = \"{BOGUS_DEVICE}\"")),
            "expected legacy fallback in {raw}"
        );
        assert!(
            !raw.contains("[device_match]"),
            "no `[device_match]` should be written when probe fails: {raw}"
        );

        let loaded = load_file(&path).unwrap();
        assert_eq!(loaded.device, Some(PathBuf::from(BOGUS_DEVICE)));
        assert!(loaded.device_match.is_none());
        cleanup(&path);
    }

    #[test]
    fn save_device_to_config_preserves_existing_values_and_comments() {
        let path = write_config(
            "preserve",
            "# my mouse settings\ngrab = false\ndeadzone_units = 42 # bigger deadzone\n\n\
             [[scroll_speed_steps]]\ndistance_units = 50\nspeed_detents_per_second = 6.0\n",
        );
        let args = args_with_config(&path);

        save_device_to_config(Path::new(BOGUS_DEVICE), &args, false).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# my mouse settings"), "{raw}");
        assert!(raw.contains("# bigger deadzone"), "{raw}");

        let loaded = load_file(&path).unwrap();
        assert_eq!(loaded.device, Some(PathBuf::from(BOGUS_DEVICE)));
        assert_eq!(loaded.grab, Some(false));
        assert_eq!(loaded.core.deadzone_units, Some(42));
        assert_eq!(
            loaded.core.scroll_speed_steps.unwrap()[0].distance_units,
            50
        );
        cleanup(&path);
    }

    #[test]
    fn add_device_requires_usb_ids() {
        let path = temp_config_path("append-bogus");
        let err = save_device_to_config(Path::new(BOGUS_DEVICE), &args_with_config(&path), true)
            .unwrap_err();
        assert!(matches!(err, DaemonError::DeviceProbe(_)), "{err:?}");
        assert!(!path.exists());
    }

    #[test]
    fn set_device_match_updates_table_in_place() {
        let mut doc: DocumentMut =
            "device = \"/dev/input/event3\"\n\n# the receiver\n[device_match]\nvendor_id = \"1234\" # old\nproduct_id = \"5678\"\nphys = \"usb-1\"\n"
                .parse()
                .unwrap();
        set_device_match(&mut doc, &entry("046d", "c539", "Mouse"));
        let text = doc.to_string();
        assert!(!text.contains("device = "), "{text}");
        assert!(text.contains("# the receiver"), "{text}");
        assert!(text.contains("vendor_id = \"046d\" # old"), "{text}");
        assert!(!text.contains("phys"), "{text}");
        let parsed: DaemonFileConfig = toml::from_str(&text).unwrap();
        assert_eq!(
            parsed.device_match.unwrap(),
            vec![entry("046d", "c539", "Mouse")]
        );
    }

    #[test]
    fn append_device_match_builds_an_array_and_skips_duplicates() {
        let first = entry("046d", "c539", "Receiver");
        let second = entry("1532", "0084", "Razer");
        let mut doc: DocumentMut = "# top\n[device_match]\nvendor_id = \"046d\"\nproduct_id = \"c539\"\nname = \"Receiver\"\n"
            .parse()
            .unwrap();

        assert!(!append_device_match(
            &mut doc,
            std::slice::from_ref(&first),
            None,
            &entry("046D", "C539", "Receiver")
        ));
        assert!(append_device_match(
            &mut doc,
            std::slice::from_ref(&first),
            None,
            &second
        ));
        let text = doc.to_string();
        assert!(text.contains("# top"), "{text}");
        assert_eq!(text.matches("[[device_match]]").count(), 2, "{text}");
        let parsed: DaemonFileConfig = toml::from_str(&text).unwrap();
        assert_eq!(parsed.device_match.unwrap(), vec![first, second]);
    }

    #[test]
    fn append_device_match_converts_a_legacy_device() {
        let legacy = entry("046d", "c539", "Receiver");
        let added = entry("1532", "0084", "Razer");
        let mut doc: DocumentMut = "device = \"/dev/input/event3\"\n".parse().unwrap();
        assert!(append_device_match(
            &mut doc,
            &[],
            Some(legacy.clone()),
            &added
        ));
        let parsed: DaemonFileConfig = toml::from_str(&doc.to_string()).unwrap();
        assert!(parsed.device.is_none());
        assert_eq!(parsed.device_match.unwrap(), vec![legacy, added]);
    }

    #[test]
    fn unknown_keys_are_rejected() {
        for text in [
            "deadzone_unit = 3\n",
            "[foreground]\nenable = true\n",
            "[[profiles]]\napps = [\"firefox\"]\nmax_speed = 40\n",
            "[hooks]\non_start = \"true\"\n",
        ] {
            assert!(toml::from_str::<DaemonFileConfig>(text).is_err(), "{text}");
        }
        let ok = "deadzone_units = 3\n[[profiles]]\napps = [\"firefox\"]\ntick_hz = 120\n";
        assert!(toml::from_str::<DaemonFileConfig>(ok).is_ok());
    }

    #[test]
    fn save_core_keys_updates_values_and_keeps_comments() {
        let path = write_config(
            "save-keys",
            "# tuned by hand\ndeadzone_units = 10 # px\n\n[foreground]\nenabled = true\n",
        );
        let base = CoreConfig {
            deadzone_units: 25,
            max_speed_detents_per_second: 40.5,
            mode: middle_scroll_core::Mode::ToggleProgressive,
            ..CoreConfig::default()
        };
        save_core_keys(
            &path,
            &[
                "deadzone_units".to_owned(),
                "max_speed_detents_per_second".to_owned(),
                "mode".to_owned(),
            ],
            &base,
            None,
        )
        .unwrap();

        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("# tuned by hand"), "{raw}");
        assert!(raw.contains("deadzone_units = 25 # px"), "{raw}");
        let resolved = resolve(&args_with_config(&path)).unwrap();
        assert_eq!(resolved.core.deadzone_units, 25);
        assert!((resolved.core.max_speed_detents_per_second - 40.5).abs() < f64::EPSILON);
        assert_eq!(
            resolved.core.mode,
            middle_scroll_core::Mode::ToggleProgressive
        );
        assert!(resolved.foreground.enabled);
        cleanup(&path);
    }

    #[test]
    fn create_dir_all_owned_creates_nested_directories() {
        let base = temp_config_path("nested");
        let dir = base.parent().unwrap().join("a").join("b");
        create_dir_all_owned(&dir, None).unwrap();
        assert!(dir.is_dir());
        cleanup(&base);
    }

    #[test]
    fn resolve_loads_min_hires_units_per_event() {
        let path = write_config("hires-threshold", "min_hires_units_per_event = 30\n");
        let resolved = resolve(&args_with_config(&path)).unwrap();
        assert_eq!(resolved.core.min_hires_units_per_event, 30);
        cleanup(&path);
    }

    #[test]
    fn resolve_loads_new_engine_and_daemon_keys() {
        let path = write_config(
            "new-keys",
            "axis_lock = \"dominant\"\ninertia = true\ninertia_half_life_ms = 450\n\
             trigger_button = \"side\"\ncontrol_socket = false\nwatch_config = false\n\n\
             [hooks]\non_enter = [\"notify-send\", \"scroll\"]\n",
        );
        let resolved = resolve(&args_with_config(&path)).unwrap();
        assert_eq!(
            resolved.core.axis_lock,
            middle_scroll_core::AxisLock::Dominant
        );
        assert!(resolved.core.inertia);
        assert_eq!(resolved.core.inertia_half_life_ms, 450);
        assert_eq!(resolved.trigger_button, TriggerButton::Side);
        assert!(!resolved.control_socket);
        assert!(!resolved.watch_config);
        assert_eq!(resolved.hooks.on_enter, ["notify-send", "scroll"]);
        assert_eq!(resolved.config_path.as_deref(), Some(path.as_path()));
        cleanup(&path);
    }

    #[test]
    fn resolve_parses_device_match_block() {
        let path = write_config(
            "device-match",
            "[device_match]\nvendor_id = \"046D\"\nproduct_id = \"C539\"\nname = \"Logitech USB Receiver\"\n",
        );
        let resolved = resolve(&args_with_config(&path)).unwrap();
        assert_eq!(resolved.device_matches.len(), 1);
        let m = &resolved.device_matches[0];
        assert_eq!(m.vendor_id, 0x046d);
        assert_eq!(m.product_id, 0xc539);
        assert_eq!(m.name.as_deref(), Some("Logitech USB Receiver"));
        assert!(resolved.device.is_none());
        cleanup(&path);
    }

    #[test]
    fn resolve_parses_several_device_matches() {
        let path = write_config(
            "device-matches",
            "[[device_match]]\nvendor_id = \"046d\"\nproduct_id = \"c539\"\n\n\
             [[device_match]]\nvendor_id = \"1532\"\nproduct_id = \"0084\"\nphys = \"usb-2\"\n",
        );
        let resolved = resolve(&args_with_config(&path)).unwrap();
        assert_eq!(resolved.device_matches.len(), 2);
        assert_eq!(resolved.device_matches[1].human_id(), "1532:0084");
        assert_eq!(resolved.device_matches[1].phys.as_deref(), Some("usb-2"));
        cleanup(&path);
    }

    #[test]
    fn resolve_rejects_a_scalar_device_match() {
        let path = write_config("device-match-scalar", "device_match = \"046d:c539\"\n");
        let err = resolve(&args_with_config(&path)).unwrap_err();
        assert!(err.to_string().contains("failed to parse config"), "{err}");
        cleanup(&path);
    }

    #[test]
    fn resolve_rejects_invalid_device_match_hex() {
        let path = write_config(
            "device-match-bad",
            "[device_match]\nvendor_id = \"not-hex\"\nproduct_id = \"c539\"\n",
        );
        let err = resolve(&args_with_config(&path)).unwrap_err();
        match err {
            DaemonError::DeviceMatchInvalid { field, value } => {
                assert_eq!(field, "vendor_id");
                assert_eq!(value, "not-hex");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        cleanup(&path);
    }

    #[test]
    fn cli_device_arg_overrides_device_match() {
        let path = write_config(
            "device-match-override",
            "[device_match]\nvendor_id = \"046d\"\nproduct_id = \"c539\"\n",
        );
        let mut args = args_with_config(&path);
        args.device = Some(PathBuf::from(BOGUS_DEVICE));

        let resolved = resolve(&args).unwrap();
        assert!(resolved.device_matches.is_empty());
        assert_eq!(resolved.device, Some(PathBuf::from(BOGUS_DEVICE)));
        cleanup(&path);
    }

    #[test]
    fn profiles_inherit_top_level_settings() {
        let path = write_config(
            "profiles",
            "deadzone_units = 20\nmax_speed_detents_per_second = 30.0\n\n\
             [foreground]\nmatch_title = true\n\n\
             [[profiles]]\nname = \"browsers\"\napps = [\"firefox\", \"chromium\"]\nmax_speed_detents_per_second = 60.0\n\n\
             [[profiles]]\napps = [\"code\"]\nmatch_title = false\ninertia = true\n",
        );
        let resolved = resolve(&args_with_config(&path)).unwrap();
        assert_eq!(resolved.profiles.len(), 2);

        let browsers = &resolved.profiles[0];
        assert_eq!(browsers.name, "browsers");
        assert!(browsers.match_title);
        assert_eq!(browsers.core.deadzone_units, 20);
        assert!((browsers.core.max_speed_detents_per_second - 60.0).abs() < f64::EPSILON);

        let code = &resolved.profiles[1];
        assert_eq!(code.name, "code");
        assert!(!code.match_title);
        assert!(code.core.inertia);
        assert!((code.core.max_speed_detents_per_second - 30.0).abs() < f64::EPSILON);

        let rebased = rebase_profiles(
            &resolved.profiles,
            &CoreConfig {
                deadzone_units: 5,
                ..CoreConfig::default()
            },
        )
        .unwrap();
        assert_eq!(rebased[0].core.deadzone_units, 5);
        assert!((rebased[0].core.max_speed_detents_per_second - 60.0).abs() < f64::EPSILON);
        cleanup(&path);
    }

    #[test]
    fn profiles_are_validated() {
        let path = write_config("profile-empty", "[[profiles]]\nname = \"empty\"\n");
        match resolve(&args_with_config(&path)).unwrap_err() {
            DaemonError::ProfileInvalid { name, .. } => assert_eq!(name, "empty"),
            other => panic!("unexpected error: {other:?}"),
        }
        cleanup(&path);

        let path = write_config(
            "profile-bad",
            "[[profiles]]\napps = [\"firefox\"]\ntick_hz = 0\n",
        );
        match resolve(&args_with_config(&path)).unwrap_err() {
            DaemonError::ProfileInvalid { name, reason } => {
                assert_eq!(name, "firefox");
                assert!(reason.contains("tick_hz"), "{reason}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
        cleanup(&path);
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
        let args = args_with_config(&path);
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
