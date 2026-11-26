//! `eon import nova`: map Nova's root `config.toml` into Eon configuration (EON-C9). Eon
//! Runtime's parser judges every entry, and no existing file is ever changed.

use eon_runtime::{Defaults, Inputs, configuration_directory, configuration_warnings};
use serde_json::{Value as Json, json};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use toml::{Table, Value};

const HEADER: &str = "# Imported from Nova by eon import nova\n";
const NATIVE: &str =
    "native configuration, not imported; Eon keeps its tools' configuration under `eon config dir`";
const SIDECARS: [&str; 6] = ["rio", "zellij", "helix", "yazi", "nu", "starship.toml"];
const FOREST: &str = "Eon has no Forest; Files (Alt+Shift+Y) is its file manager";
const SIDEBAR: &str = "Eon's sidebar lists agents; run the command in a custom popup";
const MARGINS: &str = "Eon's popup margins are fixed at 8 and 4 logical pixels";

#[derive(Clone, Copy)]
enum Kind {
    Bool,
    Str,
    Int,
    Shell,
    Editor,
    Chord,
    Agent,
    Widgets,
}

enum Rule {
    To(&'static str, Kind),
    No(&'static str),
}
use Kind::*;
use Rule::{No, To};

/// Every Nova root setting: its Eon setting, or why Eon has none.
const RULES: &[(&str, Rule)] = &[
    ("shell.program", To("shell.command", Shell)),
    ("shell.atuin", To("shell.atuin", Bool)),
    ("editor.command", To("editor.command", Editor)),
    ("welcome.enabled", To("anima.enabled", Bool)),
    ("welcome.style", To("anima.style", Str)),
    (
        "welcome.duration_seconds",
        To("anima.duration_seconds", Int),
    ),
    ("agent.command", To("popups.agent.command", Agent)),
    ("keybindings.agent", To("popups.agent.keybinding", Chord)),
    ("keybindings.git", To("popups.git.keybinding", Chord)),
    ("keybindings.menu", To("popups.commands.keybinding", Chord)),
    ("keybindings.screen", To("popups.anima.keybinding", Chord)),
    ("bar.widgets", To("bar.widgets", Widgets)),
    (
        "agent.args",
        No("args apply only to a custom agent.command"),
    ),
    (
        "appearance.mode",
        No("Eon has no dark/light setting; its palette is fixed until themes"),
    ),
    (
        "appearance.straight_border_style",
        No("[terminal] pane_frames only turns frames on or off"),
    ),
    ("open.log_level", No("Eon has no yzx-open")),
    (
        "helix.file_watcher",
        No("Eon's Helix has no file-watcher setting"),
    ),
    ("forest.enabled", No(FOREST)),
    ("forest.side", No(FOREST)),
    ("sidebar.command", No(SIDEBAR)),
    ("sidebar.args", No(SIDEBAR)),
    (
        "bottom_hints.start_hidden",
        No("Eon starts with hints shown; Alt+Shift+B hides them"),
    ),
    (
        "keybindings.bottom_hints",
        No("Alt+Shift+B is fixed in Eon"),
    ),
    (
        "keybindings.sidebar",
        No("Alt+Shift+H is fixed; [sidebar] enabled = false releases it"),
    ),
    (
        "keybindings.sidebar_focus",
        No("Eon has no Forest or Helix focus toggle"),
    ),
    (
        "keybindings.config",
        No("Eon has no config popup; edit the file `eon config path` prints"),
    ),
    ("popup.side_margin", No(MARGINS)),
    ("popup.vertical_margin", No(MARGINS)),
];

struct Leaf {
    nova: String,
    eon: String,
    value: Value,
    note: Option<&'static str>,
}

/// What one import accepted and left out.
#[derive(Default)]
pub(crate) struct Mapping {
    accepted: Table,
    mapped: Vec<Leaf>,
    unmapped: Vec<(String, String)>,
}

fn render(table: &Table) -> String {
    format!(
        "{HEADER}{}",
        toml::to_string(table).expect("plain values serialize")
    )
}

impl Mapping {
    fn leave(&mut self, nova: impl Into<String>, reason: impl Into<String>) {
        self.unmapped.push((nova.into(), reason.into()));
    }

    /// Keep `unit` only if Eon loads the result without an error or warning.
    fn offer(&mut self, sources: &[&str], unit: Vec<Leaf>, defaults: &Defaults) {
        let mut candidate = self.accepted.clone();
        for leaf in &unit {
            let mut keys: Vec<&str> = leaf.eon.split('.').collect();
            let last = keys.pop().expect("Eon paths have a key");
            let mut table = &mut candidate;
            for key in keys {
                let entry = table.entry(key).or_insert_with(|| Table::new().into());
                table = entry.as_table_mut().expect("leaves sit under tables");
            }
            table.insert(last.into(), leaf.value.clone());
        }
        let label = Path::new("config.toml");
        let refusal = match configuration_warnings(&render(&candidate), label, defaults) {
            Ok(warnings) if warnings.is_empty() => {
                self.accepted = candidate;
                return self.mapped.extend(unit);
            }
            Ok(warnings) => warnings.join("; "),
            Err(error) => error,
        };
        let refusal = refusal.replace("eon: ", "");
        let refusal = refusal.trim_start_matches("invalid Eon configuration config.toml: ");
        for source in sources {
            self.leave(*source, refusal);
        }
    }
}

fn get<'a>(nova: &'a Table, path: &str) -> Option<&'a Value> {
    let (first, rest) = path.split_once('.')?;
    rest.split('.')
        .try_fold(nova.get(first)?, |value, key| value.get(key))
}

