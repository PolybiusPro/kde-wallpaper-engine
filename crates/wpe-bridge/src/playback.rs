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
//!
//! A single covered screen is paused by the engine itself: the KWin script also
//! reports the covered screens' rectangles (SetCovered), which go to wpehost
//! through a file, and wpehost keeps a stand-in Wine window on each of them.

use crate::config::{self, Config};
use crate::proc;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
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

    /// The covered screens as "x,y,w,h,kind;..." with kind "fullscreen" or "maximized".
    fn set_covered(&self, rects: String) {
        if let Err(e) = write_covers(&cover_file(), &cover_lines(&rects, &Real.user_settings())) {
            log(&format!("cannot write the cover file: {e}"));
        }
    }
}

/// Where wpehost looks for the covered screens (see `WPE_COVER_FILE` in start).
pub fn cover_file() -> PathBuf {
    config::state_dir().join("cover")
}

/// The cover file's lines for "x,y,w,h,kind;..."; anything malformed is dropped.
///
/// Only screens whose setting is "pause" get a line. The engine stops every
/// monitor at once, so a stop is left to the all-screens-covered rule above, and
/// a "run" needs nothing.
///
/// The engine pauses a maximized stand-in on its own monitor, but it follows one
/// foreground window for fullscreen, which only ever holds a single screen. So
/// when both settings are the same action, a fullscreen window is passed on as a
/// maximized one and every screen takes the reliable path.
fn cover_lines(rects: &str, user: &Map<String, Value>) -> String {
    let action = |kind: &str| user.get(&format!("playback{kind}")).and_then(Value::as_str).unwrap_or("run").to_string();
    let as_maximized = action("fullscreen") == action("maximized");
    let mut out = String::new();
    for rect in rects.split(';') {
        let f: Vec<&str> = rect.split(',').collect();
        let numbers: Option<Vec<i32>> = f.iter().take(4).map(|v| v.trim().parse().ok()).collect();
        let (Some(n), Some(kind)) = (numbers, f.get(4)) else { continue };
        if n.len() != 4 || !matches!(*kind, "fullscreen" | "maximized") || action(kind) != "pause" {
            continue;
        }
        let kind = if as_maximized { "maximized" } else { kind };
        out.push_str(&format!("{} {} {} {} {kind}\n", n[0], n[1], n[2], n[3]));
    }
    out
}

/// Replaces the file in one step, so wpehost never reads half of it.
pub fn write_covers(path: &Path, lines: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, lines)?;
    std::fs::rename(&tmp, path)
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

/// What the playback logic needs from the machine, so tests can fake it.
trait Host {
    fn engine_pid(&self) -> Option<u32>;
    /// The `general.user` settings from the engine's config.json.
    fn user_settings(&self) -> Map<String, Value>;
    fn control(&self, cmd: &str) -> bool;
}

struct Real;

impl Host for Real {
    fn engine_pid(&self) -> Option<u32> {
        proc::find("wallpaper64.exe")
    }

    fn user_settings(&self) -> Map<String, Value> {
        settings(&std::fs::read_to_string(Config::load().install.join("config.json")).unwrap_or_default())
    }

    fn control(&self, cmd: &str) -> bool {
        control(cmd)
    }
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

    fn apply(&mut self, host: &dyn Host) {
        self.due = None;
        let Some(state) = self.state.as_deref() else { return };
        if host.engine_pid().is_none() {
            return;
        }
        let want = desired(state, &host.user_settings());
        if want == self.applied.unwrap_or("run") {
            return;
        }
        // Only undo what we did, so a pause made in the engine's UI survives.
        let ok = if want == "run" {
            host.control("play")
        } else {
            if self.applied == Some("stop") {
                host.control("play"); // a stopped engine does not take a pause
            }
            host.control(want)
        };
        if ok {
            self.applied = if want == "run" { None } else { Some(want) };
        }
    }

