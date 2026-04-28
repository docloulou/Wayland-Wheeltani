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

V1 uses configurable distance steps by default:

1. offset inside `deadzone_units` => no scroll;
2. each `scroll_speed_steps` entry maps an absolute axis distance from the
   original press point to a speed in wheel detents per second;
3. the last reached step controls the current speed;
4. `max_offset_units` caps the tracked offset;
5. `max_detents_per_tick` caps bursts after long scheduler delays.

Example: if the current absolute distance is 100 units and the configured steps
are 40=>4 detents/s and 80=>10 detents/s, the engine scrolls at 10 detents/s.

Moving back toward the press point can drop to a slower step or stop inside the
deadzone. Crossing the press point reverses direction on that axis.

For users who prefer the older continuous curve, `scroll_speed_steps = []` makes
the engine fall back to `min_speed_detents_per_second`,
`max_speed_detents_per_second`, `full_speed_units`, and
`acceleration_exponent`.

## Configuration and setup

The daemon accepts CLI overrides and a TOML config file. `--setup` enumerates
candidate mice, selects one interactively when needed, and saves the device path
to the config file. Services should run with `--no-interactive` so they fail
instead of blocking on prompts.
