# Wayland-Wheeltani 1.3.0-beta.1 (beta)

Beta pre-release. This is published as a semver pre-release, so a plain
`cargo install wayland-wheeltani` keeps installing the latest **stable**
release. You only get this build if you opt in explicitly (see below).

This build adds an optional **per-application filter** that turns autoscroll on
or off depending on the focused window, and relicenses the project under the
permissive **0BSD**.

## Highlights

### Per-application foreground filter (opt-in)

Keep the native middle-click in some apps (a browser, a game, a design tool)
while keeping progressive autoscroll everywhere else. Add a `[foreground]`
table:

```toml
[foreground]
enabled = true
provider = "auto"          # auto | none | hyprland | sway | gnome | command
mode = "denylist"          # denylist | allowlist
unknown_policy = "enabled" # what to do when the app can't be determined
deny_apps = ["firefox", "org.mozilla.firefox", "steam"]
```

How it behaves:

- **Off by default** — leaving the table out keeps the historical behaviour, so
  this release is fully backward compatible.
- When an app is disabled, mouse events are **passed straight through** to the
  virtual device, so the middle click, middle-drag and the wheel work natively
  there; only the autoscroll gesture is suppressed.
- The decision is **latched per gesture** (taken at middle-button press, held
  until release), so changing focus mid-scroll never leaves a button stuck or a
  scroll half-done. It is also cleared on a hot-reconnect.
- Matching is case-insensitive against `app_id`, `class` and `resource_class`
  (and the title with `match_title = true`); a trailing `.desktop` is ignored.

### Providers with auto-detection

`provider = "auto"` tries, in order: **hyprland** (event socket) → **sway**/i3
(IPC) → **gnome** (bundled Shell extension over the session bus via `gdbus`) →
**command** (your own script, e.g. for KWin) → **none**. Detection relies on the
session bus rather than desktop environment variables, so it works from a
`systemd --user` service even when `XDG_CURRENT_DESKTOP` is not exported.

### GNOME support

GNOME (Wayland) has no portable focused-window API, so the `gnome` provider uses
a small bundled GNOME Shell extension that publishes the focused window on the
session bus. Install it from [`integrations/gnome/`](../integrations/gnome/):

```bash
integrations/gnome/install.sh
# On Wayland, log out/in so GNOME Shell loads the extension, then:
gnome-extensions enable wheeltani-foreground@docloulou.github.io
```

### `--detect-foreground` helper

Not sure what to put in `deny_apps` / `allow_apps`? Run the detector, focus the
target window during the countdown, and it prints the exact identifier to copy
(works with every provider, never touches the mouse):

```bash
wayland-wheeltani --detect-foreground
```

### Relicensed under 0BSD

The project is now released under the **BSD Zero Clause License (0BSD)**,
replacing the previous `MIT OR Apache-2.0`. 0BSD is the most permissive option:
use, copy, modify and distribute freely, with no attribution required.

## Testing status & feedback

> **Only the GNOME (Wayland) provider has been tested so far.** The `hyprland`,
> `sway`/i3 and `command` providers are implemented but **not yet verified on
> real sessions**. If you run one of those, please open an issue to report
> whether it works — it will be confirmed or fixed for the stable release.

The core autoscroll behaviour is unchanged when the filter is left disabled, so
this is safe to try: if anything misbehaves, remove the `[foreground]` table (or
set `enabled = false`) and you are back to the previous behaviour.

## Debugging the foreground filter

First, check what the provider reports for the window you care about:

```bash
wayland-wheeltani --detect-foreground
# focus the target window during the countdown; it prints app_id / class /
# resource_class / title and the exact identifier to copy into deny_apps.
```

Then run the daemon in the foreground with debug logs to watch each decision
(stop the service first so the manual run can grab the mouse exclusively):

```bash
wayland-wheeltani --stop
wayland-wheeltani --no-interactive --config ~/.config/wayland-wheeltani/config.toml -v
# startup: `foreground provider selected: <provider>`
#          (or `foreground provider unsupported: ...` if none matched)
# per gesture: `foreground decision ... decision=Enabled|Disabled`
# Ctrl-C when done, then:
wayland-wheeltani --start
```

Other useful checks:

```bash
journalctl --user -u wayland-wheeltani -f      # follow the service logs
wayland-wheeltani --list-devices               # confirm the mouse is resolved

# GNOME only — confirm the extension is enabled and answering:
gnome-extensions list --enabled | grep wheeltani-foreground
gdbus call --session \
  --dest org.docloulou.WheeltaniForeground \
  --object-path /org/docloulou/WheeltaniForeground \
  --method org.docloulou.WheeltaniForeground.GetFocused
```

## Migrating an existing install

Nothing to do beyond installing this build. Existing configs work unchanged and
gain nothing unless you opt into `[foreground]`.

## Installing the beta

Because this is a pre-release version, Cargo does not pick it up automatically.
Install it explicitly:

```bash
cargo install wayland-wheeltani --version 1.3.0-beta.1
```

Or from the release archive attached to the GitHub pre-release
(`wayland-wheeltani-v1.3.0-beta.1-linux-*.tar.gz`).

To go back to the latest stable release afterwards:

```bash
cargo install wayland-wheeltani --force
```

## Maintainer: cutting this beta

Bump the workspace version (the bump script supports an explicit version, since
this is a new minor series rather than a `-beta.N` increment):

```bash
scripts/bump-version.sh --version 1.3.0-beta.1
```

Then trigger the "Release Linux binaries" workflow (`workflow_dispatch`) from
this branch with the version input `v1.3.0-beta.1`. It builds the Linux
binaries, creates a GitHub release tagged `v1.3.0-beta.1`, and publishes both
workspace crates to crates.io as a pre-release using `CARGO_REGISTRY_TOKEN`.
