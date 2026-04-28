# Wayland-Wheeltani

Progressive middle-click autoscroll for Wayland.

Hold the middle mouse button, move vertically or horizontally, and
Wayland-Wheeltani emits smooth wheel events through a virtual mouse. Release the
middle button and scrolling stops immediately. A short middle click still behaves
like a normal middle click.

```text
hold middle button
  ├─ tiny movement inside deadzone       -> normal middle click on release
  ├─ move down from press position       -> continuous scroll down
  ├─ move right from press position      -> continuous horizontal scroll right
  ├─ move farther from press position    -> faster scroll
  ├─ return near press position          -> scroll slows/stops
  └─ cross the press position            -> scroll reverses on that axis
```

No GUI, no overlay, no network, no keyboard capture. The project is split into a
portable, unit-tested Rust core and a Linux backend using `evdev` + `uinput`.

## User Installation

### Requirements

- Linux Wayland desktop session.
- A physical mouse exposed through `/dev/input/eventX`.
- `/dev/uinput` available (`sudo modprobe uinput` if missing).
- `systemd --user` for the recommended service installation.

For daily use without running the daemon as root, install a targeted udev rule
for your mouse and `/dev/uinput`. Wayland-Wheeltani can generate that rule for
USB mice with `ID_VENDOR_ID` and `ID_MODEL_ID` udev properties.

### Option A: install with Cargo

```bash
cargo install wayland-wheeltani
```

`cargo install` only installs the binary. It does not run setup prompts, install
udev rules, or create systemd services automatically.

If `wayland-wheeltani` is not found after install, make sure Cargo's bin
directory is loaded in your shell. Try Cargo's env file first:

```bash
. "$HOME/.cargo/env"
wayland-wheeltani --version
```

If that file does not exist or the command is still not found, add Cargo's bin
directory to `PATH` directly:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
hash -r
wayland-wheeltani --version
```

For a permanent setup, add one of these lines to your shell config, for example
`~/.bashrc` for Bash or `~/.zshrc` for Zsh:

```bash
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
# or, if ~/.cargo/env is missing or does not work:
export PATH="$HOME/.cargo/bin:$PATH"
```

Do not install with `sudo cargo install`; that installs the binary for `root`
instead of your normal user.

First-time setup for the recommended user service:

```bash
sudo "$HOME/.cargo/bin/wayland-wheeltani" --setup --install-udev-rule
wayland-wheeltani --install-service
```

The first command runs with `sudo` because it writes
`/etc/udev/rules.d/60-wayland-wheeltani.rules`. It still saves the config for the
original `SUDO_USER`. The second command must run without `sudo`; it installs and
starts the `systemd --user` service.

The explicit `"$HOME/.cargo/bin/wayland-wheeltani"` path avoids the common error
`sudo: wayland-wheeltani: command not found`. Many systems reset `PATH` under
`sudo`, so root cannot find binaries installed by `cargo install` for your user.

### Option B: install from a release archive

Download the archive matching your Linux architecture from the GitHub release:

- `wayland-wheeltani-vX.Y.Z-linux-x86_64-gnu.tar.gz`
- `wayland-wheeltani-vX.Y.Z-linux-aarch64-gnu.tar.gz`

Install the binary:

```bash
tar -xzf wayland-wheeltani-vX.Y.Z-linux-x86_64-gnu.tar.gz
install -Dm755 wayland-wheeltani-vX.Y.Z-linux-x86_64-gnu/wayland-wheeltani \
  ~/.local/bin/wayland-wheeltani
```

Then run the same setup flow:

```bash
sudo ~/.local/bin/wayland-wheeltani --setup --install-udev-rule
~/.local/bin/wayland-wheeltani --install-service
```

### Manage the user service

```bash
wayland-wheeltani --start
wayland-wheeltani --stop
wayland-wheeltani --restart
journalctl --user -u wayland-wheeltani -f
```

Remove the user service and udev rule:

```bash
wayland-wheeltani --remove-service
sudo "$(command -v wayland-wheeltani)" --remove-udev-rule
```

If `sudo "$(command -v wayland-wheeltani)" ...` cannot resolve the binary, use
the absolute install path instead: `"$HOME/.cargo/bin/wayland-wheeltani"` for
Cargo installs or `"$HOME/.local/bin/wayland-wheeltani"` for release archives.

If installed with Cargo, remove the binary with:

```bash
cargo uninstall wayland-wheeltani
```

### CLI reference

```text
wayland-wheeltani [OPTIONS]

