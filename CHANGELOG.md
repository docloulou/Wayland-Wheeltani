# Changelog

All notable changes to **Wayland-Wheeltani** are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Pre-releases (`-beta.N`) are published as semver pre-releases, so a plain
`cargo install wayland-wheeltani` keeps installing the latest **stable** release.

## [1.3.2] - 2026-07-03

### Changed

- **Smooth progressive scroll is now the default speed profile.** The speed
  interpolates continuously between `min_speed_detents_per_second` and
  `max_speed_detents_per_second` as the pointer moves away from the press
  point (shaped by `acceleration_exponent`, reaching max at
  `full_speed_units` past the deadzone), instead of jumping between the five
  built-in `scroll_speed_steps`. **Backward compatible**: config files that
  define `[[scroll_speed_steps]]` keep the stepped profile with their exact
  values — only installs that relied on the built-in default steps switch to
  the smooth curve.
- **Pending-motion replay is now bounded**: the tiny pointer drift replayed
  before a short middle click (`replay_pending_motion_on_click`) is stored as
  a compacted net delta instead of a list of every motion event, so a long
  press can no longer grow memory without limit. The pointer ends up in the
  same position; the replay is emitted as a single motion event.
- **Hot input path no longer allocates per event**: the engine gained
  `process_into(&mut Vec)`, the daemon reuses action/batch/uinput buffers
  across events, and pending evdev events are dispatched straight from the
  kernel buffer iterator instead of being collected into a `Vec` on every
  wakeup — removing four heap allocations per forwarded motion event
  (thousands per second on high-rate mice).

### Fixed

- **Hyprland/Sway providers now work under `systemd --user`**: socket discovery
  no longer depends solely on `HYPRLAND_INSTANCE_SIGNATURE` / `SWAYSOCK`, which
  are not exported to the service environment. When the variables are missing,
  the runtime directories are scanned for the live compositor socket
  (`$XDG_RUNTIME_DIR/hypr/*/.socket2.sock`, `$XDG_RUNTIME_DIR/sway-ipc.*.sock`),
  so `provider = "auto"` and the explicit providers work in the recommended
  service setup — previously only GNOME did.
