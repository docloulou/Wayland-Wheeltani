use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context};
use directories::ProjectDirs;
use nix::unistd::Uid;

use crate::cli::Args;
use crate::config_loader;

pub const SERVICE_NAME: &str = "wayland-wheeltani.service";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
    Remove,
}

pub const fn requested_action(args: &Args) -> Option<ServiceAction> {
    if args.start {
        Some(ServiceAction::Start)
    } else if args.stop {
        Some(ServiceAction::Stop)
    } else if args.restart {
        Some(ServiceAction::Restart)
    } else if args.remove_service {
        Some(ServiceAction::Remove)
    } else {
        None
    }
}

pub fn run_action(action: ServiceAction) -> anyhow::Result<()> {
    ensure_user_service_context()?;
    if action == ServiceAction::Remove {
        remove_user_service()?;
        return Ok(());
    }

    let (action_name, past_tense) = match action {
        ServiceAction::Start => ("start", "Started"),
        ServiceAction::Stop => ("stop", "Stopped"),
        ServiceAction::Restart => ("restart", "Restarted"),
        ServiceAction::Remove => unreachable!(),
    };
    systemctl_user(&[action_name, SERVICE_NAME])?;
    println!("{past_tense} {SERVICE_NAME}");
    Ok(())
}

pub fn install_user_service(args: &Args) -> anyhow::Result<PathBuf> {
    ensure_user_service_context()?;

    let unit_path = user_service_path()?;
    let unit_dir = unit_path
        .parent()
        .context("systemd user service path has no parent")?;
    std::fs::create_dir_all(unit_dir)
        .with_context(|| format!("failed to create {}", unit_dir.display()))?;

    let exe = std::env::current_exe().context("failed to resolve current executable path")?;
    let config_path = config_loader::effective_config_path(args)
        .context("failed to resolve default config path")?;
    let unit = render_user_service(&exe, &config_path)?;
    std::fs::write(&unit_path, unit)
        .with_context(|| format!("failed to write {}", unit_path.display()))?;

    systemctl_user(&["daemon-reload"])?;
    systemctl_user(&["enable", "--now", SERVICE_NAME])?;

    println!(
        "Installed and started {SERVICE_NAME} at {}",
        unit_path.display()
    );
    Ok(unit_path)
}

fn remove_user_service() -> anyhow::Result<()> {
    let unit_path = user_service_path()?;

    let status = Command::new("systemctl")
        .arg("--user")
        .args(["disable", "--now", SERVICE_NAME])
        .status()
        .context("failed to execute systemctl --user")?;
    if !status.success() {
        eprintln!(
            "Warning: systemctl --user disable --now {SERVICE_NAME} failed with {status}; continuing removal"
        );
    }

    if unit_path.exists() {
        std::fs::remove_file(&unit_path)
            .with_context(|| format!("failed to remove {}", unit_path.display()))?;
        println!("Removed {}", unit_path.display());
    } else {
        println!("No installed user service found at {}", unit_path.display());
    }

    systemctl_user(&["daemon-reload"])?;
    let status = Command::new("systemctl")
        .arg("--user")
        .args(["reset-failed", SERVICE_NAME])
        .status()
        .context("failed to execute systemctl --user")?;
    if !status.success() {
        eprintln!("Warning: systemctl --user reset-failed {SERVICE_NAME} failed with {status}");
    }
    Ok(())
}

fn ensure_user_service_context() -> anyhow::Result<()> {
    if Uid::effective().is_root() {
        bail!("refusing to manage a systemd user service as root; run this command without sudo");
    }
    Ok(())
}

fn user_service_path() -> anyhow::Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "Wayland-Wheeltani")
        .context("failed to resolve user config directory")?;
    let config_dir = dirs.config_dir();
    let user_config_root = config_dir
        .parent()
        .context("failed to resolve ~/.config directory")?;
    Ok(user_config_root
        .join("systemd")
        .join("user")
        .join(SERVICE_NAME))
}

fn render_user_service(exe: &Path, config_path: &Path) -> anyhow::Result<String> {
    let config_dir = config_path
        .parent()
        .context("config path has no parent directory")?;
    Ok(format!(
        "[Unit]\n\
Description=Progressive middle-button autoscroll daemon for Wayland\n\
Documentation=https://github.com/lucas/Wayland-Wheeltani\n\
After=graphical-session.target\n\
PartOf=graphical-session.target\n\
\n\
[Service]\n\
Type=simple\n\
ExecStart={} --no-interactive --config {}\n\
Restart=on-failure\n\
RestartSec=2\n\
PrivateTmp=true\n\
NoNewPrivileges=true\n\
ProtectSystem=strict\n\
ProtectHome=read-only\n\
ReadWritePaths={}\n\
\n\
[Install]\n\
WantedBy=default.target\n",
        quote_systemd_arg(exe),
        quote_systemd_arg(config_path),
        quote_systemd_arg(config_dir)
    ))
}

fn quote_systemd_arg(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if raw
        .chars()
        .all(|c| !c.is_whitespace() && c != '"' && c != '\\')
    {
        raw.into_owned()
    } else {
        format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

fn systemctl_user(args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("systemctl")
        .arg("--user")
        .args(args)
        .status()
        .context("failed to execute systemctl --user")?;
    if !status.success() {
        bail!("systemctl --user {} failed with {status}", args.join(" "));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_unit_uses_current_binary_and_config() {
        let unit = render_user_service(
            Path::new("/home/me/.cargo/bin/wayland-wheeltani"),
            Path::new("/home/me/.config/Wayland-Wheeltani/config.toml"),
        )
        .unwrap();

        assert!(unit.contains(
            "ExecStart=/home/me/.cargo/bin/wayland-wheeltani --no-interactive --config /home/me/.config/Wayland-Wheeltani/config.toml"
        ));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("WantedBy=default.target"));
    }

    #[test]
    fn service_unit_quotes_paths_with_spaces() {
        let unit = render_user_service(
            Path::new("/home/me/bin with spaces/wayland-wheeltani"),
            Path::new("/home/me/config with spaces/config.toml"),
        )
        .unwrap();

        assert!(unit.contains(
            "ExecStart=\"/home/me/bin with spaces/wayland-wheeltani\" --no-interactive --config \"/home/me/config with spaces/config.toml\""
        ));
        assert!(unit.contains("ReadWritePaths=\"/home/me/config with spaces\""));
    }

    #[test]
    fn requested_action_detects_remove_service() {
        let args = Args {
            device: None,
            config: None,
            setup: false,
            install_service: false,
            remove_service: true,
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
        };

        assert_eq!(requested_action(&args), Some(ServiceAction::Remove));
    }
}
