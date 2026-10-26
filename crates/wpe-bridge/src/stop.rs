//! Stops the Wallpaper Engine Proton container started by `start`. Walks up
//! from the engine to its srt-bwrap and ends that, so Steam's other
//! pressure-vessel containers (e.g. the web helper) are left alone.

use crate::proc;

const ENGINE: &str = "wallpaper64.exe";

#[derive(Debug, PartialEq)]
pub enum Outcome {
    NotRunning,
    Stopped { forced: bool },
    Stuck,
}

fn engine_container(engine: &str) -> Option<u32> {
    let mut p = proc::find(engine)?;
    while p > 1 {
        if proc::comm(p).as_deref() == Some("srt-bwrap") {
            return Some(p);
        }
        p = proc::ppid(p)?;
    }
    None
}

pub fn stop() -> Outcome {
    stop_named(ENGINE)
}

fn stop_named(engine: &str) -> Outcome {
    let Some(c) = engine_container(engine) else { return Outcome::NotRunning };
    proc::signal(c, libc::SIGTERM);
    let mut forced = false;
    if !proc::wait_for(20, || !proc::alive(c)) {
        proc::signal(c, libc::SIGKILL);
        forced = true;
    }
    // The engine can outlive its container for a moment; callers restart right after.
    if proc::wait_for(15, || proc::find(engine).is_none()) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;
    use std::fs;
    use std::process::{Child, Command};

    /// A copy of `src` under a new process name (the kernel takes it from the file name).
    fn copy_as(dir: &Scratch, src: &str, name: &str) -> std::path::PathBuf {
        let dst = dir.path().join(name);
        fs::copy(src, &dst).unwrap();
        dst
    }

    /// Reaps `child` in the background, so a stopped process does not linger as a zombie.
    fn reap(mut child: Child) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let _ = child.wait();
        })
    }

    #[test]
    fn nothing_to_stop() {
        assert_eq!(stop_named("wpe-test-absent"), Outcome::NotRunning);
    }

    #[test]
    fn an_engine_outside_a_container_is_left_alone() {
        let s = Scratch::new();
        let sleeper = copy_as(&s, "/usr/bin/sleep", "wpe-test-orphan");
        let mut child = Command::new(sleeper).arg("30").spawn().unwrap();
        assert!(proc::wait_for(5, || proc::find("wpe-test-orphan").is_some()));
        assert_eq!(stop_named("wpe-test-orphan"), Outcome::NotRunning);
        assert!(proc::alive(child.id()));
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn stopping_ends_the_container_around_the_engine() {
        let s = Scratch::new();
        let engine = copy_as(&s, "/usr/bin/sleep", "wpe-test-engine");
        let bwrap = copy_as(&s, "/bin/sh", "srt-bwrap");
        // Like the real container, it takes its engine down with it.
        let script = format!("{} 30 & child=$!; trap 'kill $child; exit 0' TERM; wait", engine.display());
        let child = Command::new(bwrap).args(["-c", &script]).spawn().unwrap();
        let container = child.id();
        let reaper = reap(child);
        assert!(proc::wait_for(5, || proc::find("wpe-test-engine").is_some()));
        assert_eq!(engine_container("wpe-test-engine"), Some(container));

        assert_eq!(stop_named("wpe-test-engine"), Outcome::Stopped { forced: false });
        assert_eq!(proc::find("wpe-test-engine"), None);
        reaper.join().unwrap();
    }
}