- **Hung helpers can no longer freeze a provider**: every helper subprocess
  (`gdbus`, `kdotool`, the user's `command`) is now killed after a 5s timeout
  instead of blocking its provider thread forever with a stale snapshot.
- **Invalid wheel config is rejected at startup**: `emit_hires_wheel = false`
  together with `emit_legacy_wheel = false` is now a validation error instead
  of a silently dead autoscroll.

### Changed (foreground providers)

- **No provider is started when the filter is disabled** (the default): the
  daemon previously auto-detected and ran a foreground provider — background
  threads plus periodic `gdbus`/`kdotool` subprocess spawns — even with
  `[foreground] enabled = false`.
- **Sway provider is now event-driven in steady state**: the focused app is
  taken from the container carried by each `window` event; the full `GET_TREE`
  round-trip (new socket + full tree parse per focus change) only remains for
  the initial sync and window-close resyncs. IPC payloads are also capped at
  16 MiB so a corrupt stream cannot trigger an unbounded allocation.
- **GNOME resync poll relaxed from 2s to 5s** (the low-latency path is the
  `gdbus monitor` push stream), cutting the daemon's steady-state subprocess
  churn by ~60%. Reconnect backoff for the GNOME monitor and the Hyprland
  event stream now only resets once the stream actually delivers data,
  preventing a spawn storm when a helper dies instantly.

### Compatibility

- Config files are fully compatible: every 1.3.x key keeps its meaning, and
  explicit `[[scroll_speed_steps]]` entries preserve the stepped scroll
  behaviour exactly. Installs that never configured steps move to the new
  smooth progressive curve; add the previous default steps back (see
  `examples/config.toml`) to restore the old feel.
- The only newly rejected configuration is `emit_hires_wheel = false` combined
  with `emit_legacy_wheel = false`, which previously produced a daemon that
  scrolled nothing.

## [1.3.1] - 2026-07-01

### Added

- **`wlw` short command alias**: `cargo install wayland-wheeltani` (and
  building from source) now also installs a `wlw` binary — the exact same CLI
  and daemon, just under a shorter name. `wlw --start`, `wlw --setup`,
  `wlw --detect-foreground`, etc. all behave identically to their
  `wayland-wheeltani` equivalents, including `--help`/`--version`, which now
  reflect whichever name was used to invoke the binary.

### Compatibility

- Fully backward compatible. `wayland-wheeltani` keeps working exactly as
  before; `wlw` is purely additive.

## [1.3.0] - 2026-07-01

First stable release of the 1.3.0 series, consolidating
[1.3.0-beta.1] and [1.3.0-beta.2]. It adds an optional **per-application
foreground filter** (so
autoscroll can be turned off in a browser, a game, a design tool while staying on
everywhere else), ships **foreground providers** for Hyprland, Sway/i3, GNOME
and KDE Plasma, and **relicenses the project under 0BSD**. Everything is opt-in
and disabled by default, so an unconfigured daemon behaves exactly as in 1.2.x.

### Added

- **Per-application foreground filter** (new optional `[foreground]` table) that
  turns autoscroll on or off depending on the focused window. **Disabled by
  default**, so an unconfigured daemon behaves exactly as before.
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
  in order, hyprland → sway → gnome → kde → command → none):
  - `hyprland` — reads the Hyprland event socket (no helper needed).
  - `sway` / i3 — reads the Sway/i3 IPC (no helper needed).
  - `gnome` — talks to a **bundled GNOME Shell extension** over the session bus
    via `gdbus`, with a low-latency push path (`gdbus monitor`) plus an
    authoritative resync. The extension lives in
    [`integrations/gnome/`](integrations/gnome/).
  - `kde` — KDE Plasma / KWin foreground provider. KWin (Wayland) exposes no
    readable focused-window API, so this provider uses the
    [`kdotool`](https://github.com/jinliu/kdotool) helper (which drives KWin's
    scripting API) and polls it on a background thread, exactly like the
    `command` provider. Install `kdotool` (e.g. `cargo install kdotool` or your
    distribution's package) to use it. See the
    [KDE setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/KDE-Setup)
    wiki page.
    - `auto` selects KDE only when KWin is on the session bus **and** `kdotool`
      is available, so it never shadows an already-working provider.
    - `--detect-foreground` works with the new provider too (it reuses the same
      provider selection) and reports the source as `Kde`.
  - `command` — runs a user command (for KWin or any other compositor) that
    prints the focused app as plain text or JSON.
- **Bundled KWin example script** for users who prefer not to install
  `kdotool`: [`integrations/kde/wheeltani-kwin-active-window.sh`](integrations/kde/)
  prints the focused window's class via KWin's scripting D-Bus interface and
  plugs into the generic `command` provider.
- **`--detect-foreground` CLI command**: focus a window during a short countdown
  and it prints the window's identity plus the exact string to drop into
  `deny_apps` / `allow_apps`. Works with every provider and never touches the
  mouse.

### Changed

- **License is now [0BSD](LICENSE)** (BSD Zero Clause License), replacing the
  previous `MIT OR Apache-2.0` dual license. 0BSD is the most permissive option:
  use, copy, modify and distribute freely, with **no attribution required**.
  `LICENSE-MIT` and `LICENSE-APACHE` were removed in favour of a single
  `LICENSE` file.
- Documentation reorganised: the README is now a concise quick-start and the
  full guides (configuration, foreground filter, GNOME setup, KDE setup,
  troubleshooting) live in the
  [GitHub wiki](https://github.com/docloulou/Wayland-Wheeltani/wiki).

### Compatibility

- Fully backward compatible. Existing configs work unchanged; the foreground
  filter only activates when you add `[foreground]` with `enabled = true`.
- The daemon must run inside your graphical session (the bundled
  `systemd --user` service does) to see the focused window. Detection relies on
  the session bus, not on desktop environment variables, so it works from a
  `--user` service even when `XDG_CURRENT_DESKTOP` is not exported.

### Testing status

- **Only the GNOME (Wayland) provider has been tested by the author.** The
  `hyprland`, `sway`/i3, `kde` and `command` providers are implemented but
  **not yet verified on real sessions** — feedback is very welcome so they can
  be confirmed or fixed. The core autoscroll behaviour is unchanged when the
  filter is left disabled.
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

## [1.3.0-beta.2] - 2026-06-30

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

## [1.2.0] - 2026-06-30

First stable release of the 1.2.0 series, consolidating `1.2.0-beta.1` and
`1.2.0-beta.2`. It makes mouse selection robust across reboots, USB ports and
live unplug/replug — no more editing `/dev/input/eventXX` paths.

### Added

- **Runtime hot-reconnect**: the daemon survives a live unplug/replug without a
  restart. It detects the physical disconnect (`POLLHUP`/`POLLERR` or a failed
  read), keeps running, and re-resolves the mouse **by USB id** when it comes
  back — even on a different port or `/dev/input/eventXX` node.
- The virtual mouse is created once and kept alive across reconnections, so the
  compositor never sees it disappear. Any in-flight gesture is cleared on
  disconnect so no virtual button can stay stuck down.
- As a `systemd --user` service (`--no-interactive`), the daemon waits
  indefinitely for the configured mouse to appear, so the service can start
  before the mouse is plugged in.

### Changed

- **Port-independent mouse matching**: `[device_match]` resolves the mouse by
  USB id at startup, so moving it to another USB port keeps working. A pinned
  `phys` (USB port) is relaxed automatically as a fallback, and `--setup` no
  longer writes `phys` unless you pass `--pin-port`.

### Compatibility

- Backward compatible with 1.1.x configs. Existing `[device_match]` blocks keep
  working; a previously pinned `phys` is now a soft hint (with USB-id fallback)
  rather than a hard requirement.

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

[1.3.2]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.3.2
[1.3.1]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.3.1
[1.3.0]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.3.0
[1.3.0-beta.2]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.3.0-beta.2
[1.3.0-beta.1]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.3.0-beta.1
[1.2.0]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.2.0
[1.2.0-beta.2]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.2.0-beta.2
[1.2.0-beta.1]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.2.0-beta.1
[1.1.4]: https://github.com/docloulou/Wayland-Wheeltani/releases/tag/v1.1.4