fn argv(first: &str, rest: &[Value]) -> Value {
    Value::Array([&[Value::from(first)], rest].concat())
}

/// Nova's `Alt Shift A` as Eon's physical `Alt+Shift+A`; `false` unbinds.
fn chord(value: &Value) -> Option<Value> {
    if value.as_bool() == Some(false) {
        return Some(Value::from(""));
    }
    let (modifiers, key) = value.as_str()?.rsplit_once(' ')?;
    let key = match key {
        "Left" | "Right" | "Up" | "Down" => format!("Arrow{key}"),
        "Esc" => "Escape".into(),
        "Enter" | "Tab" | "Backspace" | "Space" | "Home" | "End" | "PageUp" | "PageDown" => {
            key.into()
        }
        _ if key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric() => key.to_uppercase(),
        _ => return None,
    };
    let words: Vec<&str> = modifiers.split(' ').collect();
    let known = words
        .iter()
        .all(|word| matches!(*word, "Ctrl" | "Alt" | "Shift"));
    known.then(|| Value::from(format!("{}+{key}", words.join("+"))))
}

/// A string list, or `[]` when absent.
fn strings(value: Option<&Value>) -> Option<&[Value]> {
    let items = value.map_or(Some(&[][..]), |value| value.as_array().map(Vec::as_slice))?;
    items.iter().all(Value::is_str).then_some(items)
}

