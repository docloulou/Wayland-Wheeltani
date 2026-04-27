# Wayland-Wheeltani

Progressive middle-click autoscroll for Wayland.

Hold the middle mouse button, move vertically, and Wayland-Wheeltani emits smooth
wheel events through a virtual mouse. Release the middle button and scrolling
stops immediately. A short middle click still behaves like a normal middle click.

```text
hold middle button
  ├─ tiny movement inside deadzone       → normal middle click on release
  ├─ move down from press position       → continuous scroll down
  ├─ move farther from press position    → faster scroll
  ├─ return near press position          → scroll slows/stops
  └─ cross above press position          → scroll reverses
```

No GUI, no overlay, no network, no keyboard capture. The project is split into a
portable, unit-tested Rust core and a Linux backend using `evdev` + `uinput`.

## Status

- Core engine: implemented and unit-tested.
- Linux backend: implemented for Wayland compositors through `/dev/input` and
  `/dev/uinput`.
- macOS: supported as a development/build host only. The daemon itself runs on
  Linux because it needs evdev/uinput.
- End-to-end UX still needs real validation on an Ubuntu Wayland session with a
  physical mouse.

## Workspace layout

```text
crates/
├── middle-scroll-core/      # platform-independent state machine + tests
└── middle-scroll-linux/     # Linux daemon, CLI, config, evdev/uinput backend
contrib/
├── 60-wayland-wheeltani.rules
├── wayland-wheeltani.service
└── wayland-wheeltani-root.service
examples/
└── config.toml
```

## Build

### Linux native build

Use this when building directly on the target Linux machine.

```bash
sudo apt update
sudo apt install -y build-essential pkg-config
cargo build --release -p middle-scroll-linux
```

Binary output:

```text
target/release/wayland-wheeltani
```

### macOS development checks

The core can be tested natively on macOS:

```bash
cargo test -p middle-scroll-core
cargo clippy -p middle-scroll-core --all-targets -- -D warnings
```

### Cross-compile Linux ARM64 from macOS Apple Silicon

This is the recommended macOS -> Linux build path. Plain `cargo build --target`
fails at the linker step on macOS because Apple `ld` cannot link Linux ELF
binaries. `cargo-zigbuild` uses Zig as the Linux cross-linker.

```bash
brew install zig
cargo install cargo-zigbuild
rustup target add aarch64-unknown-linux-gnu
cargo zigbuild --release -p middle-scroll-linux --target aarch64-unknown-linux-gnu
```

Binary output:

```text
target/aarch64-unknown-linux-gnu/release/wayland-wheeltani
```

### Cross-compile Linux x86_64 from macOS

```bash
brew install zig
cargo install cargo-zigbuild
rustup target add x86_64-unknown-linux-gnu
cargo zigbuild --release -p middle-scroll-linux --target x86_64-unknown-linux-gnu
```

Binary output:

```text
target/x86_64-unknown-linux-gnu/release/wayland-wheeltani
```

### Copy a cross-built binary to Linux

```bash
scp target/aarch64-unknown-linux-gnu/release/wayland-wheeltani user@linux-box:~/.local/bin/
ssh user@linux-box 'chmod +x ~/.local/bin/wayland-wheeltani'
```

Verify on the Linux machine:

```bash
~/.local/bin/wayland-wheeltani --help
```

## Quick start on Ubuntu Wayland

During development, start with `sudo` plus a short safety timeout:

```bash
sudo ./target/release/wayland-wheeltani --list-devices
sudo ./target/release/wayland-wheeltani --setup
sudo ./target/release/wayland-wheeltani --dry-run --verbose --safety-timeout-seconds 60
sudo ./target/release/wayland-wheeltani --safety-timeout-seconds 120
```

`--setup` lists candidate mice, lets you choose one, and saves it to the config
file. If the command is run through `sudo`, the default config path is resolved
for the original `SUDO_USER` and the file ownership is restored to that user.

Default config path:

```text
~/.config/Wayland-Wheeltani/config.toml
```

Override config path:

```bash
wayland-wheeltani --config ./my-config.toml --setup
wayland-wheeltani --config ./my-config.toml --no-interactive
```

## CLI reference

