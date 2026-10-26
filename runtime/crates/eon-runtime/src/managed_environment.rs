use super::diagnostics::DiagnosticLog;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

use eon_workspace_protocol::v20::{
    ALT, CTRL, MAX_ENTRIES, MAX_ENTRY_ID_BYTES, MAX_LABEL_BYTES, PopupGeometry, SHIFT, SUPER,
    Shortcut,
};

pub(super) fn nonempty_environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct EonConfig {
    shell: ShellSettings,
    terminal: TerminalSettings,
    bar: BarSettings,
    anima: AnimationSettings,
    workspace: WorkspaceSettings,
    sidebar: SidebarSettings,
    popups: BTreeMap<String, PopupEntrySettings>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

impl EonConfig {
    fn unknown_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for (prefix, unknown) in [
            ("", &self.unknown),
            ("shell.", &self.shell.unknown),
            ("terminal.", &self.terminal.unknown),
            ("bar.", &self.bar.unknown),
            ("anima.", &self.anima.unknown),
            ("workspace.", &self.workspace.unknown),
            ("sidebar.", &self.sidebar.unknown),
        ] {
            keys.extend(unknown.keys().map(|key| format!("{prefix}{key}")));
        }
        for (id, popup) in &self.popups {
            keys.extend(popup.unknown.keys().map(|key| format!("popups.{id}.{key}")));
        }
        keys.sort();
        keys
    }
}

