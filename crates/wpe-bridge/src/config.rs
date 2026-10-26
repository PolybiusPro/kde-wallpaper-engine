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
    xdg_dir(env::var_os(var), &home(), fallback)
}

/// `value` of an XDG variable if it is set and not empty, else `home/fallback`.
fn xdg_dir(value: Option<std::ffi::OsString>, home: &Path, fallback: &str) -> PathBuf {
    match value {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => home.join(fallback),
    }
}

pub fn state_dir() -> PathBuf {
    xdg("XDG_STATE_HOME", ".local/state").join("wpe-bridge")
}

pub fn config_file() -> PathBuf {
    xdg("XDG_CONFIG_HOME", ".config").join("wpe-bridge/config")
}

/// Where the package keeps wpehost.exe and wtsapi32.dll: next to
/// the bin directory the binary lives in, so ~/.local and /usr both work.
pub fn share_dir() -> PathBuf {
    let exe = env::current_exe().and_then(|p| p.canonicalize()).unwrap_or_default();
    share_dir_of(&exe)
}

fn share_dir_of(exe: &Path) -> PathBuf {
    exe.parent().and_then(Path::parent).unwrap_or(Path::new("/usr")).join("share/wpe-bridge")
}

pub struct Config {
    pub install: PathBuf,
    pub proton: Option<PathBuf>,
}

impl Config {
    pub fn load() -> Config {
        let file = std::fs::read_to_string(config_file()).ok();
        Config::from_sources(file.as_deref(), &|k| env::var(k).ok(), &steam_dir())
    }

    /// `file` is the config file's text, `getenv` the environment.
    fn from_sources(file: Option<&str>, getenv: &dyn Fn(&str) -> Option<String>, steam: &Path) -> Config {
        let file = file.map(|t| parse(t, getenv)).unwrap_or_default();
        let get = |key: &str| file.get(key).cloned().or_else(|| getenv(key)).filter(|v| !v.is_empty());
        Config {
            install: get("WPE_INSTALL")
                .map(PathBuf::from)
                .unwrap_or_else(|| steam.join("steamapps/common/wallpaper_engine")),
            proton: get("WPE_PROTON").map(PathBuf::from),
        }
    }
}

type GetEnv<'a> = &'a dyn Fn(&str) -> Option<String>;

fn parse(text: &str, getenv: GetEnv) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, val)) = line.split_once('=') else { continue };
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        vars.insert(key.to_string(), unquote(val.trim(), getenv));
    }
    vars
}

/// Quotes come off; `$VAR`, `${VAR}` and a leading `~/` expand unless the value is single-quoted.
fn unquote(val: &str, getenv: GetEnv) -> String {
    for q in ['\'', '"'] {
        if let Some(inner) = val.strip_prefix(q).and_then(|v| v.strip_suffix(q)) {
            return if q == '\'' { inner.to_string() } else { expand(inner, getenv) };
        }
    }
    expand(val, getenv)
}

