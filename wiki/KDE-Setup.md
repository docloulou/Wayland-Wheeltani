# KDE Plasma / KWin setup

KWin (Wayland) has no portable, readable D-Bus API for the focused window — the
only reliable route is its scripting API. Instead of shipping an untested
in-house KWin script, the `kde` foreground provider relies on
[`kdotool`](https://github.com/jinliu/kdotool), a small, well-maintained
xdotool-like helper for Plasma 6 that drives KWin's scripting API for you. The
daemon polls `kdotool` on a background thread (the input path never blocks).

This is only needed if you use the **[Foreground filter](Foreground-Filter)**
with `provider = "kde"` (or `provider = "auto"` on KDE). Plain autoscroll does
not require it.

> **Tested status:** the `kde` provider has **not** been verified by the author
> on a real Plasma session yet — feedback is very welcome.

## Install kdotool

Use your distribution's package if available, or install it with Cargo:

```bash
cargo install kdotool
# make sure it is on PATH:
kdotool --help
```

`kdotool` works with **KDE Plasma 6** (Wayland and X11). Plasma 5 is not
supported by recent kdotool versions.

## Enable the provider

```toml
[foreground]
enabled = true
provider = "kde"     # or "auto" — it detects KDE automatically when kdotool is present
mode = "denylist"
deny_apps = ["firefox", "org.kde.dolphin"]
command_refresh_ms = 500
```

`auto` selects KDE when KWin is on the session bus **and** `kdotool` is
installed. If `gdbus` is not available on your system, auto-detection can't see
KWin — in that case set `provider = "kde"` explicitly.

## Verify it works

Check that `kdotool` resolves the active window (focus another window first):

```bash
kdotool getactivewindow getwindowclassname
# e.g. -> org.kde.dolphin   (or firefox for an X/XWayland app)
```

Then confirm Wayland-Wheeltani sees the same identity:

```bash
wayland-wheeltani --detect-foreground
# Detected foreground application (source: Kde):
#   class          : org.kde.dolphin
#   resource_class : org.kde.dolphin
#   ...
```

Copy the printed identifier into `deny_apps` / `allow_apps`.

## Notes

- The provider returns the window **resource class** (used for both `class` and
  `resource_class`). For native KDE apps this is usually the desktop id, e.g.
  `org.kde.dolphin`; for X/XWayland apps it is the WM class, e.g. `firefox`. The
  window title and pid are not exposed by this provider.
- The daemon must run inside your Plasma session (the bundled `systemd --user`
  service does) so `kdotool` can reach KWin. Running it as root via plain `sudo`
  cannot reach your session.
- `command_refresh_ms` controls the poll interval (default 500 ms). Each poll
  asks `kdotool` once; the decision itself is still taken at middle-button press
  using the latest polled value.

## Alternative: without kdotool (a KWin script)

If you would rather not install `kdotool`, use the generic `command` provider
with the bundled example script
[`integrations/kde/wheeltani-kwin-active-window.sh`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/integrations/kde/wheeltani-kwin-active-window.sh).
It loads a tiny KWin script over D-Bus (the same trick `kdotool` uses) and prints
the focused window's resource class:

```toml
[foreground]
enabled = true
provider = "command"
mode = "denylist"
deny_apps = ["org.kde.dolphin", "firefox"]
command = ["/full/path/to/integrations/kde/wheeltani-kwin-active-window.sh"]
command_refresh_ms = 500
```

Test it directly first (focus another window, then run it):

```bash
integrations/kde/wheeltani-kwin-active-window.sh
# -> org.kde.dolphin   (or firefox for an X/XWayland app)
```

What the script does, step by step:

1. writes a one-shot KWin script that `print()`s
   `workspace.activeWindow.resourceClass`;
2. loads + runs + unloads it through the `org.kde.kwin.Scripting` D-Bus interface
   (via `qdbus6` / `qdbus-qt6` / `qdbus`);
3. scrapes the printed line back out of the systemd journal.

Caveats: it needs `qdbus6` and `journalctl` (Plasma 6); journal scraping adds
latency; and if your KWin logs to the **system** journal instead of the user
journal, drop the `--user` flag on the `journalctl` line in the script. It is an
untested example — the `kdotool` path above is preferred. See
[`integrations/kde/README.md`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/integrations/kde/README.md)
for details.
