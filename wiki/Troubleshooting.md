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
sudo "$HOME/.cargo/bin/wayland-wheeltani" --remove-udev-rule
```

Both commands reload udev and apply the change to the connected devices
themselves; no `udevadm` command is needed afterwards.

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

## I want to keep the middle click and scroll with another button

Set `trigger_button = "side"` (or `"extra"`, `"forward"`, `"back"`, `"task"`)
to start autoscroll with a thumb button; the middle button then works exactly
as without the daemon. Which name matches which physical button depends on the
mouse: run `sudo evtest /dev/input/eventXX`, press the button and look for
`BTN_SIDE`, `BTN_EXTRA`, …

## A side button or the tilt wheel stopped working

The virtual mouse copies every button and axis the physical mouse has, and the
daemon forwards them unchanged. If one still goes missing, run the daemon with
`-vv` while pressing it and open an issue with the output of
`wayland-wheeltani --list-devices`.

## `--status`, `--pause` or `--tune` say "no running daemon found"

- Check that the daemon runs: `systemctl --user status wayland-wheeltani`.
- Run the command as the same user as the daemon, without `sudo`. A root
  daemon listens on `/run/wayland-wheeltani/control.sock` instead.
- A unit installed by 1.3.3 or older does not give the daemon its runtime
  directory, and the journal says `control socket unavailable`. Regenerate the
  unit with `wlw --install-service && wlw --restart`.
- `control_socket = false` in the config turns the socket off.

## A config change is not applied

The daemon reloads the file when it is saved. Check the journal:

```bash
journalctl --user -u wayland-wheeltani -n 20
```

- `config reload failed; keeping the current settings` means the new file is
  invalid; the message says which key. Fix it and save again.
- `configuration reloaded` means it worked. A gesture in progress keeps the old
  settings until the button is released.
- `grab`, `dry_run` and the `[foreground]` provider settings need
  `wayland-wheeltani --restart`.
- If the file is replaced in a way the watcher misses (some editors, a
  symlinked config), run `systemctl --user reload wayland-wheeltani` or
  `wayland-wheeltani --reload`.

## A hook does not run

- A hook is a program and its arguments, not a shell line: write
  `["sh", "-c", "…"]` to use pipes, `$VARIABLES` or `;`.
- The program must be on the service's `PATH`; use an absolute path when in
  doubt. The journal shows `failed to run hook` with the reason.
- `notify-send` needs the session bus, which the `systemd --user` service has
  and a root daemon does not.

## The foreground filter does not disable/enable the right apps

- **Check which provider was selected.** The daemon logs
  `foreground provider selected: ...` when it finds one. With
  `provider = "auto"`, `no compositor detected yet; retrying in the background`
  means nothing answered at startup; the daemon keeps looking and logs the
  provider it finds later. If nothing is ever selected, set the provider
  explicitly and check its helper below.
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
- **On KDE Plasma**, install `kdotool` and verify it with
  `kdotool getactivewindow getwindowclassname getwindowname`. If `provider = "auto"` does not
  pick KDE, set `provider = "kde"` explicitly (auto-detection needs `gdbus` to
  see KWin on the bus). See **[KDE setup](KDE-Setup)**.
- **A profile is not applied.** Scroll once in the application, then run
  `wayland-wheeltani --status`: it shows the profile of the last gesture. The
  first `[[profiles]]` entry whose `apps` match wins, so put the most specific
  ones first.
- Run with `-vv` to log each decision (`foreground decision ...`).

## Security notes

`/dev/input/event*` is sensitive. Some devices expose keyboard input through the
same kernel interface. Wayland-Wheeltani filters mouse-like devices and ignores
keyboard events, but Linux permissions still matter.

Recommended posture:

- do not run the daemon as root for daily use;
- do not add your user to the broad `input` group;
- install a udev rule that matches only your physical mouse. The rule written
  by `--install-udev-rule` opens only the mouse interfaces of the configured
  devices (`ID_INPUT_MOUSE`), never the keyboard interface that wireless
  receivers and gaming mice expose under the same USB ids. Rules installed by
  1.3.3 or older did not make that difference: run
  `sudo "$(command -v wayland-wheeltani)" --install-udev-rule` once to replace
  yours;
- use a `systemd --user` service, not a system service;
- keep services on `--no-interactive`.
