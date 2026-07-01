# Wayland-Wheeltani 1.3.1

This release adds a short command alias: `wlw`. There is no behavior change
to the daemon itself — everything works exactly as in 1.3.0.

## Added

### `wlw` short command alias

`cargo install wayland-wheeltani` (and building from source) now installs a
second binary, `wlw`, alongside `wayland-wheeltani`. It is the exact same CLI
and daemon under a shorter name — every flag and workflow behaves identically:

```bash
wlw --setup --install-udev-rule
wlw --install-service
wlw --start | --stop | --restart
wlw --detect-foreground
wlw --list-devices
```

`--help` and `--version` reflect whichever name you actually typed, so
`wlw --help` prints `Usage: wlw [OPTIONS]` and `wayland-wheeltani --help`
prints `Usage: wayland-wheeltani [OPTIONS]`.

Release archives (`wayland-wheeltani-v1.3.1-linux-*.tar.gz`) now ship both
binaries too.

## Upgrade & How to Apply

```bash
# Cargo install
cargo install wayland-wheeltani --force

# Or from the checked-out source tree
cargo install --path crates/middle-scroll-linux --force
```

`wlw` will then be available next to `wayland-wheeltani` in the same Cargo bin
directory. No config, udev rule, or systemd service changes are needed — a
previously installed `wayland-wheeltani.service` keeps running unchanged; you
only need to reinstall the binary to gain the new `wlw` alias.

## Compatibility

Fully backward compatible. `wayland-wheeltani` keeps working exactly as
before; `wlw` is purely additive.

## Maintainer: cutting this release

The workspace version was bumped from `1.3.0` to `1.3.1` (a patch release, no
behavior change to the daemon):

```bash
scripts/bump-version.sh --version 1.3.1
```

Then trigger the "Release Linux binaries" workflow (`workflow_dispatch`) from
`main` with the version input `v1.3.1`. It builds the Linux binaries (both
`wayland-wheeltani` and `wlw`), creates a GitHub release tagged `v1.3.1`, and
publishes both workspace crates to crates.io using `CARGO_REGISTRY_TOKEN`.
