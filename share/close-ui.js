// One-shot KWin script: closes the Wallpaper Engine control window that the
// engine opens at startup (titled "Wallpaper UI" while loading, then
// "Wallpaper Engine ..."). Closing it leaves the engine and its WPE-Host
// desktop window running.
workspace.windowList().forEach(function (w) {
    if (w.resourceClass === "steam_app_431960" && w.caption.indexOf("Wallpaper") === 0) {
        w.closeWindow();
    }
});
