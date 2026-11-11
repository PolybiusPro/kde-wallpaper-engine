//! Adds a "Wallpaper Engine Bridge" non-Steam shortcut that runs
//! `wpe-bridge-start`, in each account's shortcuts.vdf. Steam launches it once
//! and sees it exit, so it does not show as running the way the real app does.
//!
//! shortcuts.vdf is binary VDF and Steam rewrites it from memory when it exits,
//! so it is only touched while Steam is closed. Entries that are not ours are
//! copied through byte for byte.

use crate::{config, proc};
use std::fs;
use std::ops::Range;
use std::path::{Path, PathBuf};

const MARK: &str = "wpe-bridge-start";
const NAME: &str = "Wallpaper Engine Bridge";

struct Entry {
    /// The bytes after the key, up to and including the entry's closing 0x08.
    body: Range<usize>,
    exe: String,
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Option<u8> {
        let v = *self.b.get(self.i)?;
        self.i += 1;
        Some(v)
    }

    fn cstr(&mut self) -> Option<String> {
        let rest = self.b.get(self.i..)?;
        let n = rest.iter().position(|&c| c == 0)?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.i += n + 1;
        Some(s)
    }

    /// Reads fields up to the map's closing 0x08; returns its "Exe" string, if any.
    fn map(&mut self) -> Option<Option<String>> {
        let mut exe = None;
        loop {
            match self.byte()? {
                8 => return Some(exe),
                0 => {
                    self.cstr()?;
                    self.map()?;
                }
                1 => {
                    let key = self.cstr()?;
                    let val = self.cstr()?;
                    if key.eq_ignore_ascii_case("exe") {
                        exe = Some(val);
                    }
                }
                2 => {
                    self.cstr()?;
                    self.i += 4;
                    if self.i > self.b.len() {
                        return None;
                    }
                }
                _ => return None,
            }
        }
    }
}

/// The entries of a shortcuts.vdf, or None when it is not the shape we know.
fn parse(b: &[u8]) -> Option<Vec<Entry>> {
    let mut r = Reader { b, i: 0 };
    if r.byte()? != 0 || !r.cstr()?.eq_ignore_ascii_case("shortcuts") {
        return None;
    }
    let mut entries = Vec::new();
    loop {
        match r.byte()? {
            8 => return Some(entries),
            0 => {
                r.cstr()?;
                let start = r.i;
                let exe = r.map()?;
                entries.push(Entry { body: start..r.i, exe: exe.unwrap_or_default() });
            }
            _ => return None,
        }
    }
}

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    !c
}

/// One shortcut entry. Steam derives the app id of a shortcut from its exe and
/// name, with the top bit set.
fn entry_body(name: &str, exe: &str, dir: &str) -> Vec<u8> {
    fn int(o: &mut Vec<u8>, key: &str, v: u32) {
        o.push(2);
        o.extend(key.as_bytes());
        o.push(0);
        o.extend(v.to_le_bytes());
    }
    fn text(o: &mut Vec<u8>, key: &str, v: &str) {
        o.push(1);
        o.extend(key.as_bytes());
        o.push(0);
        o.extend(v.as_bytes());
        o.push(0);
    }
    let mut o = Vec::new();
    int(&mut o, "appid", crc32(format!("{exe}{name}").as_bytes()) | 0x8000_0000);
    text(&mut o, "AppName", name);
    text(&mut o, "Exe", exe);
    text(&mut o, "StartDir", dir);
    text(&mut o, "icon", "");
    text(&mut o, "ShortcutPath", "");
    text(&mut o, "LaunchOptions", "");
    int(&mut o, "IsHidden", 0);
    int(&mut o, "AllowDesktopConfig", 1);
    int(&mut o, "AllowOverlay", 1);
    int(&mut o, "OpenVR", 0);
    int(&mut o, "Devkit", 0);
    text(&mut o, "DevkitGameID", "");
    int(&mut o, "DevkitOverrideAppID", 0);
    int(&mut o, "LastPlayTime", 0);
    text(&mut o, "FlatpakAppID", "");
    text(&mut o, "sortas", "");
    o.push(0);
    o.extend(b"tags\0");
    o.push(8);
    o.push(8);
    o
}

