#!/usr/bin/env bash
# Installs the Wayland-Wheeltani Foreground GNOME Shell extension into the
# current user's extensions directory and enables it.
set -euo pipefail

UUID="wheeltani-foreground@docloulou.github.io"
SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/${UUID}"
DEST_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/gnome-shell/extensions/${UUID}"

if [ ! -f "${SRC_DIR}/extension.js" ]; then
    echo "error: ${SRC_DIR}/extension.js not found" >&2
    exit 1
fi

echo "Installing ${UUID}"
echo "  from: ${SRC_DIR}"
echo "  to:   ${DEST_DIR}"
mkdir -p "${DEST_DIR}"
cp -f "${SRC_DIR}/metadata.json" "${SRC_DIR}/extension.js" "${DEST_DIR}/"

if command -v gnome-extensions >/dev/null 2>&1; then
    gnome-extensions enable "${UUID}" 2>/dev/null \
        || echo "note: enable later with 'gnome-extensions enable ${UUID}'"
fi

cat <<EOF

Installed.

On Wayland, GNOME Shell only loads newly installed extensions after you log out
and back in. After that, ensure it is enabled:

    gnome-extensions enable ${UUID}

Verify the D-Bus service is up:

    gdbus call --session --dest org.docloulou.WheeltaniForeground \\
      --object-path /org/docloulou/WheeltaniForeground \\
      --method org.docloulou.WheeltaniForeground.GetFocused

Then set 'provider = "gnome"' (or "auto") in your Wayland-Wheeltani config.
EOF
