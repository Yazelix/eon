//! Yu's guided check, after the shared automatic run, and its evidence bundle.

use crate::{
    mac, retina, run,
    scenario::{self, Environment, Scenario},
    step::{Failure, Log, execute, progress, succeed},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, BufRead, Write},
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const MINUTE: Duration = Duration::from_secs(60);

struct Item {
    title: &'static str,
    action: &'static str,
    expected: &'static str,
    slug: &'static str,
}

const ITEMS: &[Item] = &[
    Item {
        title: "Look",
        action: "On a Retina screen (HiDPI monitor or a MacBook's display), look at Eon's window.",
        expected: "text and the Eon Bar are sharp, not blurry, and the bar's CPU and RAM chips show numbers that change.",
        slug: "look",
    },
    Item {
        title: "Second display (answer s to skip if you have only one)",
        action: "Drag the window to the other display and back, then resize it.",
        expected: "text stays sharp on each display and the panes refit with no gaps or garbage.",
        slug: "second-display",
    },
    Item {
        title: "Typing and input methods",
        action: "Type `echo hello` and press Return. Then switch to a Japanese or Chinese input source, compose one word and commit it.",
        expected: "the composition shows at the cursor and the committed word lands in the shell.",
        slug: "typing-and-input-methods",
    },
    Item {
        title: "Fonts",
        action: "Run `cat glyphs.txt`.",
        expected: "Han characters, emoji (one with a skin tone, one flag) and accented letters each show as one glyph, with no empty boxes or overlaps.",
        slug: "fonts",
    },
    Item {
        title: "Keyboard",
        action: "Open Shortcuts with Option+/. Then use the keys it lists to open a tab (it opens in HOME, so its label shows your account name), split a pane, move between panes and open Project with Option+Z.",
        expected: "each key does what Shortcuts says.",
        slug: "keyboard",
    },
    Item {
        title: "Yazi images",
        action: "Open Files with Option+Shift+Y and select `eon.png`.",
        expected: "the picture shows in Yazi's preview.",
        slug: "yazi-images",
    },
    Item {
        title: "Agent",
        action: "In a pane, run `claude` (Claude Code 2.1.295 or later) and ask one short question.",
        expected: "the sidebar marks the agent working, then done when it answers.",
        slug: "agent",
    },
    Item {
        title: "Copy out",
        action: "Select text in Eon with the pointer, press Command+C and paste into another app (for example Notes).",
        expected: "the same text arrives.",
        slug: "copy-out",
    },
];

pub struct Options {
    pub out: Option<PathBuf>,
    pub limit: Duration,
    pub dry_run: bool,
}

pub fn help() -> String {
    let mut text = "\
mac-proof final: Yu's automatic run and guided final check, in one evidence archive

  --out DIR      a fresh evidence directory (default $HOME/eon-mac-final-<rev12>-<UTC date>,
                 with -2, -3 and so on when taken); its name is the archive's top folder
                 and shows in its paths, so choose one without your name
  --timeout DUR  the automatic run's time limit, as 30s, 90m or 12h (default 12h)
  --dry-run      run only the source step and walk the items, on any system, with no captures

Requires Apple Silicon macOS, Screen Recording (window captures) and Accessibility (automatic
keys), granted to your terminal app in System Settings > Privacy & Security. Missing grants
open their settings panes; grant them, quit and reopen the terminal app, then rerun.
On macOS 27 or later, macOS asks once whether your terminal app may \"bypass the system private
window picker and directly access your screen and audio\". The check takes one throwaway screen
capture at the start so the dialog comes then; choose Allow. Screen Recording does not answer
it; left open, it covers Eon in every capture and the translucency check is skipped.
The automatic part captures whole screens, every connected display included, and the desktop
before launch; close or hide windows you would not send before starting. The check asks you to
hide desktop widgets (System Settings > Desktop & Dock), turn on Do Not Disturb (Control Center
> Focus), then press Enter. The end-of-run notification may be silent; the terminal bell still rings.
The archive replaces the home path with ~ and per-account temporary paths with /var/folders/<id>.
It replaces the account name with <user> in logs and text files. The check's own summary.txt and
final.json keep their wording, with your notes and its errors redacted; other JSON files keep
their keys. Names matching status words or hashes stay unchanged, with a terminal warning.
Captures (pixels) can still show names: item 5's tab opens in HOME and its label shows the account
name. Look inside the archive before sending it.
The automatic part takes hours. Leave the Mac awake and untouched until it rings: its GUI
scenario types into Eon. The manual part uses your HOME and environment, with private Eon
configuration and state and the default configuration. Only Eon's largest window is captured,
even when other windows cover it, without changing focus.
During the check, Eon's Starship prompt hides the host and user names. A prompt your own shell
configuration sets replaces Eon's Starship prompt and shows as usual; check the captures for
host or user names before sending. The check keeps Cargo's downloads out of HOME. At the end,
its HOME report offers entries named for Eon's components, CodexBar included, for removal with
your consent; other new entries are counted in the archive and listed by path only on the
terminal. If you already use Nushell, Atuin or Zoxide, the check's commands join their existing
history; the check removes only whole entries it created.
For each item, do the action, press Enter, answer p/f/s (or pass/fail/skip), then add an optional
one-line note. EOF leaves the remaining items unanswered. Ctrl-C stops the tool; a rerun starts
over, and leftovers may need `eon stop all` with the private environment in manual/final.log.

Items:
"
    .to_owned();
    for (index, item) in ITEMS.iter().enumerate() {
        text += &item_text(index, item);
    }
    text += "\nThe archive is <out>.tar.gz beside the folder. Look inside before sending it to Lucca.\n\
Exits 0 when every step of the automatic run passed, every item passed or was skipped, and no\n\
capture, reporting or cleanup errors occurred; 1 otherwise, and 2 when it could not start or\n\
could not write the bundle. A dry run proves no Mac behavior.";
    text
}

fn item_text(index: usize, item: &Item) -> String {
    format!(
        "{}. {}. {}\nExpected: {}\n",
        index + 1,
        item.title,
        item.action,
        item.expected
    )
}

struct Record {
    number: usize,
    title: &'static str,
    answer: &'static str,
    note: String,
    capture: Option<String>,
    seconds: u64,
}

fn records(answer: &'static str, reason: &str) -> Vec<Record> {
    ITEMS
        .iter()
        .enumerate()
        .map(|(index, item)| Record {
            number: index + 1,
            title: item.title,
            answer,
            note: reason.to_owned(),
            capture: None,
            seconds: 0,
        })
        .collect()
}

struct Bundle {
    rev: String,
    tree: String,
    date: String,
    dry_run: bool,
    run_result: &'static str,
    items: Vec<Record>,
    errors: Vec<String>,
}

impl Bundle {
    fn redact(&mut self, redaction: &Redaction) {
        for item in &mut self.items {
            item.note = redaction.text(&item.note, false);
        }
        for error in &mut self.errors {
            *error = redaction.text(error, false);
        }
    }

    fn json(&self) -> Value {
        json!({
            "schema": 1,
            "rev": self.rev,
            "tree": self.tree,
            "date": self.date,
            "dry_run": self.dry_run,
            "run_result": self.run_result,
            "errors": self.errors,
            "items": self.items.iter().map(|item| json!({
                "number": item.number, "title": item.title, "answer": item.answer,
                "note": item.note, "capture": item.capture, "seconds": item.seconds,
            })).collect::<Vec<_>>(),
        })
    }

