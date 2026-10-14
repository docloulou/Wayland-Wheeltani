{
  description = "Progressive middle-button autoscroll daemon for Wayland (evdev/uinput)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      workspace = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).workspace.package;
    in
    {
      packages = forAllSystems (pkgs: rec {
        wayland-wheeltani = pkgs.rustPlatform.buildRustPackage {
          pname = "wayland-wheeltani";
          inherit (workspace) version;
          src = pkgs.lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;

          # `default-members` is the portable core only.
          cargoBuildFlags = [ "-p" "wayland-wheeltani" ];
          cargoTestFlags = [ "--workspace" ];

          nativeBuildInputs = [ pkgs.installShellFiles ];

          postInstall =
            # The man pages and completions come from the binaries themselves,
            # which a cross build cannot run.
            pkgs.lib.optionalString (pkgs.stdenv.buildPlatform.canExecute pkgs.stdenv.hostPlatform) ''
              for bin in wayland-wheeltani wlw; do
                "$out/bin/$bin" --man > "$bin.1"
                installManPage "$bin.1"
                installShellCompletion --cmd "$bin" \
                  --bash <("$out/bin/$bin" --completions bash) \
                  --zsh <("$out/bin/$bin" --completions zsh) \
                  --fish <("$out/bin/$bin" --completions fish)
              done
            ''
            + ''
              install -Dm644 packaging/systemd/wayland-wheeltani.service -t "$out/lib/systemd/user"
              substituteInPlace "$out/lib/systemd/user/wayland-wheeltani.service" \
                --replace-fail /usr/bin/wayland-wheeltani "$out/bin/wayland-wheeltani"
              mkdir -p "$out/share/wayland-wheeltani"
              cp -r integrations contrib examples "$out/share/wayland-wheeltani/"
            '';

          meta = {
            inherit (workspace) description;
            homepage = "https://github.com/docloulou/Wayland-Wheeltani";
            license = pkgs.lib.licenses.bsd0;
            mainProgram = "wayland-wheeltani";
            platforms = pkgs.lib.platforms.linux;
          };
        };
        default = wayland-wheeltani;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          inputsFrom = [ self.packages.${pkgs.stdenv.hostPlatform.system}.wayland-wheeltani ];
          packages = with pkgs; [
            clippy
            rustfmt
            rust-analyzer
            shellcheck
          ];
        };
      });
    };
}