/// Map every setting of a parsed Nova configuration.
pub(crate) fn map(nova: &Table, defaults: &Defaults) -> Mapping {
    let mut mapping = Mapping::default();
    let mut consumed = HashSet::new();
    for (path, rule) in RULES {
        let (To(eon, kind), Some(value)) = (rule, get(nova, path)) else {
            continue;
        };
        consumed.insert(path.to_string());
        let mut sources = vec![*path];
        let typed = |valid: bool| valid.then(|| value.clone());
        let mapped = match kind {
            Bool => typed(value.is_bool()),
            Str => typed(value.is_str()),
            Int => typed(value.is_integer()),
            Shell => match value.as_str() {
                Some("nu") => Some(argv("eon-nu", &[])),
                Some("bash") => Some(argv("eon-bash", &[])),
                Some(shell @ ("zsh" | "fish")) => Some(argv(shell, &[])),
                _ => None,
            },
            Editor => match value.as_str() {
                Some("yzx-hx" | "hx") => Some(argv("hx", &[])),
                Some(editor) if !editor.is_empty() && !editor.contains(' ') => {
                    Some(argv(editor, &[]))
                }
                _ => None,
            },
            Chord => chord(value),
            Agent => match (value.as_str(), get(nova, "agent.args")) {
                (Some("auto"), _) => Some(Value::from("auto")),
                (Some(command), args) if !command.is_empty() => {
                    if args.is_some() {
                        consumed.insert("agent.args".into());
                        sources.push("agent.args");
                    }
                    let Some(args) = strings(args) else {
                        mapping.leave("agent.args", "not a valid Nova value");
                        mapping.leave(*path, "left out with its invalid agent.args");
                        continue;
                    };
                    Some(argv(command, args))
                }
                _ => None,
            },
            Widgets => {
                let Some(value) = widgets(&mut mapping, value) else {
                    continue;
                };
                Some(value)
            }
        };
        let Some(mapped) = mapped else {
            mapping.leave(*path, "not a valid Nova value");
            continue;
        };
        let note = match kind {
            Shell if matches!(value.as_str(), Some("zsh" | "fish")) => {
                Some("runs it from PATH without Eon's shell integrations")
            }
            Agent if sources.len() > 1 => Some("includes agent.args"),
            Widgets if mapped.to_string().contains("claude_quota") => {
                Some("adds Eon's clock and version; claude_quota_scoped is not added")
            }
            Widgets => Some("adds Eon's clock and version"),
            _ => None,
        };
        let leaf = Leaf {
            nova: path.to_string(),
            eon: eon.to_string(),
            value: mapped,
            note,
        };
        mapping.offer(&sources, vec![leaf], defaults);
    }
    let builtins: HashSet<&str> = defaults
        .popups
        .entries
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    for (id, popup) in nova
        .get("popups")
        .and_then(Value::as_table)
        .into_iter()
        .flatten()
    {
        let base = format!("popups.{id}");
        let Some(popup) = popup.as_table() else {
            consumed.insert(base.clone());
            mapping.leave(base, "not a valid Nova value");
            continue;
        };
        for field in ["command", "args", "title", "keybinding", "keep_alive"] {
            consumed.insert(format!("{base}.{field}"));
        }
        let valid_id = id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if id.is_empty() || !valid_id || builtins.contains(id.as_str()) {
            mapping.leave(
                &base,
                format!("`{id}` is an Eon built-in popup or no Nova popup id"),
            );
            continue;
        }
        let label = popup
            .get("title")
            .map(|title| title.as_str().map(Value::from));
        let (Some(command), Some(key), Some(args), Some(keep_alive), None | Some(Some(_))) = (
            popup
                .get("command")
                .and_then(Value::as_str)
                .filter(|command| !command.is_empty()),
            popup
                .get("keybinding")
                .filter(|key| key.is_str())
                .and_then(chord),
            strings(popup.get("args")),
            popup.get("keep_alive").map_or(Some(false), Value::as_bool),
            &label,
        ) else {
            mapping.leave(
                &base,
                "Nova requires command and a key chord, with valid fields",
            );
            continue;
        };
        let leaf = |field: &str, eon: &str, value: Value| Leaf {
            nova: format!("{base}.{field}"),
            eon: format!("{base}.{eon}"),
            value,
            note: None,
        };
        let mut unit = vec![
            leaf("command", "command", argv(command, args)),
            leaf("keybinding", "keybinding", key),
            leaf("keep_alive", "keep_alive", Value::from(keep_alive)),
        ];
        if popup.contains_key("args") {
            unit[0].note = Some("includes args");
        }
        if !popup.contains_key("keep_alive") {
            unit[2].note = Some("Nova's default false");
        }
        unit.extend(label.flatten().map(|label| leaf("title", "label", label)));
        mapping.offer(&[&base], unit, defaults);
    }
    let mut leaves = Vec::new();
    paths(nova, "", &consumed, &mut leaves);
    for path in leaves {
        let reason = match RULES.iter().find(|(nova, _)| *nova == path) {
            Some((_, No(reason))) => reason,
            _ => "not a Nova setting this import knows",
        };
        mapping.leave(path, reason);
    }
    mapping
}