```text
wayland-wheeltani [OPTIONS]

Options:
  --device <PATH>                 evdev node, e.g. /dev/input/event12
  --config <FILE>                 override config path
  --setup                         choose a mouse interactively, save config, exit
  --list-devices                  list candidate mice and exit
  --no-grab                       do not grab the physical mouse exclusively
  --dry-run                       do not create /dev/uinput; log actions only
  --no-interactive                never prompt; fail if no device is configured
  --safety-timeout-seconds <N>    auto-exit after N seconds
  -v, --verbose                   -v: debug logs, -vv: trace logs
```

Precedence is:

```text
CLI flags > config file > built-in defaults
```

See [`examples/config.toml`](examples/config.toml) for every tunable option.

### Scroll speed steps

Scroll speed is configurable by distance from the original middle-button press
point. The config uses ordered `[[scroll_speed_steps]]` entries:

```toml
[[scroll_speed_steps]]
distance_units = 40
speed_detents_per_second = 4.0

[[scroll_speed_steps]]
distance_units = 80
speed_detents_per_second = 10.0
```

The last reached distance step wins. So if the pointer is 90 units away from the
press point, the example above scrolls at `10.0` detents/s. Direction is still
based on whether the pointer moved above or below the original press point.

Set `scroll_speed_steps = []` to disable stepped mode and use the continuous
fallback curve controlled by `min_speed_detents_per_second`,
`max_speed_detents_per_second`, `full_speed_units`, and
`acceleration_exponent`.

## Does it need sudo?

For initial testing: **sudo is the simplest path** because the daemon needs to:

1. read the physical mouse from `/dev/input/eventX`, and
2. create a virtual mouse through `/dev/uinput`.

For normal daily use: **sudo is not required** if you install a targeted udev
rule that grants the active desktop user access to exactly the mouse and uinput
device.

Recommended approach:

- use `TAG+="uaccess"` in udev rules;
- match a specific USB vendor/product ID for the physical mouse;
- avoid adding your user to the broad `input` group, because that grants access
  to keyboards too.

Find your mouse IDs:

```bash
lsusb
udevadm info -a -n /dev/input/event12 | less
```

Install a device-specific rule:

```bash
sudo install -Dm644 contrib/60-wayland-wheeltani.rules /etc/udev/rules.d/60-wayland-wheeltani.rules
sudoedit /etc/udev/rules.d/60-wayland-wheeltani.rules
# Replace REPLACE_VENDOR_ID and REPLACE_PRODUCT_ID.

sudo udevadm control --reload-rules
sudo udevadm trigger
```

Verify access:

```bash
udevadm info -q property -n /dev/input/event12 | grep TAGS
getfacl /dev/input/event12
getfacl /dev/uinput
```

After that, run as your normal user:

```bash
~/.local/bin/wayland-wheeltani --setup
~/.local/bin/wayland-wheeltani --no-interactive
```

The systemd unit provided in `contrib/` is a **user service**. It does **not**
run with `sudo` automatically. It only works without sudo after the udev rule
has granted your normal desktop user access to the selected `/dev/input/eventX`
node and to `/dev/uinput`.

## systemd services

There are two supported service styles:

| Mode | Runs as | Needs udev rule? | Install command | Recommended use |
|---|---|---:|---|---|
| **User service** | your desktop user | yes | `systemctl --user ...` | daily use, least privilege |
| **Root service** | root | no | `sudo systemctl ...` | simpler setup, more privileged |

Use **one** of the two service modes, not both at the same time.

### Option A: systemd user daemon (recommended)

This mode does not run with sudo. Install the udev rule first so your normal
desktop user can open `/dev/input/eventX` and `/dev/uinput`.

Install the binary and config:

```bash
mkdir -p ~/.local/bin ~/.config/Wayland-Wheeltani ~/.config/systemd/user
cp target/release/wayland-wheeltani ~/.local/bin/
cp examples/config.toml ~/.config/Wayland-Wheeltani/config.toml
```

Run setup once interactively:

```bash
~/.local/bin/wayland-wheeltani --setup
```

Install and start the user service:

```bash
cp contrib/wayland-wheeltani.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wayland-wheeltani.service
journalctl --user -u wayland-wheeltani -f
```

The service uses `--no-interactive`, so it will fail loudly instead of blocking
on a prompt if the device is not configured. Do not start this unit with
`sudo systemctl`; use `systemctl --user` as the desktop user.

### Option B: systemd root daemon (sudo/system service)

This mode runs the daemon as root through the system service manager. It does
not need the udev `uaccess` rule because root can open `/dev/input/eventX` and
`/dev/uinput` directly. It is easier to install, but it is more privileged than
the user service.

