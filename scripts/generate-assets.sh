#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage: scripts/generate-assets.sh [BIN_DIR] [OUT_DIR]

Writes the man pages and the bash/zsh/fish completions of both binaries
(wayland-wheeltani and its short alias wlw), for packaging:

  OUT_DIR/man/wayland-wheeltani.1         OUT_DIR/man/wlw.1
  OUT_DIR/completions/wayland-wheeltani.bash   (and wlw.bash)
  OUT_DIR/completions/_wayland-wheeltani       (zsh, and _wlw)
  OUT_DIR/completions/wayland-wheeltani.fish   (and wlw.fish)

BIN_DIR holds the built binaries (default: target/release, built with
`cargo build --release -p wayland-wheeltani`). OUT_DIR defaults to
target/assets. The binaries must run on this machine, so a cross-compiled
build cannot be used; the output does not depend on the architecture.
USAGE
}

case "${1:-}" in
  -h | --help)
    usage
    exit 0
    ;;
esac

root_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${1:-$root_dir/target/release}"
out_dir="${2:-$root_dir/target/assets}"

for bin in wayland-wheeltani wlw; do
  if [[ ! -x "$bin_dir/$bin" ]]; then
    printf 'error: %s not found; build it first (cargo build --release -p wayland-wheeltani)\n' \
      "$bin_dir/$bin" >&2
    exit 1
  fi
done

mkdir -p "$out_dir/man" "$out_dir/completions"
for bin in wayland-wheeltani wlw; do
  "$bin_dir/$bin" --man >"$out_dir/man/$bin.1"
  "$bin_dir/$bin" --completions bash >"$out_dir/completions/$bin.bash"
  "$bin_dir/$bin" --completions zsh >"$out_dir/completions/_$bin"
  "$bin_dir/$bin" --completions fish >"$out_dir/completions/$bin.fish"
done

printf 'Wrote man pages and completions to %s\n' "$out_dir"
