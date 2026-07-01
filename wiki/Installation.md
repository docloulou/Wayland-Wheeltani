# Installation

## Requirements

- Linux Wayland desktop session.
- A physical mouse exposed through `/dev/input/eventX`.
- `/dev/uinput` available (`sudo modprobe uinput` if missing).
- `systemd --user` for the recommended service installation.

For daily use without running the daemon as root, install a targeted udev rule
for your mouse and `/dev/uinput`. Wayland-Wheeltani can generate that rule for
USB mice with `ID_VENDOR_ID` and `ID_MODEL_ID` udev properties.

## Option A: install with Cargo

```bash
cargo install wayland-wheeltani
```

`cargo install` only installs the binary. It does not run setup prompts, install
udev rules, or create systemd services automatically.

It also installs `wlw`, a short alias for the exact same binary — every
command below works identically with `wlw` instead of `wayland-wheeltani`.

If `wayland-wheeltani` is not found after install, make sure Cargo's bin
directory is on your `PATH`:

```bash
. "$HOME/.cargo/env"      # try Cargo's env file first
wayland-wheeltani --version

# or add the bin directory directly:
export PATH="$HOME/.cargo/bin:$PATH"
hash -r
wayland-wheeltani --version
```

For a permanent setup, add one of these to your shell config (`~/.bashrc`,
`~/.zshrc`, ...):

```bash
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
# or, if ~/.cargo/env is missing or does not work:
export PATH="$HOME/.cargo/bin:$PATH"
```

> Do not install with `sudo cargo install`; that installs the binary for `root`
> instead of your normal user.

### First-time setup (recommended user service)

```bash
sudo "$HOME/.cargo/bin/wayland-wheeltani" --setup --install-udev-rule
sudo udevadm control --reload-rules
wayland-wheeltani --install-service
```

The first command runs with `sudo` because it writes
`/etc/udev/rules.d/60-wayland-wheeltani.rules`; it still saves the config for the
original `SUDO_USER`. The last command must run **without** `sudo`; it installs
and starts the `systemd --user` service.

The explicit `"$HOME/.cargo/bin/wayland-wheeltani"` path avoids the common error
`sudo: wayland-wheeltani: command not found` (many systems reset `PATH` under
`sudo`, so root cannot find binaries installed by `cargo install` for your user).

## Option B: install from a release archive

Download the archive matching your Linux architecture from the GitHub release:

- `wayland-wheeltani-vX.Y.Z-linux-x86_64-gnu.tar.gz`
- `wayland-wheeltani-vX.Y.Z-linux-aarch64-gnu.tar.gz`

```bash
tar -xzf wayland-wheeltani-vX.Y.Z-linux-x86_64-gnu.tar.gz
install -Dm755 wayland-wheeltani-vX.Y.Z-linux-x86_64-gnu/wayland-wheeltani \
  ~/.local/bin/wayland-wheeltani

# Same setup flow:
sudo ~/.local/bin/wayland-wheeltani --setup --install-udev-rule
sudo udevadm control --reload-rules
~/.local/bin/wayland-wheeltani --install-service
```

## Build from source

### Native Linux build

```bash
sudo apt update
sudo apt install -y build-essential pkg-config
cargo build --release -p wayland-wheeltani
# Binaries: target/release/wayland-wheeltani and target/release/wlw (alias)

install -Dm755 target/release/wayland-wheeltani ~/.local/bin/wayland-wheeltani
sudo ~/.local/bin/wayland-wheeltani --setup --install-udev-rule
sudo udevadm control --reload-rules
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
# x86_64:
rustup target add x86_64-unknown-linux-gnu
cargo zigbuild --release -p wayland-wheeltani --target x86_64-unknown-linux-gnu
```

## Manage the user service

```bash
wayland-wheeltani --start
wayland-wheeltani --stop
wayland-wheeltani --restart
journalctl --user -u wayland-wheeltani -f
```

## Uninstall

```bash
wayland-wheeltani --remove-service
sudo "$(command -v wayland-wheeltani)" --remove-udev-rule
```

If `sudo "$(command -v wayland-wheeltani)" ...` cannot resolve the binary, use
the absolute install path instead (`"$HOME/.cargo/bin/wayland-wheeltani"` for
Cargo installs, or `"$HOME/.local/bin/wayland-wheeltani"` for release archives).

If installed with Cargo, remove the binary with:

```bash
cargo uninstall wayland-wheeltani
```
