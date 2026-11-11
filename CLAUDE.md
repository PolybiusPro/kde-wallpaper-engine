# Wallpaper Engine on Linux

Runs the real Windows Wallpaper Engine (Steam 431960) under Proton and shows it as the KDE Plasma wallpaper. README.md covers what it is and how to install it; this file is what you need to work on it.

## Layout

Everything native is Rust, in a cargo workspace. The C and bash versions are gone from the tree (git history only).

- `crates/wpehost`: Windows exe. Fake Progman/WorkerW desktop plus the "WPE-Host" window. Starts the engine, and publishes the engine's render windows in its own title.
- `crates/wtsapi32`: Windows proxy DLL that fixes the rotated-monitor size.
- `crates/wpe-bridge`: Linux multicall binary. Dispatches on argv0 (`wpe-bridge-start` etc., symlinks) or on its first argument: `start|stop|control|playback|setup|paths`. `share_dir()` is `<bindir>/../share/wpe-bridge`, found via `current_exe().canonicalize()`.
- `kwin/wpe-bridge/contents/code/main.js`: KWin script. Parks the host off-screen and publishes per-screen crop rects through plasmashell `evaluateScript`.
- `plasma/org.local.wpebridge`: the wallpaper plugin. It streams the host over PipeWire and crops it.

## Build and install

```bash
. ~/.cargo/env        # rustup lives in ~
make all              # linux binary + the two windows-gnu crates (linker: x86_64-w64-mingw32-gcc, see .cargo/config.toml)
make check            # cargo test for the workspace
make coverage         # cargo llvm-cov summary (cargo install cargo-llvm-cov; rustup component add llvm-tools-preview)
./install.sh
```

`install.sh` builds, installs under `~/.local`, then runs `wpe-bridge-setup`, which copies the DLL into the game folder, toggles the KWin script off and on, and restarts plasmashell. There is no UI test suite; verify on screen. `wpe-bridge paths` shows how Steam, Proton and the runtime were resolved.

## How the host title works

The host's window title is the protocol between `wpehost` and `main.js`: `WPE-Host[:clone]|x,y,w,h;x,y,w,h`, host-relative. A change to either side needs the other to match. `main.js` finds the host with `caption.indexOf("WPE-Host") === 0`.

- Render windows come in four classes, one per wallpaper type: `WPEDesktopDX11Window`, `WPEDesktopCEFWindow`, `WPEVideoWallpaper`, `WPEAppIntermediateWorker`. A new class has to be added to `is_render_class()` in wpehost, or that type shows the Plasma fallback.
- Engine `config.json` `wallpaperconfig.layout`: 0 per monitor, 1 fit/span (one host-sized window), 2 clone (one window, flagged `:clone`). Parse the exact quoted key `"wallpaperconfig"`; `wallpaperconfigrecent` and `wallpaperconfigscreensaver` share the prefix.

## Gotchas

- Do not `pkill -f` or `pgrep -f` a script name from Bash. The pattern matches your own shell and kills it. `wpe-bridge-stop` matches exact comm names for the same reason.
- A running KWin script is NOT reloaded by `reconfigure`. Symptom of a stale one: the plugin shows the fallback with `assigned=false`. `wpe-bridge-setup` now toggles `wpe-bridgeEnabled` off, waits 2 s, and on again. By hand: `kwriteconfig6 --file kwinrc --group Plugins --key wpe-bridgeEnabled false`, `busctl --user call org.kde.KWin /KWin org.kde.KWin reconfigure`, then true and reconfigure again.
- plasmashell's appletsrc on disk can be stale. Read the live plugin state with `qdbus6 org.kde.plasmashell /PlasmaShell org.kde.PlasmaShell.evaluateScript` and `d.currentConfigGroup = ['Wallpaper','org.local.wpebridge','General']` per desktop.
- To read the host title, load a one-line KWin script that `print`s `workspace.windowList()` captions and read `journalctl --user`. `wmctrl` cannot see the parked window. `print()` in the KWin script goes to the same journal.
- Under layout 1, `-control openWallpaper -monitor 1` is silently dropped. To test Monitor 1, edit `config.json` with the engine stopped, and check `wpe-bridge-stop` printed "stopped" first or the dying engine overwrites the edit.
- When checking layouts on screen, use wallpapers that differ from the Plasma fallback. A scene that looks like the fallback makes a failed stream look like a pass.
- `spectacle -b -f -n -o x.png` captures the desktop. KWin ScreenShot2 `CaptureWindow` of the host returns RGBA, not BGRA.
- `S:` in the wine prefix maps to the library root on the newer Proton and to `steamapps` on the old one. `start` reads the symlink to cope.
- Valve's numbered Proton builds are looked up by scanning the library `common` dirs for `Proton <version>*`. The old bash `proton_[0-9]*` glob did not expand.
- A Steam "verify files" or Wallpaper Engine update can delete the `wtsapi32.dll` in the game folder. Rerun `wpe-bridge-setup`.
- Known limitation: Wine snapshots the monitor layout when its session starts, so after rotating a screen the engine window keeps its old size until the engine restarts (`wpe-bridge-stop` then `wpe-bridge-start`). In the live session nothing can change it: a new process still sees the old layout, SetWindowPos on the render window is reverted by the engine within seconds, and closeWallpaper plus openWallpaper on one monitor recreates the window at the old size. An automatic restart was built and reverted at the user's request; the patch is not kept in the repo.
- Rotate for testing with `kscreen-doctor output.DP-2.rotation.normal|left`.
- `shortcut.rs` adds a non-Steam shortcut "Wallpaper Engine Bridge" (exe `<bindir>/wpe-bridge-start`) to every `userdata/*/config/shortcuts.vdf` (binary VDF; other entries are copied byte for byte, keys renumbered). Steam rewrites that file from memory on exit, so it writes only when no process named `steam` exists and otherwise tells the user to close Steam and rerun setup. `setup --remove` takes it out. `launchopt.rs` only strips the old `LaunchOptions` that earlier versions set on app 431960, which made Steam show it as running. How Steam is kept from listing the engine as running (all three needed, checked on screen and in `~/.local/share/Steam/logs/gameprocess_log.txt`): (1) `start.rs` sets `UMU_ID`, which makes Proton's `proton` script skip its `steam.exe` wrapper; that wrapper is what Steam logs as `AppID 431960 ... launched outside Steam`. Stripping SteamGameId, SteamAppId or STEAM_COMPAT_APP_ID does not help, and without an app id the container bus name becomes App0 so `start` and `control` break. (2) The engine exe is started directly (`wallpaper64.exe -language <steam language> -updateuicmd`, what launcher.exe passes) instead of through launcher.exe. (3) The engine and the playback helper run via `systemd-run --user`: Steam's reaper is a subreaper and keeps a shortcut "running" until every descendant exits, so anything detached from `start` keeps it alive. The playback unit is named, so a second start is refused instead of piling up helpers. Do not run umu-run against the real prefix: it remaps S: to /home and rewrites the prefix (the engine then cannot open its S: wallpaper paths).
- Keep test helpers out of the game folder when done, and restore the user's `config.json`.

## Conventions

- Commit only when asked, never push unasked. The Rust port lives on local branch `rust`.
- Match the surrounding comment density: short comments that say why.
- Not handled yet: engine focus, battery, sleep and audio settings; clone groups, a non-default clone source and flip.
- Packaging: `packaging/aur/PKGBUILD` (GPL-3.0-or-later). The maintainer line is a placeholder, and AUR `mingw-w64-rust` can lag the local rustc.
