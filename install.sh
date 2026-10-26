#!/bin/bash
# Installs the bridge for the current user. Paths come from
# ${XDG_CONFIG_HOME:-~/.config}/wpe-bridge/config (WPE_INSTALL, WPE_PROTON),
# or from the environment when running this script.
set -euo pipefail
cd "$(dirname "$0")"

CONF=${XDG_CONFIG_HOME:-$HOME/.config}/wpe-bridge/config
[ -r "$CONF" ] && . "$CONF"
INSTALL=${WPE_INSTALL:-$HOME/.local/share/Steam/steamapps/common/wallpaper_engine}
[ -d "$INSTALL" ] || { echo "Wallpaper Engine not found at $INSTALL (set WPE_INSTALL)" >&2; exit 1; }

# rustup keeps cargo out of PATH until its env file is sourced.
command -v cargo >/dev/null || . "$HOME/.cargo/env"
make all

BIN=$HOME/.local/bin
SHARE=$HOME/.local/share
WIN=target/x86_64-pc-windows-gnu/release
mkdir -p "$BIN" "$SHARE/wpe-bridge" "$SHARE/kwin/scripts" "$SHARE/plasma/wallpapers" \
         "$HOME/.config/autostart"

# Replace the old binary by rename, so a running helper keeps its inode.
install -m 755 target/release/wpe-bridge "$BIN/wpe-bridge.new"
mv -f "$BIN/wpe-bridge.new" "$BIN/wpe-bridge"
for cmd in start stop control playback setup; do ln -sf wpe-bridge "$BIN/wpe-bridge-$cmd"; done
rm -f "$BIN/wpe-bridge-ui"   # an earlier version installed this command
install -m 644 "$WIN/wpehost.exe" "$WIN/wtsapi32.dll" "$SHARE/wpe-bridge/"

rm -rf "$SHARE/kwin/scripts/wpe-bridge" "$SHARE/plasma/wallpapers/org.local.wpebridge"
cp -r kwin/wpe-bridge "$SHARE/kwin/scripts/wpe-bridge"
cp -r plasma/org.local.wpebridge "$SHARE/plasma/wallpapers/org.local.wpebridge"

rm -f "$SHARE/applications/wpe-bridge-ui.desktop"   # an earlier version installed a menu entry
sed "s#@BINDIR@#$BIN#" share/wpe-bridge.desktop.in > "$HOME/.config/autostart/wpe-bridge.desktop"

# Puts wtsapi32.dll into the game folder, enables the KWin script, restarts plasmashell.
"$BIN/wpe-bridge-setup"