/// Eon's widgets for Nova's, plus clock and version; None keeps Eon's default widgets.
fn widgets(mapping: &mut Mapping, widgets: &Value) -> Option<Value> {
    let Some(items) = widgets.as_array().filter(|items| !items.is_empty()) else {
        let reason = "not a nonempty Nova widget list; Eon keeps its default widgets";
        mapping.leave("bar.widgets", reason);
        return None;
    };
    let mut names = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let eon: &[&str] = match item.as_str() {
            Some("cpu") => &["cpu"],
            Some("ram") => &["ram"],
            Some("codex_usage") => &["codex_quota_5h", "codex_quota_7d"],
            Some("claude_usage") => &["claude_quota_5h", "claude_quota_7d"],
            name => {
                let reason = match name {
                    Some("session") => "`session`: the Eon Bar always shows the session".into(),
                    Some(name @ ("editor" | "shell" | "term" | "opencode_go_usage")) => {
                        format!("`{name}`: the Eon Bar has no {name} widget")
                    }
                    _ => "not a Nova widget".into(),
                };
                mapping.leave(format!("bar.widgets[{index}]"), reason);
                &[]
            }
        };
        for name in eon {
            if !names.contains(name) {
                names.push(*name);
            }
        }
    }
    (!names.is_empty()).then(|| [&names[..], &["clock", "version"]].concat().into())
}

/// Every setting not yet consumed; an empty table holds none.
fn paths(table: &Table, prefix: &str, consumed: &HashSet<String>, out: &mut Vec<String>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value.as_table() {
            _ if consumed.contains(&path) => {}
            Some(table) => paths(table, &path, consumed, out),
            None => out.push(path),
        }
    }
}

struct Failure {
    code: &'static str,
    detail: String,
}

fn fail(code: &'static str) -> impl Fn(String) -> Failure {
    move |detail| Failure { code, detail }
}

#[derive(Default)]
struct Report {
    source: Option<PathBuf>,
    destination: Option<PathBuf>,
    mode: Option<&'static str>,
    mapping: Mapping,
    sidecars: Vec<PathBuf>,
    toml: Option<String>,
    warnings: Vec<String>,
}

/// `eon import ARGS`; returns the exit status.
pub(crate) fn run(arguments: &[OsString], inputs: &Inputs) -> u8 {
    let (mut from, mut dry_run, mut json) = (None, false, false);
    let mut flags = arguments.iter();
    let mut valid = flags.next().is_some_and(|word| word == "nova");
    while let Some(flag) = flags.next() {
        valid &= match flag.to_str() {
            Some("--from") => flags
                .next()
                .is_some_and(|path| from.replace(path.into()).is_none()),
            Some("--dry-run") => !std::mem::replace(&mut dry_run, true),
            Some("--json") => !std::mem::replace(&mut json, true),
            _ => false,
        };
    }
    if !valid {
        eprintln!("eon: invalid import arguments\nRun eon --help for commands and examples.");
        return 2;
    }
    let mut report = Report::default();
    let result = import(&mut report, from, dry_run, &inputs.defaults);
    if json {
        let output = format!("{}\n", report.json(result.as_ref().err()));
        return crate::doctor::write_stdout(&output, u8::from(result.is_err()));
    }
    if let Err(failure) = result {
        eprintln!("eon: {}", failure.detail);
        return 1;
    }
    report
        .warnings
        .iter()
        .for_each(|warning| eprintln!("eon: {warning}"));
    crate::doctor::write_stdout(&report.human(), 0)
}

