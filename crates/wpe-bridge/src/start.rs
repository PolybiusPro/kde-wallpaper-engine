//! Starts Wallpaper Engine (Steam app 431960) under Proton in desktop mode.
//! wpehost.exe first builds the Progman/WorkerW windows the engine looks for, so
//! the engine attaches its per-monitor wallpapers to one "WPE-Host" window and
//! honours its own config (apply from the UI, mute, pause). The wpe-bridge KWin
//! script parks that window off-screen and the org.local.wpebridge Plasma
//! wallpaper plugin streams the right part of it onto each screen.
//!
//! Needs Steam running (the game talks to the native client).
//!
//! Steam must not list the engine as a running game, which three things
//! prevent: Proton's `steam.exe` wrapper (what Steam spots) is skipped by
//! setting UMU_ID, the engine exe is started directly instead of through
//! launcher.exe, and the engine and the playback helper run as systemd user
//! services so a Steam shortcut running us can finish once we return.

use crate::config::{self, Config, APPID};
use crate::steam::{is_exec, Paths};
use crate::{proc, stop};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
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

/// `cmd` as a transient systemd user service instead of as our child.
/// Steam's reaper keeps a shortcut "running" until every descendant of it has
/// exited, and adopts anything we detach, so only a service (a child of the user
/// manager) lets the shortcut finish when `start` does. With `unit` a second
/// run is refused while the first is up.
fn service_command(cmd: &Command, unit: Option<&str>, log_name: &str) -> Command {
    let log = format!("append:{}", config::state_dir().join(log_name).display());
    let mut run = Command::new("systemd-run");
    run.args(["--user", "--quiet", "--collect", "--no-block", "--description=Wallpaper Engine Bridge"]);
    if let Some(unit) = unit {
        run.arg(format!("--unit={unit}"));
    }
    if let Some(dir) = cmd.get_current_dir() {
        run.arg(format!("--working-directory={}", dir.display()));
    }
    run.arg(format!("--property=StandardOutput={log}")).arg(format!("--property=StandardError={log}"));
    for (key, val) in cmd.get_envs() {
        if let Some(val) = val {
            run.arg(format!("--setenv={}={}", key.to_string_lossy(), val.to_string_lossy()));
        }
    }
    run.arg("--").arg(cmd.get_program()).args(cmd.get_args());
    run
}

/// Runs `cmd` through `service_command`. Err when systemd-run cannot be run.
fn systemd_run(cmd: &Command, unit: Option<&str>, log_name: &str) -> std::io::Result<ExitStatus> {
    service_command(cmd, unit, log_name).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status()
}

/// Steam's UI language, which launcher.exe used to pass to the engine.
fn steam_language(registry: &str) -> String {
    let found = registry.lines().find_map(|line| {
        let mut quoted = line.split('"').skip(1).step_by(2);
        (quoted.next()? == "language").then(|| quoted.next().map(str::to_string)).flatten()
    });
    found.filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_lowercase())).unwrap_or_else(|| "english".into())
}

/// The engine exe as Wine sees it. Proton's S: drive is the steamapps folder in
/// older builds and its parent in newer ones.
fn engine_exe(paths: &Paths) -> String {
    let rel = paths.install.strip_prefix(&paths.library).unwrap_or(&paths.install);
    let rel = rel.to_string_lossy().replace('/', "\\");
    let drive = fs::read_link(paths.compat.join("pfx/dosdevices/s:")).ok();
    if drive.as_deref() == Some(paths.library.as_path()) {
        format!("S:\\{rel}\\wallpaper64.exe")
    } else {
        format!("S:\\steamapps\\{rel}\\wallpaper64.exe")
    }
}

