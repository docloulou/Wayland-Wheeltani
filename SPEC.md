# Wayland-Wheeltani V1 Specification

## Goal

Wayland-Wheeltani turns **hold middle mouse button + vertical or horizontal
pointer motion** into progressive continuous wheel scrolling on Linux Wayland
desktops.

## Non-goals

- No GUI or overlay in V1.
- No X11/XTest event injection.
- No keyboard capture.
- No network access.
- No automatic system installation.

## Input model

The Linux backend reads a selected physical mouse through `/dev/input/eventX`
using evdev. Only mouse buttons, relative pointer motion, and wheel events are
routed into the core engine. Keyboard-like events are ignored.

## Output model

The backend emits a virtual mouse through `/dev/uinput`. The virtual device
supports pointer motion, standard mouse buttons, vertical and horizontal legacy
wheel detents, and vertical and horizontal hi-res wheel units where 120 hi-res
units equal one legacy detent.

## State machine

```text
Idle --MiddleDown--> MiddlePending --motion beyond deadzone--> Scrolling
  ^                       |                                  |
  |                       +--MiddleUp inside deadzone---------+
  +--------------------------MiddleUp while scrolling---------+
```

- `Idle`: forward ordinary input.
- `MiddlePending`: suppress the initial middle-down while waiting to determine
  whether this is a click or a scroll gesture.
- `Scrolling`: emit periodic wheel events until middle release.

## Click preservation

If the middle button is released before motion exceeds the configured deadzone on
any enabled axis, the engine emits a synthetic middle click. This preserves
common desktop behavior such as opening links in new tabs or
paste-primary-selection.

## Progressive speed

Offset from the original press position controls scroll speed. Vertical offset
drives vertical wheel output. When `horizontal_scroll = true`, horizontal offset
drives horizontal wheel output. Both axes use the same speed profile and can emit
simultaneously during diagonal motion.

The default profile is a smooth progressive curve:

1. offset inside `deadzone_units` => no scroll;
2. beyond the deadzone the speed interpolates continuously between
   `min_speed_detents_per_second` and `max_speed_detents_per_second`, reaching
   the maximum `full_speed_units` past the deadzone edge, shaped by
   `acceleration_exponent` (1.0 = linear, >1 = soft start);
3. `max_offset_units` caps the tracked offset;
4. `max_detents_per_tick` caps bursts after long scheduler delays.

Configs that define `[[scroll_speed_steps]]` entries switch to a stepped
profile instead (this was the built-in default up to 1.3.1, so older config
files keep their exact behaviour): each entry maps an absolute axis distance
from the original press point to a speed in wheel detents per second, and the
last reached step controls the current speed. Example: if the current absolute
distance is 100 units and the configured steps are 40=>4 detents/s and
80=>10 detents/s, the engine scrolls at 10 detents/s.

In both profiles, moving back toward the press point slows down or stops inside
the deadzone, and crossing the press point reverses direction on that axis.

## Configuration and setup

The daemon accepts CLI overrides and a TOML config file. `--setup` enumerates
candidate mice, selects one interactively when needed, and saves a
`[device_match]` block keyed by USB vendor and product id. This match is
port-independent by default, so the mouse keeps working on any USB port without
re-running setup; `--setup --pin-port` additionally records the current port
(`phys`) to disambiguate identical mice. At startup the daemon resolves the
match to a `/dev/input/eventX` node, falling back to USB-id matching when a
previously pinned port no longer holds the device. `--install-udev-rule`
installs a root-owned udev rule for the selected mouse and `/dev/uinput`;
`--remove-udev-rule` removes it.
`--install-service` installs and starts a systemd user service using the saved
config; `--remove-service` stops, disables, and removes it. Services should run
with `--no-interactive` so they fail instead of blocking on prompts.

## Foreground application filter

An optional `[foreground]` config table enables or disables autoscroll based on
the focused application. It is disabled by default, so an unconfigured daemon is
unchanged. When enabled, a decision layer sits between the event router and the
core engine: at each middle-button press it evaluates the focused app and either
lets the engine handle the gesture (autoscroll) or passes the raw events through
to the virtual mouse untouched (native middle click/wheel). The decision is
latched for the duration of a gesture, so a focus change mid-scroll cannot leave
the engine in an inconsistent state, and it is reset on device reconnect.

Matching is case-insensitive against `app_id`, `class`, and `resource_class`
(and `title` only when `match_title = true`), with a trailing `.desktop`
ignored. `mode` selects `denylist` or `allowlist`; `unknown_policy` decides
behaviour when the focused app cannot be resolved (defaulting to `enabled` so a
missing provider never breaks autoscroll).

The focused window is resolved by a provider, chosen explicitly or via
`provider = "auto"` (tried in order: Hyprland event socket, Sway/i3 IPC, GNOME,
external command, then none). Providers run on a background thread and expose a
non-blocking snapshot, so the input hot path never performs I/O. The GNOME
provider talks to a bundled GNOME Shell extension over the session bus via the
`gdbus` binary (no D-Bus crate dependency); the daemon must run inside the user
session (its `systemd --user` service does) to reach the bus and compositor
sockets. The extension and its installer live in `integrations/gnome/`.
