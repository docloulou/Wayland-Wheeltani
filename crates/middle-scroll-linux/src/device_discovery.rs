use std::io::Write;
use std::path::{Path, PathBuf};

use evdev::{BusType, Device, KeyCode, RelativeAxisCode};

use crate::virtual_mouse::VIRTUAL_MOUSE_NAME;

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub path: PathBuf,
    pub name: String,
    pub phys: Option<String>,
    pub unique_name: Option<String>,
    pub vendor_id: u16,
    pub product_id: u16,
    pub bus_type: u16,
    pub keys: Vec<KeyCode>,
    pub axes: Vec<RelativeAxisCode>,
}

impl DeviceInfo {
    pub fn vendor_hex(&self) -> String {
        format!("{:04x}", self.vendor_id)
    }

    pub fn product_hex(&self) -> String {
        format!("{:04x}", self.product_id)
    }
}

/// Criteria for matching a physical device across reboots. `vendor_id` and
/// `product_id` are mandatory; `name` and `phys` further narrow the match
/// when several devices share the same USB IDs (e.g. two identical mice).
#[derive(Debug, Clone)]
pub struct MatchCriteria<'a> {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: Option<&'a str>,
    pub phys: Option<&'a str>,
}

impl MatchCriteria<'_> {
    /// Returns a copy of the criteria with `phys` cleared, so the match no
    /// longer depends on the USB port topology. Used as a fallback for legacy
    /// configs that pinned a port: the mouse is then found on any port.
    #[must_use]
    pub const fn without_phys(&self) -> Self {
        Self {
            phys: None,
            ..*self
        }
    }
}

