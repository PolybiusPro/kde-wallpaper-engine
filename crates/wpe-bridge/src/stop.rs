//! Stops the Wallpaper Engine Proton container started by `start`. Walks up
//! from the engine to its srt-bwrap and ends that, so Steam's other
//! pressure-vessel containers (e.g. the web helper) are left alone.

use crate::proc;

const ENGINE: &str = "wallpaper64.exe";

pub enum Outcome {
    NotRunning,
    Stopped { forced: bool },
    Stuck,
}

fn engine_container() -> Option<u32> {
    let mut p = proc::find(ENGINE)?;
    while p > 1 {
        if proc::comm(p).as_deref() == Some("srt-bwrap") {
            return Some(p);
        }
        p = proc::ppid(p)?;
    }
    None
}

pub fn stop() -> Outcome {
    let Some(c) = engine_container() else { return Outcome::NotRunning };
    proc::signal(c, libc::SIGTERM);
    let mut forced = false;
    if !proc::wait_for(20, || !proc::alive(c)) {
        proc::signal(c, libc::SIGKILL);
        forced = true;
    }
    // The engine can outlive its container for a moment; callers restart right after.
    if proc::wait_for(15, || proc::find(ENGINE).is_none()) {
        Outcome::Stopped { forced }
    } else {
        Outcome::Stuck
    }
}

pub fn run() -> i32 {
    match stop() {
        Outcome::NotRunning => println!("engine not running"),
        Outcome::Stopped { forced: false } => println!("stopped"),
        Outcome::Stopped { forced: true } => println!("stopped (forced)"),
        Outcome::Stuck => {
            eprintln!("engine still present after stop");
            return 1;
        }
    }
    0
}
