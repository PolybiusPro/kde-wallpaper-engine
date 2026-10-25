//! Applies Wallpaper Engine's "other application fullscreen / maximized"
//! settings (run, pause or stop) to the real desktop.
//!
//! The engine runs under Wine and cannot see native windows, so it never acts
//! on those settings itself. The wpe-bridge KWin script reports, over D-Bus,
//! which of the screens that show the engine are covered by a fullscreen or
//! maximized window: SetState("fullscreen,none") means the first screen has a
//! fullscreen window and the second has nothing. This service reads the
//! engine's own settings from its config.json (so changes made in its UI apply
//! at once) and sends the matching `control` command.
//!
//! Pause and stop are global, so they are sent only when every screen showing
//! the engine is covered; otherwise a screen you can see would freeze.

use crate::config::Config;
use crate::proc;
use serde_json::{Map, Value};
use std::process::Command;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

const NAME: &str = "org.local.WpeBridge";
const PATH: &str = "/Playback";
const RANK: [&str; 3] = ["run", "pause", "stop"];
/// Video players toggle fullscreen in quick bursts, and every command spawns a
/// Wine process, so wait for the state to settle before acting on it.
const SETTLE: Duration = Duration::from_millis(1000);
/// A fresh engine needs a moment before it takes commands.
const ENGINE_START: Duration = Duration::from_millis(8000);
const WATCH: Duration = Duration::from_secs(3);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

struct Service(mpsc::Sender<String>);

#[zbus::interface(name = "org.local.WpeBridge.Playback")]
impl Service {
    fn set_state(&self, state: String) {
        let _ = self.0.send(state);
    }
}

fn log(msg: &str) {
    println!("{msg}");
}

/// The `general.user` settings of the first profile in the engine's config.json.
fn settings(config_json: &str) -> Map<String, Value> {
    let Ok(Value::Object(cfg)) = serde_json::from_str::<Value>(config_json) else { return Map::new() };
    for profile in cfg.values() {
        if let Some(user) = profile.get("general").and_then(|g| g.get("user")) {
            return user.as_object().cloned().unwrap_or_default();
        }
    }
    Map::new()
}

fn rank(action: &str) -> usize {
    RANK.iter().position(|r| *r == action).unwrap_or(0)
}

/// The action for a state like "fullscreen,maximized": the gentlest one wins.
fn desired(state: &str, user: &Map<String, Value>) -> &'static str {
    let kinds: Vec<&str> = state.split(',').filter(|k| !k.is_empty()).collect();
    if kinds.is_empty() || kinds.contains(&"none") {
        return "run";
    }
    let action = |kind: &&str| {
        let v = user.get(&format!("playback{kind}")).and_then(Value::as_str).unwrap_or("run");
        RANK[rank(v)]
    };
    kinds.iter().map(action).min_by_key(|a| rank(a)).unwrap_or("run")
}

fn control(cmd: &str) -> bool {
    let mut child = match std::env::current_exe().and_then(|exe| Command::new(exe).args(["control", cmd]).spawn()) {
        Ok(c) => c,
        Err(e) => {
            log(&format!("control {cmd} -> {e}"));
            return false;
        }
    };
    let start = Instant::now();
    let rc = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) if start.elapsed() > CONTROL_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                break -1;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break -1,
        }
    };
    log(&format!("control {cmd} -> {rc}"));
    rc == 0
}

#[derive(Default)]
struct Playback {
    /// Last state from the KWin script.
    state: Option<String>,
    /// "pause" or "stop" while we are holding the engine.
    applied: Option<&'static str>,
    pid: Option<u32>,
    due: Option<Instant>,
}

impl Playback {
    fn schedule(&mut self, after: Duration) {
        self.due = Some(Instant::now() + after);
    }

    fn set_state(&mut self, state: String) {
        if self.state.as_deref() != Some(&state) {
            log(&format!("state {state:?}"));
            self.state = Some(state);
        }
        self.schedule(SETTLE);
    }

    fn apply(&mut self) {
        self.due = None;
        let Some(state) = self.state.as_deref() else { return };
        if proc::find("wallpaper64.exe").is_none() {
            return;
        }
        let json = std::fs::read_to_string(Config::load().install.join("config.json")).unwrap_or_default();
        let want = desired(state, &settings(&json));
        if want == self.applied.unwrap_or("run") {
            return;
        }
        // Only undo what we did, so a pause made in the engine's UI survives.
        let ok = if want == "run" {
            control("play")
        } else {
            if self.applied == Some("stop") {
                control("play"); // a stopped engine does not take a pause
            }
            control(want)
        };
        if ok {
            self.applied = if want == "run" { None } else { Some(want) };
        }
    }

    fn watch_engine(&mut self) {
        let pid = proc::find("wallpaper64.exe");
        if pid != self.pid {
            let show = |p: Option<u32>| p.map_or("None".to_string(), |p| p.to_string());
            log(&format!("engine pid {} -> {}", show(self.pid), show(pid)));
            self.pid = pid;
            // A new engine is playing, whatever we sent to the old one.
            self.applied = None;
            if pid.is_some() {
                self.schedule(ENGINE_START);
            }
        }
    }
}

pub fn run() -> i32 {
    let (tx, rx) = mpsc::channel();
    let built = zbus::blocking::connection::Builder::session()
        .and_then(|b| b.name(NAME))
        .and_then(|b| b.serve_at(PATH, Service(tx)))
        .and_then(|b| b.build());
    let _conn = match built {
        Ok(c) => c,
        Err(zbus::Error::NameTaken) => {
            log(&format!("could not own {NAME} (already running?)"));
            return 0;
        }
        Err(e) => {
            log(&format!("cannot serve {NAME}: {e}"));
            return 1;
        }
    };
    log("playback helper started");

    let mut playback = Playback::default();
    let mut next_watch = Instant::now() + WATCH;
    loop {
        let now = Instant::now();
        let wake = playback.due.map_or(next_watch, |d| d.min(next_watch));
        match rx.recv_timeout(wake.saturating_duration_since(now)) {
            Ok(state) => playback.set_state(state),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return 1,
        }
        let now = Instant::now();
        if playback.due.is_some_and(|d| d <= now) {
            playback.apply();
        }
        if next_watch <= now {
            playback.watch_engine();
            next_watch = now + WATCH;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> Map<String, Value> {
        settings(r#"{"other":1,"Main":{"general":{"user":{"playbackfullscreen":"pause","playbackmaximized":"stop"}}}}"#)
    }

    #[test]
    fn reads_first_profile_with_user_settings() {
        assert_eq!(user().len(), 2);
        assert!(settings("not json").is_empty());
    }

    #[test]
    fn gentlest_action_wins() {
        let u = user();
        assert_eq!(desired("fullscreen", &u), "pause");
        assert_eq!(desired("maximized", &u), "stop");
        assert_eq!(desired("fullscreen,maximized", &u), "pause");
    }

    #[test]
    fn any_uncovered_screen_means_run() {
        let u = user();
        assert_eq!(desired("fullscreen,none", &u), "run");
        assert_eq!(desired("", &u), "run");
        assert_eq!(desired("unknownkind", &u), "run");
    }
}
