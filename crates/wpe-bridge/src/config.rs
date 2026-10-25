//! Paths and the optional ${XDG_CONFIG_HOME:-~/.config}/wpe-bridge/config file:
//! plain `KEY=value` lines, WPE_INSTALL (Wallpaper Engine's folder) and
//! WPE_PROTON (the proton script to run it with). The file wins over the
//! environment, which wins over the defaults.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

pub const APPID: u32 = 431960;

pub fn home() -> PathBuf {
    env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

pub fn steam_dir() -> PathBuf {
    home().join(".local/share/Steam")
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home().join(fallback),
    }
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("wpe-bridge")
}

pub fn config_file() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("wpe-bridge/config")
}

/// Where the package keeps wpehost.exe, close-ui.js and wtsapi32.dll: next to
/// the bin directory the binary lives in, so ~/.local and /usr both work.
pub fn share_dir() -> PathBuf {
    let exe = env::current_exe().and_then(|p| p.canonicalize()).unwrap_or_default();
    exe.parent().and_then(Path::parent).unwrap_or(Path::new("/usr")).join("share/wpe-bridge")
}

pub struct Config {
    pub install: PathBuf,
    pub proton: Option<PathBuf>,
}

impl Config {
    pub fn load() -> Config {
        let file = std::fs::read_to_string(config_file()).map(|t| parse(&t)).unwrap_or_default();
        let get = |key: &str| {
            file.get(key)
                .cloned()
                .or_else(|| env::var(key).ok())
                .filter(|v| !v.is_empty())
        };
        Config {
            install: get("WPE_INSTALL")
                .map(PathBuf::from)
                .unwrap_or_else(|| steam_dir().join("steamapps/common/wallpaper_engine")),
            proton: get("WPE_PROTON").map(PathBuf::from),
        }
    }
}

fn parse(text: &str) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, val)) = line.split_once('=') else { continue };
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        vars.insert(key.to_string(), unquote(val.trim()));
    }
    vars
}

/// Quotes come off; `$VAR`, `${VAR}` and a leading `~/` expand unless the value is single-quoted.
fn unquote(val: &str) -> String {
    for q in ['\'', '"'] {
        if let Some(inner) = val.strip_prefix(q).and_then(|v| v.strip_suffix(q)) {
            return if q == '\'' { inner.to_string() } else { expand(inner) };
        }
    }
    expand(val)
}

fn expand(val: &str) -> String {
    let val = match val.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", home().display()),
        None => val.to_string(),
    };
    let mut out = String::new();
    let mut chars = val.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&n) = chars.peek() {
            if n.is_ascii_alphanumeric() || n == '_' {
                name.push(n);
                chars.next();
            } else {
                break;
            }
        }
        if braced && chars.peek() == Some(&'}') {
            chars.next();
        }
        out.push_str(&env::var(&name).unwrap_or_default());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shell_style_assignments() {
        let v = parse("# comment\nWPE_INSTALL=\"/mnt/x/we\"\nexport WPE_PROTON='/p/$HOME/proton'\nbad key=1\n");
        assert_eq!(v["WPE_INSTALL"], "/mnt/x/we");
        assert_eq!(v["WPE_PROTON"], "/p/$HOME/proton");
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn expands_home() {
        env::set_var("HOME", "/home/u");
        assert_eq!(unquote("$HOME/x"), "/home/u/x");
        assert_eq!(unquote("\"${HOME}/x\""), "/home/u/x");
        assert_eq!(unquote("~/x"), "/home/u/x");
    }
}