Install the binary globally:

```bash
sudo install -Dm755 target/release/wayland-wheeltani /usr/local/bin/wayland-wheeltani
```

Create the root-owned config directory:

```bash
sudo install -d -m 0755 /etc/wayland-wheeltani
```

Run setup once with sudo and an explicit system config path:

```bash
sudo /usr/local/bin/wayland-wheeltani \
  --setup \
  --config /etc/wayland-wheeltani/config.toml
```

Install and start the root service:

```bash
sudo install -Dm644 contrib/wayland-wheeltani-root.service \
  /etc/systemd/system/wayland-wheeltani.service
sudo systemctl daemon-reload
sudo systemctl enable --now wayland-wheeltani.service
sudo journalctl -u wayland-wheeltani -f
```

Stop/remove the root service:

```bash
sudo systemctl disable --now wayland-wheeltani.service
sudo rm -f /etc/systemd/system/wayland-wheeltani.service
sudo systemctl daemon-reload
```

If `/dev/uinput` does not exist yet:

```bash
sudo modprobe uinput
```

## Tray icon / top bar indicator

The current daemon is headless on purpose. It can run as a `systemd --user`
service without any GUI.

Showing an icon next to the clock on Ubuntu GNOME/Wayland is possible, but it
requires an AppIndicator/KStatusNotifier implementation plus GNOME’s
AppIndicator extension. GNOME Shell does not provide a native always-available
legacy tray for arbitrary daemons.

Practical plan for a future version:

1. keep `wayland-wheeltani` as the privileged/headless input daemon;
2. add a separate unprivileged `wayland-wheeltani-tray` helper;
3. have the helper expose AppIndicator/KStatusNotifier actions such as Start,
   Stop, Setup, Open Config, and Quit;
4. document the Ubuntu dependency:

```bash
sudo apt install gnome-shell-extension-appindicator libappindicator3-dev
gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com
```

This keeps input handling, permissions, and UI dependencies separate.

## How it works

```text
/dev/input/eventX
      │
      ▼
middle-scroll-linux
  ├─ reads physical mouse events through evdev
  ├─ optionally grabs the physical device
  ├─ routes events into middle-scroll-core
  └─ emits synthetic mouse/wheel events through /dev/uinput
      │
      ▼
Wayland compositor sees "Wayland-Wheeltani virtual mouse"
```

The virtual device emits standard mouse buttons, relative pointer motion,
legacy wheel detents, and hi-res wheel units (`REL_WHEEL_HI_RES`, 120 units per
detent). Legacy and hi-res wheel events are batched together for smoother app
compatibility.

## Troubleshooting

### `device not specified`

Run setup:

```bash
wayland-wheeltani --setup
```

For services/CI, use:

```bash
wayland-wheeltani --no-interactive --device /dev/input/event12
```

### `failed to create /dev/uinput virtual mouse`

Load uinput and check permissions:

```bash
sudo modprobe uinput
ls -l /dev/uinput
getfacl /dev/uinput
```

### `failed to grab device ... EBUSY`

Another program has an exclusive grab. Stop the previous daemon instance or any
`evtest -g`/debugging process.

### The cursor still moves while scrolling

You are probably running with `--no-grab`. Re-enable grabbing for normal use.

### Short middle clicks become scrolls too easily

Increase `deadzone_units` in the config.

## Security notes

`/dev/input/event*` is sensitive. Some devices expose keyboard input through
the same kernel interface. Wayland-Wheeltani filters mouse-like devices and
ignores keyboard events, but Linux permissions still matter.

Recommended security posture:

- do not run the daemon as root for daily use;
- do not add your user to the `input` group;
- install a udev rule that matches only your physical mouse;
- use a `systemd --user` service, not a system service;
- keep `--setup` interactive and keep services on `--no-interactive`.

## Development verification

```bash
cargo fmt --check
cargo test -p middle-scroll-core
cargo clippy -p middle-scroll-core --all-targets -- -D warnings
cargo clippy -p middle-scroll-linux --target aarch64-unknown-linux-gnu -- -D warnings
cargo clippy -p middle-scroll-linux --target x86_64-unknown-linux-gnu -- -D warnings
```

Build a Linux ARM64 release from macOS:

```bash
cargo zigbuild --release -p middle-scroll-linux --target aarch64-unknown-linux-gnu
```

## License

Dual-licensed under MIT or Apache-2.0, at your option.