/// Nova's root file: `--from`, else under the first set configuration root.
fn source_path(from: Option<PathBuf>) -> Result<PathBuf, Failure> {
    let variable = |name| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let path = from.or_else(|| {
        let xdg = variable("XDG_CONFIG_HOME").filter(|path| path.is_absolute());
        let root = variable("YAZELIX_CONFIG_HOME")
            .or_else(|| xdg.map(|xdg| xdg.join("yazelix")))
            .or_else(|| variable("HOME").map(|home| home.join(".config/yazelix")));
        root.map(|root| root.join("config.toml"))
    });
    let missing = fail("missing-nova-configuration");
    let path = path.ok_or_else(|| {
        missing("YAZELIX_CONFIG_HOME, XDG_CONFIG_HOME and HOME are unset; use --from PATH".into())
    })?;
    std::path::absolute(&path)
        .map_err(|e| missing(format!("cannot resolve {}: {e}", path.display())))
}

fn import(
    report: &mut Report,
    from: Option<PathBuf>,
    dry_run: bool,
    defaults: &Defaults,
) -> Result<(), Failure> {
    let explicit = from.is_some();
    let source = report.source.insert(source_path(from)?).clone();
    let root = configuration_directory().map_err(fail("unreadable-configuration"))?;
    let destination = report.destination.insert(root.join("config.toml")).clone();
    let nova_root = source.parent().unwrap_or(Path::new("/"));
    let unreadable = fail("unreadable-nova-configuration");
    let text = match fs::read(&source) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| unreadable(format!("{} is not UTF-8", source.display())))?,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            if fs::symlink_metadata(&source).is_ok() {
                return Err(unreadable(format!("{} is a broken link", source.display())));
            }
            // Without a root `config.toml`, Nova runs on its defaults.
            if !explicit && nova_root.is_dir() {
                report.sidecars = sidecars(nova_root);
                (report.mode, report.toml) = (Some("nova-defaults"), Some(String::new()));
                return Ok(());
            }
            let detail = format!("no Nova configuration at {}", source.display());
            return Err(fail("missing-nova-configuration")(detail));
        }
        Err(e) => return Err(unreadable(format!("cannot read {}: {e}", source.display()))),
    };
    let nova: Table = toml::from_str(&text).map_err(|e| {
        fail("invalid-nova-configuration")(format!(
            "invalid Nova configuration {}: {e}",
            source.display()
        ))
    })?;
    report.sidecars = sidecars(nova_root);
    report.mapping = map(&nova, defaults);
    let toml = report.toml.insert(render(&report.mapping.accepted)).clone();
    if dry_run {
        report.mode = Some("dry-run");
        return Ok(());
    }
    report.mode = Some(match fs::symlink_metadata(&destination) {
        Ok(_) => "print-only-exists",
        Err(e) if e.kind() == ErrorKind::NotFound => {
            publish(&root, &destination, &toml, &mut report.warnings)?
        }
        Err(e) => {
            let detail = format!("cannot inspect {}: {e}", destination.display());
            return Err(fail("unreadable-configuration")(detail));
        }
    });
    Ok(())
}

fn sidecars(root: &Path) -> Vec<PathBuf> {
    let paths = SIDECARS.iter().map(|name| root.join(name));
    paths
        .filter(|path| fs::symlink_metadata(path).is_ok())
        .collect()
}

