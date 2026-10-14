# Development

## Workspace layout

```text
crates/
├── middle-scroll-core/      # platform-independent state machine + tests
└── middle-scroll-linux/     # package wayland-wheeltani
    ├── src/lib.rs           # library wayland_wheeltani: config, evdev/uinput, daemon
    ├── src/main.rs          # binary wayland-wheeltani
    ├── src/bin/wlw.rs       # binary wlw (same program, shorter name)
    └── tests/uinput.rs      # end-to-end tests against the kernel (ignored by default)
contrib/
├── 60-wayland-wheeltani.rules
├── wayland-wheeltani.service
└── wayland-wheeltani-root.service
examples/
└── config.toml              # every option with its default
integrations/
├── gnome/                   # bundled GNOME Shell extension for the gnome provider
└── kde/                     # KWin script for the command provider
packaging/                   # Arch PKGBUILD, packaged systemd unit (deb/rpm metadata is in Cargo.toml)
scripts/
├── bump-version.sh          # version bump across the workspace and the PKGBUILD
├── generate-assets.sh       # man pages and completions from the built binaries
└── publish-wiki.sh          # mirror wiki/ to the GitHub wiki
flake.nix                    # Nix package and dev shell
```

The core (`middle-scroll-core`) is a pure, OS-independent state machine with no
reference to evdev/uinput, so it can be unit-tested on any host (including
macOS). The Linux backend (`middle-scroll-linux`) is a library that wires evdev
input and uinput output around that core, plus the two thin binaries
`wayland-wheeltani` and `wlw`.

## How it works

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

The virtual device declares every button and relative axis of the physical
mice it serves, on top of the standard mouse buttons, pointer motion and the
legacy and hi-res wheels. Events the engine does not handle (extra buttons,
the real wheel, tilt) are forwarded unchanged, one frame per physical frame.
Legacy and hi-res wheel events are batched together for app compatibility.

A gate sits between event routing and the core. At each trigger press it asks
the foreground provider which application has focus, then decides whether the
engine handles the gesture (autoscroll) or passes it straight through
(**[Foreground filter](Foreground-Filter)**), and which `[[profiles]]` entry
supplies the settings.

The daemon loop polls the mice, the control socket
(`$XDG_RUNTIME_DIR/wayland-wheeltani/control.sock`), the config file watcher
(inotify) and the signals. It only wakes on a timer while a scroll or an
inertia coast is running.

## Verification

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build --release -p wayland-wheeltani
shellcheck scripts/*.sh integrations/*/*.sh packaging/arch/PKGBUILD
```

`default-members` lists only the portable core, so plain `cargo test` and
`cargo clippy` skip the Linux daemon; pass `--workspace` (on Linux) or
`-p wayland-wheeltani`.

The end-to-end tests in `tests/uinput.rs` create real virtual mice and read
them back through evdev. They need `/dev/uinput` and access to
`/dev/input`, so they are ignored by default:

```bash
sudo -E cargo test -p wayland-wheeltani --test uinput -- --ignored
```

Each test grabs the virtual mouse it creates, so nothing reaches the desktop.

### Continuous integration

`.github/workflows/ci.yml` runs on every push to `main` and every pull request:

- `rustfmt`;
- `clippy` with `-D warnings` on a pinned toolchain (`CLIPPY_TOOLCHAIN`; the
  pedantic and nursery groups gain lints with each Rust release, so bump it on
  purpose);
- tests on stable and on the MSRV (1.85), plus the core alone on its own MSRV
  (1.74);
- man pages and completions generation;
- `shellcheck` on the shell scripts and the PKGBUILD.

## Releasing

Bump the workspace version with the helper script (it updates `Cargo.toml`, the
inter-crate dependency, `Cargo.lock`, and `pkgver` in the Arch PKGBUILD for
stable versions):

```bash
scripts/bump-version.sh beta              # 1.3.0-beta.1 -> 1.3.0-beta.2
scripts/bump-version.sh release           # 1.3.0-beta.N -> 1.3.0
scripts/bump-version.sh --version X.Y.Z   # explicit
```

Then trigger the "Release Linux binaries" workflow (`workflow_dispatch`) with
the matching `vX.Y.Z` input. It:

1. refuses a tag that does not match the version in `Cargo.toml`;
2. runs the test suite and generates the man pages and completions;
3. builds the x86_64 and aarch64 binaries with a pinned Zig;
4. creates the GitHub release (a `-beta.N` version is marked as a prerelease)
   with archives holding both binaries, the man pages and the completions;
5. publishes the workspace crates to crates.io.

After a stable release, run `updpkgsums` in `packaging/arch/`. The Nix flake
has no `flake.lock` yet: run `nix flake lock` once and commit the file so
builds pin their nixpkgs.

Release notes live in
[`release_notes/`](https://github.com/docloulou/Wayland-Wheeltani/tree/main/release_notes)
and the user-facing summary is in
[`CHANGELOG.md`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/CHANGELOG.md).
