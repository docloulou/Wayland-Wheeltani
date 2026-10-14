# Configuration

## Config file and precedence

Default config path:

```text
~/.config/wayland-wheeltani/config.toml
```

Precedence is:

```text
CLI flags > config file > built-in defaults
```

See
[`examples/config.toml`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/examples/config.toml)
for every tunable option.

A running daemon reloads the file as soon as it is saved (`watch_config = true`,
the default), on `systemctl --user reload wayland-wheeltani` (SIGHUP) and on
`wayland-wheeltani --reload`. If the new file is invalid, the error is logged
and the running settings are kept. A gesture in progress finishes with the old
settings. The device list (`[device_match]`) is reloaded too; `grab` and
`dry_run` need a restart.

## CLI reference

```text
wayland-wheeltani [OPTIONS]

Setup:
  --setup                         choose a mouse interactively and save it to the config
  --pin-port                      with --setup, also pin the mouse to its current USB port
  --add-device                    choose another mouse and add it next to the saved ones
  --install-udev-rule             install the udev access rule for the configured mice and /dev/uinput (sudo)
  --remove-udev-rule              remove the installed udev rule (sudo)
  --install-service               install, enable, and start the systemd user service
  --remove-service                stop, disable, and remove the systemd user service
  --start | --stop | --restart    control the installed systemd user service

Running daemon:
  --status                        active or paused, devices, active profile
  --pause | --resume              pause or resume autoscroll; the mouse keeps working
  --toggle                        pause if active, resume if paused
  --reload                        reload the config file (same as SIGHUP)
  --tune                          change settings live, then save them

Inspection:
  --list-devices                  list the connected mice with their USB ids
  --detect-foreground             print the focused window's identity (see Foreground filter)

Running in the foreground:
  --device <PATH>                 mouse event node, e.g. /dev/input/event12
  --config <FILE>                 config file to read and write
  --no-grab                       do not grab the physical mouse exclusively (debugging)
  --dry-run                       do not create the virtual mouse; log actions only
  --no-interactive                never prompt; fail if no device is configured
  --safety-timeout-seconds <N>    exit after N seconds
  -v, --verbose                   -v: debug logs, -vv: trace logs

Generated files:
  --completions <SHELL>           print a completion script (bash, zsh, fish, elvish, powershell)
  --man                           print the man page
```

`wlw` accepts exactly the same options.

## Stable device selection across reboots and USB ports

The kernel renumbers `/dev/input/eventXX` every boot, so a config like
`device = "/dev/input/event12"` regularly breaks. The recommended fix is to let
the daemon match the mouse by its **USB vendor and product ids**, which are
stable for a given device regardless of which port it is plugged into.

Run `wayland-wheeltani --setup` and it will write a `[device_match]` block
automatically (comments and the rest of the file are kept):

```toml
[device_match]
vendor_id = "046d"
product_id = "c539"
# name = "Logitech USB Receiver"     # optional; disambiguates duplicates
# phys = "usb-0000:00:14.0-5/input2" # optional; pin to a specific USB port
```

By default the match is **port-independent**: `phys` is not written, so moving
the mouse to a different USB port keeps working with no re-setup. If you
specifically want to pin the mouse to the exact USB port it is on now (for
example to disambiguate two identical mice), run
`wayland-wheeltani --setup --pin-port`, which adds the `phys` line.

At startup the daemon enumerates `/dev/input/event*` and opens the matching
mouse. If the mouse is not plugged in yet, it waits for the device to appear
(indefinitely as a `systemd --user` service; with a short timeout for
interactive runs). Configs that pinned a `phys` line keep working: if the mouse
is no longer on the pinned port, the daemon logs a warning and falls back to
matching by USB id.

The legacy `device = "/dev/input/event..."` form still works for one-shot
overrides (or `--device <PATH>`), but new configs should prefer `[device_match]`.

Find a device's USB ids with:

```bash
wayland-wheeltani --list-devices
# the line `usb-id: vvvv:pppp` gives you the values to use
```

### Several mice