fn is_ours(e: &Entry) -> bool {
    Path::new(e.exe.trim_matches('"')).file_name().is_some_and(|n| n == MARK)
}

/// Quotes a path the way Steam stores it in shortcuts.vdf.
fn quoted(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

#[derive(Debug)]
enum Edit {
    Changed(Vec<u8>),
    Unchanged,
    /// The file is not shortcuts.vdf as we know it; left alone.
    Skipped,
}

/// Adds our shortcut (`want` = its exe and start dir, already quoted) or takes
/// ours out. Keys are renumbered 0, 1, 2... as Steam writes them.
fn edit(old: &[u8], want: Option<(&str, &str)>) -> Edit {
    let entries = if old.is_empty() {
        Vec::new()
    } else {
        match parse(old) {
            Some(e) => e,
            None => return Edit::Skipped,
        }
    };
    let ours = entries.iter().any(is_ours);
    if want.is_some() == ours {
        return Edit::Unchanged;
    }
    let mut bodies: Vec<Vec<u8>> =
        entries.iter().filter(|e| !is_ours(e)).map(|e| old[e.body.clone()].to_vec()).collect();
    if let Some((exe, dir)) = want {
        bodies.push(entry_body(NAME, exe, dir));
    }
    let mut out = b"\0shortcuts\0".to_vec();
    for (n, body) in bodies.iter().enumerate() {
        out.push(0);
        out.extend(n.to_string().as_bytes());
        out.push(0);
        out.extend(body);
    }
    out.extend([8, 8]);
    Edit::Changed(out)
}

/// The shortcuts.vdf of every Steam account that has logged in here.
fn shortcut_files(userdata: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(userdata)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("config"))
        .filter(|c| c.join("localconfig.vdf").is_file())
        .map(|c| c.join("shortcuts.vdf"))
        .collect();
    found.sort();
    found
}

/// Adds the shortcut to every Steam account (`enable`), or takes ours out
/// again. Says so when Steam is running and the change has to wait for it to close.
pub fn apply(enable: bool) {
    let want = if enable {
        let Some(bindir) = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()) else { return };
        let Some(bindir) = bindir.parent().map(Path::to_path_buf) else { return };
        Some((quoted(&bindir.join(MARK)), quoted(&bindir)))
    } else {
        None
    };
    let want = want.as_ref().map(|(e, d)| (e.as_str(), d.as_str()));
    apply_in(&config::steam_dir().join("userdata"), want, proc::find("steam").is_some());
}