fn expand(val: &str, getenv: GetEnv) -> String {
    let val = match val.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", getenv("HOME").unwrap_or_default()),
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
        out.push_str(&getenv(&name).unwrap_or_default());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    const HOME: &[(&str, &str)] = &[("HOME", "/home/u"), ("X", "ex")];

    #[test]
    fn parses_shell_style_assignments() {
        let v = parse("# comment\nWPE_INSTALL=\"/mnt/x/we\"\nexport WPE_PROTON='/p/$HOME/proton'\nbad key=1\n", &env_of(HOME));
        assert_eq!(v["WPE_INSTALL"], "/mnt/x/we");
        assert_eq!(v["WPE_PROTON"], "/p/$HOME/proton");
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn parse_skips_lines_that_are_not_assignments() {
        let v = parse("=novalue\njust words\n\n  A_1=spaced  \nB = spaced\n1-2=x\n", &env_of(HOME));
        assert_eq!(v.len(), 1, "spaces around = make the key invalid, as in a shell");
        assert_eq!(v["A_1"], "spaced");
    }

    #[test]
    fn expands_home() {
        let e = env_of(HOME);
        assert_eq!(unquote("$HOME/x", &e), "/home/u/x");
        assert_eq!(unquote("\"${HOME}/x\"", &e), "/home/u/x");
        assert_eq!(unquote("~/x", &e), "/home/u/x");
    }

    #[test]
    fn expansion_edges() {
        let e = env_of(HOME);
        assert_eq!(unquote("'$HOME/x'", &e), "$HOME/x", "single quotes do not expand");
        assert_eq!(unquote("\"~/x\"", &e), "/home/u/x", "double quotes still expand ~/");
        assert_eq!(unquote("'~/x'", &e), "~/x");
        assert_eq!(unquote("a${NOPE}b", &e), "ab", "unknown variables are empty");
        assert_eq!(unquote("$X$X", &e), "exex");
        assert_eq!(unquote("${X", &e), "ex", "an unclosed brace still names the variable");
        assert_eq!(unquote("cost $", &e), "cost ", "a lone dollar is empty");
        assert_eq!(unquote("~/x", &env_of(&[])), "/x", "no HOME");
        assert_eq!(unquote("\"", &e), "\"", "a lone quote stays");
    }

    #[test]
    fn the_file_wins_over_the_environment_which_wins_over_the_defaults() {
        let steam = Path::new("/s");
        let env = env_of(&[("WPE_INSTALL", "/env/we"), ("WPE_PROTON", "/env/proton")]);
        let c = Config::from_sources(Some("WPE_INSTALL=/file/we\n"), &env, steam);
        assert_eq!(c.install, Path::new("/file/we"));
        assert_eq!(c.proton.as_deref(), Some(Path::new("/env/proton")));

        let c = Config::from_sources(None, &env_of(&[]), steam);
        assert_eq!(c.install, Path::new("/s/steamapps/common/wallpaper_engine"));
        assert_eq!(c.proton, None);
    }

    #[test]
    fn empty_values_count_as_unset() {
        let env = env_of(&[("WPE_PROTON", "")]);
        let c = Config::from_sources(None, &env, Path::new("/s"));
        assert_eq!(c.proton, None);
        // An empty value in the file still shadows the environment, so the default applies.
        let env = env_of(&[("WPE_INSTALL", "/env/we")]);
        let c = Config::from_sources(Some("WPE_INSTALL=\n"), &env, Path::new("/s"));
        assert_eq!(c.install, Path::new("/s/steamapps/common/wallpaper_engine"));
    }

    #[test]
    fn file_values_expand_against_the_environment() {
        let c = Config::from_sources(Some("WPE_INSTALL=~/we\n"), &env_of(HOME), Path::new("/s"));
        assert_eq!(c.install, Path::new("/home/u/we"));
    }

    #[test]
    fn xdg_dirs_fall_back_to_home() {
        let home = Path::new("/home/u");
        assert_eq!(xdg_dir(Some("/x".into()), home, ".config"), Path::new("/x"));
        assert_eq!(xdg_dir(Some("".into()), home, ".config"), Path::new("/home/u/.config"));
        assert_eq!(xdg_dir(None, home, ".local/state"), Path::new("/home/u/.local/state"));
    }

    #[test]
    fn share_sits_next_to_the_bin_directory() {
        assert_eq!(share_dir_of(Path::new("/home/u/.local/bin/wpe-bridge")), Path::new("/home/u/.local/share/wpe-bridge"));
        assert_eq!(share_dir_of(Path::new("/usr/bin/wpe-bridge")), Path::new("/usr/share/wpe-bridge"));
        assert_eq!(share_dir_of(Path::new("")), Path::new("/usr/share/wpe-bridge"));
    }

    #[test]
    fn the_real_paths_end_where_they_should() {
        assert!(state_dir().ends_with("wpe-bridge"));
        assert!(config_file().ends_with("wpe-bridge/config"));
        assert!(steam_dir().ends_with(".local/share/Steam"));
        assert!(share_dir().ends_with("share/wpe-bridge"));
    }
}
