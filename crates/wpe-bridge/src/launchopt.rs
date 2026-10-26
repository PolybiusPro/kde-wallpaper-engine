//! Points Steam's Play button for Wallpaper Engine at `wpe-bridge-start`, by
//! setting the app's LaunchOptions in each account's localconfig.vdf.
//! `%command%` is where Steam would put its own Proton command line; start
//! ignores its arguments, so the stock launcher never runs.
//!
//! Steam rewrites localconfig.vdf from memory when it exits, so the file is only
//! touched while Steam is closed. The edit is textual: nothing but the one line
//! changes, and anything this parser does not understand is left alone.

use crate::config::{self, APPID};
use crate::proc;
use std::fs;
use std::path::{Path, PathBuf};

const MARK: &str = "wpe-bridge-start";
const PATH: [&str; 5] = ["UserLocalConfigStore", "Software", "Valve", "Steam", "apps"];

struct Entry {
    key: String,
    /// Start of the line the key is on.
    line: usize,
    node: Node,
}

enum Node {
    Value { start: usize, end: usize, text: String },
    Block(Block),
}

struct Block {
    entries: Vec<Entry>,
    /// Position of the closing brace.
    close: usize,
}

struct Parser<'a> {
    s: &'a str,
    i: usize,
}

impl Parser<'_> {
    fn skip(&mut self) {
        let b = self.s.as_bytes();
        while self.i < b.len() {
            if b[self.i].is_ascii_whitespace() {
                self.i += 1;
            } else if b[self.i..].starts_with(b"//") {
                while self.i < b.len() && b[self.i] != b'\n' {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    /// A quoted string: (start, end after the closing quote, unescaped text).
    fn string(&mut self) -> Option<(usize, usize, String)> {
        let b = self.s.as_bytes();
        let start = self.i;
        if b.get(start) != Some(&b'"') {
            return None;
        }
        let mut out = Vec::new();
        self.i += 1;
        while let Some(&c) = b.get(self.i) {
            self.i += 1;
            match c {
                b'"' => return Some((start, self.i, String::from_utf8(out).ok()?)),
                b'\\' => {
                    let e = *b.get(self.i)?;
                    self.i += 1;
                    out.push(match e {
                        b'n' => b'\n',
                        b't' => b'\t',
                        other => other,
                    });
                }
                _ => out.push(c),
            }
        }
        None
    }

    /// Entries up to a closing brace (`top` is false) or the end of the file.
    fn entries(&mut self, top: bool) -> Option<Block> {
        let mut entries = Vec::new();
        loop {
            self.skip();
            let b = self.s.as_bytes();
            match b.get(self.i) {
                None if top => return Some(Block { entries, close: self.i }),
                Some(b'}') if !top => {
                    self.i += 1;
                    return Some(Block { entries, close: self.i - 1 });
                }
                Some(b'"') => {}
                _ => return None,
            }
            let line = self.s[..self.i].rfind('\n').map_or(0, |n| n + 1);
            let (_, _, key) = self.string()?;
            self.skip();
            let node = match b.get(self.i) {
                Some(b'{') => {
                    self.i += 1;
                    Node::Block(self.entries(false)?)
                }
                Some(b'"') => {
                    let (start, end, text) = self.string()?;
                    Node::Value { start, end, text }
                }
                _ => return None,
            };
            entries.push(Entry { key, line, node });
        }
    }
}

fn child<'a>(block: &'a Block, key: &str) -> Option<&'a Entry> {
    block.entries.iter().find(|e| e.key.eq_ignore_ascii_case(key))
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Start of the line holding the brace at `close`, and that line's indent, if
/// the brace is the first thing on it.
fn brace_line(text: &str, close: usize) -> Option<(usize, &str)> {
    let line = text[..close].rfind('\n').map_or(0, |n| n + 1);
    let indent = &text[line..close];
    indent.chars().all(|c| c == ' ' || c == '\t').then_some((line, indent))
}

fn is_ours(text: &str) -> bool {
    text.contains(MARK)
}

#[derive(Debug, PartialEq)]
pub enum Edit {
    Changed(String),
    Unchanged,
    /// The app already has launch options that are not ours; they are left alone.
    Foreign(String),
    /// No apps section, or a file this parser does not follow.
    Skipped,
}

/// Sets (`want` is Some) or removes (None) our LaunchOptions for `appid` in the
/// text of a localconfig.vdf.
pub fn edit(text: &str, appid: u32, want: Option<&str>) -> Edit {
    let mut parser = Parser { s: text, i: 0 };
    let Some(root) = parser.entries(true) else { return Edit::Skipped };
    let mut block = &root;
    for key in PATH {
        match child(block, key).map(|e| &e.node) {
            Some(Node::Block(b)) => block = b,
            _ => return Edit::Skipped,
        }
    }
    let apps = block;
    let app = match child(apps, &appid.to_string()).map(|e| &e.node) {
        Some(Node::Block(b)) => Some(b),
        _ => None,
    };

    let changed = |from: usize, to: usize, with: &str| Edit::Changed(format!("{}{with}{}", &text[..from], &text[to..]));
    let Some(app) = app else {
        // The app has no section yet: add one.
        let Some(value) = want else { return Edit::Unchanged };
        let Some((line, indent)) = brace_line(text, apps.close) else { return Edit::Skipped };
        let section = format!(
            "{indent}\t\"{appid}\"\n{indent}\t{{\n{indent}\t\t\"LaunchOptions\"\t\t{}\n{indent}\t}}\n",
            quote(value)
        );
        return changed(line, line, &section);
    };

    match (child(app, "LaunchOptions").map(|e| (e, &e.node)), want) {
        (Some((entry, Node::Value { start, end, text: old })), _) => match want {
            Some(value) if old == value => Edit::Unchanged,
            Some(value) if old.is_empty() || is_ours(old) => changed(*start, *end, &quote(value)),
            Some(_) => Edit::Foreign(old.clone()),
            None if is_ours(old) => {
                let eol = text[*end..].find('\n').map_or(text.len(), |n| *end + n + 1);
                changed(entry.line, eol, "")
            }
            None => Edit::Unchanged,
        },
        (None, Some(value)) => {
            let Some((line, indent)) = brace_line(text, app.close) else { return Edit::Skipped };
            changed(line, line, &format!("{indent}\t\"LaunchOptions\"\t\t{}\n", quote(value)))
        }
        _ => Edit::Unchanged,
    }
}

/// `<bindir>/wpe-bridge-start %command%`, shell-quoted for Steam.
fn launch_value() -> Option<String> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let start = exe.parent()?.join(MARK).to_string_lossy().into_owned();
    let plain = start.chars().all(|c| c.is_ascii_alphanumeric() || "_./-+@:,=~".contains(c));
    let start = if plain { start } else { format!("'{}'", start.replace('\'', "'\\''")) };
    Some(format!("{start} %command%"))
}

fn local_configs(userdata: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = fs::read_dir(userdata)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("config/localconfig.vdf"))
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found
}

