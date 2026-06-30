# Troubleshooting

## `wayland-wheeltani: command not found`

Check that Cargo installed the binary and that its bin directory is on `PATH`:

```bash
ls -l ~/.cargo/bin/wayland-wheeltani
. "$HOME/.cargo/env"
wayland-wheeltani --version

# If ~/.cargo/env is missing or does not update PATH:
export PATH="$HOME/.cargo/bin:$PATH"
hash -r
wayland-wheeltani --version
```

For a permanent fix, add one of these to `~/.bashrc`, `~/.zshrc`, or your
shell's startup file:

```bash
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
# or:
export PATH="$HOME/.cargo/bin:$PATH"
```

Avoid `sudo cargo install wayland-wheeltani`; it installs into root's Cargo
directory, not yours. If the command works as your user but fails only with
`sudo`, use the absolute Cargo binary path for udev commands:

```bash
sudo "$HOME/.cargo/bin/wayland-wheeltani" --setup --install-udev-rule
sudo udevadm control --reload-rules
sudo "$HOME/.cargo/bin/wayland-wheeltani" --remove-udev-rule
```

This is expected on systems where `sudo` resets `PATH`.

## `device not specified`

```bash
wayland-wheeltani --setup
# For services or CI, pass a device explicitly:
wayland-wheeltani --no-interactive --device /dev/input/event12
```

## `udev rule installation requires root`

Install/remove udev rules with `sudo`:

```bash
sudo "$HOME/.cargo/bin/wayland-wheeltani" --setup --install-udev-rule
sudo udevadm control --reload-rules
sudo "$HOME/.cargo/bin/wayland-wheeltani" --remove-udev-rule
```

Do not run `--install-service`, `--remove-service`, `--start`, `--stop`, or
`--restart` with `sudo`; those manage your normal user's `systemd --user`
service.

## `failed to find ID_VENDOR_ID and ID_MODEL_ID`

Automatic udev rule generation needs USB-style udev properties. Check the device:

```bash
udevadm info -q property -n /dev/input/event12
```

If the IDs are missing, install the template manually and replace the
placeholders:

```bash
sudo install -Dm644 contrib/60-wayland-wheeltani.rules /etc/udev/rules.d/60-wayland-wheeltani.rules
sudoedit /etc/udev/rules.d/60-wayland-wheeltani.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

## `failed to create /dev/uinput virtual mouse`

```bash
sudo modprobe uinput
ls -l /dev/uinput
getfacl /dev/uinput
```

## `failed to grab device ... EBUSY`

Another program has an exclusive grab. Stop any previous daemon instance or any
`evtest -g`/debugging process.

## The cursor still moves while scrolling

You are probably running with `--no-grab`. Re-enable grabbing for normal use.

## Short middle clicks become scrolls too easily

Increase `deadzone_units` in the config.

## The foreground filter does not disable/enable the right apps

- **Check which provider was selected.** The daemon logs
  `foreground provider selected: ...` at startup. If you see
  `foreground provider unsupported`, no provider matched your session.
- **Run inside your graphical session.** The `systemd --user` service does; a
  root daemon (plain `sudo`) cannot reach the compositor/session bus and will
  fall back to `unknown_policy`.
- **Use the exact identifier.** Run `wayland-wheeltani --detect-foreground` to
  print the focused window's `app_id` / `class` / `resource_class` and the exact
  string to put in `deny_apps` / `allow_apps`. App identity differs between
  Wayland (`app_id`, e.g. `org.mozilla.firefox`) and XWayland
  (`class`/`resource_class`, e.g. `firefox`) — list both spellings when in doubt.
- **On GNOME**, make sure the bundled extension is installed and enabled, and
  verify it with the `gdbus call ... GetFocused` command. See
  **[GNOME setup](GNOME-Setup)**.
- Run with `-vv` to log each decision (`foreground decision ...`).

## Security notes

`/dev/input/event*` is sensitive. Some devices expose keyboard input through the
same kernel interface. Wayland-Wheeltani filters mouse-like devices and ignores
keyboard events, but Linux permissions still matter.

Recommended posture:

- do not run the daemon as root for daily use;
- do not add your user to the broad `input` group;
- install a udev rule that matches only your physical mouse;
- use a `systemd --user` service, not a system service;
- keep services on `--no-interactive`.