/// Proton's own launch script, inside the Steam Linux Runtime, running wpehost
/// (which then starts the engine) with the environment Steam would give it,
/// minus anything that makes Steam list the engine as a running game.
fn engine_command(paths: &Paths, proton: &Path, host_exe: &Path, exe: &str, language: &str) -> Command {
    let appid = APPID.to_string();
    let proton_dir = proton.parent().unwrap_or(Path::new("/"));
    let mut cmd = Command::new(paths.slr.join("_v2-entry-point"));
    cmd.args(["--verb=waitforexitandrun", "--"])
        .arg(proton)
        .args(["waitforexitandrun"])
        .arg(host_exe)
        .arg(exe)
        .args(["-language", language, "-updateuicmd"])
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
        // Makes Proton skip its steam.exe wrapper, which Steam tracks as the game.
        .env("UMU_ID", "0")
        .env("WINEDLLOVERRIDES", "wtsapi32=n,b")
        // Keep Proton on its X11 (XWayland) driver rather than native Wayland.
        .env("PROTON_ENABLE_WAYLAND", "0")
        // Where the playback helper lists the screens a native window covers.
        .env("WPE_COVER_FILE", crate::playback::cover_file());
    cmd
}

fn start_engine(paths: &Paths, proton: &Path, host_exe: &Path, exe: &str, language: &str) -> std::io::Result<()> {
    let mut cmd = engine_command(paths, proton, host_exe, exe, language);
    if systemd_run(&cmd, None, "engine.log").is_ok_and(|s| s.success()) {
        return Ok(());
    }
    cmd.stdin(Stdio::null()).stdout(append_log("engine.log")).stderr(append_log("engine.log"));
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
    let exe = engine_exe(&paths);
    let language = steam_language(&fs::read_to_string(config::home().join(".steam/registry.vdf")).unwrap_or_default());
    let host_exe = share.join("wpehost.exe"); // fake desktop; starts the engine

    // Applies the engine's fullscreen/maximized settings; exits at once if one is
    // already running (it owns a D-Bus name), and outlives `stop`.
    if let Ok(exe) = std::env::current_exe() {
        let mut helper = Command::new(exe);
        helper.arg("playback");
        // The unit name makes a second start refuse instead of piling up helpers.
        if systemd_run(&helper, Some("wpe-bridge-playback"), "playback.log").is_err() {
            helper.stdin(Stdio::null()).stdout(append_log("playback.log")).stderr(append_log("playback.log"));
            let _ = detach(&mut helper).spawn();
        }
    }

    if proc::find("wallpaper64.exe").is_none() {
        let _ = crate::playback::write_covers(&crate::playback::cover_file(), "");
        // At login Steam starts alongside this; the game needs its client running.
        if !proc::wait_for(300, || proc::find("steamwebhelper").is_some()) {
            log("steam is not running");
            return 1;
        }
        std::thread::sleep(Duration::from_secs(10));
        for attempt in 1..=3 {
            log(&format!("starting engine (attempt {attempt})"));
            if let Err(e) = start_engine(&paths, &proton, &host_exe, &exe, &language) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;
    use std::ffi::OsStr;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    fn paths(library: &Path) -> Paths {
        Paths {
            install: library.join("common/wallpaper_engine"),
            library: library.to_path_buf(),
            steam: PathBuf::from("/steam"),
            slr: PathBuf::from("/slr"),
            compat: library.join("compatdata/431960"),
            proton: None,
            tool: None,
        }
    }

    #[test]
    fn the_engine_path_follows_which_folder_s_points_at() {
        let s = Scratch::new();
        let p = paths(&s.path().join("steamapps"));
        let dosdevices = p.compat.join("pfx/dosdevices");
        fs::create_dir_all(&dosdevices).unwrap();

        // No s: drive at all: the old layout.
        assert_eq!(engine_exe(&p), r"S:\steamapps\common\wallpaper_engine\wallpaper64.exe");
        // Older Proton: S: is steamapps itself.
        symlink(&p.library, dosdevices.join("s:")).unwrap();
        assert_eq!(engine_exe(&p), r"S:\common\wallpaper_engine\wallpaper64.exe");
        // Newer Proton: S: is the library root, one level up.
        fs::remove_file(dosdevices.join("s:")).unwrap();
        symlink(p.library.parent().unwrap(), dosdevices.join("s:")).unwrap();
        assert_eq!(engine_exe(&p), r"S:\steamapps\common\wallpaper_engine\wallpaper64.exe");
    }

    #[test]
    fn the_engine_command_goes_through_the_runtime_and_proton() {
        let p = paths(Path::new("/lib/steamapps"));
        let cmd = engine_command(&p, Path::new("/tools/p/proton"), Path::new("/share/wpehost.exe"), r"S:\x\wallpaper64.exe", "german");
        assert_eq!(cmd.get_program(), "/slr/_v2-entry-point");
        let args: Vec<&OsStr> = cmd.get_args().collect();
        assert_eq!(
            args,
            [
                "--verb=waitforexitandrun",
                "--",
                "/tools/p/proton",
                "waitforexitandrun",
                "/share/wpehost.exe",
                r"S:\x\wallpaper64.exe",
                "-language",
                "german",
                "-updateuicmd",
            ]
        );
        assert_eq!(cmd.get_current_dir(), Some(p.install.as_path()));
    }

    #[test]
    fn the_engine_command_sets_the_environment_steam_would() {
        let p = paths(Path::new("/lib/steamapps"));
        let cmd = engine_command(&p, Path::new("/tools/p/proton"), Path::new("/h.exe"), "e", "english");
        let env = |k: &str| {
            cmd.get_envs().find(|(n, _)| *n == k).and_then(|(_, v)| v).map(|v| v.to_string_lossy().into_owned())
        };
        assert_eq!(env("SteamAppId").as_deref(), Some("431960"));
        assert_eq!(env("STEAM_COMPAT_APP_ID").as_deref(), Some("431960"));
        assert_eq!(env("UMU_ID").as_deref(), Some("0"));
        assert_eq!(env("STEAM_COMPAT_DATA_PATH").as_deref(), Some("/lib/steamapps/compatdata/431960"));
        assert_eq!(env("STEAM_COMPAT_INSTALL_PATH").as_deref(), Some("/lib/steamapps/common/wallpaper_engine"));
        assert_eq!(env("STEAM_COMPAT_LIBRARY_PATHS").as_deref(), Some("/steam/steamapps:/lib/steamapps"));
        assert_eq!(env("STEAM_COMPAT_TOOL_PATHS").as_deref(), Some("/tools/p:/slr"));
        assert_eq!(env("STEAM_COMPAT_MOUNTS").as_deref(), Some("/slr"));
        assert_eq!(env("WINEDLLOVERRIDES").as_deref(), Some("wtsapi32=n,b"));
        assert_eq!(env("PROTON_ENABLE_WAYLAND").as_deref(), Some("0"));
    }

    #[test]
    fn the_language_comes_from_steams_registry() {
        let reg = "\"Registry\"\n{\n\t\"HKCU\"\n\t{\n\t\t\"Steam\"\n\t\t{\n\t\t\t\"Rate\"\t\t\"30000\"\n\t\t\t\"language\"\t\t\"schinese\"\n\t\t}\n\t}\n}\n";
        assert_eq!(steam_language(reg), "schinese");
        assert_eq!(steam_language(""), "english");
        assert_eq!(steam_language("\"language\"\t\"\"\n"), "english");
        assert_eq!(steam_language("\"language\"\t\"../x\"\n"), "english");
    }

    #[test]
    fn a_service_command_carries_the_environment_and_directory() {
        let mut cmd = Command::new("/bin/prog");
        cmd.args(["a", "b"]).current_dir("/work").env("K", "v=1").env_remove("GONE");
        let run = service_command(&cmd, Some("u"), "x.log");
        assert_eq!(run.get_program(), "systemd-run");
        let args: Vec<String> = run.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&"--unit=u".to_string()));
        assert!(args.contains(&"--working-directory=/work".to_string()));
        assert!(args.contains(&"--setenv=K=v=1".to_string()));
        assert!(!args.iter().any(|a| a.contains("GONE")));
        assert!(args.iter().any(|a| a.starts_with("--property=StandardOutput=append:") && a.ends_with("/x.log")));
        assert_eq!(&args[args.len() - 4..], ["--", "/bin/prog", "a", "b"]);
        assert!(!service_command(&cmd, None, "x.log").get_args().any(|a| a.to_string_lossy().starts_with("--unit")));
    }

    #[test]
    fn timestamps_are_clock_times() {
        let t = timestamp();
        let parts: Vec<&str> = t.split(':').collect();
        assert_eq!(parts.len(), 3, "{t}");
        assert!(parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_digit())), "{t}");
    }

    #[test]
    fn a_detached_child_runs_in_its_own_session() {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", "exit 0"]);
        let mut child = detach(&mut cmd).spawn().unwrap();
        assert!(child.wait().unwrap().success());
    }
}