    fn watch_engine(&mut self, host: &dyn Host) {
        let pid = host.engine_pid();
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
    // Covers from an earlier session would pause screens nothing covers now.
    let _ = write_covers(&cover_file(), "");

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
            playback.apply(&Real);
        }
        if next_watch <= now {
            playback.watch_engine(&Real);
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

    fn pause_settings(fullscreen: &str, maximized: &str) -> Map<String, Value> {
        let json = format!(
            r#"{{"a":{{"general":{{"user":{{"playbackfullscreen":"{fullscreen}","playbackmaximized":"{maximized}"}}}}}}}}"#
        );
        settings(&json)
    }

    #[test]
    fn cover_lines_keep_well_formed_screens_only() {
        let differ = pause_settings("pause", "pause");
        assert_eq!(
            cover_lines("0,222,2560,1440,maximized;2560,0,1080,1920,fullscreen", &differ),
            "0 222 2560 1440 maximized\n2560 0 1080 1920 maximized\n"
        );
        assert_eq!(cover_lines("", &differ), "");
        let junk = "1,2,3,x,maximized;1,2,3,4,sideways;1,2,3,maximized;5,6,7,8,maximized";
        assert_eq!(cover_lines(junk, &differ), "5 6 7 8 maximized\n");
    }

    #[test]
    fn fullscreen_keeps_its_own_stand_in_when_the_settings_differ() {
        let differ = pause_settings("pause", "run");
        assert_eq!(cover_lines("0,0,10,10,fullscreen;20,0,10,10,maximized", &differ), "0 0 10 10 fullscreen\n");
    }

    #[test]
    fn only_a_pause_gets_a_stand_in() {
        let stop = pause_settings("stop", "pause");
        // The engine stops every monitor at once, so a covered screen must not trigger it.
        assert_eq!(cover_lines("0,0,10,10,fullscreen", &stop), "");
        assert_eq!(cover_lines("0,0,10,10,maximized", &stop), "0 0 10 10 maximized\n");
        assert_eq!(cover_lines("0,0,10,10,fullscreen", &Map::new()), "");
    }

    #[test]
    fn the_cover_file_is_replaced_whole() {
        let dir = std::env::temp_dir().join(format!("wpe-cover-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cover");
        write_covers(&path, "a\n").unwrap();
        write_covers(&path, "").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert!(!path.with_extension("tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    use std::cell::{Cell, RefCell};

    /// A fake machine: an engine that may be running, user settings, and a log
    /// of every control command. `fail` makes control report failure.
    struct Fake {
        pid: Cell<Option<u32>>,
        user: Map<String, Value>,
        sent: RefCell<Vec<String>>,
        fail: Cell<bool>,
    }

    impl Fake {
        fn new() -> Fake {
            Fake { pid: Cell::new(Some(100)), user: user(), sent: RefCell::default(), fail: Cell::new(false) }
        }

        fn sent(&self) -> Vec<String> {
            self.sent.borrow().clone()
        }
    }

    impl Host for Fake {
        fn engine_pid(&self) -> Option<u32> {
            self.pid.get()
        }

        fn user_settings(&self) -> Map<String, Value> {
            self.user.clone()
        }

        fn control(&self, cmd: &str) -> bool {
            self.sent.borrow_mut().push(cmd.to_string());
            !self.fail.get()
        }
    }

    fn with_state(state: &str) -> Playback {
        let mut p = Playback::default();
        p.set_state(state.to_string());
        p
    }

    #[test]
    fn settings_edge_cases() {
        assert!(settings("").is_empty());
        assert!(settings("[1,2]").is_empty());
        assert!(settings(r#"{"a":{"general":{}}}"#).is_empty());
        // A profile whose user block is not an object yields nothing, and the search stops there.
        assert!(settings(r#"{"a":{"general":{"user":5}},"b":{"general":{"user":{"x":"y"}}}}"#).is_empty());
        // Profiles without a general block are skipped.
        assert_eq!(settings(r#"{"a":{"nope":1},"b":{"general":{"user":{"x":"y"}}}}"#).len(), 1);
    }

    #[test]
    fn rank_orders_run_pause_stop_and_defaults_to_run() {
        assert_eq!(rank("run"), 0);
        assert_eq!(rank("pause"), 1);
        assert_eq!(rank("stop"), 2);
        assert_eq!(rank("whatever"), 0);
    }

    #[test]
    fn a_missing_or_odd_setting_means_run() {
        let none = Map::new();
        assert_eq!(desired("fullscreen", &none), "run");
        let odd = settings(r#"{"a":{"general":{"user":{"playbackfullscreen":"explode","playbackmaximized":5}}}}"#);
        assert_eq!(desired("fullscreen", &odd), "run");
        assert_eq!(desired("maximized", &odd), "run");
    }

    #[test]
    fn set_state_schedules_and_only_logs_changes() {
        let mut p = Playback::default();
        assert!(p.due.is_none());
        p.set_state("fullscreen".into());
        assert_eq!(p.state.as_deref(), Some("fullscreen"));
        assert!(p.due.is_some());
        p.due = None;
        p.set_state("fullscreen".into());
        assert!(p.due.is_some(), "a repeated state still reschedules");
    }

    #[test]
    fn nothing_happens_without_a_state_or_an_engine() {
        let host = Fake::new();
        let mut p = Playback::default();
        p.schedule(Duration::ZERO);
        p.apply(&host);
        assert!(p.due.is_none());
        assert!(host.sent().is_empty());

        let mut p = with_state("fullscreen");
        host.pid.set(None);
        p.apply(&host);
        assert!(host.sent().is_empty());
        assert_eq!(p.applied, None);
    }

    #[test]
    fn a_covered_screen_pauses_and_uncovering_plays() {
        let host = Fake::new();
        let mut p = with_state("fullscreen");
        p.apply(&host);
        assert_eq!(host.sent(), ["pause"]);
        assert_eq!(p.applied, Some("pause"));

        // The same state again is a no-op.
        p.apply(&host);
        assert_eq!(host.sent(), ["pause"]);

        p.set_state("none".into());
        p.apply(&host);
        assert_eq!(host.sent(), ["pause", "play"]);
        assert_eq!(p.applied, None);
    }

    #[test]
    fn going_from_stop_to_pause_plays_first() {
        let host = Fake::new();
        let mut p = with_state("maximized");
        p.apply(&host);
        assert_eq!(host.sent(), ["stop"]);
        assert_eq!(p.applied, Some("stop"));

        p.set_state("fullscreen".into());
        p.apply(&host);
        assert_eq!(host.sent(), ["stop", "play", "pause"]);
        assert_eq!(p.applied, Some("pause"));
    }

    #[test]
    fn a_pause_made_in_the_ui_survives() {
        // We never applied anything, so "run" must not send play.
        let host = Fake::new();
        let mut p = with_state("none");
        p.apply(&host);
        assert!(host.sent().is_empty());
    }

    #[test]
    fn a_failed_command_is_retried_next_time() {
        let host = Fake::new();
        host.fail.set(true);
        let mut p = with_state("fullscreen");
        p.apply(&host);
        assert_eq!(p.applied, None);

        host.fail.set(false);
        p.apply(&host);
        assert_eq!(host.sent(), ["pause", "pause"]);
        assert_eq!(p.applied, Some("pause"));
    }

    #[test]
    fn a_new_engine_forgets_what_we_sent_and_gets_time_to_start() {
        let host = Fake::new();
        let mut p = with_state("fullscreen");
        p.apply(&host);
        assert_eq!(p.applied, Some("pause"));

        p.watch_engine(&host);
        assert_eq!(p.pid, Some(100));
        assert_eq!(p.applied, None, "the first sighting is a new engine");
        assert!(p.due.is_some());

        p.due = None;
        p.watch_engine(&host);
        assert!(p.due.is_none(), "same engine, nothing to do");

        host.pid.set(Some(200));
        p.applied = Some("pause");
        p.watch_engine(&host);
        assert_eq!(p.pid, Some(200));
        assert_eq!(p.applied, None);
        assert!(p.due.is_some());
    }

    #[test]
    fn an_engine_that_goes_away_schedules_nothing() {
        let host = Fake::new();
        let mut p = Playback::default();
        p.watch_engine(&host);
        p.due = None;
        host.pid.set(None);
        p.applied = Some("stop");
        p.watch_engine(&host);
        assert_eq!(p.pid, None);
        assert_eq!(p.applied, None);
        assert!(p.due.is_none());
    }
}
