#!/usr/bin/env bash
#
# wheeltani-kwin-active-window.sh
# ===============================
#
# Example helper for Wayland-Wheeltani's `command` foreground provider on
# KDE Plasma 6 (KWin / Wayland). It prints the *resource class* (WM class) of the
# currently focused window on stdout, one line, e.g.:
#
#   org.kde.dolphin      # native KDE app  (desktop id)
#   firefox              # X / XWayland app (WM class)
#
# Wire it into ~/.config/wayland-wheeltani/config.toml:
#
#   [foreground]
#   enabled = true
#   provider = "command"
#   mode = "denylist"
#   deny_apps = ["org.kde.dolphin", "firefox"]
#   command = ["/full/path/to/wheeltani-kwin-active-window.sh"]
#   command_refresh_ms = 500
#
# Find the exact string to put in deny_apps/allow_apps with:
#
#   wayland-wheeltani --detect-foreground
#
# ---------------------------------------------------------------------------
# HOW IT WORKS
# ---------------------------------------------------------------------------
# KWin (Wayland) exposes no readable "active window" D-Bus API. The only reliable
# route is its *scripting* API: we load a tiny KWin script through the
# `org.kde.kwin.Scripting` D-Bus interface, run it (the script `print()`s the
# class into the journal), then scrape that line back out. This is exactly the
# technique `kdotool` uses internally.
#
# PREFER `provider = "kde"` + kdotool if you can install it (cleaner and the
# tested path). This script is the "no extra binary" fallback and is provided as
# an example — it is NOT tested by the project author. Adjust to taste.
#
# Requirements: a qdbus binary (qdbus6 / qdbus-qt6 / qdbus) and journalctl
# (systemd). KDE Plasma 6.
# ---------------------------------------------------------------------------
set -euo pipefail

# 1) Locate a qdbus binary. Plasma 6 ships it as qdbus6 or qdbus-qt6; some
#    distributions still provide a plain `qdbus`.
qdbus_bin=""
for candidate in qdbus6 qdbus-qt6 qdbus; do
  if command -v "$candidate" >/dev/null 2>&1; then
    qdbus_bin="$candidate"
    break
  fi
done
if [ -z "$qdbus_bin" ]; then
  echo "wheeltani-kwin: no qdbus binary found (install qdbus6 / qdbus-qt6)" >&2
  exit 1
fi

# 2) Write a one-shot KWin script. A unique marker makes the journal line easy to
#    find regardless of KWin's own `js:` log prefix. An empty active window (e.g.
#    focus on the desktop) prints an empty value -> the daemon treats it as
#    "unknown" and applies unknown_policy.
marker="WHEELTANI_FG"
script_file="$(mktemp --suffix=.js)"
trap 'rm -f "$script_file"' EXIT
printf 'var w = workspace.activeWindow; print("%s:" + (w ? w.resourceClass : ""));\n' \
  "$marker" > "$script_file"

# 3) Load + run + unload the script through KWin's Scripting interface. A unique
#    plugin name per invocation avoids clashing with a previous run.
plugin="wheeltani_active_$$"
since="$(date '+%Y-%m-%d %H:%M:%S')"

script_id="$("$qdbus_bin" org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript \
  "$script_file" "$plugin" 2>/dev/null || true)"
if [ -n "$script_id" ]; then
  "$qdbus_bin" org.kde.KWin "/Scripting/Script${script_id}" org.kde.kwin.Script.run \
    >/dev/null 2>&1 || true
fi
"$qdbus_bin" org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript "$plugin" \
  >/dev/null 2>&1 || true

# 4) Give KWin a moment to flush the print, then scrape the most recent marker.
#    KWin logs to the systemd *user* journal on a normal Plasma session; if your
#    setup logs to the system journal instead, drop `--user` below.
sleep 0.05
line="$(journalctl --user --since "$since" -o cat 2>/dev/null \
  | grep -o "${marker}:.*" | tail -n 1 || true)"
if [ -z "$line" ]; then
  line="$(journalctl --since "$since" -o cat 2>/dev/null \
    | grep -o "${marker}:.*" | tail -n 1 || true)"
fi

# 5) Emit the class on stdout (empty line if none -> unknown_policy applies).
printf '%s\n' "${line#"${marker}":}"