    fn summary(&self) -> String {
        let mut text = format!(
            "Automatic run: {}{}\n",
            self.run_result,
            if self.dry_run { " (dry run)" } else { "" }
        );
        for item in &self.items {
            text += &format!("{}. {}: {}", item.number, item.title, item.answer);
            if !item.note.is_empty() {
                text += &format!(" — {}", item.note);
            }
            text += "\n";
        }
        for error in &self.errors {
            text += &format!("Manual: {error}\n");
        }
        text
    }

    fn passed(&self) -> bool {
        self.run_result == "pass"
            && self.errors.is_empty()
            && self
                .items
                .iter()
                .all(|item| matches!(item.answer, "pass" | "skip"))
    }
}

fn preflight(dry_run: bool) -> Result<(), String> {
    if dry_run {
        return Ok(());
    }
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return Err("final needs Apple Silicon macOS; use --dry-run elsewhere".to_owned());
    }
    let (screen, keys) = mac::grants()?;
    for (granted, name, purpose, pane) in [
        (
            screen,
            "Screen Recording",
            "capture Eon's window",
            "ScreenCapture",
        ),
        (
            keys,
            "Accessibility",
            "send the automatic scenario's keys",
            "Accessibility",
        ),
    ] {
        if !granted {
            eprintln!("mac-proof: {name} is needed to {purpose}.");
            let _ = Command::new("open")
                .arg(format!(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_{pane}"
                ))
                .stdin(Stdio::null())
                .status();
        }
    }
    if screen && keys {
        Ok(())
    } else {
        Err("grant your terminal app both permissions, quit and reopen it, then rerun".to_owned())
    }
}

/// A fresh private source copy, never included in the evidence.
struct Source(PathBuf);

impl Source {
    fn copy(source: &Path) -> Result<Self, String> {
        for name in ["flake.nix", "Cargo.toml", "eon/tools/mac-proof/Cargo.toml"] {
            if !source.join(name).is_file() {
                return Err("no usable Eon source; use `nix run github:Yazelix/eon/<rev>#mac-proof -- final` or run from an Eon checkout".to_owned());
            }
        }
        let source = fs::canonicalize(source).map_err(|error| format!("source: {error}"))?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("mac-proof-final-{}-{stamp}", std::process::id()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .map_err(|error| error.to_string())?;
        let copy = Source(root);
        for entry in fs::read_dir(&source).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            // Git metadata can point back at a worktree; target holds build artifacts, not source.
            if entry.file_name() == ".git" || entry.file_name() == "target" {
                continue;
            }
            let status = Command::new("cp")
                .arg("-R")
                .arg(entry.path())
                .arg(&copy.0)
                .stdin(Stdio::null())
                .status()
                .map_err(|error| format!("copy source: {error}"))?;
            if !status.success() {
                return Err(format!("copy source: cp exited with {status}"));
            }
        }
        writable(&copy.0).map_err(|error| format!("make source writable: {error}"))?;
        Ok(copy)
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!(
                "mac-proof: remove source copy {}: {error}",
                self.0.display()
            );
        }
    }
}

fn writable(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_symlink() {
        return Ok(());
    }
    let mut permissions = metadata.permissions();
    permissions.set_mode(permissions.mode() | if metadata.is_dir() { 0o700 } else { 0o600 });
    fs::set_permissions(path, permissions)?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            writable(&entry?.path())?;
        }
    }
    Ok(())
}

fn failure(failure: Failure) -> String {
    match failure {
        Failure::Timeout => "timed out".to_owned(),
        Failure::Command(message) | Failure::Check(message) | Failure::Unavailable(message) => {
            message
        }
    }
}

fn archive_path(out: &Path) -> PathBuf {
    let mut name = out.as_os_str().to_owned();
    name.push(".tar.gz");
    PathBuf::from(name)
}

fn archive(out: &Path) -> Result<PathBuf, String> {
    // Archiving must not append unredacted command paths to the evidence.
    let mut log = Log::new(
        File::options()
            .append(true)
            .open("/dev/stderr")
            .map_err(|error| error.to_string())?,
    );
    let archive = archive_path(out);
    succeed(
        &mut log,
        Command::new("tar")
            .arg("--numeric-owner")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(out.parent().ok_or("--out has no parent")?)
            .arg("--")
            .arg(out.file_name().ok_or("--out has no name")?),
        Instant::now() + 10 * MINUTE,
        false,
    )
    .map_err(failure)?;
    fs::set_permissions(&archive, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    Ok(archive)
}

struct Redaction {
    homes: Vec<String>,
    names: Vec<String>,
}

impl Redaction {
    fn current() -> Self {
        let mut homes = Vec::new();
        if let Some(home) = std::env::var_os("HOME") {
            if let Ok(canonical) = fs::canonicalize(&home) {
                homes.push(canonical.to_string_lossy().into_owned());
            }
            homes.push(home.to_string_lossy().into_owned());
        }
        let mut names = Vec::new();
        if let Ok(output) = Command::new("id")
            .arg("-un")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            && output.status.success()
            && let Ok(name) = String::from_utf8(output.stdout)
        {
            names.push(name.trim_end_matches(['\r', '\n']).to_owned());
        }
        for key in ["USER", "LOGNAME"] {
            if let Ok(name) = std::env::var(key) {
                names.push(name);
            }
        }
        Self::new(homes, names)
    }

    fn new(homes: Vec<String>, names: Vec<String>) -> Self {
        let sorted = |values: Vec<String>| {
            let mut values: Vec<_> = values
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            values.sort_by_key(|value| std::cmp::Reverse(value.len()));
            values
        };
        let names = sorted(names).into_iter().filter(|name| {
            let reserved = matches!(name.to_ascii_lowercase().as_str(),
                "pass" | "fail" | "skip" | "unanswered" | "not-run" | "yes" | "no" | "dirty" | "true" | "false" | "null" | "planned" | "skipped" | "timeout" | "left for yu"
            ) || (name.len() >= 7 && name.bytes().all(|byte| byte.is_ascii_hexdigit()));
            if reserved {
                progress(&format!("mac-proof: account name {name:?} matches a status word or hash; it may appear in the archive"));
            }
            !reserved
        }).collect();
        Self {
            homes: sorted(homes),
            names,
        }
    }

    fn text(&self, text: &str, path_only: bool) -> String {
        let mut result = String::with_capacity(text.len());
        let mut rest = text;
        let word_boundary =
            |byte: u8| !byte.is_ascii_alphanumeric() && !matches!(byte, b'_' | b'-');
        let path_boundary = |ch: char| {
            ch == '/'
                || ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\'' | '\\' | '`' | ')' | ']' | '}' | ',' | ';' | ':'
                )
        };
        let temp_character = |ch: char| {
            ch != '/'
                && !ch.is_whitespace()
                && !matches!(ch, '"' | '\'')
                && (!path_only || ch != '\\')
        };
        while !rest.is_empty() {
            if let Some(home) = self.homes.iter().find(|home| {
                rest.starts_with(home.as_str())
                    && rest[home.len()..].chars().next().is_none_or(path_boundary)
            }) {
                result.push('~');
                rest = &rest[home.len()..];
                continue;
            }
            let temporary = ["/private/var/folders/", "/var/folders/"]
                .into_iter()
                .find_map(|prefix| {
                    let tail = rest.strip_prefix(prefix)?;
                    let first = tail.find('/')?;
                    if first == 0
                        || &tail[..first] == "<id>"
                        || !tail[..first].chars().all(temp_character)
                    {
                        return None;
                    }
                    let tail = &tail[first + 1..];
                    let second = tail.find(|ch| !temp_character(ch)).unwrap_or(tail.len());
                    (second > 0).then_some(prefix.len() + first + 1 + second)
                });
            if let Some(length) = temporary {
                result.push_str("/var/folders/<id>");
                rest = &rest[length..];
                continue;
            }
            let before = text.as_bytes()[..text.len() - rest.len()].last().copied();
            if !path_only
                && let Some(name) = self.names.iter().find(|name| {
                    rest.starts_with(name.as_str())
                        && before.is_none_or(word_boundary)
                        && rest
                            .as_bytes()
                            .get(name.len())
                            .copied()
                            .is_none_or(word_boundary)
                })
            {
                result.push_str("<user>");
                rest = &rest[name.len()..];
                continue;
            }
            let ch = rest.chars().next().expect("nonempty text");
            result.push(ch);
            rest = &rest[ch.len_utf8()..];
        }
        result
    }

    fn value(&self, value: &mut Value) -> bool {
        let mut changed = false;
        match value {
            Value::String(text) => {
                let redacted = self.text(text, false);
                if redacted != *text {
                    *text = redacted;
                    changed = true;
                }
            }
            Value::Array(values) => {
                for value in values {
                    changed |= self.value(value);
                }
            }
            Value::Object(values) => {
                for value in values.values_mut() {
                    changed |= self.value(value);
                }
            }
            _ => {}
        }
        changed
    }

    fn rewrite(&self, out: &Path) -> io::Result<()> {
        self.rewrite_path(out, out)
    }

    fn rewrite_path(&self, out: &Path, path: &Path) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.is_symlink() {
            return Ok(());
        }
        if metadata.is_dir() {
            for entry in fs::read_dir(path)? {
                self.rewrite_path(out, &entry?.path())?;
            }
        } else if metadata.is_file()
            && let Ok(text) = String::from_utf8(fs::read(path)?)
        {
            let redacted = if path == out.join("summary.txt") || path == out.join("final.json") {
                self.text(&text, true)
            } else if path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".json"))
                && let Ok(mut value) = serde_json::from_str::<Value>(&text)
            {
                if !self.value(&mut value) {
                    return Ok(());
                }
                serde_json::to_string_pretty(&value).expect("a JSON value serializes") + "\n"
            } else {
                self.text(&text, false)
            };
            if redacted != text {
                fs::write(path, redacted)?;
                fs::set_permissions(path, metadata.permissions())?;
            }
        }
        Ok(())
    }
}

