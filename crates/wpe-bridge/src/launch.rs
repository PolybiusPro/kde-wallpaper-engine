//! `control`: run the engine's own command line inside its running Proton
//! container, through the Steam Linux Runtime launch client.

use crate::config::{Config, APPID};
use crate::proc;
use crate::steam::Paths;
use std::os::unix::process::CommandExt;
use std::process::Command;

pub const COMMANDS: [&str; 5] = ["pause", "play", "stop", "mute", "unmute"];

fn exec_in_game(engine_args: &str) -> i32 {
    let paths = Paths::resolve(&Config::load(), false);
    let err = Command::new(paths.launch_client())
        .arg(format!("--bus-name=com.steampowered.App{APPID}"))
        .arg(format!("--directory={}", paths.install.display()))
        .args(["--", "/bin/sh", "-c"])
        .arg(format!("WINEDEBUG=-all wine wallpaper64.exe{engine_args}"))
        .exec();
    eprintln!("cannot run {}: {err}", paths.launch_client().display());
    127
}

/// Sends one command to the running engine. Does nothing when it is not
/// running; the command line would otherwise start a second engine outside the
/// bridge.
pub fn control(cmd: Option<&str>) -> i32 {
    let Some(cmd) = cmd.filter(|c| COMMANDS.contains(c)) else {
        eprintln!("usage: wpe-bridge-control pause|play|stop|mute|unmute");
        return 2;
    };
    if proc::find("wallpaper64.exe").is_none() {
        eprintln!("engine not running");
        return 1;
    }
    exec_in_game(&format!(" -control {cmd}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Only the paths that return before touching the engine: a valid command
    // would run `wine` inside the real container.
    #[test]
    fn a_missing_or_unknown_command_is_a_usage_error() {
        assert_eq!(control(None), 2);
        assert_eq!(control(Some("")), 2);
        assert_eq!(control(Some("explode")), 2);
        assert_eq!(control(Some("PAUSE")), 2);
        assert_eq!(control(Some("pause --extra")), 2);
    }
}
