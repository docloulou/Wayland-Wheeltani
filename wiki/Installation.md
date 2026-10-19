# Installation

## Requirements

- Linux Wayland desktop session.
- A physical mouse exposed through `/dev/input/eventX`.
- `/dev/uinput` available (`sudo modprobe uinput` if missing).
- `systemd --user` for the recommended service installation.

For daily use without running the daemon as root, install a targeted udev rule
for your mouse and `/dev/uinput`. Wayland-Wheeltani can generate that rule for
USB mice with `ID_VENDOR_ID` and `ID_MODEL_ID` udev properties. The rule gives
the logged-in user access to the mouse interfaces of the configured devices
only (never to the keyboard interface of a wireless receiver or gaming mouse),
and `--install-udev-rule` reloads udev and applies it at once.

## Option A: install with Cargo

```bash
cargo install wayland-wheeltani
```

`cargo install` only installs the binary. It does not run setup prompts, install
udev rules, or create systemd services automatically.

It also installs `wlw`, a shorter name for the same program — every command
below works identically with `wlw` instead of `wayland-wheeltani`.

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
wayland-wheeltani --install-service
```

The first command runs with `sudo` because it writes
`/etc/udev/rules.d/60-wayland-wheeltani.rules`; it still saves the config for the
original `SUDO_USER`. The last command must run **without** `sudo`; it installs
and starts the `systemd --user` service.

The explicit `"$HOME/.cargo/bin/wayland-wheeltani"` path avoids the common error
`sudo: wayland-wheeltani: command not found` (many systems reset `PATH` under
`sudo`, so root cannot find binaries installed by `cargo install` for your user).

### Shell completions and man pages

`cargo install` does not install them, but both programs print their own:

```bash
mkdir -p ~/.local/share/bash-completion/completions ~/.local/share/man/man1
wlw --completions bash > ~/.local/share/bash-completion/completions/wlw
wayland-wheeltani --completions bash > ~/.local/share/bash-completion/completions/wayland-wheeltani
wayland-wheeltani --man > ~/.local/share/man/man1/wayland-wheeltani.1
```

`--completions` also accepts `zsh`, `fish`, `elvish` and `powershell`. Release
archives and distribution packages already ship the man pages and the bash,
zsh and fish completions.

## Option B: install from a release archive

Download the archive matching your Linux architecture from the GitHub release:

- `wayland-wheeltani-vX.Y.Z-linux-x86_64-gnu.tar.gz`
- `wayland-wheeltani-vX.Y.Z-linux-aarch64-gnu.tar.gz`

```bash
tar -xzf wayland-wheeltani-vX.Y.Z-linux-x86_64-gnu.tar.gz
cd wayland-wheeltani-vX.Y.Z-linux-x86_64-gnu
install -Dm755 -t ~/.local/bin wayland-wheeltani wlw
install -Dm644 -t ~/.local/share/man/man1 man/*.1

# Same setup flow:
sudo ~/.local/bin/wayland-wheeltani --setup --install-udev-rule
~/.local/bin/wayland-wheeltani --install-service
```

Each archive also holds `completions/` (bash, zsh, fish), `contrib/`,
`integrations/` and `examples/`.

## Option C: distribution packages and Nix

The repository ships an Arch `PKGBUILD`, `.deb` and `.rpm` metadata
(cargo-deb, cargo-generate-rpm) and a Nix flake. They install both programs,
the man pages, the completions and a systemd user unit in
`/usr/lib/systemd/user/`. See
[`packaging/README.md`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/packaging/README.md)
for the build commands. After installing a package:

```bash
wayland-wheeltani --setup
sudo wayland-wheeltani --install-udev-rule
systemctl --user enable --now wayland-wheeltani
```

Use the packaged unit **or** `--install-service`, not both: `--install-service`
writes its own unit to `~/.config/systemd/user/`, which takes precedence.

## Build from source

### Native Linux build

```bash
sudo apt update
sudo apt install -y build-essential pkg-config
cargo build --release -p wayland-wheeltani
# Binaries: target/release/wayland-wheeltani and target/release/wlw

install -Dm755 -t ~/.local/bin target/release/wayland-wheeltani target/release/wlw
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
# x86_64:
rustup target add x86_64-unknown-linux-gnu
cargo zigbuild --release -p wayland-wheeltani --target x86_64-unknown-linux-gnu
```

## Manage the user service

```bash
wayland-wheeltani --start
wayland-wheeltani --stop
wayland-wheeltani --restart
systemctl --user reload wayland-wheeltani   # re-read the config file
journalctl --user -u wayland-wheeltani -f
```

A running daemon also re-reads the config by itself when the file is saved, and
`wlw --status`, `--pause`, `--resume`, `--toggle` and `--tune` talk to it
directly (see [Configuration](Configuration#controlling-the-running-daemon)).

### Update an existing service

Upgrading the binary alone does not update an installed unit's sandbox settings.
Neither `--start` nor `--restart` regenerates the unit. After upgrading, run
these commands **without** `sudo` to regenerate it and apply it to the running
service (`wlw` and `wayland-wheeltani` accept the same options):

```bash
wlw --install-service
wlw --restart
```

If you originally installed with a custom config path, use
`wlw --config "/absolute/path/to/config.toml" --install-service` **instead of**
the first command, preserving that original path; then run `wlw --restart`.

`--install-service` rewrites the unit, reloads systemd, and enables/starts the
service, but an already running service needs the explicit restart afterward.

### Upgrading from 1.3.3 or older

Run both steps once:

```bash
wlw --install-service && wlw --restart
sudo "$(command -v wayland-wheeltani)" --install-udev-rule
```

- The new unit adds `ExecReload` (`systemctl --user reload`) and the runtime
  directory of the control socket. With an old unit the daemon still scrolls,
  but `--status`, `--pause`, `--toggle` and `--tune` cannot reach it.
- The old udev rule also gave your user access to the keyboard interface of
  wireless receivers and gaming mice. The new rule covers the mouse interfaces
  only, and `--install-udev-rule` replaces the old file.

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