fn discard_screen_capture(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn taken(path: &Path) -> bool {
    path.symlink_metadata().is_ok() || archive_path(path).symlink_metadata().is_ok()
}

fn default_out(home: &Path, rev: &str, date: &str) -> PathBuf {
    let name = format!("eon-mac-final-{rev}-{date}");
    let mut path = home.join(&name);
    let mut suffix = 2;
    while taken(&path) {
        path = home.join(format!("{name}-{suffix}"));
        suffix += 1;
    }
    path
}

type HomeSnapshot = BTreeMap<&'static str, io::Result<BTreeSet<PathBuf>>>;

fn home_snapshot(home: &Path, out: &Path) -> HomeSnapshot {
    let archive = archive_path(out);
    [
        "",
        ".config",
        ".local/share",
        ".local/state",
        ".cache",
        "Library/Application Support",
        "Library/Caches",
        "Library/Preferences",
        "Library/Saved Application State",
    ]
    .into_iter()
    .map(|directory| {
        let entries = match fs::canonicalize(home.join(directory)).and_then(fs::read_dir) {
            Ok(entries) => entries
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<io::Result<BTreeSet<_>>>()
                .map(|entries| {
                    entries
                        .into_iter()
                        .filter(|path| path != out && *path != archive)
                        .collect()
                }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(BTreeSet::new()),
            Err(error) => Err(error),
        };
        (directory, entries)
    })
    .collect()
}

fn check_home(
    home: &Path,
    out: &Path,
    before: &HomeSnapshot,
    names: &[String],
    input: &mut impl BufRead,
    errors: &mut Vec<String>,
) -> String {
    let after = home_snapshot(home, out);
    let mut new = BTreeSet::new();
    for (&directory, before) in before {
        match (before, &after[directory]) {
            (Ok(before), Ok(after)) => new.extend(after.difference(before)),
            (Err(error), _) | (_, Err(error)) => errors.push(format!(
                "HOME: cannot list {}: {error}",
                if directory.is_empty() { "." } else { directory }
            )),
        }
    }
    let (tools, other): (Vec<_>, Vec<_>) = new.into_iter().partition(|path| {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let name = name.strip_prefix('.').unwrap_or(&name);
        std::iter::once("eon")
            .chain(names.iter().map(String::as_str))
            .any(|id| {
                name == id
                    || name
                        .strip_prefix(id)
                        .is_some_and(|suffix| suffix.starts_with(['.', '-', '_']))
            })
            || name.contains("yazelix")
    });
    for path in &other {
        progress(&format!("HOME: other new entry: {}", path.display()));
    }
    let mut report = format!(
        "HOME: {} other new entries, listed on the terminal only\n",
        other.len()
    );
    if tools.is_empty() {
        return report;
    }
    for path in &tools {
        progress(&format!("HOME: entry from Eon's tools: {}", path.display()));
    }
    let yes = line(
        input,
        &format!(
            "Remove these {} entries the check created in your HOME? [y/N]",
            tools.len()
        ),
    )
    .is_some_and(|answer| matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes"));
    if !yes {
        report += &format!(
            "HOME: {} entries from Eon's tools, kept (answer: no)\n",
            tools.len()
        );
        return report;
    }
    let mut removed = 0;
    for path in &tools {
        let result = fs::symlink_metadata(path).and_then(|metadata| {
            if metadata.is_dir() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            }
        });
        match result {
            Ok(()) => removed += 1,
            Err(error) => errors.push(format!("HOME: remove entry: {error}")),
        }
    }
    report += &format!(
        "HOME: {} entries from Eon's tools, {removed} removed (answer: yes)\n",
        tools.len()
    );
    report
}

fn component_names(source: &Path) -> Result<Vec<String>, String> {
    let read = || -> Result<Vec<String>, String> {
        let bytes =
            fs::read(source.join("components/eon-v5.json")).map_err(|error| error.to_string())?;
        let manifest: Value = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        manifest["components"]
            .as_array()
            .ok_or("missing components array")?
            .iter()
            .map(|component| {
                component["id"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .map(str::to_ascii_lowercase)
                    .ok_or_else(|| "missing component id".to_owned())
            })
            .collect()
    };
    read().map_err(|error| format!("HOME component manifest: {error}"))
}

/// Gregorian civil date in UTC, counting whole days since the Unix epoch.
fn civil_date(seconds: u64) -> String {
    let days = (seconds / 86400) as i64 + 719468;
    let era = days / 146097;
    let day = days % 146097;
    let year = (day - day / 1460 + day / 36524 - day / 146096) / 365;
    let day_of_year = day - (365 * year + year / 4 - year / 100);
    let month = (5 * day_of_year + 2) / 153;
    let date = day_of_year - (153 * month + 2) / 5 + 1;
    let month = month + if month < 10 { 3 } else { -9 };
    let year = year + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{date:02}")
}

struct Awake(Option<Child>);

impl Awake {
    fn start(log: &mut Log) -> Self {
        let spawn = (|| {
            Command::new("caffeinate")
                .args(["-d", "-i", "-s", "-w", &std::process::id().to_string()])
                .stdin(Stdio::null())
                .stdout(log.stdio()?)
                .stderr(log.stdio()?)
                .spawn()
        })();
        match spawn {
            Ok(child) => Awake(Some(child)),
            Err(error) => {
                progress(&format!(
                    "mac-proof: caffeinate unavailable: {error}; keep the Mac awake"
                ));
                log.line(&format!("caffeinate: {error}"));
                Awake(None)
            }
        }
    }

    fn check(&mut self, log: &mut Log) {
        if let Some(child) = &mut self.0 {
            match child.try_wait() {
                Ok(None) => {}
                result => {
                    progress("mac-proof: caffeinate ended; keep the Mac awake");
                    log.line(&format!("caffeinate: {result:?}"));
                }
            }
        }
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn ring(log: &mut Log) {
    progress("\x07The automatic part is over. Yu's manual check starts now.");
    if let Err(error) = succeed(
        log,
        Command::new("osascript").args([
            "-e",
            "display notification \"The manual check is ready.\" with title \"Eon mac-proof\"",
        ]),
        Instant::now() + MINUTE,
        false,
    ) {
        log.line(&format!("notification: {}", failure(error)));
    }
}

fn line(input: &mut impl BufRead, prompt: &str) -> Option<String> {
    progress(prompt);
    let _ = io::stdout().flush();
    let mut text = String::new();
    match input.read_line(&mut text) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(text.trim_end_matches(['\r', '\n']).to_owned()),
    }
}

fn parse_answer(text: &str) -> Option<&'static str> {
    match text.trim().to_ascii_lowercase().as_str() {
        "p" | "pass" => Some("pass"),
        "f" | "fail" => Some("fail"),
        "s" | "skip" => Some("skip"),
        _ => None,
    }
}

fn answer(input: &mut impl BufRead) -> Option<&'static str> {
    loop {
        let text = line(input, "[p]ass, [f]ail or [s]kip?")?;
        if let Some(answer) = parse_answer(&text) {
            return Some(answer);
        }
        progress("Please answer p, f, s, pass, fail or skip.");
    }
}

fn capture(
    scenario: &mut Scenario,
    index: usize,
    out: &Path,
    log: &mut Log,
) -> Result<String, String> {
    let deadline = Instant::now() + MINUTE;
    let venus = match scenario.venus(log, deadline).map_err(failure)? {
        Some(venus) => venus,
        None => {
            progress("Venus is gone; re-attaching Eon once.");
            scenario.reattach(log, deadline).map_err(failure)?
        }
    };
    let (windows, _) = mac::windows(venus)?;
    let window = windows
        .into_iter()
        .filter(|window| {
            window.bounds.iter().all(|n| n.is_finite())
                && window.bounds[2] > 0.0
                && window.bounds[3] > 0.0
        })
        .max_by(|a, b| (a.bounds[2] * a.bounds[3]).total_cmp(&(b.bounds[2] * b.bounds[3])))
        .ok_or("no Eon window to capture")?;
    let name = format!("manual/{:02}-{}.png", index + 1, ITEMS[index].slug);
    let path = out.join(&name);
    let result = succeed(
        log,
        Command::new("screencapture")
            .args(["-x", "-o", "-l", &window.id.to_string()])
            .arg(&path),
        deadline,
        false,
    );
    if let Err(error) = result {
        let _ = fs::remove_file(path);
        return Err(format!("capture: {}", failure(error)));
    }
    if !path.is_file() {
        return Err("screencapture wrote no file".to_owned());
    }
    Ok(name)
}

fn walk(
    input: &mut impl BufRead,
    mut capture: impl FnMut(usize) -> Result<Option<String>, String>,
    errors: &mut Vec<String>,
) -> (Vec<Record>, bool) {
    let mut items = records("unanswered", "stdin closed or at EOF");
    let mut closed = false;
    for (index, record) in items.iter_mut().enumerate() {
        let started = Instant::now();
        progress(&item_text(index, &ITEMS[index]));
        if line(
            input,
            "Do this item, then press Enter. Only Eon's window is captured.",
        )
        .is_none()
        {
            record.seconds = started.elapsed().as_secs();
            closed = true;
            break;
        }
        record.note.clear();
        match capture(index) {
            Ok(path) => record.capture = path,
            Err(reason) => {
                progress(&format!(
                    "mac-proof: {reason}; continuing without a capture"
                ));
                errors.push(format!(
                    "item {} ({}): capture: {reason}",
                    record.number, record.title
                ));
                record.note = reason;
            }
        }
        let Some(answer) = answer(input) else {
            closed = true;
            if !record.note.is_empty() {
                record.note += "; ";
            }
            record.note += "stdin closed or at EOF";
            record.seconds = started.elapsed().as_secs();
            break;
        };
        record.answer = answer;
        let note = line(input, "Optional one-line note (Enter for none):");
        if let Some(note) = &note
            && !note.is_empty()
        {
            if !record.note.is_empty() {
                record.note += "; ";
            }
            record.note += note;
        }
        record.seconds = started.elapsed().as_secs();
        if note.is_none() {
            closed = true;
            break;
        }
    }
    (items, closed)
}

fn manual(
    package: PathBuf,
    source: &Path,
    out: &Path,
    bundle: &mut Bundle,
    log: &mut Log,
    input: &mut impl BufRead,
) {
    let mut scenario = Scenario::new(package, out.join("manual"), Environment::User, true);
    let launched = (|| {
        scenario.prepare()?;
        fs::write(
            scenario.work().join("glyphs.txt"),
            "漢字テスト 中文\n👍🏽 🇯🇵\ne\u{0301} a\u{0308}\n",
        )
        .and_then(|_| {
            fs::copy(
                source.join("assets/eon.png"),
                scenario.work().join("eon.png"),
            )
            .map(drop)
        })
        .map_err(|error| Failure::Check(format!("manual files: {error}")))?;
        scenario.start(&["run"], log)?;
        let deadline = Instant::now() + 5 * MINUTE;
        scenario.wait_for_workspace(log, deadline)?;
        scenario.wait_for_venus(log, deadline)?;
        Ok(())
    })();
    match launched {
        Ok(()) => {
            progress(
                "Eon is open with its default configuration. Dismiss its directory picker with Escape to use the prepared launch directory.",
            );
            let (items, closed) = walk(
                input,
                |index| capture(&mut scenario, index, out, log).map(Some),
                &mut bundle.errors,
            );
            bundle.items = items;
            if closed {
                bundle.errors.push("stdin closed or at EOF".to_owned());
            }
        }
        Err(error) => {
            let reason = format!("Eon failed to launch: {}", failure(error));
            log.line(&reason);
            progress(&reason);
            bundle.items = records("not-run", &reason);
        }
    }
    let versions = succeed(
        log,
        &mut scenario.eon(&["versions"]),
        Instant::now() + MINUTE,
        true,
    )
    .map_err(failure)
    .and_then(|bytes| {
        fs::write(out.join("versions.txt"), bytes).map_err(|error| error.to_string())
    });
    if let Err(error) = versions {
        bundle.errors.push(format!("versions: {error}"));
    }
    let displays = mac::displays()
        .map(|displays| retina::report(&displays))
        .and_then(|report| {
            fs::write(out.join("displays.json"), report.to_string())
                .map_err(|error| error.to_string())
        });
    if let Err(error) = displays {
        bundle.errors.push(format!("displays: {error}"));
    }
    if let Err(error) = scenario::stop(&mut scenario, log, Instant::now() + 2 * MINUTE) {
        let reason = format!("stop: {}", failure(error));
        log.line(&reason);
        bundle.errors.push(reason);
        scenario.clean(log);
    }
}

pub fn final_check(options: Options) -> Result<bool, String> {
    preflight(options.dry_run)?;
    if let Some(out) = &options.out
        && taken(out)
    {
        return Err(format!(
            "{} or its archive exists; give a fresh --out",
            out.display()
        ));
    }
    let source = std::env::var_os("MAC_PROOF_SOURCE")
        .map(PathBuf::from)
        .map(Ok)
        .unwrap_or_else(std::env::current_dir)
        .map_err(|error| error.to_string())?;
    let source = Source::copy(&source)?;
    let names = component_names(&source.0);
    if let Err(error) = &names {
        progress(error);
    }
    // The real source step records this identity in run/; preparation must not add a file to it.
    let mut source_log = Log::new(
        File::options()
            .write(true)
            .open("/dev/null")
            .map_err(|error| error.to_string())?,
    );
    let tree = run::source_tree(&source.0, &mut source_log, Instant::now() + 10 * MINUTE)
        .map_err(failure)?;
    let rev = match std::env::var("MAC_PROOF_REV") {
        Ok(rev) if !rev.is_empty() && rev != "dirty" => {
            if !rev.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("MAC_PROOF_REV must be a Git revision hash or dirty".to_owned());
            }
            rev.chars().take(12).collect()
        }
        _ => format!("tree-{}", tree.chars().take(12).collect::<String>()),
    };
    let date = civil_date(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_secs(),
    );
    let out = match options.out {
        Some(out) => out,
        None => default_out(
            &PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unset; give --out")?),
            &rev,
            &date,
        ),
    };
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&out)
        .map_err(|error| format!("{}: {error}", out.display()))?;
    let out = fs::canonicalize(out).map_err(|error| error.to_string())?;
    fs::create_dir(out.join("manual")).map_err(|error| error.to_string())?;
    let mut log = Log::new(
        File::options()
            .create_new(true)
            .append(true)
            .open(out.join("manual/final.log"))
            .map_err(|error| error.to_string())?,
    );
    fs::write(out.join("versions.txt"), "not run\n").map_err(|error| error.to_string())?;
    fs::write(out.join("displays.json"), "null\n").map_err(|error| error.to_string())?;
    let home = if options.dry_run {
        None
    } else {
        let home = PathBuf::from(std::env::var_os("HOME").ok_or("HOME is unset")?);
        let home = fs::canonicalize(&home).unwrap_or(home);
        let before = home_snapshot(&home, &out);
        Some((home, before))
    };
    let screen_access = out.join("screen-access.png");
    let redaction = Redaction::current();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    if !options.dry_run {
        progress(
            "If macOS asks whether your terminal app may bypass the system private window picker and access your screen, choose Allow: left open, that dialog covers Eon in every capture.",
        );
        let result = match execute(
            &mut log,
            Command::new("screencapture").arg("-x").arg(&screen_access),
            Instant::now() + MINUTE,
            false,
            true,
        ) {
            Ok((status, _)) => status.to_string(),
            Err(error) => failure(error),
        };
        let present = screen_access.is_file();
        let cleanup = discard_screen_capture(&screen_access);
        log.line(&format!(
            "screen-access capture: {result}; file present: {present}; cleanup: {cleanup:?}"
        ));
        line(
            &mut input,
            "Hide desktop widgets (System Settings > Desktop & Dock) and turn on Do Not Disturb (Control Center > Focus): whole-screen captures include them. Press Enter to continue.",
        );
    }
    progress(
        "The automatic run takes hours. Keep the Mac awake and untouched until it rings. The GUI scenario types into Eon.",
    );
    let mut awake = if options.dry_run {
        Awake(None)
    } else {
        Awake::start(&mut log)
    };
    let report = run::run(
        run::Options {
            evidence: out.join("run"),
            expect_tree: Some(tree.clone()),
            limit: options.limit,
            dry_run: options.dry_run,
            personal_mac: true,
        },
        &source.0,
    )?;
    awake.check(&mut log);
    let mut bundle = Bundle {
        rev,
        tree: if report.tree.is_empty() {
            tree
        } else {
            report.tree
        },
        date,
        dry_run: options.dry_run,
        run_result: if !report.passed {
            "fail"
        } else if report.left_for_yu > 0 {
            "left for Yu"
        } else {
            "pass"
        },
        items: Vec::new(),
        errors: Vec::new(),
    };
    if options.dry_run {
        let (items, closed) = walk(&mut input, |_| Ok(None), &mut bundle.errors);
        bundle.items = items;
        if closed {
            bundle.errors.push("stdin closed or at EOF".to_owned());
        }
    } else {
        match report.package {
            Ok(package) => {
                ring(&mut log);
                manual(package, &source.0, &out, &mut bundle, &mut log, &mut input);
            }
            Err(reason) => bundle.items = records("not-run", &reason),
        }
    }
    let home_report = home.map_or_else(String::new, |(home, before)| {
        check_home(
            &home,
            &out,
            &before,
            names.as_deref().unwrap_or(&[]),
            &mut input,
            &mut bundle.errors,
        )
    });
    if let Err(error) = names {
        bundle.errors.push(error);
    }
    for line in home_report.lines() {
        log.line(line);
    }
    bundle.redact(&redaction);
    fs::write(out.join("summary.txt"), bundle.summary() + &home_report)
        .map_err(|error| error.to_string())?;
    fs::write(
        out.join("final.json"),
        serde_json::to_string_pretty(&bundle.json()).expect("a JSON value serializes") + "\n",
    )
    .map_err(|error| error.to_string())?;
    // A timed-out screencapture may write its file late.
    discard_screen_capture(&screen_access)
        .map_err(|error| format!("remove throwaway screen capture: {error}"))?;
    drop(log);
    redaction
        .rewrite(&out)
        .map_err(|error| format!("redact evidence: {error}"))?;
    let archive = archive(&out)?;
    progress(&format!(
        "Archive: {}\nYu, you may look inside before sending it to Lucca.",
        archive.display()
    ));
    Ok(bundle.passed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_redaction_preserves_boundaries_binary_files_and_json_facts() {
        let root = std::env::temp_dir().join(format!("mac-proof-redaction-{}", std::process::id()));
        let out = root.join("evidence");
        fs::create_dir_all(out.join("run/nested")).unwrap();
        let original = "path=/Users/yu/log\nhome=\"/Users/yu\" raw=/raw/yu/file\nUSER=\"yu\" LOGNAME='yu.long'\nyu\n/Users/yux yus rename_yu yu_1\nodd=yu+friend\nnot-run yu-bundle bundle-yu run yu rev\n/var/folders/ab/account-id/T/log /private/var/folders/cd/account…\n";
        let expected = "path=~/log\nhome=\"~\" raw=~/file\nUSER=\"<user>\" LOGNAME='<user>'\n<user>\n/Users/yux yus rename_yu yu_1\nodd=<user>\nnot-run yu-bundle bundle-yu <user> <user> <user>\n/var/folders/<id>/T/log /var/folders/<id>\n";
        let text = out.join("run/nested/session.log");
        fs::write(&text, original).unwrap();
        fs::set_permissions(&text, fs::Permissions::from_mode(0o640)).unwrap();
        let binary = b"\xffyu /Users/yu\0";
        fs::write(out.join("capture.png"), binary).unwrap();
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("secret"), original).unwrap();
        std::os::unix::fs::symlink(&outside, out.join("linked-directory")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret"), out.join("linked-file")).unwrap();
        let facts = json!({
            "statuses": ["pass", "fail", "skip", "unanswered", "not-run", "yes", "no", "dirty", "true", "false", "null", "planned", "skipped", "timeout"],
            "rev": "abcdef0123456789", "tree": "abcdef0", "dry_run": true, "capture": null, "yu": "yu",
            "generated": "Automatic run: left for Yu",
        });
        for name in ["final.json", "run/summary.json", "run/final.json"] {
            let mut fixture = facts.clone();
            fixture["first_failure"] = json!("yu: /Users/yu/failure");
            fixture["details"] = json!([{"yu": "yu /Users/yu/detail"}, "yu /raw/yu/second"]);
            fixture["note"] = json!(
                "yu /Users/yu /raw/yu /private/var/folders/ab/full-id/T /var/folders/ab/ellips… \"/var/folders/ef/quoted\""
            );
            fs::write(out.join(name), fixture.to_string()).unwrap();
            fs::set_permissions(out.join(name), fs::Permissions::from_mode(0o640)).unwrap();
        }
        let summary = "Automatic run: left for Yu\nmac-proof pass: 90 steps, 0 not passed\nyu: /Users/yu /private/var/folders/ab/account-id/T\n";
        let expected_summary = "Automatic run: left for Yu\nmac-proof pass: 90 steps, 0 not passed\nyu: ~ /var/folders/<id>/T\n";
        for name in ["summary.txt", "run/summary.txt"] {
            fs::write(out.join(name), summary).unwrap();
        }
        fs::write(out.join("run/broken.json"), "broken { yu /Users/yu/file\n").unwrap();
        let unchanged = r#"{"rev":"abcdef0123456789","status":"pass","ok":true}"#;
        fs::write(out.join("run/unchanged.json"), unchanged).unwrap();
        fs::set_permissions(
            out.join("run/unchanged.json"),
            fs::Permissions::from_mode(0o400),
        )
        .unwrap();
        let mut names = vec![
            "yu",
            "yu.long",
            "yu+friend",
            "run",
            "rev",
            "for",
            "steps",
            "not",
            "yu",
            "",
            "abcdef0",
            "abcdef0123456789",
        ];
        names.extend(
            facts["statuses"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap()),
        );
        let redaction = Redaction::new(
            vec!["/Users/yu".to_owned(), "/raw/yu".to_owned()],
            names.into_iter().map(str::to_owned).collect(),
        );
        redaction.rewrite(&out).unwrap();
        assert_eq!(fs::read_to_string(&text).unwrap(), expected);
        assert_eq!(
            fs::metadata(&text).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(fs::read(out.join("capture.png")).unwrap(), binary);
        assert_eq!(
            fs::read_to_string(outside.join("secret")).unwrap(),
            original
        );
        for name in ["final.json", "run/summary.json", "run/final.json"] {
            let text = fs::read_to_string(out.join(name)).unwrap();
            let fixture: Value = serde_json::from_str(&text).unwrap();
            let mut expected = facts.clone();
            if name == "final.json" {
                expected["note"] =
                    json!("yu ~ ~ /var/folders/<id>/T /var/folders/<id> \"/var/folders/<id>\"");
                expected["first_failure"] = json!("yu: ~/failure");
                expected["details"] = json!([{"yu": "yu ~/detail"}, "yu ~/second"]);
            } else {
                expected["yu"] = json!("<user>");
                expected["generated"] = json!("Automatic <user>: left <user> Yu");
                expected["note"] =
                    json!("<user> ~ ~ /var/folders/<id>/T /var/folders/<id> \"/var/folders/<id>\"");
                expected["first_failure"] = json!("<user>: ~/failure");
                expected["details"] = json!([{"yu": "<user> ~/detail"}, "<user> ~/second"]);
            }
            assert_eq!(fixture, expected, "{name}");
            if name != "final.json" {
                assert!(text.starts_with("{\n") && text.ends_with('\n'));
            }
            assert_eq!(
                fs::metadata(out.join(name)).unwrap().permissions().mode() & 0o777,
                0o640
            );
        }
        assert_eq!(
            fs::read_to_string(out.join("summary.txt")).unwrap(),
            expected_summary
        );
        assert_eq!(
            fs::read_to_string(out.join("run/summary.txt")).unwrap(),
            "Automatic <user>: left <user> Yu\nmac-proof pass: 90 <user>, 0 <user> passed\n<user>: ~ /var/folders/<id>/T\n"
        );
        assert_eq!(
            fs::read_to_string(out.join("run/broken.json")).unwrap(),
            "broken { <user> ~/file\n"
        );
        assert_eq!(
            fs::read_to_string(out.join("run/unchanged.json")).unwrap(),
            unchanged
        );
        assert_eq!(
            fs::metadata(out.join("run/unchanged.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o400
        );
        assert!(redaction.rewrite(&out.join("missing")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundle_redaction_changes_notes_and_errors_without_changing_generated_fields() {
        let out =
            std::env::temp_dir().join(format!("mac-proof-bundle-redaction-{}", std::process::id()));
        fs::create_dir(&out).unwrap();
        let mut bundle = Bundle {
            rev: "abcdef012345".to_owned(),
            tree: "abcdef0123456789".to_owned(),
            date: "2026-10-29".to_owned(),
            dry_run: false,
            run_result: "left for Yu",
            items: records(
                "not-run",
                "yu /Users/yu/note /var/folders/ab/account-id/T/note",
            ),
            errors: vec![
                "yu: /Users/yu/error /private/var/folders/cd/account-id/T/error".to_owned(),
            ],
        };
        bundle.items.truncate(1);
        bundle.items[0].title = "yu for steps not";
        let redaction = Redaction::new(
            vec!["/Users/yu".to_owned()],
            ["yu", "for", "steps", "not"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        );
        bundle.redact(&redaction);
        fs::write(out.join("final.json"), bundle.json().to_string()).unwrap();
        fs::write(out.join("summary.txt"), bundle.summary()).unwrap();
        redaction.rewrite(&out).unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(out.join("final.json")).unwrap()).unwrap();
        assert_eq!(
            manifest["items"][0]["note"],
            "<user> ~/note /var/folders/<id>/T/note"
        );
        assert_eq!(
            manifest["errors"],
            json!(["<user>: ~/error /var/folders/<id>/T/error"])
        );
        assert_eq!(manifest["run_result"], "left for Yu");
        assert_eq!(manifest["items"][0]["answer"], "not-run");
        assert_eq!(manifest["items"][0]["title"], "yu for steps not");
        assert_eq!(manifest["rev"], "abcdef012345");
        assert_eq!(manifest["tree"], "abcdef0123456789");
        assert_eq!(manifest["date"], "2026-10-29");
        assert_eq!(
            fs::read_to_string(out.join("summary.txt")).unwrap(),
            "Automatic run: left for Yu\n1. yu for steps not: not-run — <user> ~/note /var/folders/<id>/T/note\nManual: <user>: ~/error /var/folders/<id>/T/error\n"
        );
        fs::remove_dir_all(out).unwrap();
    }

    #[test]
    fn archives_omit_owner_and_group_names() {
        let root = std::env::temp_dir().join(format!("mac-proof-archive-{}", std::process::id()));
        let out = root.join("evidence");
        fs::create_dir_all(out.join("nested")).unwrap();
        fs::write(out.join("nested/log"), "evidence\n").unwrap();
        let archive = archive(&out).unwrap();
        let output = Command::new("gzip")
            .arg("-dc")
            .arg(&archive)
            .output()
            .unwrap();
        assert!(output.status.success());
        let mut offset = 0;
        let mut headers = 0;
        while offset + 512 <= output.stdout.len() {
            let header = &output.stdout[offset..offset + 512];
            if header.iter().all(|byte| *byte == 0) {
                break;
            }
            assert!(
                header[265..329].iter().all(|byte| *byte == 0),
                "owner/group: {:?}",
                &header[265..329]
            );
            let size = std::str::from_utf8(&header[124..136])
                .unwrap()
                .trim_matches(['\0', ' ']);
            let size = usize::from_str_radix(size, 8).unwrap();
            offset += 512 + size.div_ceil(512) * 512;
            headers += 1;
        }
        assert!(headers >= 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn throwaway_screen_capture_cleanup_handles_missing_and_late_files() {
        let out =
            std::env::temp_dir().join(format!("mac-proof-screen-access-{}", std::process::id()));
        fs::create_dir(&out).unwrap();
        let path = out.join("screen-access.png");
        discard_screen_capture(&path).unwrap();
        for content in ["capture", "late capture"] {
            fs::write(&path, content).unwrap();
            discard_screen_capture(&path).unwrap();
            assert!(!path.exists());
        }
        fs::create_dir(&path).unwrap();
        let error = discard_screen_capture(&path);
        fs::remove_dir_all(out).unwrap();
        assert!(error.is_err());
    }

    #[test]
    fn home_cleanup_needs_consent_and_never_reports_entry_names() {
        for (index, (answer, remove)) in [
            (" yEs \n", true),
            ("y\n", true),
            ("no\n", false),
            ("\n", false),
            ("", false),
            ("a private answer\n", false),
        ]
        .into_iter()
        .enumerate()
        {
            let root =
                std::env::temp_dir().join(format!("mac-proof-home-{}-{index}", std::process::id()));
            let home = root.join("home");
            let existing = home.join(".config/NuShell");
            fs::create_dir_all(&existing).unwrap();
            fs::create_dir_all(home.join("Library/Application Support")).unwrap();
            fs::create_dir(root.join("components")).unwrap();
            fs::write(
                root.join("components/eon-v5.json"),
                r#"{"components":[{"id":"atuin"},{"id":"codexbar"},{"id":"bash"},{"id":"helix"}]}"#,
            )
            .unwrap();
            let names = component_names(&root).unwrap();
            fs::write(existing.join("history.txt"), "keep history").unwrap();
            let cache = root.join("cache");
            fs::create_dir(&cache).unwrap();
            std::os::unix::fs::symlink(&cache, home.join(".cache")).unwrap();
            let out = home.join(".cache/yazelix-evidence");
            fs::create_dir(&out).unwrap();
            let out = fs::canonicalize(out).unwrap();
            let before = home_snapshot(&home, &out);
            assert!(
                before
                    .values()
                    .all(|entries| !entries.as_ref().unwrap().contains(&out))
            );
            let tool = home.join(".config/AtUiN");
            fs::create_dir(&tool).unwrap();
            fs::write(tool.join("history"), "new history").unwrap();
            let target = root.join("keep-target");
            fs::create_dir(&target).unwrap();
            fs::write(target.join("kept"), "keep target").unwrap();
            let link = home.join(".cache/my-YaZeLiX-link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            let other = home.join("other-private-name");
            fs::write(&other, "keep other").unwrap();
            let notes = home.join("my-notes");
            fs::write(&notes, "keep notes").unwrap();
            let cargo = home.join(".cargo");
            fs::create_dir(&cargo).unwrap();
            let components = [
                ".codexbar",
                ".config/codexbar",
                ".bash_history",
                "Library/Application Support/helix",
            ];
            for name in components {
                fs::write(home.join(name), "created by component").unwrap();
            }
            fs::write(archive_path(&out), "keep archive").unwrap();
            let mut errors = Vec::new();
            let report = check_home(
                &home,
                &out,
                &before,
                &names,
                &mut answer.as_bytes(),
                &mut errors,
            );
            let log_path = root.join("final.log");
            let mut log = Log::new(File::create(&log_path).unwrap());
            log.line(&report);
            assert!(errors.is_empty());
            assert_eq!(tool.exists(), !remove);
            assert_eq!(link.symlink_metadata().is_ok(), !remove);
            for name in components {
                assert_eq!(home.join(name).exists(), !remove);
            }
            assert!(target.join("kept").exists());
            assert_eq!(
                fs::read_to_string(existing.join("history.txt")).unwrap(),
                "keep history"
            );
            assert!(
                other.exists()
                    && notes.exists()
                    && cargo.exists()
                    && out.exists()
                    && archive_path(&out).exists()
            );
            assert_eq!(
                report,
                format!(
                    "HOME: 3 other new entries, listed on the terminal only\nHOME: 6 entries from Eon's tools, {}\n",
                    if remove {
                        "6 removed (answer: yes)"
                    } else {
                        "kept (answer: no)"
                    }
                )
            );
            assert_eq!(fs::read_to_string(log_path).unwrap(), format!("{report}\n"));
            assert!(
                !report.contains("private")
                    && !report.contains("AtUiN")
                    && !report.contains("YaZeLiX")
            );
            let before = home_snapshot(&home, &out);
            let mut untouched = &b"yes\n"[..];
            check_home(&home, &out, &before, &names, &mut untouched, &mut errors);
            assert_eq!(untouched, b"yes\n");
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn closed_input_keeps_home_entries_and_removal_errors_omit_paths() {
        let home =
            std::env::temp_dir().join(format!("mac-proof-home-error-{}", std::process::id()));
        fs::create_dir_all(home.join(".cache")).unwrap();
        let out = home.join("evidence");
        let before = home_snapshot(&home, &out);
        let tool = home.join(".cache/eon");
        fs::create_dir(&tool).unwrap();
        fs::write(tool.join("private-history"), "history").unwrap();
        let mut errors = Vec::new();
        let mut closed = io::BufReader::new(File::open(&home).unwrap());
        let report = check_home(&home, &out, &before, &[], &mut closed, &mut errors);
        assert!(report.contains("kept (answer: no)") && tool.exists());
        assert!(errors.is_empty());
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o500)).unwrap();
        let report = check_home(&home, &out, &before, &[], &mut &b"yes\n"[..], &mut errors);
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(&home).unwrap();
        assert!(report.contains("0 removed (answer: yes)"));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("HOME: remove entry: "));
        assert!(!errors[0].contains(&home.display().to_string()));
        assert!(!errors[0].contains("private-history"));
    }

    #[test]
    fn an_unlisted_home_directory_is_never_offered_for_removal() {
        for unreadable_before in [true, false] {
            let home = std::env::temp_dir().join(format!(
                "mac-proof-home-unlisted-{}-{unreadable_before}",
                std::process::id()
            ));
            let directory = home.join(".config");
            let existing = directory.join("nushell");
            fs::create_dir_all(&existing).unwrap();
            fs::write(existing.join("history.txt"), "keep history").unwrap();
            let out = home.join("evidence");
            if unreadable_before {
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o000)).unwrap();
            }
            let before = home_snapshot(&home, &out);
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
            let new = directory.join("atuin");
            fs::create_dir(&new).unwrap();
            if !unreadable_before {
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o000)).unwrap();
            }
            let mut input = &b"yes\n"[..];
            let mut errors = Vec::new();
            let report = check_home(&home, &out, &before, &[], &mut input, &mut errors);
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
            let kept = new.exists()
                && fs::read_to_string(existing.join("history.txt"))
                    .ok()
                    .as_deref()
                    == Some("keep history");
            fs::remove_dir_all(&home).unwrap();
            assert!(kept);
            assert_eq!(input, b"yes\n");
            assert_eq!(
                report,
                "HOME: 0 other new entries, listed on the terminal only\n"
            );
            assert_eq!(errors.len(), 1);
            assert!(errors[0].starts_with("HOME: cannot list .config: "));
            assert!(!errors[0].contains(&home.display().to_string()));
            assert!(!errors[0].contains("nushell") && !errors[0].contains("atuin"));
        }
    }

    #[test]
    fn the_approved_items_have_actions_and_one_sentence_expectations() {
        assert!(ITEMS.len() <= 10);
        for item in ITEMS {
            assert!(!item.title.is_empty() && !item.action.is_empty());
            assert_eq!(item.expected.matches('.').count(), 1);
            assert!(item.expected.ends_with('.'));
        }
        assert!(help().contains(&item_text(7, &ITEMS[7])));
    }

    #[test]
    fn answers_retry_and_eof_leaves_the_rest_unanswered() {
        for (text, expected) in [
            ("p", "pass"),
            ("PASS", "pass"),
            (" F ", "fail"),
            ("Fail", "fail"),
            ("s", "skip"),
            ("SKIP", "skip"),
        ] {
            assert_eq!(parse_answer(text), Some(expected));
        }
        for text in ["", "yes", "no", "passed", "ps"] {
            assert_eq!(parse_answer(text), None);
        }
        let mut input = &b"\nwrong\nPASS\n\n\nf\nblurred\n\nSkip\n\n"[..];
        let (items, closed) = walk(&mut input, |_| Ok(None), &mut Vec::new());
        assert!(closed);
        assert_eq!(
            items.iter().map(|item| item.answer).collect::<Vec<_>>(),
            [
                "pass",
                "fail",
                "skip",
                "unanswered",
                "unanswered",
                "unanswered",
                "unanswered",
                "unanswered"
            ]
        );
        assert_eq!(items[1].note, "blurred");
        assert!(items[3].note.contains("EOF"));
        let (items, closed) = walk(
            &mut &b"\np\n"[..],
            |_| Err("no window".to_owned()),
            &mut Vec::new(),
        );
        assert!(closed);
        assert_eq!(items[0].answer, "pass");
        assert_eq!(items[0].note, "no window");
        assert_eq!(items[1].answer, "unanswered");
        assert!(items[0].capture.is_none());
        let (items, closed) = walk(
            &mut ("\np\n\n".repeat(7) + "\np\n").as_bytes(),
            |_| Ok(None),
            &mut Vec::new(),
        );
        assert!(closed && items.iter().all(|item| item.answer == "pass"));
    }

    #[test]
    fn capture_errors_fail_the_bundle_without_changing_answers_and_dry_runs_need_no_capture() {
        let mut bundle = Bundle {
            rev: "abc".to_owned(),
            tree: "def".to_owned(),
            date: "2026-10-28".to_owned(),
            dry_run: false,
            run_result: "pass",
            items: Vec::new(),
            errors: Vec::new(),
        };
        let input = "\np\n\n".repeat(ITEMS.len());
        let (items, closed) = walk(
            &mut input.as_bytes(),
            |index| {
                if index == 0 {
                    Err("screencapture failed".to_owned())
                } else {
                    Ok(Some(format!("manual/{index}.png")))
                }
            },
            &mut bundle.errors,
        );
        bundle.items = items;
        assert!(!closed && bundle.items.iter().all(|item| item.answer == "pass"));
        assert_eq!(bundle.items[0].note, "screencapture failed");
        assert!(bundle.items[0].capture.is_none());
        assert!(!bundle.passed());
        assert_eq!(
            bundle.json()["errors"],
            json!(["item 1 (Look): capture: screencapture failed"])
        );
        assert!(
            bundle
                .summary()
                .contains("Manual: item 1 (Look): capture: screencapture failed\n")
        );

        bundle.dry_run = true;
        bundle.errors.clear();
        let (items, closed) = walk(&mut input.as_bytes(), |_| Ok(None), &mut bundle.errors);
        bundle.items = items;
        assert!(!closed && bundle.items.iter().all(|item| item.capture.is_none()));
        assert!(bundle.passed());
    }

    #[test]
    fn fixed_inputs_write_the_manifest_and_summary_and_determine_success() {
        let mut items = records("not-run", "package step skipped");
        items.truncate(2);
        items[0].answer = "fail";
        items[0].note = "blurred".to_owned();
        items[0].capture = Some("manual/01-look.png".to_owned());
        items[0].seconds = 12;
        let bundle = Bundle {
            rev: "abc".to_owned(),
            tree: "def".to_owned(),
            date: "2026-10-28".to_owned(),
            dry_run: false,
            run_result: "fail",
            items,
            errors: vec!["stop: failed".to_owned()],
        };
        assert_eq!(
            bundle.json(),
            json!({
                "schema": 1, "rev": "abc", "tree": "def", "date": "2026-10-28", "dry_run": false, "run_result": "fail",
                "errors": ["stop: failed"],
                "items": [
                    {"number": 1, "title": "Look", "answer": "fail", "note": "blurred", "capture": "manual/01-look.png", "seconds": 12},
                    {"number": 2, "title": ITEMS[1].title, "answer": "not-run", "note": "package step skipped", "capture": null, "seconds": 0}
                ]
            })
        );
        assert_eq!(
            bundle.summary(),
            "Automatic run: fail\n1. Look: fail — blurred\n2. Second display (answer s to skip if you have only one): not-run — package step skipped\nManual: stop: failed\n"
        );
        assert!(!bundle.passed());
        let mut bundle = Bundle {
            run_result: "pass",
            items: records("skip", ""),
            errors: Vec::new(),
            ..bundle
        };
        assert!(bundle.passed());
        for answer in ["fail", "unanswered", "not-run"] {
            bundle.items[0].answer = answer;
            assert!(!bundle.passed());
        }
    }

    #[test]
    fn utc_dates_and_default_names_handle_boundaries_and_collisions() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(951782400), "2000-02-29");
        assert_eq!(civil_date(1609459199), "2020-12-31");
        assert_eq!(civil_date(1609459200), "2021-01-01");
        let home = std::env::temp_dir().join(format!("mac-proof-names-{}", std::process::id()));
        fs::create_dir_all(&home).unwrap();
        let first = default_out(&home, "tree-0123456789ab", "2026-10-28");
        assert_eq!(
            first.file_name().unwrap(),
            "eon-mac-final-tree-0123456789ab-2026-10-28"
        );
        fs::create_dir(&first).unwrap();
        let second = default_out(&home, "tree-0123456789ab", "2026-10-28");
        assert!(second.to_string_lossy().ends_with("-2"));
        fs::write(archive_path(&second), "").unwrap();
        assert!(
            default_out(&home, "tree-0123456789ab", "2026-10-28")
                .to_string_lossy()
                .ends_with("-3")
        );
        fs::remove_dir_all(home).unwrap();
    }
}