pub fn enumerate_mice() -> Vec<DeviceInfo> {
    let mut out = Vec::new();
    for (path, dev) in evdev::enumerate() {
        if let Some(info) = inspect(&path, &dev) {
            out.push(info);
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Returns the first mouse-like device whose attributes match the given
/// criteria. Stable across reboots as long as the USB device is plugged in,
/// regardless of which `/dev/input/eventXX` number the kernel assigns.
pub fn find_match(criteria: &MatchCriteria<'_>) -> Option<DeviceInfo> {
    enumerate_mice().into_iter().find(|d| matches(d, criteria))
}

/// Opens a specific evdev node and returns its `DeviceInfo`. Returns `None`
/// if the device does not look like a mouse or cannot be opened.
pub fn probe(path: &Path) -> Option<DeviceInfo> {
    let dev = Device::open(path).ok()?;
    inspect(path, &dev)
}

fn matches(dev: &DeviceInfo, criteria: &MatchCriteria<'_>) -> bool {
    if dev.vendor_id != criteria.vendor_id || dev.product_id != criteria.product_id {
        return false;
    }
    if let Some(expected) = criteria.name {
        if dev.name != expected {
            return false;
        }
    }
    if let Some(expected) = criteria.phys {
        if dev.phys.as_deref() != Some(expected) {
            return false;
        }
    }
    true
}

fn inspect(path: &Path, dev: &Device) -> Option<DeviceInfo> {
    let name = dev.name().unwrap_or("").to_owned();

    if name == VIRTUAL_MOUSE_NAME {
        return None;
    }

    let keys = dev.supported_keys()?;
    let axes = dev.supported_relative_axes()?;

    let has_btn_left = keys.contains(KeyCode::BTN_LEFT);
    let has_rel_xy =
        axes.contains(RelativeAxisCode::REL_X) && axes.contains(RelativeAxisCode::REL_Y);

    if !(has_btn_left && has_rel_xy) {
        return None;
    }

    let input_id = dev.input_id();
    Some(DeviceInfo {
        path: path.to_path_buf(),
        name,
        phys: dev.physical_path().map(str::to_owned),
        unique_name: dev.unique_name().map(str::to_owned),
        vendor_id: input_id.vendor(),
        product_id: input_id.product(),
        bus_type: input_id.bus_type().0,
        keys: collect_mouse_buttons(keys),
        axes: collect_relative_axes(axes),
    })
}

fn collect_mouse_buttons(set: &evdev::AttributeSetRef<KeyCode>) -> Vec<KeyCode> {
    const INTERESTING: &[KeyCode] = &[
        KeyCode::BTN_LEFT,
        KeyCode::BTN_RIGHT,
        KeyCode::BTN_MIDDLE,
        KeyCode::BTN_SIDE,
        KeyCode::BTN_EXTRA,
        KeyCode::BTN_FORWARD,
        KeyCode::BTN_BACK,
    ];
    INTERESTING
        .iter()
        .copied()
        .filter(|k| set.contains(*k))
        .collect()
}

fn collect_relative_axes(set: &evdev::AttributeSetRef<RelativeAxisCode>) -> Vec<RelativeAxisCode> {
    const INTERESTING: &[RelativeAxisCode] = &[
        RelativeAxisCode::REL_X,
        RelativeAxisCode::REL_Y,
        RelativeAxisCode::REL_WHEEL,
        RelativeAxisCode::REL_HWHEEL,
        RelativeAxisCode::REL_WHEEL_HI_RES,
        RelativeAxisCode::REL_HWHEEL_HI_RES,
    ];
    INTERESTING
        .iter()
        .copied()
        .filter(|a| set.contains(*a))
        .collect()
}

pub fn print_listing<W: Write>(mut writer: W, devices: &[DeviceInfo]) -> std::io::Result<()> {
    if devices.is_empty() {
        writeln!(
            writer,
            "No mouse-like input devices found under /dev/input/."
        )?;
        writeln!(
            writer,
            "Hint: you may need to run this with elevated privileges."
        )?;
        return Ok(());
    }

    writeln!(writer, "Candidate mice:")?;
    writeln!(writer)?;
    for (i, dev) in devices.iter().enumerate() {
        writeln!(writer, "[{}] {}", i + 1, dev.path.display())?;
        writeln!(writer, "    name: {}", dev.name)?;
        writeln!(
            writer,
            "    usb-id: {}:{} (bus: {})",
            dev.vendor_hex(),
            dev.product_hex(),
            BusType(dev.bus_type)
        )?;
        if let Some(p) = &dev.phys {
            writeln!(writer, "    phys: {p}")?;
        }
        if let Some(u) = &dev.unique_name {
            writeln!(writer, "    unique: {u}")?;
        }
        writeln!(writer, "    supports:")?;
        if !dev.keys.is_empty() {
            let names = dev
                .keys
                .iter()
                .map(|k| format!("{k:?}"))
                .collect::<Vec<_>>()
                .join(" ");
            writeln!(writer, "      EV_KEY: {names}")?;
        }
        if !dev.axes.is_empty() {
            let names = dev
                .axes
                .iter()
                .map(|a| format!("{a:?}"))
                .collect::<Vec<_>>()
                .join(" ");
            writeln!(writer, "      EV_REL: {names}")?;
        }
        writeln!(writer)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(phys: Option<&str>) -> DeviceInfo {
        DeviceInfo {
            path: PathBuf::from("/dev/input/event0"),
            name: "Test Mouse".to_owned(),
            phys: phys.map(str::to_owned),
            unique_name: None,
            vendor_id: 0x046d,
            product_id: 0xc539,
            bus_type: 3,
            keys: Vec::new(),
            axes: Vec::new(),
        }
    }

    #[test]
    fn matches_requires_vendor_and_product() {
        let dev = device(None);
        let criteria = MatchCriteria {
            vendor_id: 0x046d,
            product_id: 0xc539,
            name: None,
            phys: None,
        };
        assert!(matches(&dev, &criteria));

        let wrong_vendor = MatchCriteria {
            vendor_id: 0x1234,
            product_id: 0xc539,
            name: None,
            phys: None,
        };
        assert!(!matches(&dev, &wrong_vendor));
    }

    #[test]
    fn matches_ignores_port_when_phys_is_none() {
        // Mouse plugged into a port; criteria does not pin a port.
        let dev = device(Some("usb-0000:00:14.0-2/input0"));
        let criteria = MatchCriteria {
            vendor_id: 0x046d,
            product_id: 0xc539,
            name: None,
            phys: None,
        };
        assert!(matches(&dev, &criteria));
    }

    #[test]
    fn matches_rejects_different_port_when_phys_pinned() {
        let dev = device(Some("usb-0000:00:14.0-2/input0"));
        let criteria = MatchCriteria {
            vendor_id: 0x046d,
            product_id: 0xc539,
            name: None,
            phys: Some("usb-0000:00:14.0-5/input2"),
        };
        assert!(!matches(&dev, &criteria));
    }

    #[test]
    fn without_phys_clears_port_but_keeps_ids_and_name() {
        let criteria = MatchCriteria {
            vendor_id: 0x046d,
            product_id: 0xc539,
            name: Some("Test Mouse"),
            phys: Some("usb-0000:00:14.0-5/input2"),
        };
        let relaxed = criteria.without_phys();
        assert_eq!(relaxed.vendor_id, 0x046d);
        assert_eq!(relaxed.product_id, 0xc539);
        assert_eq!(relaxed.name, Some("Test Mouse"));
        assert!(relaxed.phys.is_none());

        // The relaxed criteria now matches a device on a different port.
        let dev = device(Some("usb-0000:00:14.0-2/input0"));
        assert!(matches(&dev, &relaxed));
    }
}
