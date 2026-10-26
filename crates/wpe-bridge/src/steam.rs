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
        Paths::resolve_in(cfg, find_proton, config::steam_dir(), &SYSTEM_TOOL_ROOTS.map(PathBuf::from))
    }

    /// `resolve` against a given Steam folder and system compatibility-tool folders.
    fn resolve_in(cfg: &Config, find_proton: bool, steam: PathBuf, system_roots: &[PathBuf]) -> Paths {
        let install = cfg.install.clone();
        let library = install.parent().and_then(Path::parent).unwrap_or(Path::new("/")).to_path_buf();
        let slr = slr(&install, &steam);
        let compat = library.join("compatdata").join(config::APPID.to_string());
        let (mut proton, mut tool) = (cfg.proton.clone(), None);
        if proton.is_none() && find_proton {
            let vdf = fs::read_to_string(steam.join("config/config.vdf")).unwrap_or_default();
            tool = tool_name(&vdf, &config::APPID.to_string()).or_else(|| tool_name(&vdf, "0"));
            proton = tool.as_deref().and_then(|t| tool_proton(t, &steam, &library, system_roots));
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
fn slr(install: &Path, steam: &Path) -> PathBuf {
    let candidates = [
        install.join("../SteamLinuxRuntime_4"),
        steam.join("steamapps/common/SteamLinuxRuntime_4"),
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

/// Where distributions install compatibility tools system-wide.
const SYSTEM_TOOL_ROOTS: [&str; 2] = ["/usr/share/steam/compatibilitytools.d", "/usr/local/share/steam/compatibilitytools.d"];

/// The proton script of the tool called `name`.
fn tool_proton(name: &str, steam: &Path, library: &Path, system_roots: &[PathBuf]) -> Option<PathBuf> {
    let mut roots = vec![steam.join("compatibilitytools.d")];
    roots.extend_from_slice(system_roots);
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

    use crate::testutil::Scratch;

    fn tool_vdf(name: &str) -> String {
        format!("\"compatibilitytools\"\n{{\n\t\"compat_tools\"\n\t{{\n\t\t\"{name}\"\n\t\t{{\n\t\t}}\n\t}}\n}}\n")
    }

    #[test]
    fn is_exec_wants_an_executable_file() {
        let s = Scratch::new();
        assert!(is_exec(&s.exec("run", "#!/bin/sh\n")));
        assert!(!is_exec(&s.write("plain", "x")));
        assert!(!is_exec(s.path()), "a directory is not a program");
        assert!(!is_exec(&s.path().join("missing")));
    }

    #[test]
    fn the_runtime_next_to_the_game_wins() {
        let s = Scratch::new();
        let steam = s.path().join("steam");
        let install = s.path().join("lib/common/wallpaper_engine");
        fs::create_dir_all(&install).unwrap(); // the ".." in the candidate needs the folder to exist
        s.exec("lib/common/SteamLinuxRuntime_4/_v2-entry-point", "");
        s.exec("steam/steamapps/common/SteamLinuxRuntime_4/_v2-entry-point", "");
        assert_eq!(slr(&install, &steam), install.join("../SteamLinuxRuntime_4"));
    }

    #[test]
    fn the_runtime_falls_back_to_the_steam_folder() {
        let s = Scratch::new();
        let steam = s.path().join("steam");
        let install = s.path().join("lib/common/wallpaper_engine");
        let default = steam.join("steamapps/common/SteamLinuxRuntime_4");
        s.exec("steam/steamapps/common/SteamLinuxRuntime_4/_v2-entry-point", "");
        assert_eq!(slr(&install, &steam), default);
        // Not installed anywhere: still the default, so the error names a sensible path.
        let empty = Scratch::new();
        assert_eq!(slr(&install, empty.path()), empty.path().join("steamapps/common/SteamLinuxRuntime_4"));
    }

    #[test]
    fn a_tool_is_found_by_the_name_in_its_vdf() {
        let s = Scratch::new();
        s.write("steam/compatibilitytools.d/cachy/compatibilitytool.vdf", &tool_vdf("proton-cachyos"));
        let proton = s.exec("steam/compatibilitytools.d/cachy/proton", "");
        s.write("steam/compatibilitytools.d/other/compatibilitytool.vdf", &tool_vdf("proton-other"));
        s.exec("steam/compatibilitytools.d/other/proton", "");
        let (steam, lib) = (s.path().join("steam"), s.path().join("lib"));
        assert_eq!(tool_proton("proton-cachyos", &steam, &lib, &[]), Some(proton));
        assert_eq!(tool_proton("proton-missing", &steam, &lib, &[]), None);
    }

    #[test]
    fn a_tool_without_an_executable_proton_script_is_skipped() {
        let s = Scratch::new();
        s.write("steam/compatibilitytools.d/t/compatibilitytool.vdf", &tool_vdf("t"));
        s.write("steam/compatibilitytools.d/t/proton", "not executable");
        assert_eq!(tool_proton("t", &s.path().join("steam"), &s.path().join("lib"), &[]), None);
    }

    #[test]
    fn system_tools_are_searched_after_the_users() {
        let s = Scratch::new();
        s.write("sys/t/compatibilitytool.vdf", &tool_vdf("t"));
        let sys = s.exec("sys/t/proton", "");
        let roots = [s.path().join("sys")];
        assert_eq!(tool_proton("t", &s.path().join("steam"), &s.path().join("lib"), &roots), Some(sys));
        let user = s.exec("steam/compatibilitytools.d/t/proton", "");
        s.write("steam/compatibilitytools.d/t/compatibilitytool.vdf", &tool_vdf("t"));
        assert_eq!(tool_proton("t", &s.path().join("steam"), &s.path().join("lib"), &roots), Some(user));
    }

    #[test]
    fn valves_own_builds_map_to_their_game_folders() {
        let s = Scratch::new();
        let (steam, lib) = (s.path().join("steam"), s.path().join("lib"));
        let nine = s.exec("lib/common/Proton 9.0 (Beta)/proton", "");
        let exp = s.exec("lib/common/Proton - Experimental/proton", "");
        let hot = s.exec("steam/steamapps/common/Proton Hotfix/proton", "");
        s.exec("lib/common/Proton 8.0/proton", "");
        s.write("lib/common/Proton 10.0/proton", "not executable");
        assert_eq!(tool_proton("proton_9", &steam, &lib, &[]), Some(nine));
        assert_eq!(tool_proton("proton_experimental", &steam, &lib, &[]), Some(exp));
        assert_eq!(tool_proton("proton_hotfix", &steam, &lib, &[]), Some(hot));
        assert_eq!(tool_proton("proton_10", &steam, &lib, &[]), None, "the script must be executable");
        assert_eq!(tool_proton("proton_x", &steam, &lib, &[]), None, "only digits name a numbered build");
        assert_eq!(tool_proton("other", &steam, &lib, &[]), None);
    }

    fn config_at(install: PathBuf, proton: Option<PathBuf>) -> Config {
        Config { install, proton }
    }

    #[test]
    fn paths_follow_the_game_folder() {
        let s = Scratch::new();
        let install = s.path().join("lib/steamapps/common/wallpaper_engine");
        let p = Paths::resolve_in(&config_at(install.clone(), None), false, s.path().join("steam"), &[]);
        assert_eq!(p.library, s.path().join("lib/steamapps"));
        assert_eq!(p.compat, s.path().join("lib/steamapps/compatdata/431960"));
        assert_eq!(p.install, install);
        assert_eq!(p.proton, None);
        assert_eq!(p.tool, None);
        assert!(p.launch_client().ends_with("SteamLinuxRuntime_4/pressure-vessel/bin/steam-runtime-launch-client"));
    }

    #[test]
    fn the_tool_comes_from_steams_config() {
        let s = Scratch::new();
        s.write("steam/config/config.vdf", VDF);
        let nine = s.exec("lib/steamapps/common/Proton 9.0/proton", "");
        let cfg = config_at(s.path().join("lib/steamapps/common/wallpaper_engine"), None);
        let p = Paths::resolve_in(&cfg, true, s.path().join("steam"), &[]);
        assert_eq!(p.tool.as_deref(), Some("proton_9"));
        assert_eq!(p.proton, Some(nine));
    }

    #[test]
    fn the_default_tool_applies_when_the_game_has_none() {
        let s = Scratch::new();
        s.write("steam/config/config.vdf", &VDF.replace("431960", "999"));
        s.write("sys/c/compatibilitytool.vdf", &tool_vdf("proton-cachyos"));
        let cachy = s.exec("sys/c/proton", "");
        let cfg = config_at(s.path().join("lib/steamapps/common/wallpaper_engine"), None);
        let p = Paths::resolve_in(&cfg, true, s.path().join("steam"), &[s.path().join("sys")]);
        assert_eq!(p.tool.as_deref(), Some("proton-cachyos"));
        assert_eq!(p.proton, Some(cachy));
    }

    #[test]
    fn a_configured_proton_wins_and_skips_the_lookup() {
        let s = Scratch::new();
        s.write("steam/config/config.vdf", VDF);
        let forced = PathBuf::from("/forced/proton");
        let cfg = config_at(s.path().join("lib/steamapps/common/wallpaper_engine"), Some(forced.clone()));
        let p = Paths::resolve_in(&cfg, true, s.path().join("steam"), &[]);
        assert_eq!(p.proton, Some(forced));
        assert_eq!(p.tool, None);
    }

    #[test]
    fn no_steam_config_means_no_tool() {
        let s = Scratch::new();
        let cfg = config_at(s.path().join("lib/steamapps/common/wallpaper_engine"), None);
        let p = Paths::resolve_in(&cfg, true, s.path().join("steam"), &[]);
        assert_eq!((p.tool, p.proton), (None, None));
    }

    #[test]
    fn the_config_parser_ignores_other_sections_and_odd_files() {
        assert_eq!(tool_name("", "0"), None);
        assert_eq!(tool_name("\"CompatToolMapping\"", "0"), None);
        assert_eq!(tool_name("\"CompatToolMapping\"\n{\n}\n\"0\"\n{\n\"name\" \"late\"\n}", "0"), None);
    }

    #[test]
    fn the_real_resolution_runs() {
        let p = Paths::resolve(&Config::load(), false);
        assert!(p.compat.ends_with("compatdata/431960"));
    }
}
