# KDE Wallpaper Engine

Runs the real Windows Wallpaper Engine (Steam app 431960) under Proton and shows it as the KDE Plasma wallpaper, below your windows and icons, with a separate wallpaper per monitor. Because it is the real engine, scene wallpapers that the open-source reimplementations get wrong (for example the lofi cafe scene) render properly.

Developed and tested on CachyOS, Plasma 6.8 (Wayland), native Steam, Wallpaper Engine 2.8.42, a Proton build with the Steam Linux Runtime container, and two monitors (one rotated). Treat anything else as untested.

## Requirements

- Wallpaper Engine installed through Steam, with a Proton version set for it
- Steam Linux Runtime (the Proton launch goes through its container service)
- KDE Plasma 6 on Wayland, with `kwriteconfig6`, `busctl`, `systemctl --user`
- To build: Rust (rustup, with the `x86_64-pc-windows-gnu` target) and mingw-w64 as the linker

## Install

```bash
rustup target add x86_64-pc-windows-gnu
./install.sh               # builds with `make all`, installs under ~/.local, runs wpe-bridge-setup
```

Then pick "Wallpaper Engine Bridge" as the wallpaper type on each screen (Desktop settings). Start the bridge with Play in Steam or `wpe-bridge-start`; it also runs at login. `./uninstall.sh` reverses it.

Setup sets Wallpaper Engine's launch option in Steam to `wpe-bridge-start %command%`, so Play starts the bridge instead of the stock launcher. Steam rewrites its config when it exits, so the option is only written while Steam is closed: if Steam is running, setup says so, and you close Steam and run `wpe-bridge-setup` again. Launch options you already have are left alone, and a backup of the changed file is kept next to it as `localconfig.vdf.wpe-bridge.bak`.

### Configuration

Paths default to the standard Steam library. For anything else, create `~/.config/wpe-bridge/config`:

```bash
WPE_INSTALL=/path/to/steamapps/common/wallpaper_engine
WPE_PROTON=/path/to/compatibilitytools.d/<your-proton>/proton   # optional
```

Proton follows Steam: `wpe-bridge-start` reads the compatibility tool Steam has for Wallpaper Engine (its per-game choice, else the default tool) from `~/.local/share/Steam/config/config.vdf` and finds that tool's `proton` script, so changing it in Steam's settings is enough. Set `WPE_PROTON` only to force a different one.

## Commands

`wpe-bridge` is one binary, also installed under five names:

- `wpe-bridge-start` starts everything.
- `wpe-bridge-stop` stops it.
- `wpe-bridge-control` sends `pause`, `play`, `stop`, `mute` or `unmute` to the running engine.
- `wpe-bridge-playback` applies the fullscreen and maximized settings (see below). Start runs it.
- `wpe-bridge-setup` installs the DLL, enables the KWin script and sets the Steam launch option.

`wpe-bridge paths` prints what it resolved: the Steam library, Proton tool and Linux runtime.

## How it works

Wine has no Progman/WorkerW desktop layer, so the engine never finds a window to attach to and ignores apply, mute and pause. Four small pieces fix that, all written in Rust except the two Plasma ones:

- `crates/wpehost` builds a fake Progman + WorkerW pair and one "WPE-Host" window covering all monitors, then starts the engine inside it. It also lists the engine's render windows in the host's title (`WPE-Host|x,y,w,h;...`, or `WPE-Host:clone|...` in the engine's clone layout) so the Plasma side knows which screens have a wallpaper.
- `crates/wtsapi32` is a proxy DLL. The engine swaps width and height for a rotated monitor, but Wine already reports the rotated size, so a portrait screen came out landscape. The proxy patches the engine's `QueryDisplayConfig` import to report no rotation.
- `kwin/wpe-bridge` is a KWin script that parks the host window off-screen, and tells each screen which window to stream, which part of it to crop, and whether the engine has a wallpaper there.
- `plasma/org.local.wpebridge` is a Plasma wallpaper plugin that streams that window over PipeWire and crops it per screen. On a screen where the engine has no wallpaper (removed, or engine not running) it shows that screen's own regular Plasma wallpaper, copied from its `org.kde.image` settings.

`crates/wpe-bridge` is the Linux side that starts and controls them.

## Wallpaper types and layouts

Scene, video, web and application wallpapers all work, including a different type on each monitor. The engine makes one window class per type and `wpehost` lists all four in the host title.

The engine's display layout, set in its UI, decides how those windows are placed:

- **Independent** (`layout` 0 in `config.json`): each monitor gets its own window and its own wallpaper.
- **Spanning** (`layout` 1): one window across all monitors, so one wallpaper is stretched over every screen and the other monitors' picks are ignored.
- **Clone** (`layout` 2): the engine makes one window for its source monitor and normally mirrors it on the others itself, which does not happen under Wine. `wpehost` reads the layout from `config.json` and flags it in the host title, and the KWin script shows the same window on every screen, cropped to that screen's aspect ratio (centred, like Plasma's Fill). Only the default clone (everything mirrors the first monitor) is handled: clone groups, a different clone source and flipping are not.

## Fullscreen and maximized windows

The engine's "other application fullscreen / maximized" settings (run, pause or stop) cannot see native Linux windows, so the bridge applies them itself. The KWin script reports which screens showing the engine are covered by a fullscreen or maximized window. `wpe-bridge-playback` reads the settings from the engine's `config.json`, so changes in the engine's UI apply at once, and runs `wpe-bridge-control`.

Pause and stop are global to the engine, so they are sent only when every screen showing the engine is covered. Maximized means the window fills the screen's work area. The focus, battery, sleep and audio settings are not handled.

## Known limits

- Rotating a monitor while the engine runs leaves its window at the old size, because Wine reads the monitor layout once when its session starts. Restart the engine (`wpe-bridge-stop`, then `wpe-bridge-start`) after rotating.
- The fallback wallpaper is copied when the KWin script republishes (engine or screen changes, script reload), not live when you change the Plasma wallpaper. Slideshows are not followed, and the blur behind "Fit" is not reproduced.
- The engine's UI process is not hooked, so it may draw a rotated monitor as landscape. Cosmetic only.
- A Steam "verify files" or Wallpaper Engine update can delete the `wtsapi32.dll` in its folder; rerun `wpe-bridge-setup` (or `./install.sh`).
- This depends on engine behaviour that is not documented and may change in an update.

## Not affiliated

Not affiliated with Wallpaper Engine or Valve. You need your own copy of Wallpaper Engine; nothing from it is included here.
