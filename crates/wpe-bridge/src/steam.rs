//! Finds the pieces Steam would use to run Wallpaper Engine: the compatibility
//! tool (a proton script), the Steam Linux Runtime, and the paths that follow
//! from where the game is installed.

use crate::config::{self, Config};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

pub fn is_exec(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

pub struct Paths {
    pub install: PathBuf,
    /// The steamapps folder of the library the game lives in.
    pub library: PathBuf,
    pub steam: PathBuf,
    pub slr: PathBuf,
    pub compat: PathBuf,
    pub proton: Option<PathBuf>,
    /// Name of the compatibility tool Steam would pick, for log messages.
    pub tool: Option<String>,
}

impl Paths {
    pub fn resolve(cfg: &Config, find_proton: bool) -> Paths {
        let steam = config::steam_dir();
        let install = cfg.install.clone();
        let library = install.parent().and_then(Path::parent).unwrap_or(Path::new("/")).to_path_buf();
        let slr = slr(&install);
        let compat = library.join("compatdata").join(config::APPID.to_string());
        let (mut proton, mut tool) = (cfg.proton.clone(), None);
        if proton.is_none() && find_proton {
            let vdf = fs::read_to_string(steam.join("config/config.vdf")).unwrap_or_default();
            tool = tool_name(&vdf, &config::APPID.to_string()).or_else(|| tool_name(&vdf, "0"));
            proton = tool.as_deref().and_then(|t| tool_proton(t, &steam, &library));
        }
        Paths { install, library, steam, slr, compat, proton, tool }
    }

    /// The launch client that runs a command inside the game's container.
    pub fn launch_client(&self) -> PathBuf {
        self.slr.join("pressure-vessel/bin/steam-runtime-launch-client")
    }
}

/// `wpe-bridge paths`: what was resolved, for checking a setup without starting anything.
pub fn print_paths() -> i32 {
    let p = Paths::resolve(&Config::load(), true);
    let show = |o: &Option<PathBuf>| o.as_ref().map_or("(none)".into(), |p| p.display().to_string());
    println!("install  {}", p.install.display());
    println!("library  {}", p.library.display());
    println!("compat   {}", p.compat.display());
    println!("slr      {}", p.slr.display());
    println!("tool     {}", p.tool.as_deref().unwrap_or("(none)"));
    println!("proton   {}", show(&p.proton));
    0
}

/// Steam puts the runtime in whichever library it last installed into.
pub fn slr(install: &Path) -> PathBuf {
    let candidates = [
        install.join("../SteamLinuxRuntime_4"),
        config::steam_dir().join("steamapps/common/SteamLinuxRuntime_4"),
    ];
    for c in &candidates {
        if is_exec(&c.join("_v2-entry-point")) {
            return c.clone();
        }
    }
    candidates[1].clone()
}

/// Quoted strings on a line of Valve's key-value format.
fn tokens(line: &str) -> Vec<&str> {
    line.split('"').skip(1).step_by(2).collect()
}

/// The compatibility tool chosen for `appid` in Steam's config.vdf.
fn tool_name(vdf: &str, appid: &str) -> Option<String> {
    let mut lines = vdf.lines().skip_while(|l| !l.contains("\"CompatToolMapping\""));
    lines.next()?;
    let (mut depth, mut key) = (0, "");
    for line in lines {
        let t = line.trim();
        if t.contains('{') {
            depth += 1;
        }
        if t.contains('}') {
            depth -= 1;
            if depth <= 0 {
                break;
            }
        }
        let toks = tokens(t);
        match (depth, toks.as_slice()) {
            (1, [k, ..]) => key = k,
            (2, ["name", name, ..]) if key == appid => return Some(name.to_string()),
            _ => {}
        }
    }
    None
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort();
    v
}

/// The proton script of the tool called `name`.
fn tool_proton(name: &str, steam: &Path, library: &Path) -> Option<PathBuf> {
    let roots = [
        steam.join("compatibilitytools.d"),
        PathBuf::from("/usr/share/steam/compatibilitytools.d"),
        PathBuf::from("/usr/local/share/steam/compatibilitytools.d"),
    ];
    let quoted = format!("\"{name}\"");
    for d in roots.iter().flat_map(|r| subdirs(r)) {
        let names_it = fs::read_to_string(d.join("compatibilitytool.vdf"))
            .map(|t| t.lines().any(|l| l.trim() == quoted))
            .unwrap_or(false);
        if names_it && is_exec(&d.join("proton")) {
            return Some(d.join("proton"));
        }
    }
    // Valve's own builds are installed as ordinary games.
    let commons = [library.join("common"), steam.join("steamapps/common")];
    let wanted = |dir_name: &str| match name {
        "proton_experimental" => dir_name == "Proton - Experimental",
        "proton_hotfix" => dir_name == "Proton Hotfix",
        _ => name
            .strip_prefix("proton_")
            .filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
            .is_some_and(|v| dir_name.starts_with(&format!("Proton {v}"))),
    };
    for common in &commons {
        for d in subdirs(common) {
            let ok = d.file_name().and_then(|n| n.to_str()).is_some_and(wanted);
            if ok && is_exec(&d.join("proton")) {
                return Some(d.join("proton"));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const VDF: &str = "\"InstallConfigStore\"\n{\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"CompatToolMapping\"\n\t\t\t\t{\n\t\t\t\t\t\"0\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"proton-cachyos\"\n\t\t\t\t\t\t\"config\"\t\t\"\"\n\t\t\t\t\t}\n\t\t\t\t\t\"431960\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"proton_9\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t\t\"Other\"\n\t\t\t\t{\n\t\t\t\t\t\"0\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"name\"\t\t\"wrong\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    #[test]
    fn picks_per_game_then_default() {
        assert_eq!(tool_name(VDF, "431960").as_deref(), Some("proton_9"));
        assert_eq!(tool_name(VDF, "0").as_deref(), Some("proton-cachyos"));
        assert_eq!(tool_name(VDF, "12345"), None);
    }

    #[test]
    fn tokens_split_on_quotes() {
        assert_eq!(tokens("\t\"name\"\t\t\"Proton 9.0\""), ["name", "Proton 9.0"]);
    }
}