/// Publish `text` as `destination` without replacing a file that appears meanwhile.
fn publish(
    root: &Path,
    destination: &Path,
    text: &str,
    warnings: &mut Vec<String>,
) -> Result<&'static str, Failure> {
    let unwritable = fail("unwritable-configuration");
    // As Runtime's `prepare_configuration` creates the root for `eon config path`.
    let mut builder = fs::DirBuilder::new();
    builder
        .recursive(true)
        .mode(0o700)
        .create(root)
        .map_err(|e| unwritable(format!("cannot create {}: {e}", root.display())))?;
    let nanos = std::time::SystemTime::UNIX_EPOCH
        .elapsed()
        .map_or(0, |t| t.as_nanos());
    let temporary = root.join(format!(
        ".config.toml.import-{}-{nanos}",
        std::process::id()
    ));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary);
    let mut file =
        file.map_err(|e| unwritable(format!("cannot create {}: {e}", temporary.display())))?;
    let linked = file
        .write_all(text.as_bytes())
        .and_then(|()| file.sync_all());
    let linked = linked.and_then(|()| fs::hard_link(&temporary, destination));
    if let Err(error) = fs::remove_file(&temporary) {
        let temporary = temporary.display();
        warnings.push(format!("cannot remove temporary file {temporary}: {error}"));
    }
    match linked {
        Err(e) if e.kind() == ErrorKind::AlreadyExists => return Ok("print-only-exists"),
        Err(e) => {
            return Err(unwritable(format!(
                "cannot write {}: {e}",
                destination.display()
            )));
        }
        Ok(()) => {}
    }
    let synced = fs::File::open(root).and_then(|directory| directory.sync_all());
    if let Err(error) = synced {
        let destination = destination.display();
        warnings.push(format!(
            "wrote {destination}, but cannot sync its directory: {error}"
        ));
    }
    Ok("written")
}

impl Report {
    fn json(&self, error: Option<&Failure>) -> Json {
        let ok = error.is_none();
        let lossy = |path: &Option<PathBuf>| {
            path.as_ref()
                .map(|path| path.to_string_lossy().into_owned())
        };
        let list = |items: Vec<Json>| if ok { items } else { Vec::new() };
        let Mapping {
            mapped, unmapped, ..
        } = &self.mapping;
        json!({
            "source_path": lossy(&self.source),
            "destination_path": lossy(&self.destination),
            "mode": self.mode.filter(|_| ok),
            "mapped": list(mapped.iter().map(|leaf| {
                json!({"nova": leaf.nova, "eon": leaf.eon, "value": leaf.value, "note": leaf.note})
            }).collect()),
            "unmapped": list(unmapped.iter().map(|(nova, reason)| json!({"nova": nova, "reason": reason})).collect()),
            "sidecars": list(self.sidecars.iter().map(|path| {
                json!({"path": path.to_string_lossy(), "notice": NATIVE})
            }).collect()),
            "toml": self.toml.as_ref().filter(|_| ok),
            "warnings": list(self.warnings.iter().map(|warning| json!(warning)).collect()),
            "error": error.map(|failure| json!({"code": failure.code, "detail": failure.detail})),
        })
    }

    fn human(&self) -> String {
        let shown = |path: &Option<PathBuf>| {
            path.as_deref()
                .unwrap_or(Path::new(""))
                .display()
                .to_string()
        };
        let state = match self.mode {
            Some("written") => "written",
            Some("print-only-exists") => "exists, left unchanged: paste the TOML below",
            Some("dry-run") => "dry run: nothing written",
            _ => "Nova runs on its defaults: nothing to import",
        };
        let (source, destination) = (shown(&self.source), shown(&self.destination));
        let mut out = format!("Nova config: {source}\nEon config:  {destination} ({state})\n");
        if let Some(toml) = self.toml.as_ref().filter(|toml| !toml.is_empty()) {
            let mapped: Vec<String> = self
                .mapping
                .mapped
                .iter()
                .map(|leaf| match leaf.note {
                    Some(note) => format!("{} ({note})", leaf.nova),
                    None => leaf.nova.clone(),
                })
                .collect();
            out += &format!(
                "\n{toml}\nMapped ({}): {}\n",
                mapped.len(),
                mapped.join(", ")
            );
            out += &format!("Not mapped ({}):\n", self.mapping.unmapped.len());
            self.mapping
                .unmapped
                .iter()
                .for_each(|(nova, reason)| out += &format!("  {nova}: {reason}\n"));
        }
        for path in &self.sidecars {
            out += &format!("Left as it is: {}: {NATIVE}\n", path.display());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import(source: &str) -> Mapping {
        let defaults = crate::product::inputs(Err(String::new())).defaults;
        map(&toml::from_str(source).unwrap(), &defaults)
    }

    fn reason<'a>(mapping: &'a Mapping, nova: &str) -> &'a str {
        let found = mapping.unmapped.iter().find(|(path, _)| path == nova);
        found.map_or_else(
            || panic!("{nova} not reported: {:?}", mapping.unmapped),
            |(_, r)| r,
        )
    }

