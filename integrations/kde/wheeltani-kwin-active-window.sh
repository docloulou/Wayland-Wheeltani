#!/usr/bin/env bash
#
# wheeltani-kwin-active-window.sh
# ===============================
#
# Example helper for Wayland-Wheeltani's `command` foreground provider on
# KDE Plasma 6 (KWin / Wayland). It prints the currently focused window as one
# line of JSON on stdout, the format the `command` provider reads, e.g.:
#
#   {"app_id":"org.kde.dolphin","class":"dolphin","resource_class":"dolphin","title":"Home — Dolphin","pid":4242}
#
# `{}` means no window has focus (the daemon then applies unknown_policy).
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
# window into the journal), then scrape that line back out.
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

# 2) Write a one-shot KWin script. The marker is unique to this invocation, so
#    a line printed by an earlier or overlapping run (the daemon polls this
#    script, and the journal is shared) can never be taken for this one.
marker="WHEELTANI_FG_$$_$(date +%s%N)"
script_file="$(mktemp --suffix=.js)"
trap 'rm -f "$script_file"' EXIT
cat >"$script_file" <<JS
var w = workspace.activeWindow;
var info = {};
if (w) {
  if (w.desktopFileName) info.app_id = w.desktopFileName;
  if (w.resourceClass) info.class = w.resourceClass;
  if (w.resourceName) info.resource_class = w.resourceName;
  if (w.caption) info.title = w.caption;
  if (w.pid > 0) info.pid = w.pid;
}
print("${marker}:" + JSON.stringify(info));
JS

# 3) Load + run + unload the script through KWin's Scripting interface. A unique
#    plugin name per invocation avoids clashing with a previous run.
plugin="wheeltani_active_$$"
since="$(date '+%Y-%m-%d %H:%M:%S')"

script_id="$("$qdbus_bin" org.kde.KWin /Scripting org.kde.kwin.Scripting.loadScript \
  "$script_file" "$plugin" 2>/dev/null || true)"
case "$script_id" in
  '' | -*)
    echo "wheeltani-kwin: KWin refused to load the script" >&2
    exit 1
    ;;
esac
"$qdbus_bin" org.kde.KWin "/Scripting/Script${script_id}" org.kde.kwin.Script.run \
  >/dev/null 2>&1 || true
"$qdbus_bin" org.kde.KWin /Scripting org.kde.kwin.Scripting.unloadScript "$plugin" \
  >/dev/null 2>&1 || true

# 4) Wait for KWin to flush the print, up to about one second, then take the
#    line carrying this run's marker. KWin logs to the systemd *user* journal
#    on a normal Plasma session; the system journal is tried as a fallback.
scrape() {
  journalctl "$@" --since "$since" -o cat 2>/dev/null |
    grep -o "${marker}:.*" | tail -n 1 || true
}
line=""
for _ in $(seq 20); do
  line="$(scrape --user)"
  [ -n "$line" ] || line="$(scrape)"
  [ -n "$line" ] && break
  sleep 0.05
done
if [ -z "$line" ]; then
  echo "wheeltani-kwin: no reply from the KWin script within 1s" >&2
  exit 1
fi

# 5) Emit the JSON on stdout.
printf '%s\n' "${line#"${marker}":}"
