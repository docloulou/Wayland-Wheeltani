use std::io::Write;
use std::path::{Path, PathBuf};

use evdev::{Device, KeyCode, RelativeAxisCode};

use crate::virtual_mouse::VIRTUAL_MOUSE_NAME;

#[derive(Debug)]
pub struct DeviceInfo {
    pub path: PathBuf,
    pub name: String,
    pub phys: Option<String>,
    pub keys: Vec<KeyCode>,
    pub axes: Vec<RelativeAxisCode>,
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

    Some(DeviceInfo {
        path: path.to_path_buf(),
        name,
        phys: dev.physical_path().map(str::to_owned),
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
        if let Some(p) = &dev.phys {
            writeln!(writer, "    phys: {p}")?;
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
