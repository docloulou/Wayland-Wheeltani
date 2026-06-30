# Wayland-Wheeltani Foreground (GNOME Shell extension)

GNOME (Wayland) does not expose a portable API for the focused window, so the
`gnome` foreground provider relies on this small extension. It publishes the
focused window on the **session bus** and the daemon reads it through `gdbus`.

- Bus name: `org.docloulou.WheeltaniForeground`
- Object path: `/org/docloulou/WheeltaniForeground`
- Method: `GetFocused() -> s` returns the focused window as JSON (or `{}`)
- Signal: `FocusedChanged(s)` emitted on every focus change

JSON shape (all fields optional):

```json
{"app_id":"org.mozilla.firefox","class":"firefox","resource_class":"Navigator","title":"Mozilla Firefox","pid":1234}
```

## Install

```bash
integrations/gnome/install.sh
```

This copies the extension to
`~/.local/share/gnome-shell/extensions/wheeltani-foreground@docloulou.github.io/`
and enables it.

On Wayland you must **log out and back in** for GNOME Shell to load a newly
installed extension. Then make sure it is enabled:

```bash
gnome-extensions enable wheeltani-foreground@docloulou.github.io
```

## Verify

```bash
gdbus call --session --dest org.docloulou.WheeltaniForeground \
  --object-path /org/docloulou/WheeltaniForeground \
  --method org.docloulou.WheeltaniForeground.GetFocused
```

You should get a `('{...}',)` tuple describing the focused window.

## Use it

In your Wayland-Wheeltani config (`~/.config/Wayland-Wheeltani/config.toml`):

```toml
[foreground]
enabled = true
provider = "gnome"   # or "auto" (GNOME is auto-detected when this extension runs)
mode = "denylist"
deny_apps = ["firefox", "org.mozilla.firefox"]
```

The daemon must run inside your GNOME session (the bundled systemd `--user`
service does) so it can reach the session bus.

## Privacy

The extension is read-only: it reports only the focused window's identity to the
local session bus. It never accesses the network and never persists anything.
Window titles are sent on the bus but the daemon only logs them at `trace`
level and ignores them for matching unless you set `match_title = true`.

## Uninstall

```bash
gnome-extensions disable wheeltani-foreground@docloulou.github.io
rm -rf ~/.local/share/gnome-shell/extensions/wheeltani-foreground@docloulou.github.io
```
