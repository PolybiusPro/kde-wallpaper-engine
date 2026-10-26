//! One binary behind every `wpe-bridge-*` command. It dispatches on the name it
//! was run as (the installed commands are symlinks to it) or on its first
//! argument: `wpe-bridge start` is `wpe-bridge-start`.

mod config;
mod launch;
mod launchopt;
mod playback;
mod proc;
mod setup;
mod start;
mod steam;
mod stop;
#[cfg(test)]
mod testutil;

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

/// The command and its first argument, from the name the binary was run as
/// (`wpe-bridge-start`) or else from its first argument (`wpe-bridge start`).
fn command(argv0: &OsString, mut rest: Vec<OsString>) -> (String, Option<String>) {
    let name = Path::new(argv0).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let cmd = match name.strip_prefix("wpe-bridge-") {
        Some(sub) => sub.to_string(),
        None if rest.is_empty() => String::new(),
        None => rest.remove(0).to_string_lossy().into_owned(),
    };
    (cmd, rest.first().map(|a| a.to_string_lossy().into_owned()))
}

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let argv0 = args.next().unwrap_or_default();
    let (cmd, arg) = command(&argv0, args.collect());
    let code = match cmd.as_str() {
        "start" => start::run(),
        "stop" => stop::run(),
        "control" => launch::control(arg.as_deref()),
        "playback" => playback::run(),
        "setup" => setup::run(arg.as_deref()),
        "paths" => steam::print_paths(),
        _ => {
            eprintln!("usage: wpe-bridge start|stop|control|playback|setup|paths");
            2
        }
    };
    ExitCode::from(code as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(argv0: &str, rest: &[&str]) -> (String, Option<String>) {
        command(&argv0.into(), rest.iter().map(OsString::from).collect())
    }

    #[test]
    fn the_name_it_was_run_as_picks_the_command() {
        assert_eq!(run("/home/u/.local/bin/wpe-bridge-control", &["mute"]), ("control".into(), Some("mute".into())));
        assert_eq!(run("wpe-bridge-start", &[]), ("start".into(), None));
    }

    #[test]
    fn the_first_argument_picks_it_otherwise() {
        assert_eq!(run("/usr/bin/wpe-bridge", &["control", "pause"]), ("control".into(), Some("pause".into())));
        assert_eq!(run("wpe-bridge", &["setup", "--remove"]), ("setup".into(), Some("--remove".into())));
        assert_eq!(run("wpe-bridge", &["paths"]), ("paths".into(), None));
    }

    #[test]
    fn no_command_gives_an_empty_one() {
        assert_eq!(run("wpe-bridge", &[]), (String::new(), None));
        assert_eq!(run("", &[]), (String::new(), None));
    }

    #[test]
    fn the_name_wins_over_the_arguments() {
        assert_eq!(run("wpe-bridge-stop", &["start"]), ("stop".into(), Some("start".into())));
    }
}