pub struct Defaults {
    /// Every Eon Bar widget name, in the order shown when `[bar] widgets` is omitted.
    pub bar_widgets: &'static [&'static str],
    pub shell: ShellConfig,
    pub terminal: TerminalConfig,
    pub anima: StartupAnimation,
    /// Whether closing the final tab, which quits its window, asks first.
    pub warning_close_last_tab: bool,
    /// Whether full Eon offers the agent sidebar (EON-C31).
    pub sidebar_enabled: bool,
    pub popups: PopupCatalog,
    pub custom_popup_keep_alive: bool,
    pub ansi_palette: &'static str,
    pub agent_commands: &'static [&'static [&'static str]],
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StartupAnimation {
    pub enabled: bool,
    pub style: String,
    pub duration_seconds: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BarSettings {
    widgets: Option<Vec<String>>,
    #[serde(deserialize_with = "removed_show_version")]
    show_version: (),
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

fn removed_show_version<'de, D: serde::Deserializer<'de>>(_: D) -> Result<(), D::Error> {
    Err(serde::de::Error::custom(
        "bar.show_version is replaced by bar.widgets; list \"version\" there to show the version",
    ))
}

pub(crate) fn bar_widgets(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<Vec<&'static str>, String> {
    let Some(names) = read_config(root, log)?.bar.widgets else {
        return Ok(defaults.bar_widgets.to_vec());
    };
    let mut widgets = Vec::with_capacity(names.len());
    for name in names {
        let Some(&widget) = defaults.bar_widgets.iter().find(|widget| **widget == name) else {
            return Err(format!(
                "bar.widgets has unknown widget {name:?}; use {}",
                defaults.bar_widgets.join(", ")
            ));
        };
        if widgets.contains(&widget) {
            return Err(format!("bar.widgets repeats {name:?}"));
        }
        widgets.push(widget);
    }
    Ok(widgets)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AnimationSettings {
    enabled: Option<bool>,
    style: Option<String>,
    duration_seconds: Option<u64>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct WorkspaceSettings {
    warning_close_last_tab: Option<bool>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct SidebarSettings {
    enabled: Option<bool>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct PopupEntrySettings {
    command: Option<PopupCommandSetting>,
    keybinding: Option<String>,
    label: Option<String>,
    enabled: Option<bool>,
    keep_alive: Option<bool>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PopupCommandSetting {
    Named(String),
    Argv(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopupCommand {
    AgentAuto,
    Argv(Vec<OsString>),
    Project,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupDefinition {
    pub id: String,
    pub label: String,
    pub shortcut: Shortcut,
    pub command: PopupCommand,
    pub keep_alive: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PopupCatalog {
    pub geometry: PopupGeometry,
    pub entries: Vec<PopupDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ShellConfig {
    pub command: Vec<String>,
    pub starship: bool,
    pub zoxide: bool,
    pub atuin: bool,
    pub carapace: bool,
    pub mise: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ShellSettings {
    command: Option<Vec<String>>,
    starship: Option<bool>,
    zoxide: Option<bool>,
    atuin: Option<bool>,
    carapace: Option<bool>,
    mise: Option<bool>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PaddingColor {
    Extend,
    ExtendAlways,
    Background,
}

impl PaddingColor {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Extend => "extend",
            Self::ExtendAlways => "extend-always",
            Self::Background => "background",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TerminalConfig {
    pub background_opacity: f32,
    pub background_blur: bool,
    pub padding_color: PaddingColor,
    pub pane_frames: bool,
    pub focus_color: Option<String>,
    pub border_width: Option<u8>,
    pub focus_border_width: Option<u8>,
    pub cursor_trail_color: Option<String>,
    pub cursor_trail_duration: Option<f32>,
    pub font_family: Option<String>,
    pub font_fallbacks: Vec<String>,
    pub font_size: Option<f32>,
    pub line_height: Option<f32>,
    pub columns: Option<u16>,
    pub rows: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct TerminalSettings {
    background_opacity: Option<f32>,
    background_blur: Option<bool>,
    padding_color: Option<PaddingColor>,
    pane_frames: Option<bool>,
    focus_color: Option<String>,
    border_width: Option<u8>,
    focus_border_width: Option<u8>,
    cursor_trail_color: Option<String>,
    cursor_trail_duration: Option<f32>,
    font_family: Option<String>,
    font_fallbacks: Option<Vec<String>>,
    font_size: Option<f32>,
    line_height: Option<f32>,
    columns: Option<u16>,
    rows: Option<u16>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

pub(crate) fn print_defaults(defaults: &Defaults, json: bool) -> Result<String, String> {
    use serde_json::json;

    // Preserve the resolved f32 values' short decimal representation in both formats.
    let terminal = serde_json::to_string(&defaults.terminal).map_err(|error| error.to_string())?;
    let terminal: serde_json::Value =
        serde_json::from_str(&terminal).map_err(|error| error.to_string())?;
    let mut popups = serde_json::Map::new();
    for entry in &defaults.popups.entries {
        let mut keys = Vec::new();
        for (bit, name) in [
            (CTRL, "Ctrl"),
            (ALT, "Alt"),
            (SHIFT, "Shift"),
            (SUPER, "Super"),
        ] {
            if entry.shortcut.modifiers & bit != 0 {
                keys.push(name);
            }
        }
        keys.push(
            entry
                .shortcut
                .key
                .strip_prefix("Key")
                .or_else(|| entry.shortcut.key.strip_prefix("Digit"))
                .unwrap_or(&entry.shortcut.key),
        );
        let mut settings = json!({ "label": entry.label, "keybinding": keys.join("+") });
        if entry.id != "project" {
            settings["enabled"] = true.into();
        }
        if !matches!(entry.id.as_str(), "project" | "anima") {
            settings["keep_alive"] = entry.keep_alive.into();
            settings["command"] = match &entry.command {
                PopupCommand::AgentAuto => "auto".into(),
                PopupCommand::Argv(argv) => {
                    let argv = argv
                        .iter()
                        .map(|argument| {
                            argument
                                .to_str()
                                .ok_or_else(|| "default popup command is not UTF-8".to_owned())
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    json!(argv)
                }
                PopupCommand::Project => return Err("Project's command is Eon-owned".into()),
            };
        }
        popups.insert(entry.id.clone(), settings);
    }
    let mut config = json!({
        "bar": { "widgets": defaults.bar_widgets },
        "terminal": terminal,
        "shell": defaults.shell,
        "anima": defaults.anima,
        "workspace": { "warning_close_last_tab": defaults.warning_close_last_tab },
        "sidebar": { "enabled": defaults.sidebar_enabled },
        "popups": popups,
    });
    if json {
        config["terminal"]
            .as_object_mut()
            .unwrap()
            .retain(|_, value| !value.is_null());
        return serde_json::to_string_pretty(&config)
            .map(|text| text + "\n")
            .map_err(|error| error.to_string());
    }
    let mut output =
        String::from("# Built-in defaults. Unset and Eon-owned settings remain omitted.\n");
    for section in ["bar", "terminal", "shell", "anima", "workspace", "sidebar"] {
        defaults_table(&mut output, section, &config[section])?;
    }
    for (id, settings) in config["popups"].as_object().unwrap() {
        defaults_table(&mut output, &format!("popups.{id}"), settings)?;
    }
    Ok(output)
}

fn defaults_table(
    output: &mut String,
    section: &str,
    settings: &serde_json::Value,
) -> Result<(), String> {
    output.push_str(&format!("\n[{section}]\n"));
    if section == "popups.project" {
        output.push_str("# Project is the required chooser.\n");
    } else if section == "popups.anima" {
        output.push_str("# command and keep_alive are Eon-owned.\n");
    }
    for (key, value) in settings.as_object().unwrap() {
        let comment = match key.as_str() {
            "widgets" => "Eon Bar widgets, in display order.",
            "background_opacity" => "Opacity of the terminal's default background.",
            "background_blur" => "Request compositor background blur.",
            "padding_color" => "How terminal padding takes its background colour.",
            "pane_frames" => "Draw rounded pane frames.",
            "focus_color" => "Use Venus's focus colour when omitted.",
            "border_width" => "Use Venus's neutral outline width when omitted.",
            "focus_border_width" => "Use Venus's focus outline width when omitted.",
            "cursor_trail_color" => "Omission retains the workspace's random cursor colour.",
            "cursor_trail_duration" => "Cursor tail duration multiplier.",
            "font_family" => "Let Venus choose the font family when omitted.",
            "font_fallbacks" => "Ordered fallback font families.",
            "font_size" => "Let Venus choose the font size when omitted.",
            "line_height" => "Let Venus choose the line height when omitted.",
            "columns" => "Let the window choose initial columns when omitted.",
            "rows" => "Let the window choose initial rows when omitted.",
            "command" if section == "shell" => {
                "Direct argv for terminals without an explicit command."
            }
            "command" => "Direct argv, or auto for Agent's command selection.",
            "starship" => "Enable the managed Starship prompt integration.",
            "zoxide" => "Enable the managed Zoxide directory integration.",
            "atuin" => "Enable the managed Atuin history integration.",
            "carapace" => "Enable the managed Carapace completion integration.",
            "mise" => "Activate host mise when available and not already active.",
            "enabled" => "Enable this feature or popup.",
            "style" => "Startup animation style.",
            "duration_seconds" => "Startup animation duration in seconds.",
            "warning_close_last_tab" => "Ask before stopping the final tab's terminals.",
            "label" => "Popup display name.",
            "keybinding" => "Popup shortcut using physical keys.",
            "keep_alive" => "Keep this popup running when hidden.",
            _ => "Built-in setting.",
        };
        if value.is_null() {
            output.push_str(&format!("# {key}: unset. {comment}\n"));
        } else {
            let value = toml::Value::try_from(value).map_err(|error| error.to_string())?;
            output.push_str(&format!("{key} = {value} # {comment}\n"));
        }
    }
    Ok(())
}

impl TerminalConfig {
    pub(crate) fn requires_startup_admission(&self) -> bool {
        self.focus_color.is_some()
            || self.cursor_trail_color.is_some()
            || self.font_family.is_some()
            || !self.font_fallbacks.is_empty()
            || self.font_size.is_some()
            || self.line_height.is_some()
            || self.columns.is_some()
            || self.rows.is_some()
    }
}

pub struct ManagedPrograms {
    pub nu: PathBuf,
    pub bash: PathBuf,
    pub helix: PathBuf,
    pub yazi: PathBuf,
    pub ya: PathBuf,
    pub lazygit: PathBuf,
    pub nu_vendor_autoload: Option<PathBuf>,
    pub bash_rc: Option<PathBuf>,
    pub shell_bin: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tool {
    Nu,
    Bash,
    Helix,
    Yazi,
    /// The Files popup's Yazi with Eon's packaged popup configuration.
    Files,
    Ya,
    LazyGit,
}

pub(crate) fn tool(invocation: &OsStr) -> Option<Tool> {
    let name = Path::new(invocation).file_name()?.to_str()?;
    match name {
        "eon-nu" | "nu" => Some(Tool::Nu),
        "eon-bash" | "bash" => Some(Tool::Bash),
        "eon-hx" | "hx" => Some(Tool::Helix),
        "eon-yazi" | "yazi" => Some(Tool::Yazi),
        "eon-files" => Some(Tool::Files),
        "eon-ya" | "ya" => Some(Tool::Ya),
        "eon-lazygit" | "eon-lg" | "lazygit" => Some(Tool::LazyGit),
        _ => None,
    }
}

pub(crate) fn shell_command(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<Vec<String>, String> {
    Ok(read_shell_config(root, defaults, log)?.command)
}

pub(crate) fn startup_animation(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<Option<StartupAnimation>, String> {
    let settings = read_config(root, log)?.anima;
    let anima = StartupAnimation {
        enabled: settings.enabled.unwrap_or(defaults.anima.enabled),
        style: settings
            .style
            .unwrap_or_else(|| defaults.anima.style.clone()),
        duration_seconds: settings
            .duration_seconds
            .unwrap_or(defaults.anima.duration_seconds),
    };
    if anima.style.is_empty()
        || anima.style.trim() != anima.style
        || anima.style.len() > 128
        || anima.style.chars().any(char::is_control)
    {
        return Err("anima.style must be a nonempty trimmed name of at most 128 UTF-8 bytes without controls".into());
    }
    if !(1..=30).contains(&anima.duration_seconds) {
        return Err("anima.duration_seconds must be from 1 through 30".into());
    }
    Ok(anima.enabled.then_some(anima))
}

pub(crate) fn warning_close_last_tab(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<bool, String> {
    Ok(read_config(root, log)?
        .workspace
        .warning_close_last_tab
        .unwrap_or(defaults.warning_close_last_tab))
}

pub(crate) fn sidebar_enabled(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<bool, String> {
    Ok(read_config(root, log)?
        .sidebar
        .enabled
        .unwrap_or(defaults.sidebar_enabled))
}

pub(crate) fn terminal_presentation(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<TerminalConfig, String> {
    let settings = read_config(root, log)?.terminal;
    let base = &defaults.terminal;
    let terminal = TerminalConfig {
        background_opacity: settings
            .background_opacity
            .unwrap_or(base.background_opacity),
        background_blur: settings.background_blur.unwrap_or(base.background_blur),
        padding_color: settings.padding_color.unwrap_or(base.padding_color),
        pane_frames: settings.pane_frames.unwrap_or(base.pane_frames),
        focus_color: settings.focus_color.or_else(|| base.focus_color.clone()),
        border_width: settings.border_width.or(base.border_width),
        focus_border_width: settings.focus_border_width.or(base.focus_border_width),
        cursor_trail_color: settings
            .cursor_trail_color
            .or_else(|| base.cursor_trail_color.clone()),
        cursor_trail_duration: settings
            .cursor_trail_duration
            .or(base.cursor_trail_duration),
        font_family: settings.font_family.or_else(|| base.font_family.clone()),
        font_fallbacks: settings
            .font_fallbacks
            .unwrap_or_else(|| base.font_fallbacks.clone()),
        font_size: settings.font_size.or(base.font_size),
        line_height: settings.line_height.or(base.line_height),
        columns: settings.columns.or(base.columns),
        rows: settings.rows.or(base.rows),
    };
    if !terminal.background_opacity.is_finite()
        || !(0.0..=1.0).contains(&terminal.background_opacity)
    {
        return Err(
            "terminal.background_opacity must be a finite number from 0.0 through 1.0".into(),
        );
    }
    for (field, value, minimum, maximum) in [
        (
            "cursor_trail_duration",
            terminal.cursor_trail_duration,
            0.25,
            4.0,
        ),
        ("font_size", terminal.font_size, 6.0, 96.0),
        ("line_height", terminal.line_height, 1.0, 3.0),
    ] {
        if value.is_some_and(|value| !value.is_finite() || !(minimum..=maximum).contains(&value)) {
            return Err(format!(
                "terminal.{field} must be a finite number from {minimum} through {maximum}"
            ));
        }
    }
    for (field, value) in [
        ("border_width", terminal.border_width),
        ("focus_border_width", terminal.focus_border_width),
    ] {
        if value.is_some_and(|value| !(1..=3).contains(&value)) {
            return Err(format!(
                "terminal.{field} must be an integer from 1 through 3"
            ));
        }
    }
    if terminal.font_fallbacks.len() > 8 {
        return Err("terminal.font_fallbacks accepts at most eight families".into());
    }
    for (field, family) in terminal
        .font_family
        .iter()
        .map(|family| ("font_family", family))
        .chain(
            terminal
                .font_fallbacks
                .iter()
                .map(|family| ("font_fallbacks", family)),
        )
    {
        if family.is_empty()
            || family.trim() != family
            || family.len() > 128
            || family.chars().any(char::is_control)
        {
            return Err(format!(
                "terminal.{field} requires nonempty trimmed family names of at most 128 UTF-8 bytes without controls"
            ));
        }
    }
    if terminal.columns == Some(0)
        || terminal.rows == Some(0)
        || u32::from(terminal.columns.unwrap_or(1)) * u32::from(terminal.rows.unwrap_or(1))
            > orbit_protocol::MAX_CELLS as u32
    {
        return Err("terminal.columns and terminal.rows must be positive and fit Orbit's 100,000-cell limit".into());
    }
    Ok(terminal)
}

pub(crate) fn popup_catalog(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<PopupCatalog, String> {
    let mut config = read_config(root, log)?;
    // Popup margins are Eon's fixed values, not configuration (EON-C10).
    let geometry = defaults.popups.geometry;

    let mut entries = Vec::new();
    for builtin in &defaults.popups.entries {
        let id = builtin.id.as_str();
        let settings = config.popups.remove(id).unwrap_or_default();
        if id == "project" {
            if settings.command.is_some() {
                return Err("popups.project.command is Eon-owned".into());
            }
            if settings.enabled == Some(false) {
                return Err("popups.project.enabled cannot disable Eon's required chooser".into());
            }
            if settings.keep_alive.is_some_and(|value| value) {
                return Err("popups.project.keep_alive must remain false".into());
            }
        }
        if id == "anima" && settings.command.is_some() {
            return Err("popups.anima.command is Eon-owned".into());
        }
        if id == "anima" && settings.keep_alive == Some(true) {
            return Err("popups.anima.keep_alive must remain false".into());
        }
        if settings.enabled == Some(false) {
            continue;
        }
        let command = match settings.command {
            Some(command) => configured_popup_command(id, command)?,
            None => builtin.command.clone(),
        };
        entries.push(PopupDefinition {
            id: id.into(),
            label: settings.label.unwrap_or_else(|| builtin.label.clone()),
            shortcut: match settings.keybinding {
                Some(keybinding) => {
                    parse_shortcut(&format!("popups.{id}.keybinding"), &keybinding)?
                }
                None => builtin.shortcut.clone(),
            },
            command,
            keep_alive: settings.keep_alive.unwrap_or(builtin.keep_alive),
        });
    }

    for (id, settings) in config.popups {
        validate_popup_id(&id)?;
        if settings.enabled == Some(false) {
            continue;
        }
        let command = settings
            .command
            .ok_or_else(|| format!("popups.{id}.command is required"))?;
        let keybinding = settings
            .keybinding
            .ok_or_else(|| format!("popups.{id}.keybinding is required"))?;
        entries.push(PopupDefinition {
            label: settings.label.unwrap_or_else(|| id.clone()),
            shortcut: parse_shortcut(&format!("popups.{id}.keybinding"), &keybinding)?,
            command: configured_popup_command(&id, command)?,
            keep_alive: settings
                .keep_alive
                .unwrap_or(defaults.custom_popup_keep_alive),
            id,
        });
    }
    validate_popup_entries(&entries)?;
    Ok(PopupCatalog { geometry, entries })
}

fn configured_popup_command(
    id: &str,
    command: PopupCommandSetting,
) -> Result<PopupCommand, String> {
    match command {
        PopupCommandSetting::Named(value) if id == "agent" && value == "auto" => {
            Ok(PopupCommand::AgentAuto)
        }
        PopupCommandSetting::Named(_) => Err(format!(
            "popups.{id}.command must be a direct argv array{}",
            if id == "agent" { " or \"auto\"" } else { "" }
        )),
        PopupCommandSetting::Argv(argv) => {
            validate_popup_argv(id, &argv)?;
            Ok(PopupCommand::Argv(
                argv.into_iter().map(Into::into).collect(),
            ))
        }
    }
}

fn validate_popup_argv(id: &str, argv: &[String]) -> Result<(), String> {
    if argv.is_empty() || argv[0].is_empty() {
        return Err(format!(
            "popups.{id}.command requires a nonempty executable"
        ));
    }
    if argv.len() > 128 {
        return Err(format!("popups.{id}.command accepts at most 128 arguments"));
    }
    if argv.iter().any(|argument| argument.contains('\0')) {
        return Err(format!("popups.{id}.command must not contain NUL"));
    }
    if argv.iter().map(String::len).sum::<usize>() > 64 * 1024 {
        return Err(format!("popups.{id}.command exceeds 64 KiB"));
    }
    Ok(())
}

fn validate_popup_id(id: &str) -> Result<(), String> {
    let valid = !id.is_empty()
        && id.len() <= MAX_ENTRY_ID_BYTES
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "popups.{id} id must be 1-{MAX_ENTRY_ID_BYTES} ASCII letters, digits, _ or -"
        ))
    }
}

fn parse_shortcut(path: &str, value: &str) -> Result<Shortcut, String> {
    let mut parts = value.split('+').collect::<Vec<_>>();
    let key = parts
        .pop()
        .filter(|key| !key.is_empty())
        .ok_or_else(|| format!("{path} must contain modifiers and one physical key joined by +"))?;
    let mut modifiers = 0;
    for modifier in parts {
        let bit = if modifier.eq_ignore_ascii_case("shift") {
            SHIFT
        } else if modifier.eq_ignore_ascii_case("ctrl") || modifier.eq_ignore_ascii_case("control")
        {
            CTRL
        } else if modifier.eq_ignore_ascii_case("alt") {
            ALT
        } else if modifier.eq_ignore_ascii_case("super") {
            SUPER
        } else {
            return Err(format!("{path} has unsupported modifier {modifier:?}"));
        };
        if modifiers & bit != 0 {
            return Err(format!("{path} repeats modifier {modifier:?}"));
        }
        modifiers |= bit;
    }
    let key = match key.as_bytes() {
        [letter] if letter.is_ascii_alphabetic() => {
            format!("Key{}", char::from(letter.to_ascii_uppercase()))
        }
        [digit] if digit.is_ascii_digit() => format!("Digit{}", char::from(*digit)),
        _ => key.into(),
    };
    let shortcut = Shortcut { modifiers, key };
    shortcut
        .validate()
        .map_err(|_| format!("{path} is not a supported modified physical key"))?;
    Ok(shortcut)
}

fn validate_popup_entries(entries: &[PopupDefinition]) -> Result<(), String> {
    if entries.len() > MAX_ENTRIES {
        return Err(format!(
            "popups accepts at most {MAX_ENTRIES} enabled entries"
        ));
    }
    let mut shortcuts = HashMap::new();
    for entry in entries {
        validate_popup_id(&entry.id)?;
        entry.shortcut.validate().map_err(|_| {
            format!(
                "popups.{}.keybinding is not a supported modified physical key",
                entry.id
            )
        })?;
        if entry.label.is_empty()
            || entry.label.len() > MAX_LABEL_BYTES
            || entry.label.chars().any(char::is_control)
        {
            return Err(format!(
                "popups.{}.label must be nonempty, at most {MAX_LABEL_BYTES} UTF-8 bytes, and contain no controls",
                entry.id
            ));
        }
        if reserved_shortcut(&entry.shortcut) {
            return Err(format!(
                "popups.{}.keybinding is reserved for an Eon shortcut",
                entry.id
            ));
        }
        if let Some(previous) = shortcuts.insert(entry.shortcut.clone(), entry.id.as_str()) {
            return Err(format!(
                "popups.{}.keybinding conflicts with popups.{previous}.keybinding",
                entry.id
            ));
        }
    }
    Ok(())
}

/// Alt+Slash and Alt+0 through Alt+9, which no 1.x release enlarges (EON-C20). A popup
/// takes the key of any other fixed Venus binding.
fn reserved_shortcut(shortcut: &Shortcut) -> bool {
    shortcut.modifiers == ALT
        && matches!(
            shortcut.key.as_str(),
            "Digit0"
                | "Digit1"
                | "Digit2"
                | "Digit3"
                | "Digit4"
                | "Digit5"
                | "Digit6"
                | "Digit7"
                | "Digit8"
                | "Digit9"
                | "Slash"
        )
}

pub(crate) fn prepare_popup_command(
    command: &PopupCommand,
    session_bin: Option<&Path>,
    directory: &Path,
    defaults: &Defaults,
) -> Result<Vec<OsString>, String> {
    match command {
        PopupCommand::Argv(argv) => {
            if !popup_executable(&argv[0], session_bin, directory)? {
                return Err(format!(
                    "executable {:?} is unavailable on Eon's terminal PATH",
                    argv[0]
                ));
            }
            Ok(argv.clone())
        }
        PopupCommand::Project => Ok(Vec::new()),
        PopupCommand::AgentAuto => {
            for candidate in defaults.agent_commands {
                let argv = candidate.iter().map(OsString::from).collect::<Vec<_>>();
                if popup_executable(&argv[0], session_bin, directory)? {
                    return Ok(argv);
                }
            }
            Err("no supported Agent executable is available on Eon's terminal PATH; install codex, grok, opencode, pi, or claude, or configure popups.agent.command".into())
        }
    }
}

fn popup_executable(
    program: &OsStr,
    session_bin: Option<&Path>,
    directory: &Path,
) -> Result<bool, String> {
    let search_path = session_bin.map(session_path).transpose()?;
    let program = Path::new(program);
    let executable = |path: &Path| {
        fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    if program.is_absolute() {
        return Ok(executable(program));
    }
    if program.components().count() > 1 {
        return Ok(executable(&directory.join(program)));
    }
    Ok(search_path
        .or_else(|| env::var_os("PATH"))
        .as_deref()
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .any(|path| {
            let path = if path.is_absolute() {
                path
            } else {
                directory.join(path)
            };
            executable(&path.join(program))
        }))
}

pub(crate) fn session_path(prefix: &Path) -> Result<OsString, String> {
    prepend_path(prefix, env::var_os("PATH").as_deref(), "Eon terminal")
}

pub(crate) fn command(
    tool: Tool,
    programs: &ManagedPrograms,
    config: &Path,
    arguments: &[OsString],
    defaults: &Defaults,
) -> Result<Command, String> {
    let shell = match tool {
        Tool::Nu | Tool::Bash => Some(read_shell_config(config, defaults, None)?),
        _ => None,
    };
    let program = match tool {
        Tool::Nu => &programs.nu,
        Tool::Bash => &programs.bash,
        Tool::Helix => &programs.helix,
        Tool::Yazi | Tool::Files => &programs.yazi,
        Tool::Ya => &programs.ya,
        Tool::LazyGit => &programs.lazygit,
    };
    let mut command = Command::new(program);
    match tool {
        Tool::Nu => {
            command.arg("--experimental-options=native-clip");
            if let Some(path) = &programs.nu_vendor_autoload {
                command.env(
                    "NU_VENDOR_AUTOLOAD_DIR",
                    path.join(
                        integration_mask(
                            shell.as_ref().unwrap(),
                            env::var_os("ATUIN_NOBIND").is_some(),
                        )
                        .to_string(),
                    ),
                );
            }
        }
        Tool::Bash => {
            if let Some(path) = &programs.bash_rc {
                command.args([OsStr::new("--rcfile"), path.as_os_str()]);
            }
        }
        Tool::Yazi | Tool::Ya => {
            command.env_remove("YAZI_CONFIG_HOME");
        }
        Tool::Files => {
            let files = env::var_os("EON_FILES_CONFIG")
                .ok_or("the installed Eon package has no Files popup configuration")?;
            // Its Alt+Enter and edit opener call this same Eon build, to set the
            // popup's tab directory and to open files in Helix panes.
            let eon = env::current_exe()
                .map_err(|error| format!("cannot find the running Eon executable: {error}"))?;
            // Yazi prefers PWD even when it disagrees with the terminal's actual CWD.
            command
                .env("YAZI_CONFIG_HOME", files)
                .env("EON_FILES_CLI", eon)
                .env_remove("PWD");
        }
        Tool::LazyGit => {
            command
                .env_remove("CONFIG_DIR")
                .env_remove("LG_CONFIG_FILE")
                .env("XDG_CONFIG_DIRS", config);
        }
        Tool::Helix => {
            command
                .env_remove("CARGO_MANIFEST_DIR")
                .env_remove("HELIX_RUNTIME")
                .env_remove("HELIX_STEEL_CONFIG");
        }
    }
    if let Some(shell) = &shell {
        for (name, enabled) in [
            ("STARSHIP", shell.starship),
            ("ZOXIDE", shell.zoxide),
            ("ATUIN", shell.atuin),
            ("CARAPACE", shell.carapace),
            ("MISE", shell.mise),
        ] {
            command.env(format!("EON_SHELL_{name}"), if enabled { "1" } else { "0" });
        }
        if let Some(bin) = &programs.shell_bin {
            command.env(
                "PATH",
                prepend_path(bin, env::var_os("PATH").as_deref(), "managed shell")?,
            );
        }
    }
    command.args(arguments).env("EON_CONFIG_HOME", config);
    // Files needs no Eon config root, and the programs Yazi opens keep the user's.
    if shell.is_none() && tool != Tool::Files {
        command.env("XDG_CONFIG_HOME", config);
    }
    Ok(command)
}

fn integration_mask(shell: &ShellConfig, atuin_nobind: bool) -> u8 {
    u8::from(shell.starship)
        | (u8::from(shell.zoxide) << 1)
        | (u8::from(shell.atuin) << 2)
        | (u8::from(shell.carapace) << 3)
        | (u8::from(shell.atuin && atuin_nobind) << 4)
}

fn read_shell_config(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<ShellConfig, String> {
    let settings = read_config(root, log)?.shell;
    let base = &defaults.shell;
    let shell = ShellConfig {
        command: settings.command.unwrap_or_else(|| base.command.clone()),
        starship: settings.starship.unwrap_or(base.starship),
        zoxide: settings.zoxide.unwrap_or(base.zoxide),
        atuin: settings.atuin.unwrap_or(base.atuin),
        carapace: settings.carapace.unwrap_or(base.carapace),
        mise: settings.mise.unwrap_or(base.mise),
    };
    if shell.command.is_empty() || shell.command[0].is_empty() {
        return Err("shell.command must not be empty".into());
    }
    if shell.command.iter().any(|argument| argument.contains('\0')) {
        return Err("shell.command must not contain NUL".into());
    }
    Ok(shell)
}

fn read_config(root: &Path, log: Option<&DiagnosticLog>) -> Result<EonConfig, String> {
    let path = root.join("config.toml");
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(EonConfig::default());
        }
        Err(error) => {
            return Err(format!("cannot read {}: {error}", path.display()));
        }
    };
    let config: EonConfig = toml::from_str(&source)
        .map_err(|error| format!("invalid Eon configuration {}: {error}", path.display()))?;
    for key in config.unknown_keys() {
        let warning = format!(
            "eon: ignoring unknown setting {} in {}",
            key.escape_debug(),
            path.to_string_lossy().escape_debug(),
        );
        if let Some(log) = log {
            log.note(&warning);
        } else {
            let _ = writeln!(io::stderr(), "{warning}");
        }
    }
    Ok(config)
}

fn prepend_path(prefix: &Path, path: Option<&OsStr>, owner: &str) -> Result<OsString, String> {
    let mut paths = path
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .filter(|path| path != prefix)
        .collect::<Vec<_>>();
    paths.insert(0, prefix.into());
    env::join_paths(paths).map_err(|error| format!("cannot construct {owner} PATH: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{
        ManagedPrograms, PopupCommand, Tool, command as managed_command, integration_mask,
        popup_catalog, prepare_popup_command, prepend_path, read_shell_config, sidebar_enabled,
        startup_animation, terminal_presentation, tool, warning_close_last_tab,
    };
    use std::{
        ffi::{OsStr, OsString},
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        process::Command,
    };

    #[test]
    fn config_defaults_roundtrip_through_the_resolver_and_json() {
        let empty = crate::supervisor::temporary_directory();
        let printed = crate::supervisor::temporary_directory();
        let mut defaults = crate::fixtures::inputs().defaults;
        for explicit in [false, true] {
            if explicit {
                defaults.terminal.focus_color = Some("preset:ice".into());
                defaults.terminal.border_width = Some(2);
                defaults.terminal.focus_border_width = Some(3);
                defaults.terminal.cursor_trail_color = Some("preset:ice".into());
                defaults.terminal.cursor_trail_duration = Some(1.5);
                defaults.terminal.font_family = Some("Supplied Font".into());
                defaults.terminal.font_size = Some(17.5);
                defaults.terminal.line_height = Some(1.3);
                defaults.terminal.columns = Some(100);
                defaults.terminal.rows = Some(30);
                defaults.anima.enabled = false;
                defaults.popups.entries[1].command = PopupCommand::Argv(vec![
                    "fixture-git".into(),
                    "quote\"and\\slash".into(),
                    "line\nbreak".into(),
                ]);
            }
            let toml = super::print_defaults(&defaults, false).unwrap();
            let json: serde_json::Value =
                serde_json::from_str(&super::print_defaults(&defaults, true).unwrap()).unwrap();
            let parsed: toml::Value = toml::from_str(&toml).unwrap();
            assert_eq!(serde_json::to_value(parsed).unwrap(), json);
            assert!(json["popups"]["project"].get("command").is_none());
            assert!(json["popups"]["anima"].get("command").is_none());
            assert!(json["popups"]["anima"].get("keep_alive").is_none());
            assert_eq!(json["terminal"].get("font_family").is_some(), explicit);
            fs::write(printed.join("config.toml"), toml).unwrap();
            assert!(
                super::read_config(&printed, None)
                    .unwrap()
                    .unknown_keys()
                    .is_empty()
            );
            let policy = |root: &Path| {
                (
                    read_shell_config(root, &defaults, None).unwrap(),
                    terminal_presentation(root, &defaults, None).unwrap(),
                    startup_animation(root, &defaults, None).unwrap(),
                    popup_catalog(root, &defaults, None).unwrap(),
                    super::bar_widgets(root, &defaults, None).unwrap(),
                    warning_close_last_tab(root, &defaults, None).unwrap(),
                    sidebar_enabled(root, &defaults, None).unwrap(),
                )
            };
            assert_eq!(policy(&empty), policy(&printed));
        }
        fs::remove_dir_all(empty).unwrap();
        fs::remove_dir_all(printed).unwrap();
    }

    fn command_environment<'a>(command: &'a Command, name: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| value)
    }

    #[test]
    fn unknown_configuration_keys_preserve_known_settings() {
        let root = crate::supervisor::temporary_directory();
        fs::write(
            root.join("config.toml"),
            r#"
new_top_level = [1, { nested = true }, 2026-10-24T12:34:56Z]
[shell]
starship = false
new_field = true
[terminal]
background_opacity = 0.37
future_border_width = 2
[bar]
widgets = ["cpu"]
new_field = true
[anima]
enabled = false
new_field = true
[workspace]
warning_close_last_tab = false
new_field = true
[sidebar]
enabled = false
new_field = true
[popup]
side_margin = 13.0
new_field = true
[popups.agent]
label = "Configured Agent"
new_field = true
[new_table]
nested = { value = true }
"#,
        )
        .unwrap();
        let log = super::DiagnosticLog::open_in(
            Ok(root.join("logs")),
            "g1-0123456789abcdef0123456789abcdef",
            "workspace",
        );
        let config = super::read_config(&root, Some(&log)).unwrap();
        assert_eq!(config.shell.starship, Some(false));
        assert_eq!(config.terminal.background_opacity, Some(0.37));
        assert_eq!(
            config.bar.widgets.as_deref(),
            Some(["cpu".to_owned()].as_slice())
        );
        assert_eq!(config.anima.enabled, Some(false));
        assert_eq!(config.workspace.warning_close_last_tab, Some(false));
        assert_eq!(config.sidebar.enabled, Some(false));
        assert_eq!(
            config.popups["agent"].label.as_deref(),
            Some("Configured Agent")
        );
        let expected = [
            "anima.new_field",
            "bar.new_field",
            "new_table",
            "new_top_level",
            "popup",
            "popups.agent.new_field",
            "shell.new_field",
            "sidebar.new_field",
            "terminal.future_border_width",
            "workspace.new_field",
        ];
        assert_eq!(config.unknown_keys(), expected);
        let log_path = fs::read_dir(root.join("logs"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_file())
            .unwrap();
        let logged = fs::read_to_string(log_path).unwrap();
        assert_eq!(
            logged.matches("eon: ignoring unknown setting ").count(),
            expected.len()
        );
        for key in expected {
            let warning = format!(
                "eon: ignoring unknown setting {key} in {}",
                root.join("config.toml").display()
            );
            assert_eq!(
                logged
                    .lines()
                    .filter(|line| line.ends_with(&warning))
                    .count(),
                1
            );
        }

        fs::write(
            root.join("config.toml"),
            "[terminal]\nfuture_border_width = 2\nbackground_opacity = 'opaque'\n",
        )
        .unwrap();
        assert!(
            super::read_config(&root, None)
                .unwrap_err()
                .contains("background_opacity")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn supplied_defaults_survive_partial_overrides_and_repeated_reads() {
        let root = crate::supervisor::temporary_directory();
        let mut defaults = crate::fixtures::inputs().defaults;
        defaults.shell.command = vec!["supplied-shell".into(), "--interactive".into()];
        defaults.shell.starship = false;
        defaults.terminal.background_opacity = 0.37;
        defaults.terminal.background_blur = false;
        defaults.terminal.font_fallbacks = vec!["Supplied Font".into()];
        defaults.anima.style = "aquarium".into();
        defaults.anima.duration_seconds = 7;
        defaults.popups.geometry.side_margin = 23.0;
        defaults.custom_popup_keep_alive = false;
        defaults.popups.entries[1].label = "Supplied Git".into();
        defaults.popups.entries[1].command = PopupCommand::Argv(vec!["supplied-git".into()]);

        assert_eq!(
            super::shell_command(&root, &defaults, None).unwrap(),
            defaults.shell.command
        );
        assert!(
            !super::read_shell_config(&root, &defaults, None)
                .unwrap()
                .starship
        );
        assert_eq!(
            super::terminal_presentation(&root, &defaults, None)
                .unwrap()
                .font_fallbacks,
            ["Supplied Font"]
        );
        assert_eq!(
            super::startup_animation(&root, &defaults, None)
                .unwrap()
                .unwrap()
                .duration_seconds,
            7
        );
        assert_eq!(
            super::popup_catalog(&root, &defaults, None).unwrap(),
            defaults.popups
        );

        fs::write(root.join("config.toml"), "[shell]\nzoxide = false\n[terminal]\npane_frames = false\nfont_fallbacks = []\n[anima]\nduration_seconds = 2\n[popup]\nvertical_margin = 0\n[popups.git]\nkeep_alive = false\n[popups.extra]\ncommand = ['supplied-extra']\nkeybinding = 'Alt+Shift+F'\n").unwrap();
        let shell = super::read_shell_config(&root, &defaults, None).unwrap();
        assert_eq!(shell.command, defaults.shell.command);
        assert!(!shell.starship && !shell.zoxide && shell.atuin && shell.carapace);
        let terminal = super::terminal_presentation(&root, &defaults, None).unwrap();
        assert_eq!(terminal.background_opacity, 0.37);
        assert!(!terminal.background_blur && !terminal.pane_frames);
        assert!(terminal.font_fallbacks.is_empty());
        let anima = super::startup_animation(&root, &defaults, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            (anima.style.as_str(), anima.duration_seconds),
            ("aquarium", 2)
        );
        let popups = super::popup_catalog(&root, &defaults, None).unwrap();
        assert_eq!(popups.geometry, defaults.popups.geometry);
        assert_eq!(
            popups.entries[1].command,
            defaults.popups.entries[1].command
        );
        assert_eq!(popups.entries[1].label, "Supplied Git");
        assert!(!popups.entries[1].keep_alive);
        assert!(!popups.entries.last().unwrap().keep_alive);

        fs::write(root.join("config.toml"), "[shell]\ncommand = ['later-shell']\n[terminal]\nbackground_opacity = 0.0\n[anima]\nenabled = false\n").unwrap();
        assert_eq!(
            super::shell_command(&root, &defaults, None).unwrap(),
            ["later-shell"]
        );
        assert_eq!(
            super::terminal_presentation(&root, &defaults, None)
                .unwrap()
                .background_opacity,
            0.0
        );
        assert!(
            super::startup_animation(&root, &defaults, None)
                .unwrap()
                .is_none()
        );
        fs::write(root.join("config.toml"), "[terminal]\nunknown = true\n").unwrap();
        assert_eq!(
            super::terminal_presentation(&root, &defaults, None)
                .unwrap()
                .background_opacity,
            defaults.terminal.background_opacity
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn shell_configuration_is_strict_and_defaults_without_a_file() {
        let inputs = crate::fixtures::inputs();
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-0",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let default = read_shell_config(&root, &inputs.defaults, None).unwrap();
        assert_eq!(default.command, ["fixture-shell"]);
        assert!(
            default.starship && default.zoxide && default.atuin && default.carapace && default.mise
        );

        fs::write(
            root.join("config.toml"),
            "[shell]\ncommand = [\"eon-bash\", \"--norc\"]\nstarship = false\nzoxide = false\natuin = false\ncarapace = false\nmise = false\n",
        )
        .unwrap();
        let configured = read_shell_config(&root, &inputs.defaults, None).unwrap();
        assert_eq!(configured.command, ["eon-bash", "--norc"]);
        assert!(
            !configured.starship
                && !configured.zoxide
                && !configured.atuin
                && !configured.carapace
                && !configured.mise
        );
        assert_eq!(integration_mask(&configured, true), 0);
        assert_eq!(integration_mask(&default, false), 15);
        assert_eq!(integration_mask(&default, true), 31);

        for (source, expected) in [
            ("[shell]\ncommand = []\n", "shell.command must not be empty"),
            ("[shell]\nstarship = 'false'\n", "starship"),
            ("[shell\n", "invalid Eon configuration"),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert!(
                read_shell_config(&root, &inputs.defaults, None)
                    .unwrap_err()
                    .contains(expected)
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn final_tab_warning_reads_its_setting_and_rejects_non_booleans() {
        let inputs = crate::fixtures::inputs();
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-workspace",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        assert!(warning_close_last_tab(&root, &inputs.defaults, None).unwrap());
        fs::write(
            root.join("config.toml"),
            "[workspace]\nwarning_close_last_tab = false\n",
        )
        .unwrap();
        assert!(!warning_close_last_tab(&root, &inputs.defaults, None).unwrap());
        fs::write(
            root.join("config.toml"),
            "[workspace]\nwarning_close_last_tab = 'no'\n",
        )
        .unwrap();
        assert!(
            warning_close_last_tab(&root, &inputs.defaults, None)
                .unwrap_err()
                .contains("warning_close_last_tab")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_sidebar_setting_reads_its_value_and_rejects_non_booleans() {
        let inputs = crate::fixtures::inputs();
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-sidebar",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        assert!(sidebar_enabled(&root, &inputs.defaults, None).unwrap());
        fs::write(root.join("config.toml"), "[sidebar]\nenabled = false\n").unwrap();
        assert!(!sidebar_enabled(&root, &inputs.defaults, None).unwrap());
        fs::write(root.join("config.toml"), "[sidebar]\nenabled = 'no'\n").unwrap();
        assert!(
            sidebar_enabled(&root, &inputs.defaults, None)
                .unwrap_err()
                .contains("enabled")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_animation_settings_are_validated_and_can_be_disabled() {
        let inputs = crate::fixtures::inputs();
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-anima",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("config.toml"), "[anima]\nenabled = false\n").unwrap();
        assert!(
            startup_animation(&root, &inputs.defaults, None)
                .unwrap()
                .is_none()
        );
        fs::write(
            root.join("config.toml"),
            "[anima]\nstyle = 'aquarium'\nduration_seconds = 5\n",
        )
        .unwrap();
        let configured = startup_animation(&root, &inputs.defaults, None)
            .unwrap()
            .unwrap();
        assert_eq!(
            (configured.style.as_str(), configured.duration_seconds),
            ("aquarium", 5)
        );

        for (source, field) in [
            ("[anima]\nduration_seconds = 0\n", "anima.duration_seconds"),
            ("[anima]\nduration_seconds = 31\n", "anima.duration_seconds"),
            ("[anima]\nstyle = ' '\n", "anima.style"),
            ("[anima]\nstyle = \"bad\\nstyle\"\n", "anima.style"),
            ("[anima]\nenabled = 'yes'\n", "enabled"),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert!(
                startup_animation(&root, &inputs.defaults, None)
                    .unwrap_err()
                    .contains(field)
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn terminal_presentation_is_strict_and_bounded() {
        let inputs = crate::fixtures::inputs();
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-presentation",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let presentation = terminal_presentation(&root, &inputs.defaults, None).unwrap();
        assert_eq!(presentation.background_opacity, 0.6);
        assert!(presentation.background_blur);

        for (source, expected_opacity, expected_blur) in [
            ("", 0.6, true),
            ("[shell]\nstarship = false\n", 0.6, true),
            ("[terminal]\nbackground_opacity = 0.0\n", 0.0, true),
            (
                "[terminal]\nbackground_opacity = 0.88\nbackground_blur = false\n",
                0.88,
                false,
            ),
            (
                "[terminal]\nbackground_opacity = 1.0\nbackground_blur = true\n",
                1.0,
                true,
            ),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            let presentation = terminal_presentation(&root, &inputs.defaults, None).unwrap();
            assert_eq!(presentation.background_opacity, expected_opacity);
            assert_eq!(presentation.background_blur, expected_blur);
        }

        for (source, field) in [
            (
                "[terminal]\nbackground_opacity = nan\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = inf\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = -0.01\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = 1.01\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = \"0.88\"\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_opacity = 0.5\nbackground_opacity = 0.6\n",
                "background_opacity",
            ),
            (
                "[terminal]\nbackground_blur = \"true\"\n",
                "background_blur",
            ),
            (
                "[terminal]\nbackground_blur = true\nbackground_blur = false\n",
                "background_blur",
            ),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert!(
                terminal_presentation(&root, &inputs.defaults, None)
                    .unwrap_err()
                    .contains(field),
                "invalid {field} configuration did not name its field"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn popup_configuration_owns_defaults_overrides_and_collisions() {
        let inputs = crate::fixtures::inputs();
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-popups",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();

        let defaults = popup_catalog(&root, &inputs.defaults, None).unwrap();
        assert_eq!(
            defaults
                .entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["project", "git", "agent", "anima"]
        );
        assert_eq!(
            (
                defaults.geometry.side_margin,
                defaults.geometry.vertical_margin
            ),
            (11.0, 7.0)
        );

        fs::write(
            root.join("config.toml"),
            r#"[popups.git]
enabled = false

[popups.agent]
command = ["opencode", "--continue"]
keybinding = "Super+A"

[popups.files]
command = ["eon-yazi"]
keybinding = "Alt+Shift+F"
label = "Files"
keep_alive = false
"#,
        )
        .unwrap();
        let configured = popup_catalog(&root, &inputs.defaults, None).unwrap();
        assert_eq!(
            configured
                .entries
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            ["project", "agent", "anima", "files"]
        );

        fs::write(
            root.join("config.toml"),
            "[popups.files]\nenabled = false\n",
        )
        .unwrap();
        assert_eq!(
            popup_catalog(&root, &inputs.defaults, None)
                .unwrap()
                .entries
                .len(),
            4
        );

        for (source, expected) in [
            (
                "[popups.anima]\nkeep_alive = true\n",
                "popups.anima.keep_alive",
            ),
            (
                "[popups.anima]\ncommand = ['other']\n",
                "popups.anima.command",
            ),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert!(
                popup_catalog(&root, &inputs.defaults, None)
                    .unwrap_err()
                    .contains(expected)
            );
        }

        for keybinding in [
            "Alt+Slash",
            "Alt+0",
            "Alt+1",
            "Alt+2",
            "Alt+3",
            "Alt+4",
            "Alt+5",
            "Alt+6",
            "Alt+7",
            "Alt+8",
            "Alt+9",
        ] {
            fs::write(
                root.join("config.toml"),
                format!("[popups.extra]\ncommand = [\"tool\"]\nkeybinding = \"{keybinding}\"\n"),
            )
            .unwrap();
            assert!(
                popup_catalog(&root, &inputs.defaults, None)
                    .unwrap_err()
                    .contains("is reserved for an Eon shortcut"),
                "{keybinding} was not reserved"
            );
        }

        // A popup takes the key of any other fixed Venus binding.
        for keybinding in [
            "Ctrl+Shift+O",
            "Alt+Shift+N",
            "Alt+Shift+B",
            "Ctrl+Alt+M",
            "Alt+W",
            "Alt+M",
            "Ctrl+Alt+Shift+H",
            "Ctrl+Shift+C",
        ] {
            fs::write(
                root.join("config.toml"),
                format!("[popups.extra]\ncommand = [\"tool\"]\nkeybinding = \"{keybinding}\"\n"),
            )
            .unwrap();
            assert!(
                popup_catalog(&root, &inputs.defaults, None)
                    .unwrap()
                    .entries
                    .iter()
                    .any(|entry| entry.id == "extra"),
                "{keybinding} was rejected"
            );
        }

        for (source, expected) in [
            (
                "[popups.extra]\ncommand = [\"tool\"]\nkeybinding = \"Alt+Z\"\n",
                "conflicts with popups.project.keybinding",
            ),
            (
                "[popups.project]\ncommand = [\"other\"]\n",
                "popups.project.command is Eon-owned",
            ),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert!(
                popup_catalog(&root, &inputs.defaults, None)
                    .unwrap_err()
                    .contains(expected)
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn popup_executable_preflight_uses_the_session_context() {
        let inputs = crate::fixtures::inputs();
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-popup-cwd",
            std::process::id()
        ));
        let launch = root.join("launch");
        fs::create_dir_all(&launch).unwrap();
        let executable = launch.join("tool");
        fs::write(&executable, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let command = PopupCommand::Argv(vec!["./tool".into()]);

        assert_eq!(
            prepare_popup_command(&command, None, &launch, &inputs.defaults).unwrap(),
            [OsString::from("./tool")]
        );
        assert!(prepare_popup_command(&command, None, &root, &inputs.defaults).is_err());
        fs::create_dir(launch.join("bin")).unwrap();
        let path_tool = launch.join("bin/path-tool");
        fs::write(&path_tool, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path_tool, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            prepare_popup_command(
                &PopupCommand::Argv(vec!["path-tool".into()]),
                Some(Path::new("bin")),
                &launch,
                &inputs.defaults
            )
            .unwrap(),
            [OsString::from("path-tool")]
        );
        assert!(
            prepare_popup_command(
                &PopupCommand::Argv(vec![path_tool.into_os_string()]),
                Some(Path::new("invalid:path")),
                &launch,
                &inputs.defaults
            )
            .unwrap_err()
            .contains("cannot construct Eon terminal PATH")
        );
        let mut defaults = crate::fixtures::inputs().defaults;
        defaults.agent_commands = &[
            &["./preferred-agent", "--preferred"],
            &["./tool", "--fallback"],
        ];
        assert_eq!(
            prepare_popup_command(&PopupCommand::AgentAuto, None, &launch, &defaults).unwrap(),
            [OsString::from("./tool"), OsString::from("--fallback")]
        );
        let preferred = launch.join("preferred-agent");
        fs::write(&preferred, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&preferred, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            prepare_popup_command(&PopupCommand::AgentAuto, None, &launch, &defaults).unwrap(),
            [
                OsString::from("./preferred-agent"),
                OsString::from("--preferred")
            ]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_invocation_names_are_bounded() {
        use Tool::{Bash, Files, Helix, LazyGit, Nu, Ya, Yazi};

        for (name, expected) in [
            ("eon-nu", Some(Nu)),
            ("nu", Some(Nu)),
            ("eon-bash", Some(Bash)),
            ("bash", Some(Bash)),
            ("eon-zsh", None),
            ("zsh", None),
            ("eon-fish", None),
            ("fish", None),
            ("eon-hx", Some(Helix)),
            ("hx", Some(Helix)),
            ("eon-yazi", Some(Yazi)),
            ("yazi", Some(Yazi)),
            ("eon-files", Some(Files)),
            ("files", None),
            ("eon-ya", Some(Ya)),
            ("ya", Some(Ya)),
            ("eon-lazygit", Some(LazyGit)),
            ("eon-lg", Some(LazyGit)),
            ("lazygit", Some(LazyGit)),
            ("lg", None),
            ("eon-starship", None),
            ("eon-zoxide", None),
            ("eon", None),
        ] {
            assert_eq!(tool(Path::new(name).as_os_str()), expected);
        }
        assert_eq!(
            tool(Path::new("/nix/store/example/bin/eon-nu").as_os_str()),
            Some(Nu)
        );
    }

    #[test]
    fn managed_commands_use_exact_programs_and_private_configuration() {
        let inputs = crate::fixtures::inputs();
        use Tool::{Bash, Helix, LazyGit, Nu, Ya, Yazi};

        let programs = ManagedPrograms {
            nu: "/managed/nu".into(),
            bash: "/managed/bash".into(),
            helix: "/managed/hx".into(),
            yazi: "/managed/yazi".into(),
            ya: "/managed/ya".into(),
            lazygit: "/managed/lazygit".into(),
            nu_vendor_autoload: Some("/managed/autoload".into()),
            bash_rc: Some("/managed/bashrc".into()),
            shell_bin: Some("/managed/bin".into()),
        };
        let config = Path::new("/private/eon");
        for (tool, program, removed_variables) in [
            (
                Helix,
                "/managed/hx",
                &["CARGO_MANIFEST_DIR", "HELIX_RUNTIME", "HELIX_STEEL_CONFIG"][..],
            ),
            (Yazi, "/managed/yazi", &["YAZI_CONFIG_HOME"][..]),
            (Ya, "/managed/ya", &["YAZI_CONFIG_HOME"][..]),
            (
                LazyGit,
                "/managed/lazygit",
                &["CONFIG_DIR", "LG_CONFIG_FILE"][..],
            ),
        ] {
            let command = managed_command(
                tool,
                &programs,
                config,
                &["--version".into()],
                &inputs.defaults,
            )
            .unwrap();
            assert_eq!(command.get_program(), program);
            assert_eq!(
                command.get_args().map(OsString::from).collect::<Vec<_>>(),
                vec![OsString::from("--version")]
            );
            for variable in ["EON_CONFIG_HOME", "XDG_CONFIG_HOME"] {
                assert_eq!(
                    command_environment(&command, variable),
                    Some(Some(config.as_os_str()))
                );
            }
            for variable in removed_variables {
                assert_eq!(command_environment(&command, variable), Some(None));
            }
            if tool == LazyGit {
                assert_eq!(
                    command_environment(&command, "XDG_CONFIG_DIRS"),
                    Some(Some(config.as_os_str()))
                );
            }
        }

        let command = managed_command(
            Nu,
            &programs,
            config,
            &["--version".into()],
            &inputs.defaults,
        )
        .unwrap();
        assert_eq!(command.get_program(), "/managed/nu");
        assert_eq!(
            command.get_args().map(OsString::from).collect::<Vec<_>>(),
            ["--experimental-options=native-clip", "--version"].map(OsString::from)
        );
        assert_eq!(
            command_environment(&command, "EON_CONFIG_HOME"),
            Some(Some(config.as_os_str()))
        );
        assert_eq!(
            command_environment(&command, "NU_VENDOR_AUTOLOAD_DIR"),
            Some(Some(OsStr::new("/managed/autoload/15")))
        );
        assert_eq!(command_environment(&command, "XDG_CONFIG_HOME"), None);
        assert_eq!(command_environment(&command, "STARSHIP_CONFIG"), None);

        let command = managed_command(
            Bash,
            &programs,
            config,
            &["--version".into()],
            &inputs.defaults,
        )
        .unwrap();
        assert_eq!(command.get_program(), "/managed/bash");
        assert_eq!(
            command.get_args().map(OsString::from).collect::<Vec<_>>(),
            ["--rcfile", "/managed/bashrc", "--version"].map(OsString::from)
        );
        for integration in ["STARSHIP", "ZOXIDE", "ATUIN", "CARAPACE", "MISE"] {
            assert_eq!(
                command_environment(&command, &format!("EON_SHELL_{integration}")),
                Some(Some(OsStr::new("1")))
            );
        }
    }

    #[test]
    fn session_path_contains_one_managed_prefix() {
        let path = prepend_path(
            Path::new("/managed/bin"),
            Some(OsStr::new("/managed/bin:/usr/bin:/managed/bin")),
            "test",
        )
        .unwrap();

        assert_eq!(
            std::env::split_paths(&path).collect::<Vec<_>>(),
            ["/managed/bin", "/usr/bin"].map(PathBuf::from)
        );
    }
}
