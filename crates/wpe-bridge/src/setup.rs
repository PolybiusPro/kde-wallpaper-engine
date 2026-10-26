//! Per-user setup after installing the package: `wpe-bridge-setup [--remove]`.
//! Puts wtsapi32.dll into Wallpaper Engine's folder (a package cannot write into
//! a Steam library), turns on the KWin script, restarts plasmashell so it
//! picks up the wallpaper plugin, and points Steam's Play button at the bridge.
//! --remove undoes all four.

use crate::config::{self, Config};
use crate::{launchopt, stop};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::thread;
use std::time::Duration;

/// Writes the plugin's enabled flag and tells KWin to re-read its config.
fn set_script(conn: &Option<zbus::blocking::Connection>, on: bool) -> bool {
    let wrote = Command::new("kwriteconfig6")
        .args(["--file", "kwinrc", "--group", "Plugins", "--key", "wpe-bridgeEnabled"])
        .arg(on.to_string())
        .status();
    if !wrote.is_ok_and(|s| s.success()) {
        eprintln!("kwriteconfig6 failed");
        return false;
    }
    if let Some(conn) = conn {
        let _ = conn.call_method(Some("org.kde.KWin"), "/KWin", Some("org.kde.KWin"), "reconfigure", &());
    }
    true
}

pub fn run(arg: Option<&str>) -> i32 {
    let enable = match arg {
        None => true,
        Some("--remove") => false,
        Some(_) => {
            eprintln!("usage: wpe-bridge-setup [--remove]");
            return 2;
        }
    };
    let install = Config::load().install;
    if !install.is_dir() {
        eprintln!(
            "Wallpaper Engine not found at {} (set WPE_INSTALL in {})",
            install.display(),
            config::config_file().display()
        );
        return 1;
    }
    let dll = install.join("wtsapi32.dll");
    if enable {
        // Loaded by the engine in place of the real wtsapi32.dll (see `start`).
        let copied = fs::copy(config::share_dir().join("wtsapi32.dll"), &dll)
            .and_then(|_| fs::set_permissions(&dll, fs::Permissions::from_mode(0o644)));
        if let Err(e) = copied {
            eprintln!("cannot install {}: {e}", dll.display());
            return 1;
        }
    } else {
        stop::stop();
        let _ = fs::remove_file(&dll);
    }

    let conn = zbus::blocking::Connection::session().ok();
    if enable {
        // reconfigure does not reload a script that is already enabled, so a
        // reinstall would keep running the old code. Switch it off and on.
        if !set_script(&conn, false) {
            return 1;
        }
        thread::sleep(Duration::from_secs(2));
    }
    if !set_script(&conn, enable) {
        return 1;
    }
    let _ = Command::new("systemctl").args(["--user", "restart", "plasma-plasmashell.service"]).status();
    launchopt::apply(enable);

    if enable {
        println!(
            "Set up. Next:\n  1. Desktop settings -> Wallpaper: pick \"Wallpaper Engine Bridge\" on each screen\n  2. wpe-bridge-start   (also runs at login)"
        );
    } else {
        println!("Removed. Set each screen back to a regular wallpaper in Desktop settings.");
    }
    0
}
