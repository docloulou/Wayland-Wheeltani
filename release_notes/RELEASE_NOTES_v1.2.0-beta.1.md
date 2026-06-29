# Wayland-Wheeltani 1.2.0-beta.1 (beta)

Beta pre-release. This is published as a semver pre-release, so a plain
`cargo install wayland-wheeltani` keeps installing the latest **stable**
release. You only get this build if you opt in explicitly (see below).

## Highlights

### Port-independent mouse matching

Selecting a mouse with `--setup` no longer pins it to the USB port it happened
to be plugged into. The config now matches by **USB vendor and product id
only**, so moving the mouse to a different port keeps working with no re-setup.
This is the behavior dotfiles maintainers expect when sharing one config across
machines.

What changed:

- `wayland-wheeltani --setup` no longer writes the `phys` (USB port topology)
  line into `[device_match]` by default.
- New `--pin-port` flag: `wayland-wheeltani --setup --pin-port` opts back in to
  pinning the current USB port (useful to disambiguate two identical mice).
- Backward compatible: configs from older versions that already contain a
  `phys` line keep working. If the mouse is no longer on the pinned port, the
  daemon logs a warning and falls back to matching by USB id, then continues
  through its normal 10s device-appearance retry loop.

The generated udev rule was already port-independent (it matches by USB id), so
it does not need to be reinstalled when moving ports.

## Migrating an existing install

If you set up an earlier version, your config may pin a port. Check it:

```bash
grep phys ~/.config/wayland-wheeltani/config.toml
```

To drop the pinned port and migrate cleanly:

```bash
# Re-run setup (omitting --pin-port removes the `phys` line)
wayland-wheeltani --setup
# Reload the systemd user service if you use it
wayland-wheeltani --restart
```

No action is strictly required: the daemon falls back to USB-id matching on its
own. See the README section "Migrating an existing install to port-independent
matching" for details.

## Installing the beta

Because this is a pre-release version, Cargo does not pick it up automatically.
Install it explicitly:

```bash
cargo install wayland-wheeltani --version 1.2.0-beta.1
```

Or from the release archive attached to the GitHub pre-release
(`wayland-wheeltani-v1.2.0-beta.1-linux-*.tar.gz`).

To go back to the latest stable release afterwards:

```bash
cargo install wayland-wheeltani --force
```

## Maintainer: cutting this beta

The version in `Cargo.toml` is already `1.2.0-beta.1`. Trigger the existing
"Release Linux binaries" workflow (`workflow_dispatch`) from this branch with
the version input `v1.2.0-beta.1`. It builds the Linux binaries, creates a
GitHub release tagged `v1.2.0-beta.1`, and publishes both workspace crates to
crates.io as a pre-release using `CARGO_REGISTRY_TOKEN`.
