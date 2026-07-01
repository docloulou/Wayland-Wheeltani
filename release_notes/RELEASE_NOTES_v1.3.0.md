# Wayland-Wheeltani 1.3.0

First stable release of the 1.3.0 series, consolidating `1.3.0-beta.1` and
`1.3.0-beta.2`. It adds an optional **per-application foreground filter** (so
autoscroll can be turned off in a browser, a game, a design tool while staying on
everywhere else), ships **foreground providers** for Hyprland, Sway/i3, GNOME
and KDE Plasma, and **relicenses the project under 0BSD**.

A plain `cargo install wayland-wheeltani` now installs this release.

Everything here is opt-in and disabled by default, so an unconfigured daemon
behaves exactly as in 1.2.x — upgrading is safe.

## Highlights

### Per-application foreground filter (opt-in)

Keep the native middle-click in some apps (a browser, a game, a design tool)
while keeping progressive autoscroll everywhere else. Add a `[foreground]`
table:

```toml
[foreground]
enabled = true
provider = "auto"          # auto | none | hyprland | sway | gnome | kde | command
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
**kde** (KDE Plasma via [`kdotool`](https://github.com/jinliu/kdotool)) →
**command** (your own script, e.g. for KWin) → **none**. Detection relies on the
session bus rather than desktop environment variables, so it works from a
`systemd --user` service even when `XDG_CURRENT_DESKTOP` is not exported.

- `auto` only selects KDE when KWin is on the session bus **and** `kdotool` is
  available, so it never shadows an already-working provider. If `gdbus` is not
  installed (so KWin can't be probed), set `provider = "kde"` explicitly.

### GNOME support

GNOME (Wayland) has no portable focused-window API, so the `gnome` provider uses
a small bundled GNOME Shell extension that publishes the focused window on the
session bus. Install it from [`integrations/gnome/`](../integrations/gnome/):

```bash
integrations/gnome/install.sh
# On Wayland, log out/in so GNOME Shell loads the extension, then:
gnome-extensions enable wheeltani-foreground@docloulou.github.io
```

### KDE Plasma support

KWin (Wayland) exposes no portable, readable focused-window API, so the `kde`
provider uses the [`kdotool`](https://github.com/jinliu/kdotool) helper (which
drives KWin's scripting API under the hood) and polls it on a background thread
— exactly like the existing `command` provider, so the input hot path never
blocks.

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

Without `kdotool`, this release also ships an example KWin script
[`integrations/kde/wheeltani-kwin-active-window.sh`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/integrations/kde/wheeltani-kwin-active-window.sh)
that you can plug into the generic `command` provider — it prints the focused
window's class via KWin's scripting D-Bus interface:

```toml
[foreground]
enabled = true
provider = "command"
command = ["/full/path/to/integrations/kde/wheeltani-kwin-active-window.sh"]
mode = "denylist"
deny_apps = ["org.kde.dolphin", "firefox"]
```

See the
[KDE setup](https://github.com/docloulou/Wayland-Wheeltani/wiki/KDE-Setup) wiki
page for both options.

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

> **Only the GNOME (Wayland) provider has been tested by the author.** The
> `hyprland`, `sway`/i3, `kde` and `command` providers are implemented but
> **not yet verified on real sessions**. If you run one of those, please open an
> issue to report whether it works — it will be confirmed or fixed.

The core autoscroll behaviour is unchanged when the filter is left disabled, so
this is safe to try: if anything misbehaves, remove the `[foreground]` table (or
set `enabled = false`) and you are back to the previous behaviour.

**GNOME users:** keep using the `gnome` provider (the bundled Shell extension).
`kdotool` talks to KWin only and does nothing on GNOME, and `auto` already
stops at `gnome` before reaching `kde`.

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

## Migrating from 1.2.x

Nothing to do beyond installing this build. Existing configs work unchanged and
gain nothing unless you opt into `[foreground]`.

If you were running a `1.3.0-beta.N` build, also nothing to do — the stable
release is the consolidated, version-bumped equivalent of `1.3.0-beta.2`.

## Installing

```bash
cargo install wayland-wheeltani
```

Or from the release archive attached to the GitHub release
(`wayland-wheeltani-v1.3.0-linux-*.tar.gz`). See the
[Installation](https://github.com/docloulou/Wayland-Wheeltani/wiki/Installation)
wiki page for all install options (release archives, building from source,
cross-compiling, uninstall).

First-time setup (udev rule + systemd `--user` service):

```bash
sudo "$HOME/.cargo/bin/wayland-wheeltani" --setup --install-udev-rule
sudo udevadm control --reload-rules
wayland-wheeltani --install-service
wayland-wheeltani --start
```

## Maintainer: cutting this release

The workspace version was bumped from `1.3.0-beta.2` to `1.3.0` (a stable
release, not a `-beta.N` increment):

```bash
scripts/bump-version.sh --version 1.3.0
```

Then trigger the "Release Linux binaries" workflow (`workflow_dispatch`) from
`main` with the version input `v1.3.0`. It builds the Linux binaries, creates a
GitHub release tagged `v1.3.0`, and publishes both workspace crates to crates.io
as a stable release using `CARGO_REGISTRY_TOKEN`.
