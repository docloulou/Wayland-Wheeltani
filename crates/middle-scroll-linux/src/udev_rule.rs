use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};
use nix::unistd::Uid;

pub const UDEV_RULE_PATH: &str = "/etc/udev/rules.d/60-wayland-wheeltani.rules";

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceIds {
    vendor: String,
    product: String,
}

pub fn install_for_device(device_path: &Path) -> anyhow::Result<PathBuf> {
    ensure_root()?;
    let ids = read_device_ids(device_path)?;
    let rule_path = PathBuf::from(UDEV_RULE_PATH);
    write_rule(&rule_path, &render_rule(&ids))?;
    reload_rules()?;
    println!(
        "Installed udev rule for {} (vendor={}, product={}) at {}",
        device_path.display(),
        ids.vendor,
        ids.product,
        rule_path.display()
    );
    Ok(rule_path)
}

pub fn remove_rule() -> anyhow::Result<PathBuf> {
    ensure_root()?;
    let rule_path = PathBuf::from(UDEV_RULE_PATH);
    if rule_path.exists() {
        std::fs::remove_file(&rule_path)
            .with_context(|| format!("failed to remove {}", rule_path.display()))?;
        reload_rules()?;
        println!("Removed udev rule at {}", rule_path.display());
    } else {
        println!("No udev rule found at {}", rule_path.display());
    }
    Ok(rule_path)
}

pub fn ensure_root() -> anyhow::Result<()> {
    if !Uid::effective().is_root() {
        bail!("udev rule management requires root; run this command with sudo");
    }
    Ok(())
}

fn read_device_ids(device_path: &Path) -> anyhow::Result<DeviceIds> {
    let output = Command::new("udevadm")
        .args(["info", "-q", "property", "-n"])
        .arg(device_path)
        .output()
        .context("failed to execute udevadm info")?;
    if !output.status.success() {
        bail!(
            "udevadm info failed for {} with {}",
            device_path.display(),
            output.status
        );
    }
    let raw = String::from_utf8(output.stdout).context("udevadm output was not valid UTF-8")?;
    parse_device_ids(&raw).with_context(|| {
        format!(
            "failed to find ID_VENDOR_ID and ID_MODEL_ID for {}; use a USB mouse or install contrib/60-wayland-wheeltani.rules manually",
            device_path.display()
        )
    })
}

fn parse_device_ids(raw: &str) -> Option<DeviceIds> {
    let props: HashMap<_, _> = raw
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();
    let vendor = normalize_hex_id(props.get("ID_VENDOR_ID")?)?;
    let product = normalize_hex_id(props.get("ID_MODEL_ID")?)?;
    Some(DeviceIds { vendor, product })
}

fn normalize_hex_id(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.len() == 4 && value.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(value.to_ascii_lowercase())
    } else {
        None
    }
}

fn render_rule(ids: &DeviceIds) -> String {
    format!(
        "ACTION!=\"add|change\", GOTO=\"wayland_wheeltani_end\"\n\
\n\
KERNEL==\"uinput\", MODE=\"0600\", TAG+=\"uaccess\"\n\
\n\
KERNEL==\"event[0-9]*\", SUBSYSTEM==\"input\", ENV{{ID_VENDOR_ID}}==\"{}\", ENV{{ID_MODEL_ID}}==\"{}\", MODE=\"0600\", TAG+=\"uaccess\"\n\
\n\
LABEL=\"wayland_wheeltani_end\"\n",
        ids.vendor, ids.product
    )
}

fn write_rule(path: &Path, contents: &str) -> anyhow::Result<()> {
    if path.exists() {
        let meta = std::fs::symlink_metadata(path)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if meta.file_type().is_symlink() {
            bail!(
                "refusing to overwrite symlinked udev rule at {}",
                path.display()
            );
        }
    }
    let parent = path.parent().context("udev rule path has no parent")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    std::fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

fn reload_rules() -> anyhow::Result<()> {
    run_udevadm(&["control", "--reload-rules"])?;
    run_udevadm(&["trigger", "--subsystem-match=input", "--action=change"])?;
    Ok(())
}

fn run_udevadm(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("udevadm")
        .args(args)
        .status()
        .context("failed to execute udevadm")?;
    if !status.success() {
        bail!("udevadm {} failed with {status}", args.join(" "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_normalizes_usb_ids() {
        let ids = parse_device_ids("ID_VENDOR_ID=046D\nID_MODEL_ID=C539\n").unwrap();
        assert_eq!(
            ids,
            DeviceIds {
                vendor: "046d".to_owned(),
                product: "c539".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_missing_or_invalid_usb_ids() {
        assert!(parse_device_ids("ID_VENDOR_ID=046d\n").is_none());
        assert!(parse_device_ids("ID_VENDOR_ID=not-hex\nID_MODEL_ID=c539\n").is_none());
    }

    #[test]
    fn renders_targeted_uaccess_rule() {
        let rule = render_rule(&DeviceIds {
            vendor: "046d".to_owned(),
            product: "c539".to_owned(),
        });
        assert!(rule.contains("KERNEL==\"uinput\", MODE=\"0600\", TAG+=\"uaccess\""));
        assert!(rule.contains("ENV{ID_VENDOR_ID}==\"046d\""));
        assert!(rule.contains("ENV{ID_MODEL_ID}==\"c539\""));
    }

    #[test]
    fn renders_named_rule_label() {
        let rule = render_rule(&DeviceIds {
            vendor: "1234".to_owned(),
            product: "abcd".to_owned(),
        });
        assert!(rule.contains("GOTO=\"wayland_wheeltani_end\""));
        assert!(rule.contains("LABEL=\"wayland_wheeltani_end\""));
    }
}
