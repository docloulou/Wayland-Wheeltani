# Wayland-Wheeltani 1.4.0

This release introduces **live daemon control** (`--status`, `--pause`, `--resume`, `--toggle`, `--reload`), an **interactive `--tune` prompt**, **hot config reload** (via file watcher and `SIGHUP`), **per-application `[[profiles]]`**, **toggle mode** (`toggle_progressive`), a configurable **`trigger_button`**, **`axis_lock`**, **kinetic scrolling / inertia**, **multi-mouse support** (`[[device_match]]`), **indicator `[hooks]`**, shell completions, man pages, and distribution packaging — alongside major input, foreground-provider, and udev security fixes.

All new features are opt-in or backward compatible with existing `1.3.x` configurations.

## Highlights

### Live daemon control & hot reload

Control a running daemon over `$XDG_RUNTIME_DIR/wayland-wheeltani/control.sock` (`control_socket = true` by default):

```sh
wlw --status    # JSON status: state, mode, active profile, connected mice
wlw --toggle    # toggle between active and paused (ideal for a compositor keybind)
wlw --pause     # pause autoscroll; pass all mouse events straight through
wlw --resume    # resume autoscroll
wlw --reload    # reload config.toml immediately
```

The daemon also reloads `config.toml` automatically when the file is saved (`watch_config = true`) or when it receives `SIGHUP` (`systemctl --user reload wayland-wheeltani.service`). If an edited config is invalid, the error is logged and the current settings stay active.

### Interactive live tuning (`--tune`)

Adjust speed, deadzone, scroll curve, mode, axis lock, or inertia in real time while testing the feel, then write the result back to `config.toml` (preserving comments and formatting):

```sh
wlw --tune
# Commands inside --tune: show, get <key>, set <key> <value>, save, reload, reset, quit
```

When a per-application profile is active and overrides a key, `get` and `set` also display the effective value from that profile.

### Per-application `[[profiles]]`

Override any scroll engine setting when specific applications are focused (requires `[foreground] enabled = true`):

```toml
[[profiles]]
name = "browsers"
apps = ["firefox", "chromium"]
mode = "toggle_progressive"
inertia = true
inertia_half_life_ms = 220

[[profiles]]
name = "terminals"
apps = ["kitty", "org.wezfurlong.wezterm"]
axis_lock = "dominant"
max_speed_detents_per_second = 35.0
```

### Toggle mode, custom trigger button, axis lock & kinetic scrolling (inertia)

- **Toggle mode** (`mode = "toggle_progressive"`): a short click on the trigger button latches autoscroll without holding the button down; the next click stops it cleanly.
- **Custom trigger button** (`trigger_button`): trigger autoscroll with `middle` (default), `side`, `extra`, `forward`, `back`, or `task`.
- **Axis lock** (`axis_lock = "off" | "dominant" | "first"`): lock scrolling to a single axis per gesture (`first` locks to the first axis that leaves the deadzone; `dominant` follows whichever axis currently has the larger displacement).
- **Kinetic scrolling / inertia** (`inertia = true`, `inertia_half_life_ms = 180`): coast smoothly after releasing the scroll gesture; pressing the trigger button while coasting catches the scroll without emitting a click.

### Multi-mouse support (`[[device_match]]`) & `--add-device`

Use several mice simultaneously (for example a dock mouse and a wireless travel mouse). Repeat `[[device_match]]` tables in `config.toml` or append a new mouse interactively:

```sh
sudo wayland-wheeltani --add-device
```

### Indicator `[hooks]`

Run custom shell commands (for OSDs, Waybar modules, or notifications) when autoscroll starts/stops or when the daemon is paused/resumed (`WHEELTANI_STATE` is set in the child environment):

```toml
[hooks]
on_scroll_start = "notify-send -t 800 Wheeltani 'Autoscroll on'"
on_scroll_stop  = "true"
on_pause        = "pkill -RTMIN+9 waybar"
on_resume       = "pkill -RTMIN+9 waybar"
```

### Shell completions, man pages & distro packaging

- Generate completions (`bash`, `zsh`, `fish`, `elvish`, `powershell`) and man pages directly from either binary:
  ```sh
  wlw --completions zsh
  wlw --man | man -l -
  ```
- Release archives now include pre-generated man pages and shell completions alongside both `wayland-wheeltani` and `wlw`.
- Added distribution packaging files in `packaging/` (Arch `PKGBUILD`, `cargo-deb` and `cargo-generate-rpm` metadata, packaged systemd user unit, and a `flake.nix`).

## Important fixes & improvements

- **Zero idle wakeups & frame-batched motion**: the daemon now blocks in `poll()` until input arrives instead of waking 120 times per second while idle, and forwards pointer motion one kernel `SYN_REPORT` frame at a time so diagonal motion preserves libinput pointer acceleration.
- **Full button & axis passthrough**: all mouse buttons (`BTN_TASK`, `0x118`–`0x11f`, keys) and extra axes on the physical mouse are mirrored to the virtual device and passed through.
- **Synthesized hi-res wheel events**: mice that only expose standard wheel axes now also get synthesized `REL_WHEEL_HI_RES` / `REL_HWHEEL_HI_RES` events (`120` units per detent).
- **Immediate direction reversal**: reversing scroll direction resets the opposite-sign accumulator remainder immediately instead of paying it back first.
- **Foreground providers**:
  - `provider = "auto"` keeps probing in the background when the service starts before the compositor or GNOME Shell extension.
  - GNOME Shell extension v2 adds **GNOME 49 and 50** support, publishes window title changes, and prevents slow resyncs from overwriting newer D-Bus signals.
  - Hyprland queries the initial active window on connect and treats empty focus as unknown (so `unknown_policy` applies).
  - Sway follows workspace switches and ignores bare workspace containers.
  - KDE reads window titles via `kdotool`, and the bundled KWin script uses a per-run marker and outputs JSON.
  - Helper subprocesses drain stdout/stderr concurrently and kill their entire process group on timeout.
- **Security (udev rule)**: `--install-udev-rule` and `contrib/60-wayland-wheeltani.rules` now match **pointer event nodes only** (`ID_INPUT_MOUSE=="1"`, Bluetooth mice included) and grant `TAG+="uaccess"` without exposing the keyboard interface of combined wireless receivers or gaming mice.

## Upgrading an existing installation

Upgrade the binaries, refresh the systemd user service unit (adds `RuntimeDirectory=wayland-wheeltani` for the control socket and `ExecReload` for `SIGHUP`), and optionally refresh the udev rule to apply the narrower pointer-only rule:

```sh
# 1. Upgrade binaries
cargo install wayland-wheeltani --version 1.4.0 --force

# 2. Refresh the udev rule (applies immediately)
sudo "$HOME/.cargo/bin/wayland-wheeltani" --install-udev-rule

# 3. Regenerate and restart the user service
wlw --install-service
wlw --restart
```

- **Manually customized user unit:** if you keep custom `[Unit]` settings in `~/.config/systemd/user/wayland-wheeltani.service`, add `RuntimeDirectory=wayland-wheeltani`, `RuntimeDirectoryMode=0700`, and `ExecReload=/bin/kill -HUP $MAINPID` under `[Service]`, then run `systemctl --user daemon-reload && wlw --restart`.
