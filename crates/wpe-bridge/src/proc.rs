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
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The name in field 2 may hold spaces and parentheses; the rest follows the last ')'.
    stat[stat.rfind(')')? + 1..].split_whitespace().nth(1)?.parse().ok()
}

pub fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

pub fn signal(pid: u32, sig: i32) {
    unsafe { libc::kill(pid as i32, sig) };
}

/// True if some process's command line contains `needle`.
pub fn cmdline_contains(needle: &str) -> bool {
    pids().into_iter().any(|p| {
        fs::read(format!("/proc/{p}/cmdline"))
            .map(|c| String::from_utf8_lossy(&c).replace('\0', " ").contains(needle))
            .unwrap_or(false)
    })
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
