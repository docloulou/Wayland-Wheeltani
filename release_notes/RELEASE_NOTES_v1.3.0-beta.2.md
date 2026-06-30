# Wayland-Wheeltani 1.3.0-beta.2 (beta)

Beta pre-release. This is published as a semver pre-release, so a plain
`cargo install wayland-wheeltani` keeps installing the latest **stable**
release. You only get this build if you opt in explicitly (see below).

This build adds a **KDE Plasma / KWin** backend to the optional per-application
foreground filter introduced in `1.3.0-beta.1`. Everything else is unchanged.

## Highlights

### New `kde` foreground provider

KWin (Wayland) exposes no portable, readable focused-window API, so the new
`kde` provider uses the [`kdotool`](https://github.com/jinliu/kdotool) helper
(which drives KWin's scripting API under the hood) and polls it on a background
thread — exactly like the existing `command` provider, so the input hot path
never blocks.

```toml
[foreground]
enabled = true
provider = "kde"           # or "auto" — it detects KDE when kdotool is present
mode = "denylist"
deny_apps = ["firefox", "org.kde.dolphin"]
command_refresh_ms = 500   # how often the active window is polled
```

Install `kdotool` first (KDE Plasma 6):

```bash
cargo install kdotool      # or your distribution's package
kdotool getactivewindow getwindowclassname   # sanity check
```

- **Auto-detection** order is now hyprland → sway → gnome → **kde** → command →
  none. `auto` only selects KDE when KWin is on the session bus **and** `kdotool`
  is available, so it never shadows an already-working provider. If `gdbus` is
  not installed (so KWin can't be probed), set `provider = "kde"` explicitly.
- **`--detect-foreground`** works with the new provider too and reports the
  source as `Kde`.

See the
[KDE setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/KDE-Setup) wiki
page for details.

## Testing status & feedback

> **The `kde` provider has not been verified by the author on a real Plasma
> session** — feedback is very welcome (open an issue saying whether it works).
> It is opt-in and inert unless you enable the filter and select it, so it
> cannot affect existing setups.

**GNOME users:** keep using the `gnome` provider (the bundled Shell extension).
`kdotool` talks to KWin only and does nothing on GNOME, and `auto` already
stops at `gnome` before reaching `kde`.

## Migrating from beta.1

Nothing to do. The KDE provider is purely additive; existing configs are
unaffected.

## Installing the beta

Because this is a pre-release version, Cargo does not pick it up automatically.
Install it explicitly:

```bash
cargo install wayland-wheeltani --version 1.3.0-beta.2
```

Or from the release archive attached to the GitHub pre-release
(`wayland-wheeltani-v1.3.0-beta.2-linux-*.tar.gz`).

To go back to the latest stable release afterwards:

```bash
cargo install wayland-wheeltani --force
```

## Maintainer: cutting this beta

Bump the workspace version (a `-beta.N` increment):

```bash
scripts/bump-version.sh beta      # 1.3.0-beta.1 -> 1.3.0-beta.2
```

Then trigger the "Release Linux binaries" workflow (`workflow_dispatch`) from
this branch with the version input `v1.3.0-beta.2`. It builds the Linux
binaries, creates a GitHub release tagged `v1.3.0-beta.2`, and publishes both
workspace crates to crates.io as a pre-release using `CARGO_REGISTRY_TOKEN`.
