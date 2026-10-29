//! Yu's guided check, after the shared automatic run, and its evidence bundle.

use crate::{
    mac, retina, run,
    scenario::{self, Environment, Scenario},
    step::{Failure, Log, progress, succeed},
};
use serde_json::{Value, json};
use std::{
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
        action: "Open Shortcuts with Option+/. Then use the keys it lists to open a tab, split a pane, move between panes and open the Helix popup.",
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
                 with -2, -3 and so on when taken)
  --timeout DUR  the automatic run's time limit, as 30s, 90m or 12h (default 12h)
  --dry-run      run only the source step and walk the items, on any system, with no captures

Requires Apple Silicon macOS, Screen Recording (window captures) and Accessibility (automatic
keys), granted to your terminal app in System Settings > Privacy & Security. Missing grants
open their settings panes; grant them, quit and reopen the terminal app, then rerun.
The automatic part captures whole screens, every connected display included, and the desktop
before launch; close or hide windows you would not send before starting, and look inside the
archive before sending it.
The automatic part takes hours. Leave the Mac awake and untouched until it rings: its GUI
scenario types into Eon. The manual part uses your HOME and environment, with private Eon
configuration and state and the default configuration. Only Eon's largest window is captured,
even when other windows cover it, without changing focus.
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

fn manual(package: PathBuf, source: &Path, out: &Path, bundle: &mut Bundle, log: &mut Log) {
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
                &mut io::stdin().lock(),
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
        let (items, closed) = walk(&mut io::stdin().lock(), |_| Ok(None), &mut bundle.errors);
        bundle.items = items;
        if closed {
            bundle.errors.push("stdin closed or at EOF".to_owned());
        }
    } else {
        match report.package {
            Ok(package) => {
                ring(&mut log);
                manual(package, &source.0, &out, &mut bundle, &mut log);
            }
            Err(reason) => bundle.items = records("not-run", &reason),
        }
    }
    fs::write(out.join("summary.txt"), bundle.summary()).map_err(|error| error.to_string())?;
    fs::write(
        out.join("final.json"),
        serde_json::to_string_pretty(&bundle.json()).expect("a JSON value serializes") + "\n",
    )
    .map_err(|error| error.to_string())?;
    let archive = archive_path(&out);
    succeed(
        &mut log,
        Command::new("tar")
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
