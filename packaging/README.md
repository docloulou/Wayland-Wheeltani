# Packaging

Everything a distribution package needs, in one place. Every package installs
the same files:

| File | Installed to |
| --- | --- |
| `wayland-wheeltani`, `wlw` | `/usr/bin/` |
| man pages (`--man`) | `/usr/share/man/man1/` |
| bash / zsh / fish completions (`--completions`) | the shell's vendor directories |
| [`systemd/wayland-wheeltani.service`](systemd/wayland-wheeltani.service) | `/usr/lib/systemd/user/` |
| `integrations/` (GNOME extension, KWin script), `contrib/` | `/usr/share/wayland-wheeltani/` |

The man pages and completions come from the binaries themselves;
[`scripts/generate-assets.sh`](../scripts/generate-assets.sh) writes them to
`target/assets/`.

The packaged user unit runs `/usr/bin/wayland-wheeltani` with the default
config. After installing a package, each user sets up their mouse once:

```bash
wayland-wheeltani --setup                     # pick the mouse
sudo wayland-wheeltani --install-udev-rule    # let your user open it
systemctl --user enable --now wayland-wheeltani
```

The udev rule is not shipped: it names the user's own mice by USB id.

## Arch Linux

[`arch/PKGBUILD`](arch/PKGBUILD) builds the tagged release from source:

```bash
cd packaging/arch
updpkgsums   # after a version bump
makepkg -si
```

`scripts/bump-version.sh` keeps `pkgver` in sync for stable versions (Arch
versions cannot hold a `-beta.N` suffix).

## Debian / Ubuntu and Fedora / openSUSE

The Linux crate carries [cargo-deb](https://github.com/kornelski/cargo-deb)
and [cargo-generate-rpm](https://github.com/cat-in-136/cargo-generate-rpm)
metadata:

```bash
cargo install cargo-deb cargo-generate-rpm
cargo build --release --locked -p wayland-wheeltani
scripts/generate-assets.sh
cargo deb -p wayland-wheeltani --no-build           # target/debian/*.deb
cargo generate-rpm -p crates/middle-scroll-linux    # target/generate-rpm/*.rpm
```

## Nix

The repository is a flake:

```bash
nix run github:docloulou/Wayland-Wheeltani -- --list-devices
nix profile install github:docloulou/Wayland-Wheeltani
nix develop        # shell with cargo, clippy, rustfmt, rust-analyzer
```

On NixOS, add the package and enable the user unit it ships:

```nix
environment.systemPackages = [ wayland-wheeltani.packages.${pkgs.system}.default ];
systemd.packages = [ wayland-wheeltani.packages.${pkgs.system}.default ];
```

NixOS generates `/etc/udev/rules.d` from the system configuration, so
`--install-udev-rule` cannot write there: put the rule (see
[`contrib/60-wayland-wheeltani.rules`](../contrib/60-wayland-wheeltani.rules))
in `services.udev.extraRules` instead.
