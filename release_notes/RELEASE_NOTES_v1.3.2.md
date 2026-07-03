# Wayland-Wheeltani 1.3.2

This release makes **smooth progressive scrolling the default**, fixes the
Hyprland/Sway foreground providers under the recommended `systemd --user`
service setup, and hardens the daemon's memory and stability behaviour
(bounded buffers, subprocess timeouts, fewer allocations on the input hot
path).

## Changed

### Smooth progressive scroll by default

Scroll speed now follows a continuous curve between
`min_speed_detents_per_second` and `max_speed_detents_per_second` as the
pointer moves away from the press point, shaped by `acceleration_exponent`
and reaching the maximum at `full_speed_units` past the deadzone — instead of
jumping between the five built-in `scroll_speed_steps`.

**Backward compatible**: config files that define `[[scroll_speed_steps]]`
keep the stepped profile with their exact values. Only installs that relied on
the built-in default steps switch to the smooth curve; to restore the old
feel, add the previous default steps back (they are listed, commented out, in
`examples/config.toml`).

### Foreground providers

- **Hyprland and Sway now work under `systemd --user`.** Socket discovery no
  longer depends solely on `HYPRLAND_INSTANCE_SIGNATURE` / `SWAYSOCK` (absent
  from the service environment): when the variables are missing, the runtime
  directories are scanned for the live compositor socket. `provider = "auto"`
  now detects Hyprland/Sway from the service — previously only GNOME worked.
- **No provider is started when the filter is disabled** (the default), so an
  unconfigured daemon no longer runs background threads and periodic
  `gdbus`/`kdotool` subprocesses for nothing.
- **Helper subprocesses can no longer hang a provider**: `gdbus`, `kdotool`
  and the user's `command` are killed after a 5s timeout instead of freezing
  their provider thread with a stale snapshot.
- **Sway is event-driven in steady state** (focused app read from the
  `window` event container; `GET_TREE` only on initial sync and window
  close), and IPC payloads are capped at 16 MiB.
- **GNOME resync poll relaxed from 2s to 5s** (push updates still arrive
  instantly through `gdbus monitor`), and reconnect backoffs only reset once a
  stream actually delivers data, preventing spawn storms.

### Memory & stability (core)

- **Pending-motion replay is bounded**: the drift replayed before a short
  middle click is stored as a compacted net delta instead of a list of every
  motion event, so a long press cannot grow memory without limit.
- **The input hot path no longer allocates per event**: the engine gained
  `process_into(&mut Vec)`, the daemon reuses its action/batch/uinput buffers,
  and evdev events are dispatched straight from the kernel buffer iterator.
- **Invalid wheel config is rejected at startup**: `emit_hires_wheel = false`
  with `emit_legacy_wheel = false` is now a validation error instead of a
  silently dead autoscroll.

## Upgrade & How to Apply

```bash
# Cargo install
cargo install wayland-wheeltani --force

# Or from the checked-out source tree
cargo install --path crates/middle-scroll-linux --force

# Then restart the service
wayland-wheeltani --restart   # or: wlw --restart
```

No config, udev rule, or systemd service changes are needed.

## Compatibility

- Every 1.3.x config key keeps its meaning; explicit `[[scroll_speed_steps]]`
  entries preserve the stepped behaviour exactly.
- Installs that never configured steps move to the new smooth progressive
  curve.
- The only newly rejected configuration is `emit_hires_wheel = false` combined
  with `emit_legacy_wheel = false`, which previously scrolled nothing.

## Maintainer: cutting this release

The workspace version was bumped from `1.3.1` to `1.3.2`:

```bash
scripts/bump-version.sh --version 1.3.2
```

Then trigger the "Release Linux binaries" workflow (`workflow_dispatch`) from
`main` with the version input `v1.3.2`. It builds the Linux binaries (both
`wayland-wheeltani` and `wlw`), creates a GitHub release tagged `v1.3.2`, and
publishes both workspace crates to crates.io using `CARGO_REGISTRY_TOKEN`.