Options:
  --device <PATH>                 evdev node, e.g. /dev/input/event12
  --config <FILE>                 override config path
  --setup                         choose a mouse interactively and save config
  --install-udev-rule             install udev access rule for selected mouse and /dev/uinput
  --remove-udev-rule              remove the installed Wayland-Wheeltani udev rule
  --install-service               install, enable, and start the systemd user service
  --remove-service                stop, disable, and remove the systemd user service
  --start                         start the installed systemd user service
  --stop                          stop the installed systemd user service
  --restart                       restart the installed systemd user service
  --list-devices                  list candidate mice and exit
  --no-grab                       do not grab the physical mouse exclusively
  --dry-run                       do not create /dev/uinput; log actions only
  --no-interactive                never prompt; fail if no device is configured
  --safety-timeout-seconds <N>    auto-exit after N seconds
  -v, --verbose                   -v: debug logs, -vv: trace logs
```

Default config path:

```text
~/.config/Wayland-Wheeltani/config.toml
```

Precedence is:

```text
CLI flags > config file > built-in defaults
```

See [`examples/config.toml`](examples/config.toml) for every tunable option.

### Useful config options

Scroll speed is configured by distance from the original middle-button press
point. The same `[[scroll_speed_steps]]` apply to vertical and horizontal
autoscroll:

```toml
[[scroll_speed_steps]]
distance_units = 40
speed_detents_per_second = 4.0

[[scroll_speed_steps]]
distance_units = 80
speed_detents_per_second = 10.0
```

The last reached distance step wins. If the pointer is 90 units away from the
press point on either axis, the example above scrolls at `10.0` detents/s.

Other common options:

```toml
horizontal_scroll = true
invert_vertical = false
invert_horizontal = false
deadzone_units = 10
min_hires_units_per_event = 15
```

`min_hires_units_per_event` controls how many hi-res wheel units are accumulated
before one hi-res event is emitted. The default `15` gives 8 smooth samples per
detent (`120` units) and reduces tiny synthetic event spam in apps that stutter
under high-rate scrolling.

## Installation From Source

### Native Linux build

```bash
sudo apt update
sudo apt install -y build-essential pkg-config
cargo build --release -p wayland-wheeltani
```

Binary output:

```text
target/release/wayland-wheeltani
```

Install the locally built binary:

```bash
install -Dm755 target/release/wayland-wheeltani ~/.local/bin/wayland-wheeltani
```

Then run:

```bash
sudo ~/.local/bin/wayland-wheeltani --setup --install-udev-rule
~/.local/bin/wayland-wheeltani --install-service
```

### Install directly from the checked-out source tree

```bash
cargo install --path crates/middle-scroll-linux
```

### Cross-compile from macOS

The daemon runs on Linux only, but it can be cross-compiled from macOS with Zig:

```bash
brew install zig
cargo install cargo-zigbuild
rustup target add aarch64-unknown-linux-gnu
cargo zigbuild --release -p wayland-wheeltani --target aarch64-unknown-linux-gnu
```

For x86_64 Linux:

```bash
rustup target add x86_64-unknown-linux-gnu
cargo zigbuild --release -p wayland-wheeltani --target x86_64-unknown-linux-gnu
```

## Development

### Workspace layout

```text
crates/
├── middle-scroll-core/      # platform-independent state machine + tests
└── middle-scroll-linux/     # wayland-wheeltani CLI, config, evdev/uinput backend
contrib/
├── 60-wayland-wheeltani.rules
├── wayland-wheeltani.service
└── wayland-wheeltani-root.service
examples/
└── config.toml
```

### Verification

```bash
cargo fmt --check
cargo test -p middle-scroll-core
cargo test -p wayland-wheeltani
cargo clippy -p middle-scroll-core --all-targets -- -D warnings
cargo clippy -p wayland-wheeltani --all-targets -- -D warnings
cargo build --release -p wayland-wheeltani
```

On non-Linux hosts, workspace-default checks build only the portable core. Build
the Linux backend explicitly from Linux or with a Linux target.

### How it works

```text
/dev/input/eventX
      |
      v
