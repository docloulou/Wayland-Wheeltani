use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};
use nix::unistd::Uid;

pub const UDEV_RULE_PATH: &str = "/etc/udev/rules.d/60-wayland-wheeltani.rules";

/// Vendor and product id of a mouse, as evdev reports them. They are the
/// same values as `[device_match]` and as the `id/vendor`/`id/product`
/// attributes udev sees, for USB and Bluetooth mice alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbId {
    pub vendor: u16,
    pub product: u16,
}

/// Writes a rule giving the logged-in user access to `/dev/uinput` and to
/// the pointer nodes of the given mice, then reloads udev.
pub fn install_for_devices(ids: &[UsbId]) -> anyhow::Result<PathBuf> {
    ensure_root()?;
    if ids.is_empty() {
        bail!("no mouse to write a udev rule for; run `--setup` first");
    }
    let rule_path = PathBuf::from(UDEV_RULE_PATH);
    write_rule(&rule_path, &render_rule(ids))?;
    reload_rules()?;
    let listed: Vec<String> = ids
        .iter()
        .map(|id| format!("{:04x}:{:04x}", id.vendor, id.product))
        .collect();
    println!(
        "Installed udev rule for {} at {}",
        listed.join(", "),
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

fn render_rule(ids: &[UsbId]) -> String {
    let mut rule = String::from(
        "# Installed by `wayland-wheeltani --install-udev-rule`; remove it with\n\
         # `sudo wayland-wheeltani --remove-udev-rule`.\n\
         ACTION!=\"add|change\", GOTO=\"wayland_wheeltani_end\"\n\
         \n\
         KERNEL==\"uinput\", SUBSYSTEM==\"misc\", TAG+=\"uaccess\", OPTIONS+=\"static_node=uinput\"\n\
         \n\
         # One line per configured mouse. Only pointer nodes are opened to the\n\
         # logged-in user: the keyboard interface of a combined receiver keeps\n\
         # its default root-only permissions.\n",
    );
    let mut seen = Vec::new();
    for id in ids {
        if seen.contains(id) {
            continue;
        }
        seen.push(*id);
        let _ = writeln!(
            rule,
            "KERNEL==\"event[0-9]*\", SUBSYSTEM==\"input\", ENV{{ID_INPUT_MOUSE}}==\"1\", \
             ENV{{ID_INPUT_KEYBOARD}}!=\"1\", ATTRS{{id/vendor}}==\"{:04x}\", \
             ATTRS{{id/product}}==\"{:04x}\", TAG+=\"uaccess\"",
            id.vendor, id.product
        );
    }
    rule.push_str("\nLABEL=\"wayland_wheeltani_end\"\n");
    rule
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
    run_udevadm(&["trigger", "--subsystem-match=misc", "--action=change"])?;
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

    const RECEIVER: UsbId = UsbId {
        vendor: 0x046d,
        product: 0xc539,
    };

    #[test]
    fn renders_targeted_uaccess_rule() {
        let rule = render_rule(&[RECEIVER]);
        assert!(rule.contains(
            "KERNEL==\"uinput\", SUBSYSTEM==\"misc\", TAG+=\"uaccess\", OPTIONS+=\"static_node=uinput\""
        ));
        assert!(rule.contains("ATTRS{id/vendor}==\"046d\", ATTRS{id/product}==\"c539\""));
        assert!(rule.contains("GOTO=\"wayland_wheeltani_end\""));
        assert!(rule.contains("LABEL=\"wayland_wheeltani_end\""));
    }

    #[test]
    fn rule_only_opens_pointer_nodes() {
        let rule = render_rule(&[RECEIVER]);
        let line = rule
            .lines()
            .find(|l| l.contains("ATTRS{id/vendor}"))
            .unwrap();
        assert!(line.contains("ENV{ID_INPUT_MOUSE}==\"1\""));
        assert!(line.contains("ENV{ID_INPUT_KEYBOARD}!=\"1\""));
        // uaccess alone grants access; a MODE would also loosen the node for
        // whoever owns it when no seat is active.
        assert!(!rule.contains("MODE="));
    }

    #[test]
    fn renders_one_line_per_distinct_mouse() {
        let other = UsbId {
            vendor: 0x1,
            product: 0x2bc,
        };
        let rule = render_rule(&[RECEIVER, other, RECEIVER]);
        assert_eq!(rule.matches("ATTRS{id/vendor}").count(), 2);
        assert!(rule.contains("ATTRS{id/vendor}==\"0001\", ATTRS{id/product}==\"02bc\""));
    }
}