Each `[[device_match]]` table (double brackets) selects one mouse; the daemon
drives all of them with the same settings and picks up any that is plugged in
later. `--add-device` appends one for you and asks a running daemon to reload;
add `--install-udev-rule` under `sudo` so your user can open the new mouse:

```bash
sudo "$(command -v wayland-wheeltani)" --add-device --install-udev-rule
```

```toml
[[device_match]]
vendor_id = "046d"
product_id = "c539"

[[device_match]]
vendor_id = "1532"
product_id = "0084"
```

A single `[device_match]` table is the same as one `[[device_match]]` entry.

### Migrating an existing install to port-independent matching

Check whether your config pins a USB port:

```bash
grep phys ~/.config/wayland-wheeltani/config.toml
```

If a `phys = "usb-..."` line is present, the new version still works on any port
automatically (warns and falls back to USB id). To migrate cleanly and drop the
pinned port, re-run setup without `--pin-port`, then restart the service:

```bash
wayland-wheeltani --setup
wayland-wheeltani --restart
```

The udev rule matches by USB id only, so the port does not matter to it.
Reinstall it when you switch to a different mouse, and once after upgrading
from 1.3.3 or older: the new rule no longer opens the keyboard interfaces of
wireless receivers and gaming mice (see the
[CHANGELOG](https://github.com/docloulou/Wayland-Wheeltani/blob/main/CHANGELOG.md)).

```bash
sudo "$(command -v wayland-wheeltani)" --install-udev-rule
```

Verify the resolved device after restarting:

```bash
journalctl --user -u wayland-wheeltani -f
```

## Trigger button and toggle mode

```toml
trigger_button = "middle"     # middle | side | extra | forward | back | task
mode = "hold_progressive"     # hold_progressive | toggle_progressive
```

- `trigger_button` picks the button that starts autoscroll. `side` and `extra`
  are the two thumb buttons on most mice; use one of them to keep the native
  middle click (paste, open in a new tab) everywhere. A short click of the
  trigger is still sent as a normal click.
- `hold_progressive` scrolls while the button is held.
- `toggle_progressive` works like autoscroll on Windows: a short click latches
  scroll mode, the pointer then steers the speed with the button released, and
  the next click (any button) ends it without reaching the application. Holding
  and dragging still scrolls as in hold mode.

## Scroll speed

Scroll speed follows the distance from the original press point.
By default (since 1.3.2) it is a **smooth progressive curve**: the speed
interpolates continuously between `min_speed_detents_per_second` and
`max_speed_detents_per_second`, reaching the maximum `full_speed_units` past
the deadzone, shaped by `acceleration_exponent`:

```toml
min_speed_detents_per_second = 1.5
max_speed_detents_per_second = 32.0
full_speed_units = 120
acceleration_exponent = 1.6   # 1.0 = linear, >1 = soft start
```

To use a **stepped profile** instead (the built-in default up to 1.3.1), define
`[[scroll_speed_steps]]` entries; they replace the smooth curve, and the same
steps apply to vertical and horizontal autoscroll:

```toml
[[scroll_speed_steps]]
distance_units = 40
speed_detents_per_second = 4.0

[[scroll_speed_steps]]
distance_units = 80
speed_detents_per_second = 10.0
```

The last reached distance step wins. If the pointer is 90 units away from the
press point on either axis, the example above scrolls at `10.0` detents/s.
Between the deadzone and the first step (here from 11 to 39 units) the smooth
curve still applies; start the first step at `deadzone_units + 1` for a purely
stepped profile. Existing config files that already define steps keep their
exact behaviour.

## Axis lock and inertia

```toml
axis_lock = "off"             # off | dominant | first
inertia = false
inertia_half_life_ms = 300
```

- `axis_lock = "dominant"` scrolls only along the axis with the larger offset
  at each moment; `"first"` keeps the first axis that left the deadzone for the
  whole gesture. `"off"` allows diagonal scrolling.
- `inertia = true` keeps scrolling after the button is released and slows down
  exponentially: the speed halves every `inertia_half_life_ms`. Pressing the
  trigger stops the coast without clicking (hold and drag to start a new
  scroll at once); any other button or the real wheel stops it and goes
  through as usual.

## Other common options

```toml
horizontal_scroll = true
invert_vertical = false
invert_horizontal = false
deadzone_units = 10
min_hires_units_per_event = 15
```

`min_hires_units_per_event` controls how many hi-res wheel units are accumulated
before one hi-res event is emitted. The default `15` gives 8 smooth samples per
detent (`120` units) and reduces tiny synthetic event spam in apps that stutter
under high-rate scrolling.

## Controlling the running daemon

The daemon listens on `$XDG_RUNTIME_DIR/wayland-wheeltani/control.sock`
(`/run/wayland-wheeltani/control.sock` when it runs as root). Only its user can
connect. Set `control_socket = false` to turn it off.

```bash
wlw --status    # active or paused, devices, mode, active profile
wlw --pause     # autoscroll off; the mouse keeps working normally
wlw --resume
wlw --toggle
wlw --reload
```

Bind `wlw --toggle` to a keyboard shortcut to switch autoscroll off for a game
or a drawing session:

- **GNOME**: Settings → Keyboard → View and Customise Shortcuts → Custom
  Shortcuts → `+`, command `wlw --toggle`.
- **KDE Plasma**: System Settings → Keyboard → Shortcuts → Add New → Command
  or Script, command `wlw --toggle`.
- **Hyprland**: `bind = SUPER, F9, exec, wlw --toggle`
- **Sway**: `bindsym $mod+F9 exec wlw --toggle`

Pausing releases any button held by a gesture in progress, so nothing stays
stuck.

## Live tuning

`wayland-wheeltani --tune` opens a prompt connected to the running daemon:

```text
tune> get max_speed_detents_per_second
max_speed_detents_per_second = 32.0
tune> set max_speed_detents_per_second 45
max_speed_detents_per_second = 45.0
tune> set inertia true
inertia = true
tune> save
```

`set` applies at once (after the current gesture), so you can try a value with
the mouse right away. `save` writes the changed keys to the config file and
keeps its comments; `reload` drops unsaved changes; `quit` leaves the changes
live until the next reload. `help` lists the commands and the useful keys.

`get` and `set` show the top-level value, the one `save` writes. When the
profile of the last gesture overrides that key, the reply also shows the value
the profile uses, which stays in effect for its applications.

## Indicator hooks

There is no built-in on-screen indicator: Wayland gives no portable way to draw
one at the pointer. Instead, `[hooks]` runs your own commands, so each desktop
can use what it has:

```toml
[hooks]
on_enter = ["notify-send", "--expire-time=800", "Autoscroll"]
on_exit = []
on_pause = ["notify-send", "Wayland-Wheeltani paused"]
on_resume = ["notify-send", "Wayland-Wheeltani active"]
```

Each hook is a program followed by its arguments, run without a shell and in
the background, so a slow command never delays the mouse. The
`WHEELTANI_STATE` environment variable holds `scrolling`, `idle`, `paused` or
`active`. `on_exit` also runs when an inertia coast ends. An empty list
disables a hook.

Example with a Waybar `custom` module that shows the state:

```jsonc
"custom/wheeltani": {
  "exec": "cat $XDG_RUNTIME_DIR/wheeltani 2>/dev/null",
  "signal": 8
}
```

```toml
[hooks]
on_enter = ["sh", "-c", "echo $WHEELTANI_STATE > $XDG_RUNTIME_DIR/wheeltani; pkill -RTMIN+8 waybar"]
on_exit = ["sh", "-c", "echo $WHEELTANI_STATE > $XDG_RUNTIME_DIR/wheeltani; pkill -RTMIN+8 waybar"]
on_pause = ["sh", "-c", "echo $WHEELTANI_STATE > $XDG_RUNTIME_DIR/wheeltani; pkill -RTMIN+8 waybar"]
on_resume = ["sh", "-c", "echo $WHEELTANI_STATE > $XDG_RUNTIME_DIR/wheeltani; pkill -RTMIN+8 waybar"]
```

## Per-application filter and profiles

To enable or disable autoscroll depending on the focused application, or to use
different settings per application with `[[profiles]]`, see the
**[Foreground filter](Foreground-Filter)** page.