wayland-wheeltani
  ├─ reads physical mouse events through evdev
  ├─ optionally grabs the physical device
  ├─ routes events into middle-scroll-core
  └─ emits synthetic mouse/wheel events through /dev/uinput
      |
      v
Wayland compositor sees "Wayland-Wheeltani virtual mouse"
```

The virtual device emits standard mouse buttons, relative pointer motion,
vertical and horizontal legacy wheel detents, and vertical and horizontal hi-res
wheel units. Legacy and hi-res wheel events are batched together for app
compatibility.

## Troubleshooting

### `wayland-wheeltani: command not found`

Check that Cargo installed the binary:

```bash
ls -l ~/.cargo/bin/wayland-wheeltani
~/.cargo/bin/wayland-wheeltani --version
```

Then load Cargo's environment file:

```bash
. "$HOME/.cargo/env"
wayland-wheeltani --version
```

If `~/.cargo/env` is missing or does not update `PATH`, export the path directly:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
hash -r
wayland-wheeltani --version
```

For a permanent fix, add one of these lines to `~/.bashrc`, `~/.zshrc`, or your
shell's equivalent startup file:

```bash
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
# or:
export PATH="$HOME/.cargo/bin:$PATH"
```

Avoid `sudo cargo install wayland-wheeltani`; it installs into root's Cargo
directory, not yours.

If the command works as your user but fails only with `sudo`, use the absolute
Cargo binary path for udev commands:

```bash
sudo "$HOME/.cargo/bin/wayland-wheeltani" --setup --install-udev-rule
sudo "$HOME/.cargo/bin/wayland-wheeltani" --remove-udev-rule
```

This is expected on systems where `sudo` resets `PATH`.

### `device not specified`

Run setup:

```bash
wayland-wheeltani --setup
```

For services or CI, pass a device explicitly:

```bash
wayland-wheeltani --no-interactive --device /dev/input/event12
```

### `udev rule installation requires root`

Install/remove udev rules with `sudo`:

```bash
sudo "$HOME/.cargo/bin/wayland-wheeltani" --setup --install-udev-rule
sudo "$HOME/.cargo/bin/wayland-wheeltani" --remove-udev-rule
```

Do not run `--install-service`, `--remove-service`, `--start`, `--stop`, or
`--restart` with `sudo`; those manage your normal user's `systemd --user`
service.

### `failed to find ID_VENDOR_ID and ID_MODEL_ID`

Automatic udev rule generation needs USB-style udev properties. Check the device:

```bash
udevadm info -q property -n /dev/input/event12
```

If the IDs are missing, install the template manually and replace the placeholders:

```bash
sudo install -Dm644 contrib/60-wayland-wheeltani.rules /etc/udev/rules.d/60-wayland-wheeltani.rules
sudoedit /etc/udev/rules.d/60-wayland-wheeltani.rules
sudo udevadm control --reload-rules
sudo udevadm trigger
```

### `failed to create /dev/uinput virtual mouse`

Load uinput and check permissions:

```bash
sudo modprobe uinput
ls -l /dev/uinput
getfacl /dev/uinput
```

### `failed to grab device ... EBUSY`

Another program has an exclusive grab. Stop any previous daemon instance or any
`evtest -g`/debugging process.

### The cursor still moves while scrolling

You are probably running with `--no-grab`. Re-enable grabbing for normal use.

### Short middle clicks become scrolls too easily

Increase `deadzone_units` in the config.

### Security notes

`/dev/input/event*` is sensitive. Some devices expose keyboard input through the
same kernel interface. Wayland-Wheeltani filters mouse-like devices and ignores
keyboard events, but Linux permissions still matter.

Recommended posture:

- do not run the daemon as root for daily use;
- do not add your user to the broad `input` group;
- install a udev rule that matches only your physical mouse;
- use a `systemd --user` service, not a system service;
- keep services on `--no-interactive`.

## License

Dual-licensed under MIT or Apache-2.0, at your option.
