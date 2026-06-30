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

## Documentation

- **[Installation](Installation)** — install with Cargo or a release archive,
  set up the udev rule and the `systemd --user` service, uninstall.
- **[Configuration](Configuration)** — config file, CLI reference, stable device
  selection (`[device_match]`), scroll-speed tuning.
- **[Foreground filter](Foreground-Filter)** — turn autoscroll on/off per
  application (denylist/allowlist), providers, and `--detect-foreground`.
- **[GNOME setup](GNOME-Setup)** — install and verify the bundled GNOME Shell
  extension used by the `gnome` provider.
- **[Troubleshooting](Troubleshooting)** — common errors, the foreground filter,
  and security notes.
- **[Development](Development)** — workspace layout, how it works, and how to
  build/verify from source.

## Quick start

```bash
cargo install wayland-wheeltani

# First-time setup (writes a udev rule, then installs the user service):
sudo "$HOME/.cargo/bin/wayland-wheeltani" --setup --install-udev-rule
sudo udevadm control --reload-rules
wayland-wheeltani --install-service
```

See **[Installation](Installation)** for the full flow (including release
archives and building from source).

## License

Released under the **BSD Zero Clause License (0BSD)** — do whatever you want
with it, no attribution required. See the
[`LICENSE`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/LICENSE)
file and the
[`CHANGELOG`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/CHANGELOG.md).
