//! Starts Wallpaper Engine (Steam app 431960) under Proton in desktop mode.
//! wpehost.exe first builds the Progman/WorkerW windows the engine looks for, so
//! the engine attaches its per-monitor wallpapers to one "WPE-Host" window and
//! honours its own config (apply from the UI, mute, pause). The wpe-bridge KWin
//! script parks that window off-screen and the org.local.wpebridge Plasma
//! wallpaper plugin streams the right part of it onto each screen.
//!
//! Needs Steam running (the game talks to the native client).

use crate::config::{self, Config, APPID};
use crate::steam::{is_exec, Paths};
use crate::{proc, stop};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

fn timestamp() -> String {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
    }
}

fn log(msg: &str) {
    let path = config::state_dir().join("start.log");
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {msg}", timestamp());
    }
}

fn append_log(name: &str) -> Stdio {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(config::state_dir().join(name))
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null())
}

/// Detaches the child from this process's session, so it outlives us.
fn detach(cmd: &mut Command) -> &mut Command {
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        })
    }
}

/// Proton's S: drive is the steamapps folder in older builds and its parent in newer ones.
fn launcher(paths: &Paths) -> String {
    let rel = paths.install.strip_prefix(&paths.library).unwrap_or(&paths.install);
    let rel = rel.to_string_lossy().replace('/', "\\");
    let drive = fs::read_link(paths.compat.join("pfx/dosdevices/s:")).ok();
    if drive.as_deref() == Some(paths.library.as_path()) {
        format!("S:\\{rel}\\launcher.exe")
    } else {
        format!("S:\\steamapps\\{rel}\\launcher.exe")
    }
}

fn start_engine(paths: &Paths, proton: &Path, host_exe: &Path, launcher: &str) -> std::io::Result<()> {
    let appid = APPID.to_string();
    let proton_dir = proton.parent().unwrap_or(Path::new("/"));
    let mut cmd = Command::new(paths.slr.join("_v2-entry-point"));
    cmd.args(["--verb=waitforexitandrun", "--"])
        .arg(proton)
        .args(["waitforexitandrun"])
        .arg(host_exe)
        .arg(launcher)
        .args(["-run", "wallpaper64.exe", "-nobrowse"])
        .current_dir(&paths.install)
        .env("SteamAppId", &appid)
        .env("SteamGameId", &appid)
        .env("STEAM_COMPAT_APP_ID", &appid)
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", &paths.steam)
        .env("STEAM_COMPAT_DATA_PATH", &paths.compat)
        .env("STEAM_COMPAT_INSTALL_PATH", &paths.install)
        .env("STEAM_COMPAT_FLAGS", "search-cwd")
        .env(
            "STEAM_COMPAT_LIBRARY_PATHS",
            format!("{}:{}", paths.steam.join("steamapps").display(), paths.library.display()),
        )
        .env("STEAM_COMPAT_MOUNTS", &paths.slr)
        .env("STEAM_COMPAT_TOOL_PATHS", format!("{}:{}", proton_dir.display(), paths.slr.display()))
        .env("STEAM_COMPAT_LAUNCHER_SERVICE", "proton")
        .env("WINEDLLOVERRIDES", "wtsapi32=n,b")
        .stdin(Stdio::null())
        .stdout(append_log("engine.log"))
        .stderr(append_log("engine.log"));
    detach(&mut cmd).spawn().map(|_| ())
}

fn in_game_works(paths: &Paths) -> bool {
    Command::new(paths.launch_client())
        .arg(format!("--bus-name=com.steampowered.App{APPID}"))
        .arg(format!("--directory={}", paths.install.display()))
        .args(["--", "/bin/true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn run() -> i32 {
    let cfg = Config::load();
    let paths = Paths::resolve(&cfg, true);
    let _ = fs::create_dir_all(config::state_dir());
    let share = config::share_dir();

    let proton = paths.proton.clone().filter(|p| is_exec(p));
    let Some(proton) = proton else {
        log(&format!(
            "no usable proton (steam's tool: '{}'; set WPE_PROTON)",
            paths.tool.as_deref().unwrap_or("unknown")
        ));
        return 1;
    };
    let launcher = launcher(&paths);
    let host_exe = share.join("wpehost.exe"); // fake desktop; starts the engine

    // Applies the engine's fullscreen/maximized settings; exits at once if one is
    // already running (it owns a D-Bus name), and outlives `stop`.
    if let Ok(exe) = std::env::current_exe() {
        let mut helper = Command::new(exe);
        helper.arg("playback").stdin(Stdio::null()).stdout(append_log("playback.log")).stderr(append_log("playback.log"));
        let _ = detach(&mut helper).spawn();
    }

    if proc::find("wallpaper64.exe").is_none() {
        // At login Steam starts alongside this; the game needs its client running.
        if !proc::wait_for(300, || proc::find("steamwebhelper").is_some()) {
            log("steam is not running");
            return 1;
        }
        std::thread::sleep(Duration::from_secs(10));
        for attempt in 1..=3 {
            log(&format!("starting engine (attempt {attempt})"));
            if let Err(e) = start_engine(&paths, &proton, &host_exe, &launcher) {
                log(&format!("cannot start engine: {e}"));
            }
            if proc::wait_for(90, || proc::find("wallpaper64.exe").is_some()) {
                break;
            }
            log("engine did not start");
            stop::stop();
            if attempt == 3 {
                return 1;
            }
        }
    }
    if !proc::wait_for(60, || in_game_works(&paths)) {
        log("launcher service not reachable");
        return 1;
    }
    log("done");
    0
}
