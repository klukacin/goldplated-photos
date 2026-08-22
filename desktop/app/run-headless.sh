#!/bin/bash
# Run the desktop app with no monitor attached.
#
# Getting this right by hand took several wrong turns, so it is written down:
#
#   Xvfb            a display to draw on
#   openbox         a window manager — without one, xdotool cannot focus the
#                   window and clicks land nowhere
#   dbus-daemon     a session bus, which the portal needs to exist on
#   xdg-desktop-portal(-gtk)
#                   native file dialogs. Without it "Choose folder…" opens
#                   nothing at all and reports no error, which looks exactly
#                   like a broken button
#
# The library path argument skips the folder picker entirely, which is the
# short way past all of that when you only want to see the app.
#
# Usage:
#   ./run-headless.sh [library-path]        run it
#   ./run-headless.sh --shot out.png [lib]  run it, screenshot, stop
set -euo pipefail

DISPLAY_NUM="${GPP_DISPLAY:-:77}"
SCREEN="${GPP_SCREEN:-1400x900x24}"
BUS_PATH="${GPP_BUS:-/tmp/gpp-session-bus}"
BIN="$(cd "$(dirname "$0")" && pwd)/src-tauri/target/debug/gpp-desktop"

shot=""
if [ "${1:-}" = "--shot" ]; then
  shot="$2"
  shift 2
fi
library="${1:-}"

for tool in Xvfb openbox dbus-daemon; do
  command -v "$tool" >/dev/null || {
    echo "missing $tool — run .claude/hooks/session-start.sh" >&2
    exit 1
  }
done
[ -x "$BIN" ] || {
  echo "no build at $BIN — run: cd src-tauri && cargo build" >&2
  exit 1
}

pids=()
cleanup() {
  for pid in "${pids[@]:-}"; do kill "$pid" 2>/dev/null || true; done
}
trap cleanup EXIT

start() {
  "$@" >/dev/null 2>&1 &
  pids+=("$!")
}

# Reuse a display that is already up: repeated runs in one session are common,
# and restarting Xvfb loses nothing but costs a few seconds each time.
if ! DISPLAY="$DISPLAY_NUM" xdpyinfo >/dev/null 2>&1; then
  start Xvfb "$DISPLAY_NUM" -screen 0 "$SCREEN" -nolisten tcp
  for _ in $(seq 20); do
    DISPLAY="$DISPLAY_NUM" xdpyinfo >/dev/null 2>&1 && break
    sleep 0.25
  done
fi
export DISPLAY="$DISPLAY_NUM"

[ -S "$BUS_PATH" ] || start dbus-daemon --session --nofork --address="unix:path=$BUS_PATH"
sleep 1
export DBUS_SESSION_BUS_ADDRESS="unix:path=$BUS_PATH"
export XDG_CURRENT_DESKTOP=GNOME

pgrep -f xdg-desktop-portal-gtk >/dev/null || start /usr/libexec/xdg-desktop-portal-gtk
pgrep -f "libexec/xdg-desktop-portal$" >/dev/null || start /usr/libexec/xdg-desktop-portal
pgrep -x openbox >/dev/null || start openbox
sleep 2

# Software rendering: there is no GPU here, and without these WebKit spends its
# first seconds failing to find one.
export WEBKIT_DISABLE_COMPOSITING_MODE=1
export WEBKIT_DISABLE_DMABUF_RENDERER=1

if [ -n "$shot" ]; then
  start "$BIN" ${library:+"$library"}
  echo "waiting for the window…"
  sleep 12
  import -window root "$shot"
  echo "screenshot: $shot"
else
  echo "app on $DISPLAY_NUM — screenshot with: DISPLAY=$DISPLAY_NUM import -window root out.png"
  "$BIN" ${library:+"$library"}
fi
