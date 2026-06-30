# Foreground filter (per-application on/off)

Turn autoscroll on or off depending on the focused application — for example to
keep the native middle-click in a browser, Steam or Blender while keeping
progressive autoscroll everywhere else. This is **off by default**; an
unconfigured daemon behaves exactly as before.

## Quick start

Add an optional `[foreground]` table to your config:

```toml
[foreground]
enabled = true
provider = "auto"          # auto | none | hyprland | sway | gnome | command
mode = "denylist"          # denylist | allowlist
unknown_policy = "enabled" # what to do when the app can't be determined
deny_apps = ["firefox", "org.mozilla.firefox", "steam", "blender"]
# allow_apps = ["code", "chromium"]   # used when mode = "allowlist"
# match_title = false                  # also match the window title (off by default)
```

Not sure what to put in the lists? Use **[`--detect-foreground`](#finding-an-applications-identifier)**.

## How it behaves

- When an app is **disabled**, the daemon does not run autoscroll for it; it
  forwards the mouse events untouched, so the middle click, middle-drag and the
  wheel keep working normally there. (Because the daemon grabs the device, it
  re-emits the events on the virtual mouse instead of dropping them.)
- The decision is taken **once at middle-button press and held until release**,
  so changing focus mid-scroll never leaves a button stuck or a scroll
  half-done. It is also cleared on a physical-device hot-reconnect.
- Matching is **case-insensitive** against `app_id`, `class` and
  `resource_class` (a trailing `.desktop` is ignored). The title is matched only
  with `match_title = true`.
- `unknown_policy = "enabled"` (default) keeps autoscroll on when the focused app
  is unknown, so a missing provider never breaks your setup. Set it to
  `"disabled"` to pass events through whenever the app cannot be determined.

## Providers and `provider = "auto"`

`auto` selects the first provider available for your session, in this order:

1. **hyprland** — reads the Hyprland event socket. No helper needed.
2. **sway** / i3 — reads the Sway/i3 IPC. No helper needed.
3. **gnome** — needs the bundled GNOME Shell extension (see
   **[GNOME setup](GNOME-Setup)**).
4. **command** — runs your own command (KWin, unsupported compositors, scripts).
5. **none** — filter inert.

Detection relies on the **session bus / compositor sockets**, not on desktop
environment variables, so it works from a `systemd --user` service even when
`XDG_CURRENT_DESKTOP` is not exported.

> The daemon must run inside your graphical session to see the focused window.
> The bundled `systemd --user` service does; running it as root (plain `sudo`)
> will not have access to the compositor/session, and the filter falls back to
> `unknown_policy`.

## Finding an application's identifier

Run the detector, focus the target window during the short countdown, and it
prints the focused app's identity and the exact string to copy into `deny_apps`
/ `allow_apps`. It reuses the same provider selection as the daemon (so it works
on every backend) and does not touch the mouse:

```bash
wayland-wheeltani --detect-foreground
# Focus the window you want to identify; reading the focused app in 3s...
# Detected foreground application (source: Gnome):
#   app_id         : org.gnome.SystemMonitor
#   class          : org.gnome.SystemMonitor
#   resource_class : org.gnome.SystemMonitor
#   title          : System Monitor
#   pid            : 12345
#
# Use one of these identifiers (case-insensitive, `.desktop` ignored):
#     org.gnome.systemmonitor
```

> **Wayland vs XWayland:** a native Wayland app exposes an `app_id` (e.g.
> `org.mozilla.firefox`), while an XWayland app exposes a WM
> `class`/`resource_class` (e.g. `firefox`). When in doubt, list both spellings.

## GNOME

GNOME (Wayland) has no portable focused-window API, so the `gnome` provider uses
a small bundled GNOME Shell extension that publishes the focused window on the
session bus; the daemon reads it through `gdbus`. See **[GNOME setup](GNOME-Setup)**.

## KWin / other compositors (`command` provider)

Use `provider = "command"` with a script that prints the focused app id (plain
text) or a JSON object `{"app_id":"...","class":"...","title":"...","pid":123}`:

```toml
[foreground]
enabled = true
provider = "command"
mode = "denylist"
deny_apps = ["krita"]
command = ["my-focused-app-script"]
command_refresh_ms = 500
```

The command is run every `command_refresh_ms` milliseconds; its latest output is
used as the focused application.

## Troubleshooting

If the filter does not enable/disable the right apps, see the
**[Troubleshooting](Troubleshooting#the-foreground-filter-does-not-disableenable-the-right-apps)**
page.
