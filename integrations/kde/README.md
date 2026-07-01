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

This is the tested path. See the
[KDE setup](../../wiki/KDE-Setup.md) wiki page.

## 2. No extra binary: `command` provider + a KWin script

If you would rather not install kdotool, use the generic `command` provider with
the bundled example script
[`wheeltani-kwin-active-window.sh`](wheeltani-kwin-active-window.sh). It prints
the focused window's resource class by loading a tiny KWin script over D-Bus
(the same trick kdotool uses) and scraping the result from the journal.

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
# -> org.kde.dolphin   (or firefox for an X/XWayland app)
```

### How the script works

KWin (Wayland) has no readable "active window" D-Bus API. The script:

1. writes a one-shot KWin script that `print()`s `workspace.activeWindow.resourceClass`;
2. loads + runs + unloads it through the `org.kde.kwin.Scripting` D-Bus interface
   (using `qdbus6` / `qdbus-qt6` / `qdbus`);
3. scrapes the printed line back out of the systemd journal.

### Caveats

- **Untested by the project author** — it is an example/fallback. The kdotool
  path (option 1) is preferred.
- Needs `qdbus6` (or `qdbus`) and `journalctl` (systemd), KDE Plasma 6.
- Journal scraping adds latency; the daemon polls it every `command_refresh_ms`
  (the decision itself is taken once at middle-button press).
- If your KWin logs to the **system** journal rather than the user journal, edit
  the script and drop the `--user` flag on the `journalctl` line.

## Finding an app's identifier

Either method works with the detector, which prints the exact string to copy:

```bash
wayland-wheeltani --detect-foreground
```
