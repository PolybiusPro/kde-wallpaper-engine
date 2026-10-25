//! One binary behind every `wpe-bridge-*` command. It dispatches on the name it
//! was run as (the installed commands are symlinks to it) or on its first
//! argument: `wpe-bridge start` is `wpe-bridge-start`.

mod config;
mod launch;
mod playback;
mod proc;
mod setup;
mod start;
mod steam;
mod stop;

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    let argv0 = args.next().unwrap_or_default();
    let name = Path::new(&argv0).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut rest: Vec<OsString> = args.collect();
    let cmd = match name.strip_prefix("wpe-bridge-") {
        Some(sub) => sub.to_string(),
        None if rest.is_empty() => String::new(),
        None => rest.remove(0).to_string_lossy().into_owned(),
    };
    let arg = rest.first().map(|a| a.to_string_lossy().into_owned());
    let code = match cmd.as_str() {
        "start" => start::run(),
        "stop" => stop::run(),
        "ui" => launch::ui(),
        "control" => launch::control(arg.as_deref()),
        "playback" => playback::run(),
        "setup" => setup::run(arg.as_deref()),
        "paths" => steam::print_paths(),
        _ => {
            eprintln!("usage: wpe-bridge start|stop|ui|control|playback|setup|paths");
            2
        }
    };
    ExitCode::from(code as u8)
}
