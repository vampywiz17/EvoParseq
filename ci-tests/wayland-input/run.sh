#!/usr/bin/env bash
set -euo pipefail
# A fresh compositor, session bus, settings and runtime directory; never attach
# this fixture to a developer's desktop or reuse their GNOME profile.
unset DISPLAY
export XDG_RUNTIME_DIR
XDG_RUNTIME_DIR=$(mktemp -d)
chmod 700 "$XDG_RUNTIME_DIR"
export XDG_CONFIG_HOME="$XDG_RUNTIME_DIR/config"
export XDG_CACHE_HOME="$XDG_RUNTIME_DIR/cache"
export XDG_DATA_HOME="$XDG_RUNTIME_DIR/data"
export GSETTINGS_BACKEND=memory
export LIBGL_ALWAYS_SOFTWARE=true
export WAYLAND_DISPLAY=wayland-0
log="$XDG_RUNTIME_DIR/mutter.log"
mutter --version
mutter --headless --wayland --no-x11 --virtual-monitor=800x600 --virtual-monitor=800x600 >"$log" 2>&1 &
compositor=$!
trap 'kill "$compositor" 2>/dev/null || true; cat "$log"; rm -rf "$XDG_RUNTIME_DIR"' EXIT
for attempt in {1..80}; do
    if test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"; then break; fi
    kill -0 "$compositor"
    sleep 0.25
done
test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
"$1"
