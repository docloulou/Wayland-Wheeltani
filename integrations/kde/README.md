# KDE Plasma / KWin integration

Two ways to drive the **[foreground filter](../../wiki/Foreground-Filter.md)** on
KDE Plasma 6 (KWin / Wayland):

## 1. Recommended: `provider = "kde"` + kdotool

Install [`kdotool`](https://github.com/jinliu/kdotool) (`cargo install kdotool`
or your distro's package) and let the daemon talk to it:

```toml
[foreground]
enabled = true
provider = "kde"           # or "auto"
mode = "denylist"
deny_apps = ["org.kde.dolphin", "firefox"]
command_refresh_ms = 500
```

This is the preferred path. The daemon asks kdotool for the focused window's
class **and title** in one call (`getactivewindow getwindowclassname
getwindowname`), so `match_title = true` works on KDE too; a kdotool too old
to chain both falls back to the class alone. See the
[KDE setup](../../wiki/KDE-Setup.md) wiki page.

Why polling and not a push-based KWin script? A KWin script can watch
`workspace.windowActivated`, but the only way for it to reach the daemon is a
D-Bus call, and receiving that call needs a D-Bus service inside the daemon.
The project deliberately uses no D-Bus crate (it only shells out to `gdbus`),
so the daemon polls instead; the decision is still taken once per
trigger press, from the latest poll.

## 2. No extra binary: `command` provider + a KWin script

If you would rather not install kdotool, use the generic `command` provider with
the bundled example script
[`wheeltani-kwin-active-window.sh`](wheeltani-kwin-active-window.sh). It prints
the focused window as JSON (desktop file name, resource class and name, title,
pid) by loading a tiny KWin script over D-Bus and scraping the result from the
journal.

```toml
[foreground]
enabled = true
provider = "command"
mode = "denylist"
deny_apps = ["org.kde.dolphin", "firefox"]
command = ["/full/path/to/integrations/kde/wheeltani-kwin-active-window.sh"]
command_refresh_ms = 500
```

Try it directly first (focus another window, then run it):

```bash
integrations/kde/wheeltani-kwin-active-window.sh
# -> {"app_id":"org.kde.dolphin","class":"dolphin","resource_class":"dolphin","title":"Home — Dolphin","pid":4242}
```

### How the script works

KWin (Wayland) has no readable "active window" D-Bus API. The script:

1. writes a one-shot KWin script that `print()`s `workspace.activeWindow` as
   JSON, behind a marker unique to that run;
2. loads + runs + unloads it through the `org.kde.kwin.Scripting` D-Bus interface
   (using `qdbus6` / `qdbus-qt6` / `qdbus`);
3. waits up to about a second for the line with its own marker in the systemd
   journal (user journal first, then the system journal) and prints it.

Because the marker is unique per run, a late line from an earlier run (the
daemon runs the script every `command_refresh_ms`) is never mistaken for the
current window. The script exits non-zero when KWin refuses the script or no
line arrives, and the daemon then treats the window as unknown.

### Caveats

- **Untested by the project author** — it is an example/fallback. The kdotool
  path (option 1) is preferred.
- Needs `qdbus6` (or `qdbus`) and `journalctl` (systemd), KDE Plasma 6.
- Journal scraping adds latency; the daemon polls it every `command_refresh_ms`
  (the decision itself is taken once at trigger press).

## Finding an app's identifier

Either method works with the detector, which prints the exact string to copy:

```bash
wayland-wheeltani --detect-foreground
```