/// Sets Wallpaper Engine's launch option in every Steam account's config
/// (`enable`), or takes ours out again. Says so when Steam is running and the
/// change has to wait for it to close.
pub fn apply(enable: bool) {
    let want = if enable {
        match launch_value() {
            Some(v) => Some(v),
            None => return,
        }
    } else {
        None
    };
    let mut pending = Vec::new();
    for file in local_configs(&config::steam_dir().join("userdata")) {
        let Ok(text) = fs::read_to_string(&file) else { continue };
        match edit(&text, APPID, want.as_deref()) {
            Edit::Changed(new) => pending.push((file, new)),
            Edit::Foreign(old) => eprintln!(
                "Steam already has launch options for Wallpaper Engine ({old}); left alone.\nTo start the bridge from Steam, set them to: {MARK} %command%"
            ),
            Edit::Unchanged | Edit::Skipped => {}
        }
    }
    if pending.is_empty() {
        return;
    }
    let what = if enable { "set" } else { "remove" };
    if proc::find("steam").is_some() {
        eprintln!(
            "Steam is running and would overwrite the change when it exits, so Wallpaper Engine's launch option was not {}.\nClose Steam and run wpe-bridge-setup{} again.",
            if enable { "set" } else { "removed" },
            if enable { "" } else { " --remove" }
        );
        return;
    }
    for (file, new) in pending {
        let backup = file.with_extension("vdf.wpe-bridge.bak");
        if !backup.exists() {
            let _ = fs::copy(&file, &backup);
        }
        match fs::write(&file, new) {
            Ok(()) => println!("Steam: {what} Wallpaper Engine's launch option in {}", file.display()),
            Err(e) => eprintln!("cannot write {}: {e}", file.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const START: &str = "/home/u/.local/bin/wpe-bridge-start %command%";

    fn vdf(app: &str) -> String {
        format!(
            "\"UserLocalConfigStore\"\n{{\n\t\"Software\"\n\t{{\n\t\t\"Valve\"\n\t\t{{\n\t\t\t\"Steam\"\n\t\t\t{{\n\t\t\t\t\"apps\"\n\t\t\t\t{{\n\t\t\t\t\t\"409710\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"Playtime\"\t\t\"1950\"\n\t\t\t\t\t}}\n{app}\t\t\t\t}}\n\t\t\t}}\n\t\t}}\n\t}}\n\t\"apptickets\"\n\t{{\n\t\t\"431960\"\t\t\"3800aa\"\n\t}}\n}}\n"
        )
    }

    const APP: &str = "\t\t\t\t\t\"431960\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"Playtime\"\t\t\"7101\"\n\t\t\t\t\t}\n";

    fn changed(e: Edit) -> String {
        match e {
            Edit::Changed(t) => t,
            other => panic!("expected a change, got {other:?}"),
        }
    }

    #[test]
    fn adds_the_option_to_an_existing_app_section() {
        let out = changed(edit(&vdf(APP), 431960, Some(START)));
        let want = APP.replace(
            "\t\t\t\t\t}\n",
            &format!("\t\t\t\t\t\t\"LaunchOptions\"\t\t\"{START}\"\n\t\t\t\t\t}}\n"),
        );
        assert_eq!(out, vdf(&want));
    }

    #[test]
    fn creates_the_app_section_when_missing() {
        let out = changed(edit(&vdf(""), 431960, Some(START)));
        let want = format!(
            "\t\t\t\t\t\"431960\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"{START}\"\n\t\t\t\t\t}}\n"
        );
        assert_eq!(out, vdf(&want));
    }

    #[test]
    fn leaves_the_lookalike_key_in_another_section_alone() {
        let out = changed(edit(&vdf(""), 431960, Some(START)));
        assert!(out.contains("\"431960\"\t\t\"3800aa\""));
    }

    #[test]
    fn is_idempotent() {
        let once = changed(edit(&vdf(APP), 431960, Some(START)));
        assert_eq!(edit(&once, 431960, Some(START)), Edit::Unchanged);
    }

    #[test]
    fn replaces_our_old_value_and_an_empty_one() {
        let old = vdf(&APP.replace("\t\t\t\t\t}\n", "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"/old/bin/wpe-bridge-start %command%\"\n\t\t\t\t\t}\n"));
        let out = changed(edit(&old, 431960, Some(START)));
        assert!(out.contains(&format!("\"LaunchOptions\"\t\t\"{START}\"")));
        assert!(!out.contains("/old/bin"));
        let empty = vdf(&APP.replace("\t\t\t\t\t}\n", "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"\"\n\t\t\t\t\t}\n"));
        assert!(changed(edit(&empty, 431960, Some(START))).contains(START));
    }

    #[test]
    fn keeps_someone_elses_launch_options() {
        let theirs = vdf(&APP.replace("\t\t\t\t\t}\n", "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"mangohud %command%\"\n\t\t\t\t\t}\n"));
        assert_eq!(edit(&theirs, 431960, Some(START)), Edit::Foreign("mangohud %command%".into()));
        assert_eq!(edit(&theirs, 431960, None), Edit::Unchanged);
    }

    #[test]
    fn removal_takes_out_only_our_line() {
        let set = changed(edit(&vdf(APP), 431960, Some(START)));
        assert_eq!(changed(edit(&set, 431960, None)), vdf(APP));
    }

    #[test]
    fn removal_without_the_option_changes_nothing() {
        assert_eq!(edit(&vdf(APP), 431960, None), Edit::Unchanged);
        assert_eq!(edit(&vdf(""), 431960, None), Edit::Unchanged);
    }

    #[test]
    fn escapes_quotes_and_backslashes() {
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        let out = changed(edit(&vdf(APP), 431960, Some("'/p q/wpe-bridge-start' %command%")));
        assert!(out.contains("\"'/p q/wpe-bridge-start' %command%\""));
    }

    #[test]
    fn files_it_does_not_follow_are_skipped() {
        assert_eq!(edit("not vdf", 431960, Some(START)), Edit::Skipped);
        assert_eq!(edit("\"UserLocalConfigStore\"\n{\n}\n", 431960, Some(START)), Edit::Skipped);
        assert_eq!(edit("\"a\" {", 431960, Some(START)), Edit::Skipped);
    }
}
