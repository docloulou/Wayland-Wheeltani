# Configuration

## Config file and precedence

Default config path:

```text
~/.config/Wayland-Wheeltani/config.toml
```

Precedence is:

```text
CLI flags > config file > built-in defaults
```

See
[`examples/config.toml`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/examples/config.toml)
for every tunable option.

## CLI reference

```text
wayland-wheeltani [OPTIONS]

Options:
  --device <PATH>                 evdev node, e.g. /dev/input/event12
  --config <FILE>                 override config path
  --setup                         choose a mouse interactively and save config
  --pin-port                      with --setup, also pin the mouse to its current USB port
  --install-udev-rule             install udev access rule for selected mouse and /dev/uinput
  --remove-udev-rule              remove the installed Wayland-Wheeltani udev rule
  --install-service               install, enable, and start the systemd user service
  --remove-service                stop, disable, and remove the systemd user service
  --start                         start the installed systemd user service
  --stop                          stop the installed systemd user service
  --restart                       restart the installed systemd user service
  --list-devices                  list candidate mice and exit
  --detect-foreground             print the focused window's identity (see Foreground filter)
  --no-grab                       do not grab the physical mouse exclusively
  --dry-run                       do not create /dev/uinput; log actions only
  --no-interactive                never prompt; fail if no device is configured
  --safety-timeout-seconds <N>    auto-exit after N seconds
  -v, --verbose                   -v: debug logs, -vv: trace logs
```

## Stable device selection across reboots and USB ports

The kernel renumbers `/dev/input/eventXX` every boot, so a config like
`device = "/dev/input/event12"` regularly breaks. The recommended fix is to let
the daemon match the mouse by its **USB vendor and product ids**, which are
stable for a given device regardless of which port it is plugged into.

Run `wayland-wheeltani --setup` and it will write a `[device_match]` block
automatically:

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

At startup the daemon enumerates `/dev/input/event*`, finds the first match and
uses it. If the mouse is not plugged in yet, it waits for the device to appear
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

The udev rule does not need to change: it has always matched by USB id only. You
only need to reinstall it if you switch to a different physical mouse:

```bash
sudo "$(command -v wayland-wheeltani)" --setup --install-udev-rule
sudo udevadm control --reload-rules
```

Verify the resolved device after restarting:

```bash
journalctl --user -u wayland-wheeltani -f
```

## Scroll speed

Scroll speed is configured by distance from the original middle-button press
point. The same `[[scroll_speed_steps]]` apply to vertical and horizontal
autoscroll:

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

## Per-application filter

To enable or disable autoscroll depending on the focused application, see the
**[Foreground filter](Foreground-Filter)** page.
