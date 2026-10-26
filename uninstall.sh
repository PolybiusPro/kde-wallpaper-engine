#!/bin/bash
# Removes what install.sh put in place. Set each screen back to a regular
# wallpaper in Desktop settings first; the bridge leaves their old settings alone.
BIN=$HOME/.local/bin
SHARE=$HOME/.local/share

# Stops the engine, removes wtsapi32.dll from the game folder, disables the
# KWin script and restarts plasmashell.
"$BIN/wpe-bridge-setup" --remove
pkill -x wpe-bridge 2>/dev/null   # the playback helper
# wpe-bridge-ui and its menu entry come from an earlier version.
rm -f "$BIN"/wpe-bridge "$BIN"/wpe-bridge-{start,stop,ui,control,playback,setup} \
      "$HOME/.config/autostart/wpe-bridge.desktop" "$SHARE/applications/wpe-bridge-ui.desktop"
rm -rf "$SHARE/wpe-bridge" "$SHARE/kwin/scripts/wpe-bridge" "$SHARE/plasma/wallpapers/org.local.wpebridge"
echo "Logs in ~/.local/state/wpe-bridge were left."
