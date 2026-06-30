# Changelog

All notable changes to **Wayland-Wheeltani** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Pre-releases (`-beta.N`) are published as semver pre-releases, so a plain
`cargo install wayland-wheeltani` keeps installing the latest **stable** release.

## [1.3.0-beta.1] - 2026-06-30

### Added

- **Per-application foreground filter** (new optional `[foreground]` table) that
  turns autoscroll on or off depending on the focused window — for example to
  keep the native middle-click in a browser or a game while keeping autoscroll
  everywhere else. **Disabled by default**, so an unconfigured daemon behaves
  exactly as before.
  - **Denylist / allowlist** modes, matched case-insensitively against the
    window `app_id`, `class` and `resource_class` (and the title with
    `match_title = true`); a trailing `.desktop` is ignored.
  - **`unknown_policy`** (`enabled` by default) decides what happens when the
    focused app cannot be determined, so a missing provider never breaks scroll.
  - The decision is **latched per gesture**: taken once when the middle button
    goes down and held until release, so changing focus mid-scroll can never
    leave a button stuck or a scroll half-done. It is also cleared on a
    physical-device hot-reconnect.
  - When an app is disabled, mouse events are **passed straight through** to the
    virtual device (the middle click, drag and wheel keep working natively).
- **Foreground providers** with automatic detection (`provider = "auto"` tries,
  in order, hyprland → sway → gnome → command → none):
  - `hyprland` — reads the Hyprland event socket (no helper needed).
  - `sway` / i3 — reads the Sway/i3 IPC (no helper needed).
  - `gnome` — talks to a **bundled GNOME Shell extension** over the session bus
    via `gdbus`, with a low-latency push path (`gdbus monitor`) plus an
    authoritative resync. The extension lives in
    [`integrations/gnome/`](integrations/gnome/).
  - `command` — runs a user command (for KWin or any other compositor) that
    prints the focused app as plain text or JSON.
- **`--detect-foreground` CLI command**: focus a window during a short
  countdown and it prints the window's identity plus the exact string to drop
  into `deny_apps` / `allow_apps`. Works with every provider and never touches
  the mouse.

### Changed

- **License is now [0BSD](LICENSE)** (BSD Zero Clause License), replacing the
  previous `MIT OR Apache-2.0` dual license. 0BSD is the most permissive option:
  use, copy, modify and distribute freely, with **no attribution required**.
  `LICENSE-MIT` and `LICENSE-APACHE` were removed in favour of a single
  `LICENSE` file.
- Documentation reorganised: the README is now a concise quick-start and the
  full guides (configuration, foreground filter, GNOME setup, troubleshooting)
  live in the [GitHub wiki](https://github.com/docloulou/Wayland-Wheeltani/wiki).

### Compatibility

- Fully backward compatible. Existing configs work unchanged; the foreground
  filter only activates when you add `[foreground]` with `enabled = true`.
- The daemon must run inside your graphical session (the bundled
  `systemd --user` service does) to see the focused window. Detection relies on
  the session bus, not on desktop environment variables, so it works from a
  `--user` service even when `XDG_CURRENT_DESKTOP` is not exported.

### Testing status

- **Only the GNOME (Wayland) provider has been tested by the author.** The
  `hyprland`, `sway`/i3 and `command` providers are implemented but **not yet
  verified on real sessions** — feedback is very welcome so they can be
  confirmed or fixed. The core autoscroll behaviour is unchanged when the filter
  is left disabled.
- Useful commands to debug the filter:
  - `wayland-wheeltani --detect-foreground` — prints what the active provider
    reports for the focused window plus the exact identifier to put in
    `deny_apps` / `allow_apps`.
  - Run the daemon in the foreground with debug logs to watch each decision
    (stop the service first so it can grab the mouse):

    ```bash
    wayland-wheeltani --stop
    wayland-wheeltani --no-interactive --config ~/.config/wayland-wheeltani/config.toml -v
    # startup logs `foreground provider selected: <provider>`
    #   (or `foreground provider unsupported: ...` if none matched);
    # each gesture logs `foreground decision ... decision=Enabled|Disabled`.
    # Ctrl-C when done, then restart the service:
    wayland-wheeltani --start
    ```

  - `journalctl --user -u wayland-wheeltani -f` — follow the service logs.
  - GNOME only: `gnome-extensions list --enabled | grep wheeltani-foreground`
    and the `gdbus call … GetFocused` check from the
    [GNOME setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/GNOME-Setup)
    guide.

## [1.2.0-beta.2] - 2026-06-30

### Added

- **Runtime hot-reconnect**: the daemon now survives a live unplug/replug
  without a restart. It detects the physical disconnect (`POLLHUP`/`POLLERR` or
  a failed read), keeps running, and re-resolves the mouse **by USB id** when it
  comes back — even on a different port or `/dev/input/eventXX` node.
- The virtual mouse is created once and kept alive across reconnections, so the
  compositor never sees it disappear. Any in-flight gesture is cleared on
  disconnect so no virtual button can stay stuck down.
- As a `systemd --user` service (`--no-interactive`), the daemon waits
  indefinitely for the configured mouse to appear, so you can start the service
  before plugging the mouse in.

## [1.2.0-beta.1] - 2026-06-29

### Changed

- **Port-independent mouse matching**: `[device_match]` resolves the mouse by
  USB id at startup, so moving it to another USB port keeps it working. A pinned
  `phys` (USB port) is relaxed automatically as a fallback.

## [1.1.4] - 2026-05-20

### Added

- Stable device selection via the `[device_match]` configuration option
  (`vendor_id` / `product_id` / `name`), replacing fragile
  `device = "/dev/input/eventX"` node paths.

### Changed

- README troubleshooting and installation steps clarified (udev rule setup).

[1.3.0-beta.1]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.3.0-beta.1
[1.2.0-beta.2]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.2.0-beta.2
[1.2.0-beta.1]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.2.0-beta.1
[1.1.4]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.1.4
