# Wayland-Wheeltani V1 Specification

## Goal

Wayland-Wheeltani turns **hold middle mouse button + vertical or horizontal
pointer motion** into progressive continuous wheel scrolling on Linux Wayland
desktops. The button that starts it (`trigger_button`: middle, side, extra,
forward, back or task) and the way it is held (`mode`) are configurable.

## Non-goals

- No GUI or overlay in V1. State changes can run user commands (`[hooks]`)
  instead.
- No X11/XTest event injection.
- No keyboard capture.
- No network access.
- No automatic system installation.

## Input model

The Linux backend reads the selected physical mice (one or more
`[[device_match]]` entries) through `/dev/input/eventX` using evdev. The
trigger, the left and right buttons, relative pointer motion and the wheels are
routed into the core engine. Every other button and relative axis bypasses the
engine and is forwarded unchanged, one frame per physical frame. Keyboard-like
events are ignored.

## Output model

The backend emits a virtual mouse through `/dev/uinput`. The virtual device
supports pointer motion, standard mouse buttons, vertical and horizontal legacy
wheel detents, and vertical and horizontal hi-res wheel units where 120 hi-res
units equal one legacy detent. It also declares every button and relative axis
of the physical mice it serves, so the forwarded events keep working. A press
and its release never share an output frame, so clients see every click.

## State machine

| From | Event | To |
| --- | --- | --- |
| `Idle` | trigger down | `MiddlePending` |
| `MiddlePending` | motion beyond the deadzone | `Scrolling` |
| `MiddlePending` | trigger up (hold mode) | `Idle`, synthetic click |
| `MiddlePending` | trigger up (toggle mode) | `Scrolling`, latched |
| `Scrolling` | trigger up (hold, no inertia) | `Idle` |
| `Scrolling` | trigger up (hold, inertia) | `Coasting` |
| `Scrolling`, latched | any button down | `Idle`, click eaten |
| `Coasting` | trigger down | `MiddlePending` (a release there only stops the coast) |
| `Coasting` | other button or wheel | `Idle`, input forwarded |
| `Coasting` | speed decays to nothing | `Idle` |

- `Idle`: forward ordinary input.
- `MiddlePending`: suppress the initial trigger press while waiting to
  determine whether this is a click or a scroll gesture.
- `Scrolling`: emit periodic wheel events until the trigger is released. In
  `toggle_progressive` mode a short click latches this state with the button
  up; the next click of any button ends it and never reaches the application.
- `Coasting` (`inertia = true`): after the release, keep emitting wheel events
  whose speed halves every `inertia_half_life_ms`, until the speed is
  negligible or any button or the real wheel interrupts it. The trigger only
  stops the coast; other input stops it and goes through.

The engine needs periodic ticks only in `Scrolling` and `Coasting`; the daemon
sleeps in `poll` without a timeout the rest of the time.

## Click preservation

In hold mode, if the trigger is released before motion exceeds the configured
deadzone on any enabled axis, the engine emits a synthetic click of the
trigger. This preserves
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

`axis_lock` restricts the output to one axis: `dominant` scrolls only the axis
with the larger offset at each tick, `first` keeps the first axis that left the
deadzone for the whole gesture.

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
`--add-device` appends another `[[device_match]]` entry. The udev rule opens
only the mouse nodes of the configured devices (`ID_INPUT_MOUSE`, never a
keyboard interface sharing the same USB ids) and `/dev/uinput`.
`--install-service` installs and starts a systemd user service using the saved
config; `--remove-service` stops, disables, and removes it. Services should run
with `--no-interactive` so they fail instead of blocking on prompts.

## Running daemon

- **Reload.** The daemon re-reads its config when the file is saved
  (`watch_config`), on SIGHUP and on `--reload`. An invalid file is logged and
  the running settings are kept; a gesture in progress finishes with the old
  settings. `grab`, `dry_run` and the foreground provider need a restart.
- **Control socket** (`control_socket`, on by default):
  `$XDG_RUNTIME_DIR/wayland-wheeltani/control.sock` (directory 0700, socket
  0600) accepts one-line commands: `status` (JSON), `pause`, `resume`,
  `toggle`, `reload`, and `get`/`set`/`save` for `--tune`. Pausing aborts any
  gesture and releases the buttons it holds, then forwards everything
  unchanged. `save` writes the changed keys back to the config file and keeps
  its comments.
- **Hooks.** `[hooks]` `on_enter`, `on_exit`, `on_pause` and `on_resume` run a
  program with arguments, without a shell and in the background, with
  `WHEELTANI_STATE` set.

## Foreground application filter

An optional `[foreground]` config table enables or disables autoscroll based on
the focused application. It is disabled by default, so an unconfigured daemon is
unchanged. When enabled, a decision layer sits between the event router and the
core engine: at each trigger press it evaluates the focused app and either
lets the engine handle the gesture (autoscroll) or passes the raw events through
to the virtual mouse untouched (native middle click/wheel). The decision is
latched for the duration of a gesture, so a focus change mid-scroll cannot leave
the engine in an inconsistent state, and it is reset on device reconnect.

Matching is case-insensitive against `app_id`, `class`, and `resource_class`
(and `title` only when `match_title = true`), with a trailing `.desktop`
ignored. `mode` selects `denylist` or `allowlist`; `unknown_policy` decides
behaviour when the focused app cannot be resolved (defaulting to `enabled` so a
missing provider never breaks autoscroll).

`[[profiles]]` entries reuse the same detection: the first profile whose
`apps` match the focused application supplies the engine settings for the
gesture, on top of the top-level ones. Profiles work with the filter enabled or
not; a disabled application stays disabled.

The focused window is resolved by a provider, chosen explicitly or via
`provider = "auto"` (tried in order: Hyprland event socket, Sway/i3 IPC, GNOME,
KDE via `kdotool`, external command, then none). When nothing is detected at
startup, `auto` keeps retrying in the background, because the user service can
start before the compositor. No focused window counts as unknown. Providers run on a background thread and expose a
non-blocking snapshot, so the input hot path never performs I/O. The GNOME
provider talks to a bundled GNOME Shell extension over the session bus via the
`gdbus` binary (no D-Bus crate dependency); the daemon must run inside the user
session (its `systemd --user` service does) to reach the bus and compositor
sockets. The extension and its installer live in `integrations/gnome/`.
