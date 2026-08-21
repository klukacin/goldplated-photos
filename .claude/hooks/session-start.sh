#!/bin/bash
# Make a fresh container able to build and run everything in this repo.
#
# Three stacks live here and each needs something the base image lacks:
#
#   web gallery   node deps, plus ffprobe for /api/video-info
#   Rust core     nothing (cargo fetches its own) — warmed here so builds are fast
#   Tauri shell   a platform webview (WebKitGTK on Linux) and, to actually run it
#                 headless, a virtual display and a session bus
#
# Working that list out by hand cost a lot of trial and error, most of it in the
# headless run: the folder picker silently does nothing without an XDG portal,
# and clicks go nowhere without a window manager. `desktop/app/run-headless.sh`
# starts that stack correctly; this script only installs it.
set -euo pipefail

# Only the web containers are missing these. A developer's own machine has its
# own toolchain and should not have packages installed behind their back.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  echo "not a remote session — nothing to install"
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-.}"

# ---------------------------------------------------------------- apt packages

# libwebkit2gtk-4.1-dev  the webview Tauri v2 links against on Linux
# libgtk-3-dev           window/event loop under the webview
# librsvg2-dev, patchelf  bundling
# xvfb, openbox          a display, and a WM — without one, clicks never focus
# dbus-x11               a session bus for the portal to appear on
# xdg-desktop-portal(-gtk)  native file dialogs; without it they open nothing
# imagemagick, x11-utils, xdotool  drive and screenshot the app in tests
# ffmpeg                 ffprobe, which /api/video-info shells out to
APT_PACKAGES=(
  libwebkit2gtk-4.1-dev
  libgtk-3-dev
  librsvg2-dev
  patchelf
  xvfb
  openbox
  dbus-x11
  xdg-desktop-portal
  xdg-desktop-portal-gtk
  imagemagick
  x11-utils
  xdotool
  ffmpeg
)

missing=()
for pkg in "${APT_PACKAGES[@]}"; do
  dpkg-query -W -f='${Status}' "$pkg" 2>/dev/null | grep -q "install ok installed" || missing+=("$pkg")
done

if [ ${#missing[@]} -gt 0 ]; then
  echo "installing ${#missing[@]} package(s): ${missing[*]}"
  # Third-party PPAs in this image are unreachable through the proxy; the main
  # Ubuntu archive is what these come from, so a partial update is fine.
  sudo apt-get update -qq || true
  sudo DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends "${missing[@]}"
else
  echo "apt packages already present"
fi

# ------------------------------------------------------------------- node deps

# `install`, not `ci`: the container image is cached after this hook finishes, so
# reusing whatever is already in node_modules is the cheaper path on re-runs.
if [ ! -d node_modules ]; then
  echo "installing node dependencies"
  npm install --no-audit --no-fund
else
  echo "node_modules already present"
fi

# ------------------------------------------------------------------- rust deps

# Fetching without building keeps this hook short while still putting every
# crate on disk, so the first `cargo build` is compile-only.
if command -v cargo >/dev/null 2>&1; then
  echo "fetching Rust dependencies"
  (cd desktop && cargo fetch --quiet) || true
  # The shell is deliberately outside the workspace — it has its own lockfile.
  (cd desktop/app/src-tauri && cargo fetch --quiet) || true
fi

echo "environment ready"
