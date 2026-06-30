# Development

## Workspace layout

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
integrations/
└── gnome/                   # bundled GNOME Shell extension for the gnome provider
```

The core (`middle-scroll-core`) is a pure, OS-independent state machine with no
reference to evdev/uinput, so it can be unit-tested on any host (including
macOS). The Linux backend (`middle-scroll-linux`, binary `wayland-wheeltani`)
wires evdev input and uinput output around that core.

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

The virtual device emits standard mouse buttons, relative pointer motion,
vertical and horizontal legacy wheel detents, and vertical and horizontal hi-res
wheel units. Legacy and hi-res wheel events are batched together for app
compatibility.

When the optional **[Foreground filter](Foreground-Filter)** is enabled, a gate
sits between event routing and the core: it decides per middle-button gesture
whether the engine handles the events (autoscroll) or whether they are passed
straight through to the virtual device (native behaviour).

## Verification

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

## Releasing

Bump the workspace version with the helper script (it updates `Cargo.toml`, the
inter-crate dependency, and `Cargo.lock`):

```bash
scripts/bump-version.sh beta              # 1.3.0-beta.1 -> 1.3.0-beta.2
scripts/bump-version.sh release           # 1.3.0-beta.N -> 1.3.0
scripts/bump-version.sh --version X.Y.Z   # explicit
```

Then trigger the "Release Linux binaries" workflow (`workflow_dispatch`) with
the matching `vX.Y.Z` input. It builds the Linux binaries, creates the GitHub
release, and publishes the workspace crates to crates.io.

Release notes live in
[`release_notes/`](https://github.com/docloulou/Wayland-Wheeltani/tree/main/release_notes)
and the user-facing summary is in
[`CHANGELOG.md`](https://github.com/docloulou/Wayland-Wheeltani/blob/main/CHANGELOG.md).
