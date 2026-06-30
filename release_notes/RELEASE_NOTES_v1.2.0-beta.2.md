# Wayland-Wheeltani 1.2.0-beta.2 (beta)

Beta pre-release. This is published as a semver pre-release, so a plain
`cargo install wayland-wheeltani` keeps installing the latest **stable**
release. You only get this build if you opt in explicitly (see below).

This build extends the port-independent matching shipped in
`1.2.0-beta.1` from *startup* to *runtime*: the daemon now survives a live
unplug/replug without being restarted.

## Highlights

### Runtime hot-reconnect (unplug/replug without restarting the service)

`1.2.0-beta.1` made the daemon find the mouse by USB id at startup, so moving
it to another port and then starting the service kept working. But if you
unplugged the mouse **while the daemon was running** and plugged it back into a
different port, middle-click scrolling stayed dead until you restarted the
service. That is fixed.

What changed:

- The event loop now detects a physical disconnect (a `POLLHUP`/`POLLERR` hangup
  on the device, or a failed read) instead of silently spinning forever.
- On disconnect the daemon keeps running and waits for the mouse to come back,
  re-resolving it **by USB id** so it is found again even on a different
  `/dev/input/eventXX` node or a different USB port (a pinned `phys` is relaxed
  automatically, same fallback as at startup).
- The virtual mouse is created once and kept alive across reconnections, so the
  compositor never sees it disappear.
- On disconnect, any in-flight gesture is cleared: the engine is reset and all
  virtual buttons are released, so a half-finished scroll or a forwarded button
  can never stay stuck down during the gap.

### Service start before the mouse is plugged in

When launched as a systemd user service (`--no-interactive`), the daemon now
waits **indefinitely** for the configured mouse to appear instead of giving up
after ~10s and relying on a systemd restart loop. You can start the service
first and plug the mouse in afterwards (into any port) and it just connects.

Interactive runs (a plain `wayland-wheeltani` in a terminal) keep the short
device-appearance timeout so the CLI still fails fast when the mouse is absent.

Note: legacy `device = "/dev/input/eventX"` configs (a pinned node path) cannot
be matched by USB id, so they remain fatal at startup if the node is missing.
Re-run `wayland-wheeltani --setup` to migrate to `[device_match]`.

## Migrating an existing install

Nothing to do beyond installing this build. Configs written by
`1.2.0-beta.1` (USB-id `[device_match]`, with or without a pinned `phys`)
work unchanged and gain hot-reconnect automatically. If you still use a legacy
`device =` path, migrate with:

```bash
wayland-wheeltani --setup
wayland-wheeltani --restart   # if you use the systemd user service
```

## Installing the beta

Because this is a pre-release version, Cargo does not pick it up automatically.
Install it explicitly:

```bash
cargo install wayland-wheeltani --version 1.2.0-beta.2
```

Or from the release archive attached to the GitHub pre-release
(`wayland-wheeltani-v1.2.0-beta.2-linux-*.tar.gz`).

To go back to the latest stable release afterwards:

```bash
cargo install wayland-wheeltani --force
```

## Maintainer: cutting this beta

Bump the workspace version to `1.2.0-beta.2` (the bump script now supports
prerelease bumps directly):

```bash
scripts/bump-version.sh beta        # 1.2.0-beta.1 -> 1.2.0-beta.2
```

Then trigger the existing "Release Linux binaries" workflow
(`workflow_dispatch`) from this branch with the version input
`v1.2.0-beta.2`. It builds the Linux binaries, creates a GitHub release tagged
`v1.2.0-beta.2`, and publishes both workspace crates to crates.io as a
pre-release using `CARGO_REGISTRY_TOKEN`.
