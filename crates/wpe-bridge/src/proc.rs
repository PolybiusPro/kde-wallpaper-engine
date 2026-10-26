//! Process lookups straight from /proc. Names are matched exactly against the
//! kernel's process name (the way `pgrep -x` does), never against a command
//! line, so this program cannot match itself.

use std::fs;

fn pids() -> Vec<u32> {
    let mut v: Vec<u32> = fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse().ok())
        .collect();
    v.sort_unstable();
    v
}

pub fn comm(pid: u32) -> Option<String> {
    Some(fs::read_to_string(format!("/proc/{pid}/comm")).ok()?.trim_end().to_string())
}

/// The lowest pid whose process name is exactly `name`.
pub fn find(name: &str) -> Option<u32> {
    pids().into_iter().find(|&p| comm(p).as_deref() == Some(name))
}

pub fn ppid(pid: u32) -> Option<u32> {
    parent_in_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// The name in field 2 may hold spaces and parentheses; the rest follows the last ')'.
fn parent_in_stat(stat: &str) -> Option<u32> {
    stat[stat.rfind(')')? + 1..].split_whitespace().nth(1)?.parse().ok()
}

pub fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn signal(pid: u32, sig: i32) {
    unsafe { libc::kill(pid as i32, sig) };
}

/// Polls `cond` once a second for up to `secs` seconds; true as soon as it holds.
pub fn wait_for(secs: u32, mut cond: impl FnMut() -> bool) -> bool {
    for n in 0..secs {
        if cond() {
            return true;
        }
        if n + 1 < secs {
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own_pid() -> u32 {
        std::process::id()
    }

    #[test]
    fn parent_is_field_four_after_the_name() {
        assert_eq!(parent_in_stat("42 (sleep) S 7 42 42 0 -1"), Some(7));
        assert_eq!(parent_in_stat("42 (a b) c) S 9 42"), Some(9), "the name may hold spaces and parentheses");
        assert_eq!(parent_in_stat("42 (x) S"), None);
        assert_eq!(parent_in_stat("42 (x) S notanumber"), None);
        assert_eq!(parent_in_stat("no parentheses"), None);
    }

    #[test]
    fn our_own_process_is_visible() {
        let name = fs::read_to_string("/proc/self/comm").unwrap().trim_end().to_string();
        assert_eq!(comm(own_pid()).as_deref(), Some(name.as_str()));
        assert!(find(&name).is_some());
        assert!(pids().contains(&own_pid()));
        assert_eq!(ppid(own_pid()), Some(unsafe { libc::getppid() } as u32));
    }

    #[test]
    fn missing_processes_are_absent() {
        assert_eq!(find("no-such-process-name"), None);
        assert_eq!(comm(u32::MAX), None);
        assert_eq!(ppid(u32::MAX), None);
        assert!(!alive(u32::MAX >> 1));
        assert!(alive(own_pid()));
    }

    #[test]
    fn a_name_must_match_exactly() {
        let name = fs::read_to_string("/proc/self/comm").unwrap().trim_end().to_string();
        assert_eq!(find(&name[..name.len() - 1]), None);
    }

    #[test]
    fn signal_zero_does_not_disturb() {
        signal(own_pid(), 0);
        assert!(alive(own_pid()));
    }

    #[test]
    fn wait_for_returns_as_soon_as_the_condition_holds() {
        assert!(wait_for(5, || true));
        let mut calls = 0;
        assert!(wait_for(5, || {
            calls += 1;
            calls == 2
        }));
        assert_eq!(calls, 2);
    }

    #[test]
    fn wait_for_gives_up() {
        let mut calls = 0;
        assert!(!wait_for(2, || {
            calls += 1;
            false
        }));
        assert_eq!(calls, 2);
        assert!(!wait_for(0, || true));
    }
}
