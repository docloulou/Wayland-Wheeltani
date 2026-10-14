# GNOME setup

GNOME (Wayland) has no portable focused-window API, so the `gnome` foreground
provider relies on a small bundled GNOME Shell extension. The extension
publishes the focused window on the session bus, and the daemon reads it through
`gdbus` (a low-latency `gdbus monitor` push path plus a periodic resync).

This is only needed if you use the **[Foreground filter](Foreground-Filter)**
or per-application `[[profiles]]` with `provider = "gnome"` (or
`provider = "auto"` on GNOME). Plain autoscroll does not require it.

The extension supports GNOME Shell 45 to 50.

## Install the extension

The extension lives in
[`integrations/gnome/`](https://github.com/docloulou/Wayland-Wheeltani/tree/main/integrations/gnome)
(UUID `wheeltani-foreground@docloulou.github.io`).

```bash
integrations/gnome/install.sh
# On Wayland, log out and back in so GNOME Shell loads the extension, then:
gnome-extensions enable wheeltani-foreground@docloulou.github.io
```

After it is enabled, `provider = "auto"` detects GNOME automatically, or set
`provider = "gnome"` explicitly.

### Upgrading the extension

Version 2 of the extension also announces title changes (a browser switching
tabs, an editor opening another file), so `match_title` rules follow them
without waiting for the periodic resync. Re-run `integrations/gnome/install.sh`
from the new release, then log out and back in. `gnome-extensions info
wheeltani-foreground@docloulou.github.io` shows the installed version.

## Verify it works

Check that the extension is enabled:

```bash
gnome-extensions list --enabled | grep wheeltani-foreground
```

Query the focused window directly over D-Bus (this is exactly what the daemon
does):

```bash
gdbus call --session \
  --dest org.docloulou.WheeltaniForeground \
  --object-path /org/docloulou/WheeltaniForeground \
  --method org.docloulou.WheeltaniForeground.GetFocused
```

A working setup returns a single JSON string, for example:

```text
('{"class":"firefox","resource_class":"firefox","title":"...","pid":12345}',)
```

You can also confirm `--detect-foreground` sees it:

```bash
wayland-wheeltani --detect-foreground
```

## Notes

- The daemon must run inside your GNOME session (the bundled `systemd --user`
  service does) so it can reach the session bus. Running it as root via plain
  `sudo` cannot reach your user bus and the GNOME provider will report
  `gdbus`/ownership failures.
- Detection does **not** depend on `XDG_CURRENT_DESKTOP`; it checks whether the
  extension's bus name is owned, so it works even when the service environment
  does not export desktop variables. If the service starts before GNOME Shell
  has loaded the extension, `provider = "auto"` keeps checking in the background
  and switches to GNOME as soon as the extension appears.
- See
  [`integrations/gnome/README.md`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/integrations/gnome/README.md)
  for the full D-Bus API and privacy notes.