    #[test]
    fn maps_every_group() {
        let mapping = import(
            r#"
            shell = { program = "zsh", atuin = false }
            editor.command = "yzx-hx"
            welcome = { enabled = false, style = "boids", duration_seconds = 5 }
            agent = { command = "codex", args = ["--yolo"] }
            keybindings = { agent = "Ctrl y", git = false, menu = "Ctrl Alt Up", screen = "Alt Shift Esc" }
            bar.widgets = ["codex_usage", "cpu", "term", "cpu"]
            popups.btm = { command = "btm", args = ["--basic"], title = "Monitor", keybinding = "Alt Shift U" }
            "#,
        );
        let expected: Table = toml::from_str(
            r#"
            shell = { command = ["zsh"], atuin = false }
            editor.command = ["hx"]
            anima = { enabled = false, style = "boids", duration_seconds = 5 }
            bar.widgets = ["codex_quota_5h", "codex_quota_7d", "cpu", "clock", "version"]
            [popups]
            agent = { command = ["codex", "--yolo"], keybinding = "Ctrl+Y" }
            git.keybinding = ""
            commands.keybinding = "Ctrl+Alt+ArrowUp"
            anima.keybinding = "Alt+Shift+Escape"
            btm = { command = ["btm", "--basic"], keybinding = "Alt+Shift+U", keep_alive = false, label = "Monitor" }
            "#,
        )
        .unwrap();
        assert_eq!(mapping.accepted, expected, "{:?}", mapping.unmapped);
        assert_eq!(mapping.unmapped.len(), 1);
        assert!(reason(&mapping, "bar.widgets[2]").contains("no term widget"));
        let note = |nova: &str| mapping.mapped.iter().find(|l| l.nova == nova)?.note;
        let notes = [
            "agent.command",
            "popups.btm.command",
            "popups.btm.keep_alive",
        ];
        assert_eq!(
            notes.map(note),
            [
                "includes agent.args",
                "includes args",
                "Nova's default false"
            ]
            .map(Some)
        );
        assert_eq!(note("bar.widgets"), Some("adds Eon's clock and version"));
        assert!(note("shell.program").is_some_and(|note| note.contains("PATH")));
        assert!(render(&mapping.accepted).starts_with(HEADER));
    }

