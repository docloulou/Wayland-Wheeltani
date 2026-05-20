# Wayland-Wheeltani 1.1.3

This release fixes the long-standing "`/dev/input/eventXX` changes on every reboot"
issue and resolves a configuration directory naming inconsistency that could
silently scatter your settings across two locations.

## Highlights

- **Stable mouse matching across reboots.** Configure your mouse once and never
  worry about USB renumbering again — the daemon now resolves the live evdev
  node from stable USB ids declared in your config.
- **Single, lowercase config directory.** Everything now lives in
  `~/.config/wayland-wheeltani/` regardless of whether commands run under
  `sudo`. The old capitalised directory is still read transparently.
- **More informative `--list-devices` output.** USB ids and bus type are
  printed next to each candidate so picking the right device is straightforward.

## What's New

### `[device_match]` — stable device selection across reboots

Add a `[device_match]` block to your `config.toml` (or just re-run
`wayland-wheeltani --setup` and it does it for you):

```toml
[device_match]
vendor_id = "046d"
product_id = "c539"
# name = "Logitech USB Receiver"       # optional, disambiguates duplicate mice
# phys = "usb-0000:00:14.0-5/input2"   # optional, pin to a specific USB port
```

At startup the daemon enumerates `/dev/input/event*`, finds the first node
matching your `vendor_id`/`product_id` and uses it — no matter which number the
kernel assigned this boot.

If the mouse is not yet enumerated when the daemon starts (e.g. `systemd --user`
launching before USB devices come up), the daemon now waits up to **10 seconds**
in 500 ms increments for the device to appear before giving up.

The legacy `device = "/dev/input/eventXX"` form still works for one-shot
overrides and for backwards compatibility, but emits a deprecation warning at
runtime.

### Better `--list-devices` output

Each candidate now shows the information you need to fill in `[device_match]`:

```text
[1] /dev/input/event16
    name: Logitech USB Receiver
    usb-id: 046d:c539 (bus: USB)
    phys: usb-0000:00:14.0-5/input2
    supports:
      EV_KEY: BTN_LEFT BTN_RIGHT BTN_MIDDLE
      EV_REL: REL_X REL_Y REL_WHEEL REL_HWHEEL REL_WHEEL_HI_RES
```

### New error types

- `DeviceMatchNotFound { vendor_id, product_id }` — clear message that points
  to `--list-devices` for troubleshooting.
- `DeviceMatchInvalid { field, value }` — fired when a `vendor_id` or
  `product_id` cannot be parsed as a 4-digit hexadecimal USB id.

## Bug Fixes

### Config directory naming inconsistency

Previously, `wayland-wheeltani` would silently use two different config
directories depending on how it was invoked:

| Invocation                                  | Directory used                       |
| ------------------------------------------- | ------------------------------------ |
| `wayland-wheeltani --setup`                 | `~/.config/wayland-wheeltani/`       |
| `sudo wayland-wheeltani --setup`            | `~/.config/Wayland-Wheeltani/` (!)   |
| `wayland-wheeltani` run by `systemd --user` | `~/.config/wayland-wheeltani/`       |

The mismatch was caused by `directories` 5.x silently lowercasing application
names on Linux while another code path hardcoded the capitalised form.

**1.1.3** standardises on the lowercase, XDG-compliant
`~/.config/wayland-wheeltani/` everywhere — including the sudo path, the
systemd user unit, and `--install-service`.

If you only have the legacy `~/.config/Wayland-Wheeltani/` directory, the
daemon will keep reading it and log a warning suggesting you re-run
`wayland-wheeltani --setup`. If both exist, the new lowercase one wins and a
warning invites you to clean up the old one.

## Migration

You generally do not need to do anything; the daemon detects and adapts. For
the cleanest setup, however:

```bash
# 1. Compare both configs (if you have two) and pick the one you want to keep.
diff ~/.config/wayland-wheeltani/config.toml \
     ~/.config/Wayland-Wheeltani/config.toml

# 2. If needed, move the preferred file into the new lowercase directory.
mkdir -p ~/.config/wayland-wheeltani
mv ~/.config/Wayland-Wheeltani/config.toml \
   ~/.config/wayland-wheeltani/config.toml

# 3. Remove the legacy directory.
rm -rf ~/.config/Wayland-Wheeltani

# 4. Re-run setup to migrate `device = "/dev/input/eventXX"` to `[device_match]`.
wayland-wheeltani --setup
```

If your `config.toml` still contains `device = "/dev/input/eventXX"`, the
daemon keeps working and logs a one-time deprecation warning. Re-running
`--setup` is the quickest way to migrate to `[device_match]`.

## Internals

- `device_discovery::DeviceInfo` now exposes `vendor_id`, `product_id`,
  `bus_type`, and `unique_name`.
- New public APIs: `device_discovery::find_match(&MatchCriteria)` and
  `device_discovery::probe(&Path)`.
- New helper `config_loader::APP_DIR` constant used everywhere to avoid
  re-introducing the case mismatch.
- 9 new unit tests covering hex id parsing, `device_match` TOML
  round-tripping, legacy path migration and CLI overrides. Total test count:
  **22** unit tests (`cargo clippy --workspace --all-targets -- -D warnings`
  is clean).

## Upgrade

```bash
cargo install wayland-wheeltani --force
# or, from a checked-out source tree:
cargo install --path crates/middle-scroll-linux --force
```

No service or udev-rule reinstall is required.