fn apply_in(userdata: &Path, want: Option<(&str, &str)>, steam_running: bool) {
    let enable = want.is_some();
    let mut pending = Vec::new();
    for file in shortcut_files(userdata) {
        let old = fs::read(&file).unwrap_or_default();
        match edit(&old, want) {
            Edit::Changed(new) => pending.push((file, new)),
            Edit::Skipped => eprintln!("{} is not in a format we know; left alone.", file.display()),
            Edit::Unchanged => {}
        }
    }
    if pending.is_empty() {
        return;
    }
    if steam_running {
        eprintln!(
            "Steam is running and would overwrite the change when it exits, so the \"{NAME}\" shortcut was not {}.\nClose Steam and run wpe-bridge-setup{} again.",
            if enable { "added" } else { "removed" },
            if enable { "" } else { " --remove" }
        );
        return;
    }
    for (file, new) in pending {
        let backup = file.with_extension("vdf.wpe-bridge.bak");
        if file.is_file() && !backup.exists() {
            let _ = fs::copy(&file, &backup);
        }
        match fs::write(&file, new) {
            Ok(()) => println!("Steam: {} the \"{NAME}\" shortcut in {}", if enable { "added" } else { "removed" }, file.display()),
            Err(e) => eprintln!("cannot write {}: {e}", file.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Scratch;

    const EXE: &str = "\"/home/u/.local/bin/wpe-bridge-start\"";
    const DIR: &str = "\"/home/u/.local/bin\"";
    const WANT: Option<(&str, &str)> = Some((EXE, DIR));

    fn file(bodies: &[Vec<u8>]) -> Vec<u8> {
        let mut out = b"\0shortcuts\0".to_vec();
        for (n, b) in bodies.iter().enumerate() {
            out.push(0);
            out.extend(n.to_string().as_bytes());
            out.push(0);
            out.extend(b);
        }
        out.extend([8, 8]);
        out
    }

    fn other() -> Vec<u8> {
        entry_body("Some Game", "\"/games/some.exe\"", "\"/games\"")
    }

    fn changed(e: Edit) -> Vec<u8> {
        match e {
            Edit::Changed(b) => b,
            other => panic!("expected a change, got {other:?}"),
        }
    }

    #[test]
    fn crc32_matches_the_standard_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn an_entry_carries_the_app_id_steam_derives() {
        let body = entry_body(NAME, EXE, DIR);
        let want = (crc32(format!("{EXE}{NAME}").as_bytes()) | 0x8000_0000).to_le_bytes();
        assert_eq!(&body[..7], b"\x02appid\0");
        assert_eq!(&body[7..11], &want);
    }

    #[test]
    fn creates_the_file_when_there_is_none() {
        let out = changed(edit(&[], WANT));
        let entries = parse(&out).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(is_ours(&entries[0]));
        assert_eq!(entries[0].exe, EXE);
    }

    #[test]
    fn appends_after_a_foreign_entry_and_keeps_it_byte_for_byte() {
        let before = file(&[other()]);
        let out = changed(edit(&before, WANT));
        assert_eq!(out, file(&[other(), entry_body(NAME, EXE, DIR)]));
    }

    #[test]
    fn adding_twice_changes_nothing() {
        let once = changed(edit(&file(&[other()]), WANT));
        assert!(matches!(edit(&once, WANT), Edit::Unchanged));
    }

    #[test]
    fn removing_restores_the_original_and_renumbers() {
        let ours = entry_body(NAME, EXE, DIR);
        let out = changed(edit(&file(&[ours.clone(), other()]), None));
        assert_eq!(out, file(&[other()]));
        assert!(matches!(edit(&out, None), Edit::Unchanged));
    }

    #[test]
    fn removing_from_a_missing_file_does_nothing() {
        assert!(matches!(edit(&[], None), Edit::Unchanged));
    }

    #[test]
    fn a_file_in_another_shape_is_left_alone() {
        assert!(matches!(edit(b"not vdf at all", WANT), Edit::Skipped));
        assert!(matches!(edit(&file(&[other()])[..20], WANT), Edit::Skipped));
    }

    fn account(s: &Scratch) -> PathBuf {
        s.write("1234/config/localconfig.vdf", "\"UserLocalConfigStore\"\n{\n}\n");
        s.path().join("1234/config/shortcuts.vdf")
    }

    #[test]
    fn writes_the_shortcut_with_a_backup_when_steam_is_closed() {
        let s = Scratch::new();
        let path = account(&s);
        fs::write(&path, file(&[other()])).unwrap();
        apply_in(s.path(), WANT, false);
        assert_eq!(fs::read(&path).unwrap(), file(&[other(), entry_body(NAME, EXE, DIR)]));
        assert_eq!(fs::read(path.with_extension("vdf.wpe-bridge.bak")).unwrap(), file(&[other()]));
    }

    #[test]
    fn writes_nothing_while_steam_is_running() {
        let s = Scratch::new();
        let path = account(&s);
        apply_in(s.path(), WANT, true);
        assert!(!path.exists());
    }

    #[test]
    fn accounts_without_a_login_are_skipped() {
        let s = Scratch::new();
        fs::create_dir_all(s.path().join("0/config")).unwrap();
        apply_in(s.path(), WANT, false);
        assert!(!s.path().join("0/config/shortcuts.vdf").exists());
    }
}
