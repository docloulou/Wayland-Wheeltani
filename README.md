[![CodeQL](https://github.com/docloulou/Wayland-Wheeltani/actions/workflows/codeql.yml/badge.svg)](https://github.com/docloulou/Wayland-Wheeltani/actions/workflows/codeql.yml)
[![License: 0BSD](https://img.shields.io/badge/license-0BSD-brightgreen.svg)](LICENSE)

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/W7W01LZHA2)

# Wayland-Wheeltani

Progressive middle-click autoscroll for Wayland.

Hold the middle mouse button, move vertically or horizontally, and
Wayland-Wheeltani emits smooth wheel events through a virtual mouse. Release the
middle button and scrolling stops immediately. A short middle click still
behaves like a normal middle click.

```text
hold middle button
  ├─ tiny movement inside deadzone       -> normal middle click on release
  ├─ move down from press position       -> continuous scroll down
  ├─ move right from press position      -> continuous horizontal scroll right
  ├─ move farther from press position    -> faster scroll
  ├─ return near press position          -> scroll slows/stops
  └─ cross the press position            -> scroll reverses on that axis
```

No GUI, no overlay, no network, no keyboard capture. The project is split into a
portable, unit-tested Rust core and a Linux backend using `evdev` + `uinput`.

## Features

| Feature | Details | Wiki |
|---|---|---|
| **Progressive autoscroll** | Speed follows the distance from the middle-button press point; a short click still behaves as a normal middle click | — |
| **Stable device matching** | Matches your mouse by USB vendor/product id, so config survives reboots and USB port changes | [Configuration](https://github.com/docloulou/Wayland-Wheeltani/wiki/Configuration) |
| **Runtime hot-reconnect** | Survives a live unplug/replug without restarting the daemon | [Configuration](https://github.com/docloulou/Wayland-Wheeltani/wiki/Configuration) |
| **Per-application foreground filter** | Turn autoscroll on/off per focused app (denylist/allowlist); off by default | [Foreground filter](https://github.com/docloulou/Wayland-Wheeltani/wiki/Foreground-Filter) |
| **Foreground providers** | Hyprland, Sway/i3, GNOME (bundled Shell extension), KDE Plasma (`kdotool`), or any custom command | [Foreground filter](https://github.com/docloulou/Wayland-Wheeltani/wiki/Foreground-Filter) · [GNOME setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/GNOME-Setup) · [KDE setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/KDE-Setup) |
| **`--detect-foreground` helper** | Prints the exact identifier to use in `deny_apps`/`allow_apps` | [Foreground filter](https://github.com/docloulou/Wayland-Wheeltani/wiki/Foreground-Filter) |
| **`systemd --user` service** | Install, start, stop, and restart the daemon as a user service | [Installation](https://github.com/docloulou/Wayland-Wheeltani/wiki/Installation) |
| **Root-free daily use** | Generates a targeted udev rule for your mouse and `/dev/uinput` | [Installation](https://github.com/docloulou/Wayland-Wheeltani/wiki/Installation) · [Troubleshooting](https://github.com/docloulou/Wayland-Wheeltani/wiki/Troubleshooting) |
| **`wlw` short CLI alias** | Same binary as `wayland-wheeltani`, shorter to type | [Installation](https://github.com/docloulou/Wayland-Wheeltani/wiki/Installation) |
| **Dry-run & verbose logging** | `--dry-run` and `-v`/`-vv` to debug without touching the virtual mouse | [Configuration](https://github.com/docloulou/Wayland-Wheeltani/wiki/Configuration) |

## Quick start

Requirements: a Linux Wayland session, a mouse on `/dev/input/eventX`,
`/dev/uinput` available (`sudo modprobe uinput`), and `systemd --user`.

```bash
cargo install wayland-wheeltani

# First-time setup: write a udev rule (so the daemon needs no root for daily
# use), then install and start the systemd --user service.
sudo "$HOME/.cargo/bin/wayland-wheeltani" --setup --install-udev-rule
sudo udevadm control --reload-rules
wayland-wheeltani --install-service
```

`cargo install` also installs `wlw` as a short alias for the exact same
binary — `wlw --start`, `wlw --setup`, etc. all work identically to
`wayland-wheeltani`.

Manage the service:

```bash
wayland-wheeltani --start | --stop | --restart
journalctl --user -u wayland-wheeltani -f
```

Full install options (release archives, building from source, cross-compiling,
uninstall) are in the **[Installation](https://github.com/docloulou/Wayland-Wheeltani/wiki/Installation)**
guide.

## Per-application on/off (foreground filter)

Optionally keep the native middle-click in some apps (a browser, a game) while
keeping autoscroll everywhere else — disabled by default:

```toml
[foreground]
enabled = true
provider = "auto"          # auto | none | hyprland | sway | gnome | kde | command
mode = "denylist"
deny_apps = ["firefox", "steam"]
```

Run `wayland-wheeltani --detect-foreground` to print the exact identifier of any
window. GNOME needs a small bundled Shell extension; KDE needs the `kdotool`
helper. See the
**[Foreground filter](https://github.com/docloulou/Wayland-Wheeltani/wiki/Foreground-Filter)**,
**[GNOME setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/GNOME-Setup)**
and **[KDE setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/KDE-Setup)**
guides.

## Documentation

Full docs live in the **[GitHub wiki](https://github.com/docloulou/Wayland-Wheeltani/wiki)**:

- [Installation](https://github.com/docloulou/Wayland-Wheeltani/wiki/Installation)
- [Configuration](https://github.com/docloulou/Wayland-Wheeltani/wiki/Configuration)
  (config file, CLI reference, stable device matching, scroll tuning)
- [Foreground filter](https://github.com/docloulou/Wayland-Wheeltani/wiki/Foreground-Filter)
- [GNOME setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/GNOME-Setup)
- [KDE setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/KDE-Setup)
- [Troubleshooting](https://github.com/docloulou/Wayland-Wheeltani/wiki/Troubleshooting)
- [Development](https://github.com/docloulou/Wayland-Wheeltani/wiki/Development)

See [`examples/config.toml`](examples/config.toml) for every tunable option and
[`CHANGELOG.md`](CHANGELOG.md) for release notes.

> The wiki pages are maintained in [`wiki/`](wiki/) and published with
> `scripts/publish-wiki.sh`.

## License

Released under the [BSD Zero Clause License](LICENSE) (0BSD) — do whatever you
want with it, no attribution required.