    #[test]
    fn every_unmapped_setting_has_its_reason() {
        let mut source = String::new();
        for (path, rule) in RULES {
            if let No(_) = rule {
                source += &format!("{path} = true\n");
            }
        }
        source += "shell.atuin = \"yes\"\nnova.unknown = 1\nkeybindings.agent = \"Super a\"\nkeybindings.git = \"Alt F1\"\n";
        source +=
            "popups.a = { command = \"x\", keybinding = false }\npopups.b.keybinding = \"Alt b\"\n";
        source += "shell.program.bad = 1\nnova.empty = {}\npopups.c = {}\npopups.d = 3\n";
        let mapping = import(&source);
        assert!(mapping.accepted.is_empty() && mapping.mapped.is_empty());
        for (path, rule) in RULES {
            if let No(expected) = rule {
                assert_eq!(reason(&mapping, path), *expected);
            }
        }
        let invalid = ["shell.atuin", "shell.program", "keybindings.agent"];
        for path in invalid.into_iter().chain(["keybindings.git", "popups.d"]) {
            assert_eq!(reason(&mapping, path), "not a valid Nova value");
        }
        assert!(reason(&mapping, "nova.unknown").starts_with("not a Nova setting"));
        assert!(reason(&mapping, "popups.a").starts_with("Nova requires"));
        assert!(reason(&mapping, "popups.b").starts_with("Nova requires"));
        assert!(reason(&mapping, "popups.c").starts_with("Nova requires"));
        // One row per setting, and none for an empty table or a scalar's table.
        let (nested, mut seen) = (
            ["shell.program.", "popups.c.", "nova.empty"],
            HashSet::new(),
        );
        for (path, _) in &mapping.unmapped {
            let fresh = !nested.iter().any(|n| path.starts_with(n)) && seen.insert(path);
            assert!(fresh, "{path}");
        }
        assert!(import("popups = {}\nsidebar = {}").unmapped.is_empty());
        // Eon refuses a NUL; the absent agent.args is not blamed.
        let nul = import("agent.command = \"a\\u0000\"");
        assert!(nul.unmapped.len() == 1 && nul.unmapped[0].0 == "agent.command");
        let bad = import("agent = { command = \"codex\", args = \"--yolo\" }");
        assert_eq!(reason(&bad, "agent.args"), "not a valid Nova value");
        assert!(reason(&bad, "agent.command").starts_with("left out"));
        let auto = import("agent = { command = \"auto\", args = [\"x\"] }");
        assert_eq!(
            auto.accepted["popups"]["agent"]["command"],
            Value::from("auto")
        );
        assert_eq!(
            reason(&auto, "agent.args"),
            "args apply only to a custom agent.command"
        );
    }

    #[test]
    fn eon_refusals_stay_out_and_the_rest_maps() {
        let mapping = import(
            r#"
            shell.atuin = true
            welcome.duration_seconds = 45
            popups.one = { command = "a", keybinding = "Alt 1" }
            popups.two = { command = "a", keybinding = "Alt h" }
            popups.three = { command = "a", keybinding = "Alt Shift U" }
            popups.four = { command = "a", keybinding = "Alt Shift U" }
            popups.sessions = { command = "a", keybinding = "Alt Shift V" }
            popups."" = { command = "a", keybinding = "Alt Shift W" }
            "#,
        );
        let refused = |nova: &str| {
            mapping
                .unmapped
                .iter()
                .any(|(path, why)| path == nova && !why.starts_with("eon: "))
        };
        assert!(mapping.mapped.iter().any(|leaf| leaf.nova == "shell.atuin"));
        assert!(
            ["welcome.duration_seconds", "popups.one", "popups.two"]
                .into_iter()
                .all(refused)
        );
        // Of two popups on one key, only the first keeps it.
        assert_ne!(refused("popups.three"), refused("popups.four"));
        assert!(reason(&mapping, "popups.sessions").contains("built-in"));
        assert!(reason(&mapping, "popups.").contains("built-in"));
    }

    #[test]
    fn widgets_keep_eon_defaults_when_nothing_maps() {
        for source in [
            "bar.widgets = []",
            "bar.widgets = \"cpu\"",
            "bar.widgets = [\"term\", \"session\"]",
        ] {
            let mapping = import(source);
            assert!(!mapping.accepted.contains_key("bar"), "{source}");
            assert!(!mapping.unmapped.is_empty(), "{source}");
        }
        let mapping = import("bar.widgets = [\"bogus\", \"ram\"]");
        assert_eq!(
            mapping.accepted["bar"]["widgets"],
            Value::from(vec!["ram", "clock", "version"])
        );
        assert_eq!(reason(&mapping, "bar.widgets[0]"), "not a Nova widget");
        let claude = import("bar.widgets = [\"claude_usage\"]");
        let note = claude.mapped[0].note.unwrap_or_default();
        assert!(note.contains("claude_quota_scoped"));
    }
}
