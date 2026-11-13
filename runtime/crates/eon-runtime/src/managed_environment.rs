use super::{diagnostics::DiagnosticLog, sessions::CONFIG_WARNED_VARIABLE};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read, Write},
    os::unix::{ffi::OsStrExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

use eon_workspace_protocol::shortcuts::{PopupPolicy, popup_policy};
use eon_workspace_protocol::v44::{
    ALT, CTRL, MAX_ENTER_DELAY_MS, MAX_ENTRIES, MAX_ENTRY_ID_BYTES, MAX_LABEL_BYTES,
    MAX_QUOTA_SCOPE_BYTES, PopupGeometry, PopupKey, SHIFT, SUPER, Shortcut,
};

pub(super) fn nonempty_environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct EonConfig {
    shell: ShellSettings,
    terminal: TerminalSettings,
    panels: PanelSettings,
    bar: BarSettings,
    anima: AnimationSettings,
    session: SessionSettings,
    sidebar: SidebarSettings,
    agents: AgentSettings,
    editor: EditorSettings,
    updates: UpdateSettings,
    popups: BTreeMap<String, PopupEntrySettings>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
    /// The deprecated keys read through `ALIASES`, each with whether its
    /// replacement was also set and so won.
    #[serde(skip)]
    deprecated: Vec<(&'static Alias, bool)>,
    /// The renamed tables found and ignored through `TOMBSTONES`.
    #[serde(skip)]
    tombstoned: Vec<&'static Tombstone>,
}

impl EonConfig {
    fn validate(self, defaults: &Defaults) -> Result<(), String> {
        resolve_bar(self.bar, defaults)?;
        resolve_terminal(self.terminal, self.panels, defaults)?;
        resolve_popups(self.popups, defaults)?;
        shell_config(self.shell, defaults)?;
        resolve_agents(self.agents, defaults)?;
        resolve_editor(self.editor, defaults)?;
        resolve_animation(self.anima, defaults)?;
        self.updates.resolve(false)?;
        Ok(())
    }

    fn unknown_keys(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for (prefix, unknown) in [
            ("", &self.unknown),
            ("shell.", &self.shell.unknown),
            ("terminal.", &self.terminal.unknown),
            ("panels.", &self.panels.unknown),
            ("bar.", &self.bar.unknown),
            ("anima.", &self.anima.unknown),
            ("session.", &self.session.unknown),
            ("sidebar.", &self.sidebar.unknown),
            ("agents.", &self.agents.unknown),
            ("editor.", &self.editor.unknown),
            ("updates.", &self.updates.unknown),
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

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct UpdateSettings {
    pub nix_profile: Option<PathBuf>,
    pub nix_entry: Option<String>,
    pub bundle_prefix: Option<PathBuf>,
    pub allowed_signers: Option<PathBuf>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

/// The configured installation target, read without preparing directories or a window.
/// The product update helper owns its package-manager policy.
pub fn update_target(defaults: &Defaults, explicit: bool) -> Result<UpdateSettings, String> {
    if ["EON_CONFIG_HOME", "XDG_CONFIG_HOME", "HOME"]
        .iter()
        .all(|name| nonempty_environment_path(name).is_none())
    {
        return Ok(UpdateSettings::default());
    }
    let root = super::supervisor::configuration_directory()?;
    read_config(&root, defaults, None)?
        .updates
        .resolve(explicit)
}

/// Whether any installation target key is present, before another build validates settings.
pub fn update_target_configured() -> Result<bool, String> {
    if ["EON_CONFIG_HOME", "XDG_CONFIG_HOME", "HOME"]
        .iter()
        .all(|name| nonempty_environment_path(name).is_none())
    {
        return Ok(false);
    }
    let path = super::supervisor::configuration_directory()?.join(CONFIG_FILE);
    update_target_present(&read_configuration(&path)?)
        .map_err(|error| format!("invalid Eon configuration {}: {error}", path.display()))
}

fn update_target_present(source: &str) -> Result<bool, String> {
    let table: toml::Table = toml::from_str(source).map_err(|error| error.to_string())?;
    let Some(updates) = table.get("updates") else {
        return Ok(false);
    };
    let updates = updates.as_table().ok_or("updates must be a table")?;
    Ok(["nix_profile", "nix_entry", "bundle_prefix"]
        .iter()
        .any(|key| updates.contains_key(*key)))
}

impl UpdateSettings {
    fn resolve(mut self, explicit: bool) -> Result<Self, String> {
        let absolute = |path: &Path| {
            path.is_absolute()
                && !path
                    .components()
                    .any(|part| part == std::path::Component::ParentDir)
        };
        if self
            .allowed_signers
            .as_deref()
            .is_some_and(|path| !absolute(path))
        {
            return Err("updates.allowed_signers requires an absolute path without .. to an independently trusted signer file".into());
        }
        if explicit {
            self.nix_profile = None;
            self.nix_entry = None;
            self.bundle_prefix = None;
            return Ok(self);
        }
        match (&self.nix_profile, &self.nix_entry, &self.bundle_prefix) {
            (None, None, None) => (),
            (Some(profile), Some(entry), None) if absolute(profile) && !entry.is_empty() => (),
            (None, None, Some(prefix)) if absolute(prefix) => (),
            _ => return Err("updates requires an absolute bundle_prefix or both absolute nix_profile and nonempty nix_entry, never both targets".into()),
        }
        Ok(self)
    }
}

pub struct Defaults {
    /// Eon Bar widgets shown, in order, when `[bar] widgets` is omitted.
    pub bar_widgets: &'static [&'static str],
    /// When the Eon Bar's CPU, RAM and disk chips colour, and the CPU hover's averaging window.
    pub bar_load: BarLoad,
    /// The path whose filesystem the Eon Bar's disk chip measures: absolute, `~` or `~/...`.
    pub bar_disk_path: &'static str,
    pub shell: ShellConfig,
    pub terminal: TerminalConfig,
    pub anima: StartupAnimation,
    /// Whether closing the final tab, which quits its window, asks first.
    pub warning_close_last_tab: bool,
    /// Whether the working badge animates on a hardware graphics adapter.
    pub animate_working_badge: bool,
    /// Whether Alt+Shift+1..9 and Alt+Shift+0 show sessions 1..10 (EON-C33).
    pub number_keys: bool,
    /// Whether full Eon offers the agent sidebar (EON-C31).
    pub sidebar_enabled: bool,
    /// Whether `eon pane read` and `eon pane send` reach the window's
    /// terminals (EON-C35, EON-C36).
    pub terminal_access: TerminalAccess,
    /// How long `eon pane send --enter` waits after the text (EON-C36).
    pub send_enter_delay_ms: u16,
    /// Quiet interval after attached-client typing before an unforced send (EON-C36).
    pub send_typing_quiet_ms: u16,
    pub popups: PopupCatalog,
    /// The editor's argv, before the file, while `[editor] command` is unset. None in a
    /// build without Helix, where an unset command resolves from the environment at each
    /// edit (EON-C10).
    pub editor: Option<&'static [&'static str]>,
    pub editor_open_in: EditorOpenIn,
    pub editor_kind: EditorKind,
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

/// When the Eon Bar's CPU, RAM and disk chips colour (`[bar]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarLoad {
    /// RAM use, in whole percent, from which the chip is yellow.
    pub ram_notice_percent: u8,
    /// RAM use, in whole percent, from which the RAM chip is orange.
    pub ram_warning_percent: u8,
    /// RAM use, in whole percent, from which the RAM chip is red.
    pub ram_critical_percent: u8,
    /// CPU use, in whole percent, from which the chip is yellow.
    pub cpu_notice_percent: u8,
    /// CPU use over the last 2 seconds, in whole percent, from which the CPU chip is orange.
    pub cpu_warning_percent: u8,
    /// CPU use, in whole percent, from which the CPU chip is red.
    pub cpu_critical_percent: u8,
    /// How many seconds of CPU use the average in the CPU chip's hover covers.
    pub cpu_average_seconds: u16,
    /// CPU temperature in Celsius from which the CPU chip is orange.
    pub cpu_temperature_warning_celsius: u8,
    /// CPU temperature in Celsius from which the CPU chip is red.
    pub cpu_temperature_critical_celsius: u8,
    /// Disk use, in whole percent, from which the chip is yellow.
    pub disk_notice_percent: u8,
    /// Disk use, in whole percent, from which the disk chip is orange.
    pub disk_warning_percent: u8,
    /// Disk use, in whole percent, from which the disk chip is red.
    pub disk_critical_percent: u8,
}

/// The window's `[bar]` settings, read when it opens.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Bar {
    pub(crate) widgets: Vec<String>,
    pub(crate) load: BarLoad,
    /// The absolute path the disk chip measures; none when `~` names a HOME that is unset or
    /// relative, so the chip has no reading.
    pub(crate) disk_path: Option<PathBuf>,
}

#[cfg(test)]
impl Defaults {
    /// The `[bar]` settings of an empty configuration.
    pub(crate) fn bar(&self) -> Bar {
        Bar {
            widgets: self.bar_widgets.iter().map(|name| (*name).into()).collect(),
            load: self.bar_load,
            disk_path: disk_path(self.bar_disk_path, nonempty_environment_path("HOME")).unwrap(),
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct BarSettings {
    widgets: Option<Vec<String>>,
    ram_notice_percent: Option<u64>,
    ram_warning_percent: Option<u64>,
    ram_critical_percent: Option<u64>,
    cpu_notice_percent: Option<u64>,
    cpu_warning_percent: Option<u64>,
    cpu_critical_percent: Option<u64>,
    cpu_average_seconds: Option<u64>,
    cpu_temperature_warning_celsius: Option<u64>,
    cpu_temperature_critical_celsius: Option<u64>,
    disk_notice_percent: Option<u64>,
    disk_warning_percent: Option<u64>,
    disk_critical_percent: Option<u64>,
    disk_path: Option<String>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

const BAR_WIDGETS: [&str; 10] = [
    "codex_quota_5h",
    "codex_quota_7d",
    "claude_quota_5h",
    "claude_quota_7d",
    "claude_quota_scoped",
    "cpu",
    "ram",
    "disk",
    "clock",
    "version",
];
const GOOGLE_QUOTA_PERIODS: [&str; 2] = ["5h", "7d"];

fn bar_widget(name: &str) -> bool {
    if BAR_WIDGETS.contains(&name) {
        return true;
    }
    google_quota_widget(name)
}

pub(crate) fn google_quota_widget(name: &str) -> bool {
    name.strip_prefix("google_quota:")
        .and_then(|selector| selector.rsplit_once(':'))
        .is_some_and(|(scope, period)| {
            !scope.is_empty()
                && scope.len() <= MAX_QUOTA_SCOPE_BYTES
                && !scope.chars().any(char::is_control)
                && GOOGLE_QUOTA_PERIODS.contains(&period)
        })
}

/// The absolute path `value` names: itself when absolute, or under `home` for `~` and `~/...`.
/// `~` under a HOME that is unset or relative names no path.
fn disk_path(value: &str, home: Option<PathBuf>) -> Result<Option<PathBuf>, String> {
    // Redundant slashes after `~/` stay under `home`: `~//data` is `home/data`.
    let under_home = if value == "~" {
        Some("")
    } else {
        value
            .strip_prefix("~/")
            .map(|rest| rest.trim_start_matches('/'))
    };
    match under_home {
        Some(rest) => Ok(home.filter(|home| home.is_absolute()).map(|home| {
            if rest.is_empty() {
                home
            } else {
                home.join(rest)
            }
        })),
        None if Path::new(value).is_absolute() => Ok(Some(PathBuf::from(value))),
        None => Err("bar.disk_path must be ~, absolute, or start with ~/".into()),
    }
}

pub(crate) fn bar(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<Bar, String> {
    resolve_bar(read_config(root, defaults, log)?.bar, defaults)
}

fn resolve_bar(settings: BarSettings, defaults: &Defaults) -> Result<Bar, String> {
    let percent = |key: &str, value: Option<u64>, default: u8| {
        value.map_or(Ok(default), |value| {
            u8::try_from(value)
                .ok()
                .filter(|value| (1..=100).contains(value))
                .ok_or_else(|| format!("bar.{key} must be from 1 through 100"))
        })
    };
    let levels =
        |chip: &str, input: [Option<u64>; 3], defaults: [u8; 3]| -> Result<[u8; 3], String> {
            let [notice, warning, critical] = input;
            let mut notice_level = percent(&format!("{chip}_notice_percent"), notice, defaults[0])?;
            let mut warning_level =
                percent(&format!("{chip}_warning_percent"), warning, defaults[1])?;
            let mut critical_level =
                percent(&format!("{chip}_critical_percent"), critical, defaults[2])?;
            if warning.is_none() {
                if notice.is_some() && critical.is_some() && notice_level > critical_level {
                    return Err(format!(
                        "bar.{chip}_critical_percent must not be below bar.{chip}_notice_percent"
                    ));
                }
                if notice.is_some() {
                    warning_level = warning_level.max(notice_level);
                }
                if critical.is_some() {
                    warning_level = warning_level.min(critical_level);
                }
            }
            if notice.is_none() {
                notice_level = notice_level.min(warning_level);
            }
            if critical.is_none() {
                critical_level = critical_level.max(warning_level);
            }
            if notice_level > warning_level {
                return Err(format!(
                    "bar.{chip}_warning_percent must not be below bar.{chip}_notice_percent"
                ));
            }
            if warning_level > critical_level {
                return Err(format!(
                    "bar.{chip}_critical_percent must not be below bar.{chip}_warning_percent"
                ));
            }
            Ok([notice_level, warning_level, critical_level])
        };
    let [
        ram_notice_percent,
        ram_warning_percent,
        ram_critical_percent,
    ] = levels(
        "ram",
        [
            settings.ram_notice_percent,
            settings.ram_warning_percent,
            settings.ram_critical_percent,
        ],
        [
            defaults.bar_load.ram_notice_percent,
            defaults.bar_load.ram_warning_percent,
            defaults.bar_load.ram_critical_percent,
        ],
    )?;
    let [
        cpu_notice_percent,
        cpu_warning_percent,
        cpu_critical_percent,
    ] = levels(
        "cpu",
        [
            settings.cpu_notice_percent,
            settings.cpu_warning_percent,
            settings.cpu_critical_percent,
        ],
        [
            defaults.bar_load.cpu_notice_percent,
            defaults.bar_load.cpu_warning_percent,
            defaults.bar_load.cpu_critical_percent,
        ],
    )?;
    let [
        disk_notice_percent,
        disk_warning_percent,
        disk_critical_percent,
    ] = levels(
        "disk",
        [
            settings.disk_notice_percent,
            settings.disk_warning_percent,
            settings.disk_critical_percent,
        ],
        [
            defaults.bar_load.disk_notice_percent,
            defaults.bar_load.disk_warning_percent,
            defaults.bar_load.disk_critical_percent,
        ],
    )?;
    let temperature = |key: &str, value: Option<u64>, default: u8| {
        value.map_or(Ok(default), |value| {
            u8::try_from(value)
                .ok()
                .filter(|value| (30..=120).contains(value))
                .ok_or_else(|| format!("bar.{key} must be from 30 through 120"))
        })
    };
    let cpu_temperature_warning_celsius = temperature(
        "cpu_temperature_warning_celsius",
        settings.cpu_temperature_warning_celsius,
        defaults.bar_load.cpu_temperature_warning_celsius,
    )?;
    let cpu_temperature_critical_celsius = temperature(
        "cpu_temperature_critical_celsius",
        settings.cpu_temperature_critical_celsius,
        defaults.bar_load.cpu_temperature_critical_celsius,
    )?;
    if cpu_temperature_critical_celsius < cpu_temperature_warning_celsius {
        return Err("bar.cpu_temperature_critical_celsius must not be below bar.cpu_temperature_warning_celsius".into());
    }
    let disk_path = disk_path(
        settings
            .disk_path
            .as_deref()
            .unwrap_or(defaults.bar_disk_path),
        nonempty_environment_path("HOME"),
    )?;
    let load = BarLoad {
        cpu_temperature_warning_celsius,
        cpu_temperature_critical_celsius,
        ram_notice_percent,
        ram_warning_percent,
        ram_critical_percent,
        cpu_notice_percent,
        cpu_warning_percent,
        cpu_critical_percent,
        cpu_average_seconds: settings.cpu_average_seconds.map_or(
            Ok(defaults.bar_load.cpu_average_seconds),
            |seconds| {
                u16::try_from(seconds)
                    .ok()
                    .filter(|seconds| (2..=600).contains(seconds))
                    .ok_or("bar.cpu_average_seconds must be from 2 through 600")
            },
        )?,
        disk_notice_percent,
        disk_warning_percent,
        disk_critical_percent,
    };
    let Some(names) = settings.widgets else {
        return Ok(Bar {
            widgets: defaults
                .bar_widgets
                .iter()
                .map(|name| (*name).into())
                .collect(),
            load,
            disk_path,
        });
    };
    let mut widgets = Vec::with_capacity(names.len());
    for name in names {
        if !bar_widget(&name) {
            continue;
        }
        if widgets.contains(&name) {
            return Err(format!("bar.widgets repeats {name:?}"));
        }
        widgets.push(name);
    }
    Ok(Bar {
        widgets,
        load,
        disk_path,
    })
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct AnimationSettings {
    enabled: Option<bool>,
    style: Option<String>,
    duration_seconds: Option<u64>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct SessionSettings {
    warning_close_last_tab: Option<bool>,
    animate_working_badge: Option<bool>,
    number_keys: Option<bool>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct SidebarSettings {
    enabled: Option<bool>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

/// `[agents] terminal_access`: a preference and a kill switch, not a security
/// boundary (EON-C35, EON-C36).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalAccess {
    /// Any process of the user may read or send to any pane or popup of the
    /// window.
    Window,
    /// Every read and send is refused.
    None,
}

const MAX_TYPING_QUIET_MS: u16 = 60000;

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct AgentSettings {
    terminal_access: Option<TerminalAccess>,
    send_enter_delay_ms: Option<u64>,
    send_typing_quiet_ms: Option<u64>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

/// The window's `[agents]` settings, read when it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Agents {
    pub(crate) terminal_access: TerminalAccess,
    pub(crate) send_enter_delay_ms: u16,
    pub(crate) send_typing_quiet_ms: u16,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EditorOpenIn {
    Popup,
    Pane,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EditorKind {
    Auto,
    Neovim,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedCommand {
    pub(crate) argv: Vec<OsString>,
    pub(crate) listener: Option<(super::editor::ListenerKind, usize)>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct EditorSettings {
    command: Option<PopupCommandSetting>,
    open_in: Option<EditorOpenIn>,
    kind: Option<EditorKind>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum PopupCommandSetting {
    Named(String),
    Argv(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopupCommand {
    AgentAuto,
    Argv(Vec<OsString>),
    /// The built-in Editor popup, started without a file (EON-C10).
    Editor,
    /// The configured editor on this file, as `eon edit` opens it.
    Edit(OsString),
    /// Scrollback's: the configured editor on a private snapshot of the tab's
    /// shown terminal, which the supervisor writes for each press (EON-C10).
    Scrollback,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopupDefinition {
    pub id: String,
    pub label: String,
    pub shortcut: PopupKey,
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

#[derive(Debug, Default, Deserialize, Serialize)]
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

/// The position of a panel's heading, resolved by Runtime before Venus starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PanelHeading {
    Notch,
    Connected,
}

impl PanelHeading {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Notch => "notch",
            Self::Connected => "connected",
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct PanelSettings {
    heading: Option<PanelHeading>,
    #[serde(flatten)]
    unknown: BTreeMap<String, toml::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TerminalConfig {
    pub background_opacity: f32,
    pub background_blur: bool,
    pub padding_color: PaddingColor,
    pub pane_frames: bool,
    // Stored with resolved launch settings; printed under [panels], not [terminal].
    #[serde(skip_serializing)]
    pub panel_heading: PanelHeading,
    pub detect_local_hosts: bool,
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

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct TerminalSettings {
    background_opacity: Option<f32>,
    background_blur: Option<bool>,
    padding_color: Option<PaddingColor>,
    pane_frames: Option<bool>,
    detect_local_hosts: Option<bool>,
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

#[derive(Clone, Serialize)]
struct ConfigKey {
    path: String,
    kind: &'static str,
    default: serde_json::Value,
    bounds: serde_json::Value,
    choices: Vec<serde_json::Value>,
    element_forms: Vec<serde_json::Value>,
    rules: Vec<String>,
    description: String,
    takes_effect: &'static [&'static str],
    eon_owned: bool,
    required: bool,
}

impl ConfigKey {
    fn new(
        path: &str,
        kind: &'static str,
        description: &str,
        takes_effect: &'static [&'static str],
    ) -> Self {
        Self {
            path: path.into(),
            kind,
            default: serde_json::Value::Null,
            bounds: serde_json::json!({}),
            choices: Vec::new(),
            element_forms: Vec::new(),
            rules: Vec::new(),
            description: description.into(),
            takes_effect,
            eon_owned: false,
            required: false,
        }
    }
    fn bounds(mut self, bounds: serde_json::Value) -> Self {
        self.bounds = bounds;
        self
    }
    fn choices(mut self, choices: &[&str]) -> Self {
        self.choices = choices
            .iter()
            .map(|value| serde_json::json!(value))
            .collect();
        self
    }
    fn rules(mut self, rules: &[&str]) -> Self {
        self.rules = rules.iter().map(|value| (*value).into()).collect();
        self
    }
    fn description(&self, default: &serde_json::Value) -> String {
        if self.path == "editor.command" && default.is_null() {
            format!(
                "{} Unset, each edit runs $VISUAL, then $EDITOR.",
                self.description
            )
        } else {
            self.description.clone()
        }
    }
}

fn configuration_metadata() -> &'static [ConfigKey] {
    static KEYS: OnceLock<Vec<ConfigKey>> = OnceLock::new();
    KEYS.get_or_init(|| {
        use serde_json::json;
        vec![
            ConfigKey::new(
                "bar.ram_notice_percent",
                "integer",
                "RAM use from which the RAM chip turns yellow.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":100}))
            .rules(&["ram thresholds must be ordered notice <= warning <= critical; omitted thresholds adjust to configured neighbours."]),
            ConfigKey::new(
                "bar.ram_warning_percent",
                "integer",
                "RAM use from which the RAM chip turns orange.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":100}))
            .rules(&["ram thresholds must be ordered notice <= warning <= critical; omitted thresholds adjust to configured neighbours."]),
            ConfigKey::new(
                "bar.ram_critical_percent",
                "integer",
                "RAM use from which the RAM chip turns red.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":100}))
            .rules(&["ram thresholds must be ordered notice <= warning <= critical; omitted thresholds adjust to configured neighbours."]),
            ConfigKey::new(
                "bar.cpu_notice_percent",
                "integer",
                "CPU use from which the CPU chip turns yellow.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":100}))
            .rules(&["cpu thresholds must be ordered notice <= warning <= critical; omitted thresholds adjust to configured neighbours."]),
            ConfigKey::new(
                "bar.cpu_warning_percent",
                "integer",
                "CPU use from which the CPU chip turns orange.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":100}))
            .rules(&["cpu thresholds must be ordered notice <= warning <= critical; omitted thresholds adjust to configured neighbours."]),
            ConfigKey::new(
                "bar.cpu_critical_percent",
                "integer",
                "CPU use from which the CPU chip turns red.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":100}))
            .rules(&["cpu thresholds must be ordered notice <= warning <= critical; omitted thresholds adjust to configured neighbours."]),
            ConfigKey::new(
                "bar.disk_notice_percent",
                "integer",
                "Disk use from which the disk chip turns yellow.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":100}))
            .rules(&["disk thresholds must be ordered notice <= warning <= critical; omitted thresholds adjust to configured neighbours."]),
            ConfigKey::new(
                "bar.disk_warning_percent",
                "integer",
                "Disk use from which the disk chip turns orange.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":100}))
            .rules(&["disk thresholds must be ordered notice <= warning <= critical; omitted thresholds adjust to configured neighbours."]),
            ConfigKey::new(
                "bar.disk_critical_percent",
                "integer",
                "Disk use from which the disk chip turns red.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":100}))
            .rules(&["disk thresholds must be ordered notice <= warning <= critical; omitted thresholds adjust to configured neighbours."]),
            ConfigKey::new(
                "bar.cpu_average_seconds",
                "integer",
                "Seconds of CPU use that the hover's average covers.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":2,"maximum":600})),
            ConfigKey::new(
                "bar.cpu_temperature_warning_celsius",
                "integer",
                "CPU temperature in Celsius from which the CPU chip turns orange (Linux CPU sensors only).",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":30,"maximum":120}))
            .rules(&["Critical temperature must not be below warning temperature."]),
            ConfigKey::new(
                "bar.cpu_temperature_critical_celsius",
                "integer",
                "CPU temperature in Celsius from which the CPU chip turns red (Linux CPU sensors only).",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":30,"maximum":120}))
            .rules(&["Critical temperature must not be below warning temperature."]),
            ConfigKey::new(
                "bar.disk_path",
                "string",
                "Path whose filesystem the disk chip measures.",
                &["next-window-presentation"],
            )
            .rules(&["Absolute path, ~ or ~/...; an unset/relative HOME leaves ~ paths without a reading."]),
            ConfigKey::new(
                "bar.widgets",
                "string-array",
                "Eon Bar widgets: header widgets in display order; the clock sits at the bottom bar's left end and the version at its right end, wherever they are listed. Each quota widget selects a window; claude_quota_scoped selects all model-scoped Claude windows. Exact google_quota:<scope>:5h or :7d selectors are opt-in; Google collection is available; chips are pending.",
                &["next-window-presentation"],
            )
            .rules(&["Ordered unique recognized widgets; unknown or malformed names warn and are ignored. Google scopes are exact, nonempty, control-free and at most 32 UTF-8 bytes; split at the final colon. Google entries are not defaults. Clock/version positions remain fixed; Google collection is available; chips are pending."]),
            ConfigKey::new(
                "terminal.background_opacity",
                "number",
                "Opacity of the terminal's default background.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":0.0,"maximum":1.0})),
            ConfigKey::new(
                "terminal.background_blur",
                "boolean",
                "Request compositor background blur.",
                &["next-window-presentation"],
            ),
            ConfigKey::new(
                "terminal.pane_frames",
                "boolean",
                "Draw rounded pane frames.",
                &["next-window-presentation"],
            ),
            ConfigKey::new(
                "terminal.detect_local_hosts",
                "boolean",
                "Treat plain-text localhost, 127.0.0.1 and [::1] with a port as links.",
                &["next-window-presentation"],
            ),
            ConfigKey::new(
                "terminal.padding_color",
                "string",
                "How terminal padding takes its background colour.",
                &["next-window-presentation"],
            )
            .choices(&["extend","extend-always","background"]),
            ConfigKey::new(
                "panels.heading",
                "string",
                "Panel heading position: notch or connected. Applies on open, reopen or reload.",
                &["next-window-presentation"],
            )
            .choices(&["notch","connected"]),
            ConfigKey::new(
                "terminal.border_width",
                "integer",
                "Use Venus's neutral outline width when omitted.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":3})),
            ConfigKey::new(
                "terminal.focus_border_width",
                "integer",
                "Use Venus's focus outline width when omitted.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":3})),
            ConfigKey::new(
                "terminal.focus_color",
                "string",
                "Use Venus's focus colour when omitted.",
                &["next-window-presentation"],
            )
            .rules(&["Venus admission accepts preset:NAME (magma, solar, lime, forest, ice, ocean, nebula, bubblegum) or custom:#RRGGBB. Focus does not accept random."]),
            ConfigKey::new(
                "terminal.cursor_trail_color",
                "string",
                "Omission retains the session's random cursor colour.",
                &["next-window-presentation"],
            )
            .rules(&["Venus admission accepts preset:NAME (magma, solar, lime, forest, ice, ocean, nebula, bubblegum) or custom:#RRGGBB. Cursor also accepts random."]),
            ConfigKey::new(
                "terminal.cursor_trail_duration",
                "number",
                "Cursor tail duration multiplier; 0 turns the trail off.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":0.0,"maximum":4.0}))
            .rules(&["Finite 0 (off), or 0.25 through 4.0."]),
            ConfigKey::new(
                "terminal.font_family",
                "string",
                "Let Venus choose the font family when omitted.",
                &["next-window-presentation"],
            )
            .bounds(json!({"min_length":1,"max_length":128}))
            .rules(&["Trimmed, no control characters; Venus admission requires an installed monospace primary family."]),
            ConfigKey::new(
                "terminal.font_fallbacks",
                "string-array",
                "Ordered fallback font families.",
                &["next-window-presentation"],
            )
            .bounds(json!({"max_items":8,"item_min_length":1,"item_max_length":128}))
            .rules(&["Trimmed names without controls; Venus admission requires installed fallback families."]),
            ConfigKey::new(
                "terminal.font_size",
                "number",
                "Let Venus choose the font size when omitted.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":6.0,"maximum":96.0})),
            ConfigKey::new(
                "terminal.line_height",
                "number",
                "Let Venus choose the line height when omitted.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1.0,"maximum":3.0})),
            ConfigKey::new(
                "terminal.columns",
                "integer",
                "Let the window choose initial columns when omitted.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":65535}))
            .rules(&["Supplied dimensions together must not exceed Orbit's 100,000-cell limit."]),
            ConfigKey::new(
                "terminal.rows",
                "integer",
                "Let the window choose initial rows when omitted.",
                &["next-window-presentation"],
            )
            .bounds(json!({"minimum":1,"maximum":65535}))
            .rules(&["Supplied dimensions together must not exceed Orbit's 100,000-cell limit."]),
            ConfigKey::new(
                "shell.command",
                "string-array",
                "Direct argv for terminals without an explicit command.",
                &["next-terminal"],
            )
            .bounds(json!({"min_items":1}))
            .rules(&["First argument is nonempty; no argument contains NUL. Runs directly without a shell."]),
            ConfigKey::new(
                "shell.starship",
                "boolean",
                "Enable the managed Starship prompt integration.",
                &["next-managed-shell"],
            ),
            ConfigKey::new(
                "shell.zoxide",
                "boolean",
                "Enable the managed Zoxide integration and record tab-directory picks.",
                &["next-managed-shell","next-directory-visit"],
            ),
            ConfigKey::new(
                "shell.atuin",
                "boolean",
                "Enable the managed Atuin history integration.",
                &["next-managed-shell"],
            ),
            ConfigKey::new(
                "shell.carapace",
                "boolean",
                "Enable the managed Carapace completion integration.",
                &["next-managed-shell"],
            ),
            ConfigKey::new(
                "shell.mise",
                "boolean",
                "Activate host mise when available and not already active.",
                &["next-managed-shell"],
            ),
            ConfigKey::new(
                "anima.enabled",
                "boolean",
                "Enable this feature or popup.",
                &["next-fresh-window"],
            ),
            ConfigKey::new(
                "anima.style",
                "string",
                "Startup animation style.",
                &["next-fresh-window"],
            )
            .bounds(json!({"min_length":1,"max_length":128}))
            .rules(&["Trimmed style name without controls; availability is owned by Anima."]),
            ConfigKey::new(
                "anima.duration_seconds",
                "integer",
                "Startup animation duration in seconds.",
                &["next-fresh-window"],
            )
            .bounds(json!({"minimum":1,"maximum":30})),
            ConfigKey::new(
                "session.warning_close_last_tab",
                "boolean",
                "Ask before stopping the final tab's terminals.",
                &["next-window-presentation"],
            ),
            ConfigKey::new(
                "session.animate_working_badge",
                "boolean",
                "Animate the working badge; false keeps it still.",
                &["next-window-presentation"],
            ),
            ConfigKey::new(
                "session.number_keys",
                "boolean",
                "Alt+Shift+1..9 and Alt+Shift+0 show sessions 1..10; false leaves the keys to the terminal.",
                &["next-window-presentation"],
            ),
            ConfigKey::new(
                "sidebar.enabled",
                "boolean",
                "Enable this feature or popup.",
                &["next-supervisor-start"],
            ),
            ConfigKey::new(
                "agents.terminal_access",
                "string",
                "Let `eon pane read`, `history` and `send` reach this window's terminals; \"none\" refuses all three.",
                &["next-supervisor-start"],
            )
            .choices(&["window","none"]),
            ConfigKey::new(
                "agents.send_enter_delay_ms",
                "integer",
                "Wait between `eon pane send` text and its --enter.",
                &["next-supervisor-start"],
            )
            .bounds(json!({"minimum":0,"maximum":MAX_ENTER_DELAY_MS})),
            ConfigKey::new(
                "agents.send_typing_quiet_ms",
                "integer",
                "Refuse unforced sends within this many ms of typing; 0 disables (0 to 60000).",
                &["next-supervisor-start"],
            )
            .bounds(json!({"minimum":0,"maximum":MAX_TYPING_QUIET_MS})),
            ConfigKey::new(
                "editor.command",
                "command",
                "Direct argv that `eon edit` and Files run with the file appended.",
                &["next-edit","next-editor-launch"],
            )
            .bounds(json!({"min_items":1,"max_items":128}))
            .rules(&["Direct argv only; first argument nonempty; no NUL; at most 65536 total argument bytes. Existing editors retain their argv."]),
            ConfigKey::new(
                "editor.kind",
                "string",
                "Recognize packaged Helix or nvim; \"neovim\" opts a wrapper in.",
                &["next-edit","next-editor-launch"],
            )
            .choices(&["auto","neovim"])
            .rules(&["Reread for each edit and fresh Editor launch; a live owned listener retains its recorded kind."]),
            ConfigKey::new(
                "editor.open_in",
                "string",
                "Open in the Helix or Neovim Editor popup, or \"pane\" for a new pane.",
                &["next-edit"],
            )
            .choices(&["popup","pane"]),
            ConfigKey::new(
                "updates.nix_profile",
                "string",
                "Absolute profile path; set with nix_entry.",
                &["next-update"],
            )
            .bounds(json!({"min_length":1}))
            .rules(&["Absolute path without .. required.","Use nix_profile with nix_entry, or bundle_prefix, never both targets; explicit targets take precedence."]),
            ConfigKey::new(
                "updates.nix_entry",
                "string",
                "Exact profile entry name; set with nix_profile.",
                &["next-update"],
            )
            .bounds(json!({"min_length":1}))
            .rules(&["Nonempty exact entry name.","Use nix_profile with nix_entry, or bundle_prefix, never both targets; explicit targets take precedence."]),
            ConfigKey::new(
                "updates.bundle_prefix",
                "string",
                "Absolute existing managed prefix; excludes the Nix pair.",
                &["next-update"],
            )
            .bounds(json!({"min_length":1}))
            .rules(&["Absolute path without .. required.","Use nix_profile with nix_entry, or bundle_prefix, never both targets; explicit targets take precedence."]),
            ConfigKey::new(
                "updates.allowed_signers",
                "string",
                "Absolute independently trusted OpenSSH signer file.",
                &["next-update"],
            )
            .bounds(json!({"min_length":1}))
            .rules(&["Absolute path without .. required.","Trust file is independent of the installation target."]),
            ConfigKey::new(
                "popups.<id>.command",
                "command",
                "Direct argv, or auto for Agent's command selection.",
                &["next-supervisor-start"],
            )
            .bounds(json!({"min_items":1,"max_items":128}))
            .rules(&["Direct argv; first argument nonempty, no NUL, at most 65536 total argument bytes. Only built-in Agent also accepts auto."]),
            ConfigKey::new(
                "popups.<id>.keybinding",
                "string",
                "Popup shortcut using physical keys.",
                &["next-supervisor-start"],
            )
            .rules(&["Empty leaves it unbound; otherwise modifiers and one supported physical key joined by +. Reserved focus keys load unbound with a warning; other reserved keys and conflicts are rejected."]),
            ConfigKey::new(
                "popups.<id>.label",
                "string",
                "Popup display name.",
                &["next-supervisor-start"],
            )
            .bounds(json!({"min_length":1,"max_length":MAX_LABEL_BYTES}))
            .rules(&["No control characters."]),
            ConfigKey::new(
                "popups.<id>.enabled",
                "boolean",
                "Enable this feature or popup.",
                &["next-supervisor-start"],
            ),
            ConfigKey::new(
                "popups.<id>.keep_alive",
                "boolean",
                "Keep this popup running when hidden.",
                &["next-supervisor-start"],
            ),
        ]
    })
}

fn schema_keys(defaults: &Defaults) -> Result<Vec<ConfigKey>, String> {
    let values = all_defaults_config(defaults)?;
    let mut keys = Vec::new();
    for metadata in configuration_metadata() {
        if let Some(field) = metadata.path.strip_prefix("popups.<id>.") {
            let mut custom = metadata.clone();
            custom.default = match field {
                "enabled" => serde_json::json!(true),
                "keep_alive" => serde_json::json!(defaults.custom_popup_keep_alive),
                _ => serde_json::Value::Null,
            };
            if matches!(field, "command" | "keybinding") {
                custom
                    .rules
                    .push("Required for an enabled custom popup.".into());
            }
            if field == "label" {
                custom.rules.push(format!("Omitted label uses its ID. IDs are 1-{} ASCII letters, digits, _ or -; at most {} enabled entries.", MAX_ENTRY_ID_BYTES, MAX_ENTRIES));
            }
            keys.push(custom);
            for entry in &defaults.popups.entries {
                let mut key = metadata.clone();
                key.path = format!("popups.{}.{field}", entry.id);
                key.default = values["popups"][&entry.id][field].clone();
                key.eon_owned = popup_field_owned(&entry.id, field, defaults);
                key.required = popup_field_owned(&entry.id, "enabled", defaults);
                if field == "command" && entry.id == "agent" {
                    key.choices = vec![serde_json::json!("auto")];
                }
                if field == "enabled" && entry.command == PopupCommand::Editor {
                    key.takes_effect = &["next-supervisor-start", "next-edit"];
                }
                if key.eon_owned {
                    key.rules.push("Eon controls this field; overrides retain the existing rejection or ignore policy.".into());
                }
                keys.push(key);
            }
        } else {
            let mut key = metadata.clone();
            key.default = metadata
                .path
                .split('.')
                .fold(&values, |value, part| &value[part])
                .clone();
            key.description = metadata.description(&key.default);
            if key.path == "bar.widgets" {
                key.choices = BAR_WIDGETS
                    .iter()
                    .map(|name| serde_json::json!(name))
                    .collect();
                key.element_forms = GOOGLE_QUOTA_PERIODS.iter().map(|period| serde_json::json!({
                    "prefix": "google_quota:", "suffix": format!(":{period}"),
                    "min_length": 1, "max_length": MAX_QUOTA_SCOPE_BYTES, "control_free": true,
                })).collect();
            }
            keys.push(key);
        }
    }
    Ok(keys)
}

pub(crate) fn print_schema(defaults: &Defaults) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({"keys": schema_keys(defaults)?}))
        .map(|text| text + "\n")
        .map_err(|error| error.to_string())
}

fn popup_field_owned(id: &str, field: &str, defaults: &Defaults) -> bool {
    match field {
        "enabled" => matches!(id, "project" | "files"),
        "command" | "keep_alive" => {
            matches!(id, "project" | "files" | "anima" | "scrollback")
                || defaults
                    .popups
                    .entries
                    .iter()
                    .any(|entry| entry.id == id && entry.command == PopupCommand::Editor)
        }
        _ => false,
    }
}

pub(crate) fn popup_shortcut(key: &PopupKey) -> String {
    let mut keys = Vec::new();
    if let Some(shortcut) = key.bound() {
        for (bit, name) in [
            (CTRL, "Ctrl"),
            (ALT, "Alt"),
            (SHIFT, "Shift"),
            (SUPER, "Super"),
        ] {
            if shortcut.modifiers & bit != 0 {
                keys.push(name);
            }
        }
        keys.push(
            shortcut
                .key
                .strip_prefix("Key")
                .or_else(|| shortcut.key.strip_prefix("Digit"))
                .unwrap_or(&shortcut.key),
        );
    }
    keys.join("+")
}

// Keep unset and owned keys here so metadata consumers see the complete parser surface.
fn all_defaults_config(defaults: &Defaults) -> Result<serde_json::Value, String> {
    let mut popups = BTreeMap::new();
    for entry in &defaults.popups.entries {
        let keybinding = popup_shortcut(&entry.shortcut);
        let command = if popup_field_owned(&entry.id, "command", defaults) {
            None
        } else {
            Some(match &entry.command {
                PopupCommand::AgentAuto => PopupCommandSetting::Named("auto".into()),
                PopupCommand::Argv(argv) => PopupCommandSetting::Argv(
                    argv.iter()
                        .map(|argument| {
                            argument
                                .to_str()
                                .map(str::to_owned)
                                .ok_or_else(|| "default popup command is not UTF-8".to_owned())
                        })
                        .collect::<Result<_, _>>()?,
                ),
                PopupCommand::Edit(_) => return Err("a default popup cannot edit a file".into()),
                PopupCommand::Editor | PopupCommand::Scrollback => {
                    unreachable!("this popup's command is Eon-owned")
                }
            })
        };
        popups.insert(
            entry.id.clone(),
            PopupEntrySettings {
                command,
                keybinding: Some(keybinding),
                label: Some(entry.label.clone()),
                enabled: Some(true),
                keep_alive: Some(entry.keep_alive),
                unknown: BTreeMap::new(),
            },
        );
    }
    // Exhaustive literals make every added parser field require a defaults decision.
    let config = EonConfig {
        shell: ShellSettings {
            command: Some(defaults.shell.command.clone()),
            starship: Some(defaults.shell.starship),
            zoxide: Some(defaults.shell.zoxide),
            atuin: Some(defaults.shell.atuin),
            carapace: Some(defaults.shell.carapace),
            mise: Some(defaults.shell.mise),
            unknown: BTreeMap::new(),
        },
        terminal: TerminalSettings {
            background_opacity: Some(defaults.terminal.background_opacity),
            background_blur: Some(defaults.terminal.background_blur),
            padding_color: Some(defaults.terminal.padding_color),
            pane_frames: Some(defaults.terminal.pane_frames),
            detect_local_hosts: Some(defaults.terminal.detect_local_hosts),
            focus_color: defaults.terminal.focus_color.clone(),
            border_width: defaults.terminal.border_width,
            focus_border_width: defaults.terminal.focus_border_width,
            cursor_trail_color: defaults.terminal.cursor_trail_color.clone(),
            cursor_trail_duration: defaults.terminal.cursor_trail_duration,
            font_family: defaults.terminal.font_family.clone(),
            font_fallbacks: Some(defaults.terminal.font_fallbacks.clone()),
            font_size: defaults.terminal.font_size,
            line_height: defaults.terminal.line_height,
            columns: defaults.terminal.columns,
            rows: defaults.terminal.rows,
            unknown: BTreeMap::new(),
        },
        panels: PanelSettings {
            heading: Some(defaults.terminal.panel_heading),
            unknown: BTreeMap::new(),
        },
        bar: BarSettings {
            widgets: Some(
                defaults
                    .bar_widgets
                    .iter()
                    .map(|widget| (*widget).into())
                    .collect(),
            ),
            ram_notice_percent: Some(u64::from(defaults.bar_load.ram_notice_percent)),
            ram_warning_percent: Some(u64::from(defaults.bar_load.ram_warning_percent)),
            ram_critical_percent: Some(u64::from(defaults.bar_load.ram_critical_percent)),
            cpu_notice_percent: Some(u64::from(defaults.bar_load.cpu_notice_percent)),
            cpu_warning_percent: Some(u64::from(defaults.bar_load.cpu_warning_percent)),
            cpu_critical_percent: Some(u64::from(defaults.bar_load.cpu_critical_percent)),
            cpu_average_seconds: Some(u64::from(defaults.bar_load.cpu_average_seconds)),
            cpu_temperature_warning_celsius: Some(u64::from(
                defaults.bar_load.cpu_temperature_warning_celsius,
            )),
            cpu_temperature_critical_celsius: Some(u64::from(
                defaults.bar_load.cpu_temperature_critical_celsius,
            )),
            disk_notice_percent: Some(u64::from(defaults.bar_load.disk_notice_percent)),
            disk_warning_percent: Some(u64::from(defaults.bar_load.disk_warning_percent)),
            disk_critical_percent: Some(u64::from(defaults.bar_load.disk_critical_percent)),
            disk_path: Some(defaults.bar_disk_path.into()),
            unknown: BTreeMap::new(),
        },
        anima: AnimationSettings {
            enabled: Some(defaults.anima.enabled),
            style: Some(defaults.anima.style.clone()),
            duration_seconds: Some(defaults.anima.duration_seconds),
            unknown: BTreeMap::new(),
        },
        session: SessionSettings {
            warning_close_last_tab: Some(defaults.warning_close_last_tab),
            animate_working_badge: Some(defaults.animate_working_badge),
            number_keys: Some(defaults.number_keys),
            unknown: BTreeMap::new(),
        },
        sidebar: SidebarSettings {
            enabled: Some(defaults.sidebar_enabled),
            unknown: BTreeMap::new(),
        },
        agents: AgentSettings {
            terminal_access: Some(defaults.terminal_access),
            send_enter_delay_ms: Some(u64::from(defaults.send_enter_delay_ms)),
            send_typing_quiet_ms: Some(u64::from(defaults.send_typing_quiet_ms)),
            unknown: BTreeMap::new(),
        },
        editor: EditorSettings {
            open_in: Some(defaults.editor_open_in),
            kind: Some(defaults.editor_kind),
            command: defaults.editor.map(|argv| {
                PopupCommandSetting::Argv(argv.iter().map(|argument| (*argument).into()).collect())
            }),
            unknown: BTreeMap::new(),
        },
        updates: UpdateSettings {
            nix_profile: None,
            nix_entry: None,
            bundle_prefix: None,
            allowed_signers: None,
            unknown: BTreeMap::new(),
        },
        popups,
        unknown: BTreeMap::new(),
        deprecated: Vec::new(),
        tombstoned: Vec::new(),
    };
    // Preserve f32 values' short decimal representation in both formats.
    let config = serde_json::to_string(&config).map_err(|error| error.to_string())?;
    serde_json::from_str(&config).map_err(|error| error.to_string())
}

fn defaults_config(defaults: &Defaults) -> Result<serde_json::Value, String> {
    let mut config = all_defaults_config(defaults)?;
    for (id, settings) in config["popups"].as_object_mut().unwrap() {
        let settings = settings.as_object_mut().unwrap();
        settings.retain(|field, _| !popup_field_owned(id, field, defaults));
    }
    Ok(config)
}

pub(crate) fn print_defaults(defaults: &Defaults, json: bool) -> Result<String, String> {
    format_defaults(defaults_config(defaults)?, json)
}

fn omit_unset(config: &mut serde_json::Value) {
    if let Some(settings) = config.as_object_mut() {
        settings.retain(|_, value| !value.is_null());
        for value in settings.values_mut() {
            omit_unset(value);
        }
    }
}

fn format_defaults(mut config: serde_json::Value, json: bool) -> Result<String, String> {
    if json {
        omit_unset(&mut config);
        if config["updates"]
            .as_object()
            .is_some_and(serde_json::Map::is_empty)
        {
            config.as_object_mut().unwrap().remove("updates");
        }
        return serde_json::to_string_pretty(&config)
            .map(|text| text + "\n")
            .map_err(|error| error.to_string());
    }
    let mut output =
        String::from("# Built-in defaults. Unset and Eon-owned settings remain omitted.\n");
    let order = [
        "bar", "terminal", "panels", "shell", "anima", "session", "sidebar", "agents", "editor",
        "updates", "popups",
    ];
    let mut sections: Vec<_> = config.as_object().unwrap().iter().collect();
    sections.sort_by_key(|(section, _)| {
        order
            .iter()
            .position(|name| name == section)
            .unwrap_or(order.len())
    });
    for (section, settings) in sections {
        if section == "popups" {
            for (id, settings) in settings.as_object().unwrap() {
                defaults_table(&mut output, &format!("popups.{id}"), settings)?;
            }
        } else {
            defaults_table(&mut output, section, settings)?;
        }
    }
    Ok(output)
}

fn defaults_table(
    output: &mut String,
    section: &str,
    settings: &serde_json::Value,
) -> Result<(), String> {
    if section == "updates"
        && settings
            .as_object()
            .unwrap()
            .values()
            .all(serde_json::Value::is_null)
    {
        output.push_str("\n# [updates]\n");
    } else {
        output.push_str(&format!("\n[{section}]\n"));
    }
    if section == "popups.project" {
        output.push_str("# Project is required and shows the tab's Files on quick search.\n");
    } else if section == "popups.files" {
        output.push_str("# Files is required: it is the tab's directory pair.\n");
    } else if section == "popups.editor" {
        output.push_str("# Editor uses [editor] command and keeps running when hidden.\n");
    } else if matches!(section, "popups.anima" | "popups.scrollback") {
        output.push_str("# command and keep_alive are Eon-owned.\n");
    }
    let mut fields: Vec<_> = settings.as_object().unwrap().iter().collect();
    if section == "updates" {
        let order = [
            "nix_profile",
            "nix_entry",
            "bundle_prefix",
            "allowed_signers",
        ];
        fields.sort_by_key(|(key, _)| {
            order
                .iter()
                .position(|name| name == key)
                .unwrap_or(order.len())
        });
    }
    for (key, value) in fields {
        let path = if section.starts_with("popups.") {
            format!("popups.<id>.{key}")
        } else {
            format!("{section}.{key}")
        };
        let comment = configuration_metadata()
            .iter()
            .find(|metadata| metadata.path == path)
            .map_or_else(
                || "Built-in setting.".into(),
                |metadata| metadata.description(value),
            );
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
    /// The packaged Helix; none in a build without it.
    pub helix: Option<PathBuf>,
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

/// The program `tool` runs. A build without Helix refuses its names rather than
/// running an `hx` from PATH, which inside a terminal would be this same name.
pub(crate) fn program(tool: Tool, programs: &ManagedPrograms) -> Result<&Path, String> {
    Ok(match tool {
        Tool::Nu => &programs.nu,
        Tool::Bash => &programs.bash,
        Tool::Helix => programs
            .helix
            .as_deref()
            .ok_or("this Eon package includes no Helix, so it provides no hx or eon-hx")?,
        Tool::Yazi | Tool::Files => &programs.yazi,
        Tool::Ya => &programs.ya,
        Tool::LazyGit => &programs.lazygit,
    })
}

/// `[editor] command`, or this build's default while it is unset: the editor's argv
/// before the file. None when it is unset in a build without a default.
pub(crate) fn editor_setting(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<Option<Vec<String>>, String> {
    resolve_editor(read_config(root, defaults, log)?.editor, defaults)
}

fn resolve_editor(
    settings: EditorSettings,
    defaults: &Defaults,
) -> Result<Option<Vec<String>>, String> {
    let argv = match settings.command {
        None => match defaults.editor {
            Some(argv) => argv.iter().map(|&argument| argument.into()).collect(),
            None => return Ok(None),
        },
        Some(PopupCommandSetting::Argv(argv)) => argv,
        Some(PopupCommandSetting::Named(_)) => {
            return Err(
                "editor.command must be a direct argv array, such as command = [\"nvim\"]".into(),
            );
        }
    };
    validate_argv("editor.command", &argv)?;
    Ok(Some(argv))
}

/// The editor's argv before the file, reread for each edit: `editor_setting`, else
/// `$VISUAL`, then `$EDITOR`, from this process's environment, which for an edit is the
/// supervisor's (EON-C10).
pub(crate) fn editor_command(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<Vec<OsString>, String> {
    editor_from_settings(read_config(root, defaults, log)?.editor, defaults)
}

fn editor_from_settings(
    settings: EditorSettings,
    defaults: &Defaults,
) -> Result<Vec<OsString>, String> {
    if let Some(argv) = resolve_editor(settings, defaults)? {
        return Ok(argv.into_iter().map(Into::into).collect());
    }
    let (variable, argv) = environment_editor(env::var_os("VISUAL"), env::var_os("EDITOR")).ok_or(
        "no editor: this Eon package has no Helix, and neither VISUAL nor EDITOR named an \
             editor when this window started; set [editor] command in config.toml, such as \
             command = [\"nvim\"]",
    )?;
    validate_argv(variable, &argv)?;
    Ok(argv)
}

/// Resolve one edit, using a live listener before preparing a future command.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_edit(
    root: &Path,
    defaults: &Defaults,
    programs: &ManagedPrograms,
    directory: &Path,
    file: &Path,
    live: Option<&Path>,
    available: bool,
    log: Option<&DiagnosticLog>,
) -> Result<super::editor::EditCommand, eon_workspace_protocol::v44::Failure> {
    use super::editor::{self, EditCommand, ListenerKind};
    let command_error = |detail| eon_workspace_protocol::v44::Failure {
        code: "edit-command".into(),
        detail,
    };
    let config = read_config(root, defaults, log).map_err(command_error)?;
    let popup = available
        && config.editor.open_in.unwrap_or(defaults.editor_open_in) == EditorOpenIn::Popup
        && config.popups.get("editor").and_then(|popup| popup.enabled) != Some(false)
        && !file.is_dir();
    if popup
        && let Some(endpoint) = live
        && let Some(kind) = editor::listener_kind(endpoint)?
    {
        if kind != ListenerKind::Neovim || !Path::new(editor::neovim_location(file)?.path).is_dir()
        {
            return Ok(EditCommand::Reuse(kind));
        }
        let command = prepare_editor(config.editor, defaults, programs, directory, false)
            .map_err(command_error)?;
        return Ok(EditCommand::Pane(command));
    }
    let popup = popup
        && !(live.is_none()
            && config.editor.kind.unwrap_or(defaults.editor_kind) == EditorKind::Neovim
            && Path::new(editor::neovim_location(file)?.path).is_dir());
    let mut command = prepare_editor(
        config.editor,
        defaults,
        programs,
        directory,
        popup && live.is_none(),
    )
    .map_err(command_error)?;
    if popup && live.is_none() && editor::initial_file(&mut command, file)? {
        return Ok(EditCommand::Popup(command));
    }
    Ok(EditCommand::Pane(command))
}

fn prepare_editor(
    settings: EditorSettings,
    defaults: &Defaults,
    programs: &ManagedPrograms,
    directory: &Path,
    owned_listener: bool,
) -> Result<PreparedCommand, String> {
    use super::editor::ListenerKind;
    let kind = settings.kind.unwrap_or(defaults.editor_kind);
    let mut argv = editor_from_settings(settings, defaults)?;
    let neovim =
        kind == EditorKind::Neovim || Path::new(&argv[0]).file_name() == Some(OsStr::new("nvim"));
    let mut listener = if owned_listener && neovim {
        match super::editor::neovim_position(&argv) {
            Ok(position) => Some((ListenerKind::Neovim, position)),
            Err(error) if kind == EditorKind::Neovim => return Err(error),
            Err(_) => None,
        }
    } else {
        None
    };
    argv[0] = popup_executable(&argv[0], programs.shell_bin.as_deref(), directory)?
        .ok_or_else(|| {
            format!(
                "executable {:?} is unavailable on Eon's terminal PATH",
                argv[0]
            )
        })?
        .into_os_string();
    if owned_listener && !neovim {
        listener = super::editor::listen_position(&argv, programs, directory)?
            .map(|position| (ListenerKind::Helix, position));
    }
    Ok(PreparedCommand { argv, listener })
}

/// The first of `$VISUAL` and `$EDITOR` that names an editor, with its name: the value
/// split on ASCII whitespace, with no shell. A blank value names none.
fn environment_editor(
    visual: Option<OsString>,
    editor: Option<OsString>,
) -> Option<(&'static str, Vec<OsString>)> {
    [("$VISUAL", visual), ("$EDITOR", editor)]
        .into_iter()
        .find_map(|(variable, value)| {
            let argv: Vec<OsString> = value?
                .as_bytes()
                .split(u8::is_ascii_whitespace)
                .filter(|word| !word.is_empty())
                .map(|word| OsStr::from_bytes(word).to_owned())
                .collect();
            (!argv.is_empty()).then_some((variable, argv))
        })
}

pub(crate) fn zoxide_enabled(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<bool, String> {
    Ok(read_shell_config(root, defaults, log)?.zoxide)
}

pub(crate) fn startup_animation(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<Option<StartupAnimation>, String> {
    resolve_animation(read_config(root, defaults, log)?.anima, defaults)
}

fn resolve_animation(
    settings: AnimationSettings,
    defaults: &Defaults,
) -> Result<Option<StartupAnimation>, String> {
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
    Ok(read_config(root, defaults, log)?
        .session
        .warning_close_last_tab
        .unwrap_or(defaults.warning_close_last_tab))
}

pub(crate) fn animate_working_badge(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<bool, String> {
    Ok(read_config(root, defaults, log)?
        .session
        .animate_working_badge
        .unwrap_or(defaults.animate_working_badge))
}

pub(crate) fn number_keys(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<bool, String> {
    Ok(read_config(root, defaults, log)?
        .session
        .number_keys
        .unwrap_or(defaults.number_keys))
}

pub(crate) fn sidebar_enabled(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<bool, String> {
    Ok(read_config(root, defaults, log)?
        .sidebar
        .enabled
        .unwrap_or(defaults.sidebar_enabled))
}

pub(crate) fn agents(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<Agents, String> {
    resolve_agents(read_config(root, defaults, log)?.agents, defaults)
}

fn resolve_agents(settings: AgentSettings, defaults: &Defaults) -> Result<Agents, String> {
    let delay = settings
        .send_enter_delay_ms
        .unwrap_or(defaults.send_enter_delay_ms.into());
    let quiet = settings
        .send_typing_quiet_ms
        .unwrap_or(defaults.send_typing_quiet_ms.into());
    Ok(Agents {
        send_typing_quiet_ms: u16::try_from(quiet)
            .ok()
            .filter(|quiet| *quiet <= MAX_TYPING_QUIET_MS)
            .ok_or_else(|| {
                format!("agents.send_typing_quiet_ms must be from 0 through {MAX_TYPING_QUIET_MS}")
            })?,
        terminal_access: settings.terminal_access.unwrap_or(defaults.terminal_access),
        send_enter_delay_ms: u16::try_from(delay)
            .ok()
            .filter(|delay| *delay <= MAX_ENTER_DELAY_MS)
            .ok_or_else(|| {
                format!("agents.send_enter_delay_ms must be from 0 through {MAX_ENTER_DELAY_MS}")
            })?,
    })
}

pub(crate) fn terminal_presentation(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<TerminalConfig, String> {
    let config = read_config(root, defaults, log)?;
    resolve_terminal(config.terminal, config.panels, defaults)
}

fn resolve_terminal(
    settings: TerminalSettings,
    panels: PanelSettings,
    defaults: &Defaults,
) -> Result<TerminalConfig, String> {
    let base = &defaults.terminal;
    let terminal = TerminalConfig {
        background_opacity: settings
            .background_opacity
            .unwrap_or(base.background_opacity),
        background_blur: settings.background_blur.unwrap_or(base.background_blur),
        padding_color: settings.padding_color.unwrap_or(base.padding_color),
        pane_frames: settings.pane_frames.unwrap_or(base.pane_frames),
        panel_heading: panels.heading.unwrap_or(base.panel_heading),
        detect_local_hosts: settings
            .detect_local_hosts
            .unwrap_or(base.detect_local_hosts),
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
    if terminal
        .cursor_trail_duration
        .is_some_and(|value| !value.is_finite() || (value != 0.0 && !(0.25..=4.0).contains(&value)))
    {
        return Err(
            "terminal.cursor_trail_duration must be 0 (off) or a finite number from 0.25 through 4.0"
                .into(),
        );
    }
    for (field, value, minimum, maximum) in [
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
    resolve_popups(read_config(root, defaults, log)?.popups, defaults)
}

fn resolve_popups(
    mut popups: BTreeMap<String, PopupEntrySettings>,
    defaults: &Defaults,
) -> Result<PopupCatalog, String> {
    // Popup margins are Eon's fixed values, not configuration (EON-C10).
    let geometry = defaults.popups.geometry;

    let mut entries = Vec::new();
    // Whether each entry's key came from the configuration rather than a default.
    let mut configured = Vec::new();
    for builtin in &defaults.popups.entries {
        let id = builtin.id.as_str();
        let mut settings = popups.remove(id).unwrap_or_default();
        // Every tab's Files pair is the packaged, enabled, keep-alive Yazi; a pending
        // tab starts on it (EON-C18). `config_warnings` names each setting ignored here.
        let ignored = id == "files" || builtin.command == PopupCommand::Editor;
        if popup_field_owned(id, "command", defaults) {
            if ignored {
                settings.command = None;
            } else if settings.command.is_some() {
                return Err(format!("popups.{id}.command is Eon-owned"));
            }
        }
        if popup_field_owned(id, "enabled", defaults) {
            if id == "files" {
                settings.enabled = None;
            } else if settings.enabled == Some(false) {
                return Err(
                    "popups.project.enabled cannot disable Eon's required quick search".into(),
                );
            }
        }
        if popup_field_owned(id, "keep_alive", defaults) {
            if ignored {
                settings.keep_alive = None;
            } else if settings.keep_alive == Some(true) {
                return Err(format!("popups.{id}.keep_alive must remain false"));
            }
        }
        if settings.enabled == Some(false) {
            continue;
        }
        let command = match settings.command {
            Some(command) => configured_popup_command(id, command)?,
            None => builtin.command.clone(),
        };
        configured.push(settings.keybinding.is_some());
        entries.push(PopupDefinition {
            id: id.into(),
            label: settings.label.unwrap_or_else(|| builtin.label.clone()),
            shortcut: match settings.keybinding {
                Some(keybinding) => popup_key(id, &keybinding)?,
                None => builtin.shortcut.clone(),
            },
            command,
            keep_alive: settings.keep_alive.unwrap_or(builtin.keep_alive),
        });
    }

    for (id, settings) in popups {
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
        configured.push(true);
        entries.push(PopupDefinition {
            label: settings.label.unwrap_or_else(|| id.clone()),
            shortcut: popup_key(&id, &keybinding)?,
            command: configured_popup_command(&id, command)?,
            keep_alive: settings
                .keep_alive
                .unwrap_or(defaults.custom_popup_keep_alive),
            id,
        });
    }
    resolve_popup_entries(&mut entries, &configured)?;
    Ok(PopupCatalog { geometry, entries })
}

/// An empty keybinding leaves the popup without a key.
fn popup_key(id: &str, keybinding: &str) -> Result<PopupKey, String> {
    if keybinding.is_empty() {
        return Ok(PopupKey::Unbound);
    }
    parse_shortcut(&format!("popups.{id}.keybinding"), keybinding).map(PopupKey::Bound)
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
            validate_argv(&format!("popups.{id}.command"), &argv)?;
            Ok(PopupCommand::Argv(
                argv.into_iter().map(Into::into).collect(),
            ))
        }
    }
}

fn validate_argv<S: AsRef<OsStr>>(key: &str, argv: &[S]) -> Result<(), String> {
    fn bytes(argument: &impl AsRef<OsStr>) -> &[u8] {
        argument.as_ref().as_bytes()
    }
    if argv.first().is_none_or(|program| bytes(program).is_empty()) {
        return Err(format!("{key} requires a nonempty executable"));
    }
    if argv.len() > 128 {
        return Err(format!("{key} accepts at most 128 arguments"));
    }
    if argv.iter().any(|argument| bytes(argument).contains(&0)) {
        return Err(format!("{key} must not contain NUL"));
    }
    if argv
        .iter()
        .map(|argument| bytes(argument).len())
        .sum::<usize>()
        > 64 * 1024
    {
        return Err(format!("{key} exceeds 64 KiB"));
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

/// Configured keys claim their shortcuts first; a built-in default then
/// yields to the configured popup holding it (EON-C10).
fn resolve_popup_entries(
    entries: &mut [PopupDefinition],
    configured: &[bool],
) -> Result<(), String> {
    if entries.len() > MAX_ENTRIES {
        return Err(format!(
            "popups accepts at most {MAX_ENTRIES} enabled entries"
        ));
    }
    for entry in entries.iter() {
        validate_popup_id(&entry.id)?;
        if entry.label.is_empty()
            || entry.label.len() > MAX_LABEL_BYTES
            || entry.label.chars().any(char::is_control)
        {
            return Err(format!(
                "popups.{}.label must be nonempty, at most {MAX_LABEL_BYTES} UTF-8 bytes, and contain no controls",
                entry.id
            ));
        }
    }
    // Chords are unique among configured keys and among defaults; two defaults
    // on one chord are a product bug even while a configured popup holds it.
    let mut configured_keys = HashMap::new();
    let mut default_keys = HashMap::new();
    for pass in [true, false] {
        for (entry, _) in entries
            .iter_mut()
            .zip(configured)
            .filter(|(_, configured)| **configured == pass)
        {
            let Some(shortcut) = entry.shortcut.bound().cloned() else {
                continue;
            };
            shortcut.validate().map_err(|_| {
                format!(
                    "popups.{}.keybinding is not a supported modified physical key",
                    entry.id
                )
            })?;
            let policy = popup_policy(shortcut.modifiers, &shortcut.key);
            // `config_warnings` names a configured popup on a focus key.
            if pass && policy == PopupPolicy::Focus {
                entry.shortcut = PopupKey::Unbound;
                continue;
            }
            if policy != PopupPolicy::Overridable {
                return Err(format!(
                    "popups.{}.keybinding is reserved for an Eon shortcut",
                    entry.id
                ));
            }
            let keys = if pass {
                &mut configured_keys
            } else {
                &mut default_keys
            };
            if let Some(previous) = keys.insert(shortcut.clone(), entry.id.clone()) {
                return Err(format!(
                    "popups.{}.keybinding conflicts with popups.{previous}.keybinding",
                    entry.id
                ));
            }
            if !pass && let Some(holder) = configured_keys.get(&shortcut) {
                entry.shortcut = PopupKey::Overridden {
                    default: shortcut,
                    by: holder.clone(),
                };
            }
        }
    }
    Ok(())
}

pub(crate) fn prepare_popup_command(
    command: &PopupCommand,
    programs: &ManagedPrograms,
    directory: &Path,
    config: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<PreparedCommand, String> {
    if *command == PopupCommand::Editor {
        return prepare_editor(
            read_config(config, defaults, log)?.editor,
            defaults,
            programs,
            directory,
            true,
        );
    }
    let argv = match command {
        // Project, which shows the tab's Files instead.
        PopupCommand::Argv(argv) if argv.is_empty() => {
            return Err("this popup has no command".into());
        }
        PopupCommand::Argv(argv) => argv.clone(),
        PopupCommand::Editor => unreachable!("Editor was resolved above"),
        // An unavailable or unresolved editor fails the edit; nothing falls back to Helix.
        PopupCommand::Edit(file) => {
            let mut argv = editor_command(config, defaults, log)?;
            argv.push(file.clone());
            argv
        }
        PopupCommand::Scrollback => {
            return Err("the supervisor builds Scrollback's command for each press".into());
        }
        PopupCommand::AgentAuto => {
            for candidate in defaults.agent_commands {
                let argv = candidate.iter().map(OsString::from).collect::<Vec<_>>();
                if popup_executable(&argv[0], programs.shell_bin.as_deref(), directory)?.is_some() {
                    return Ok(PreparedCommand {
                        argv,
                        listener: None,
                    });
                }
            }
            return Err("no supported Agent executable is available on Eon's terminal PATH; install codex, grok, opencode, pi, or claude, or configure popups.agent.command".into());
        }
    };
    popup_executable(&argv[0], programs.shell_bin.as_deref(), directory)?.ok_or_else(|| {
        format!(
            "executable {:?} is unavailable on Eon's terminal PATH",
            argv[0]
        )
    })?;
    Ok(PreparedCommand {
        argv,
        listener: None,
    })
}

fn popup_executable(
    program: &OsStr,
    session_bin: Option<&Path>,
    directory: &Path,
) -> Result<Option<PathBuf>, String> {
    let search_path = session_bin.map(session_path).transpose()?;
    let program = Path::new(program);
    let executable = |path: &Path| {
        fs::metadata(path).is_ok_and(|metadata| metadata.is_file())
            && rustix::fs::accessat(
                rustix::fs::CWD,
                path,
                rustix::fs::Access::EXEC_OK,
                rustix::fs::AtFlags::EACCESS,
            )
            .is_ok()
    };
    if program.is_absolute() || program.components().count() > 1 {
        let path = directory.join(program);
        return Ok(executable(&path).then_some(path));
    }
    Ok(search_path
        .or_else(|| env::var_os("PATH"))
        .as_deref()
        .map(env::split_paths)
        .into_iter()
        .flatten()
        .map(|path| directory.join(path).join(program))
        .find(|path| executable(path)))
}

/// The settings file in the configuration root.
pub(super) const CONFIG_FILE: &str = "config.toml";

/// Session identity, nesting markers and session credentials that agents export to
/// the commands they run, checked against Claude Code 2.1.303, Codex 0.166.0,
/// OpenCode 2.0.32, Pi coding-agent 1.4.0 and Grok 1.0.62. When Eon starts from an
/// agent's shell, its children must not inherit them: Claude Code reads an inherited
/// `CLAUDE_CODE_CHILD_SESSION` as a child session and saves no transcript, and the
/// messaging token is that session's credential. Settings a person sets on purpose,
/// such as `CLAUDE_CONFIG_DIR` or API keys, are not listed (EON-C9).
const AGENT_SESSION_VARIABLES: &[&str] = &[
    // Claude Code.
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_SESSION_ACCESS_TOKEN",
    "CLAUDE_SESSION_INGRESS_TOKEN_FILE",
    "CLAUDE_CODE_REMOTE_SESSION_ID",
    "CLAUDE_CODE_REMOTE_SESSION_UUID",
    "CLAUDE_CODE_CLOUD_SESSION_ID",
    "CLAUDE_CODE_BRIDGE_SESSION_ID",
    "CLAUDE_CODE_HOST_SESSION_ID",
    "CLAUDE_CODE_HOST_WORKTREE",
    "CLAUDE_CODE_HOST_WORKTREE_FENCE",
    // Claude Code and OpenCode.
    "AI_AGENT",
    // Codex.
    "CODEX_SESSION_ID",
    "CODEX_THREAD_ID",
    "CODEX_TOOL_CALL_ID",
    "CODEX_VERSION",
    "CODEX_PERMISSION_PROFILE",
    "CODEX_CI",
    // OpenCode.
    "OPENCODE",
    "OPENCODE_SESSION_ID",
    "OPENCODE_TERMINAL",
    // Pi.
    "PI_CODING_AGENT",
    "PI_SESSION_ID",
    "PI_SESSION_FILE",
    "PI_REASONING_LEVEL",
    // Grok, in the commands its hooks run.
    "GROK_SESSION_ID",
];

/// Generic settings an agent's tool runner gives every command it runs, with the
/// exact value it sets, under the marker that runner sets beside them (always `1`).
/// A child loses one only when the supervisor inherited both the marker and that
/// exact value (EON-C9); a person's own different value passes through. Claude Code
/// 2.1.303's Bash tool sets `GIT_EDITOR`, which leaves a bare `git commit` with an
/// empty message, and its process sets `COREPACK_ENABLE_AUTO_PIN`. Codex 0.166.0's
/// unified exec also sets `TERM` and `COLORTERM`, which Orbit sets for each
/// terminal, and the locale in `CODEX_LOCALE`.
const AGENT_TOOL_SETTINGS: &[(&str, &[(&str, &str)])] = &[
    (
        "CLAUDECODE",
        &[("GIT_EDITOR", "true"), ("COREPACK_ENABLE_AUTO_PIN", "0")],
    ),
    (
        "CODEX_CI",
        &[
            ("NO_COLOR", "1"),
            ("PAGER", "cat"),
            ("GIT_PAGER", "cat"),
            ("GH_PAGER", "cat"),
        ],
    ),
];

/// The locale names Codex 0.166.0's unified exec sets to `C.UTF-8` under `CODEX_CI`,
/// replacing the person's. A child gets the desktop session's value back; without
/// one it loses the name, except `LANG`, which stays so the locale remains UTF-8.
const CODEX_LOCALE: &[&str] = &["LANG", "LC_CTYPE", "LC_ALL"];

/// Keeps the launching agent's session, the settings its tool runner adds, and Eon's
/// delegation markers (EON-C11) out of a child the supervisor starts.
pub(crate) fn remove_agent_session(command: &mut Command) -> &mut Command {
    remove_agent_environment(command, |name| env::var_os(name), desktop_locale)
}

fn remove_agent_environment(
    command: &mut Command,
    inherited: impl Fn(&str) -> Option<OsString>,
    desktop: impl Fn(&str) -> Option<OsString>,
) -> &mut Command {
    let set = |name: &str, value: &str| inherited(name).is_some_and(|inherited| inherited == value);
    for (marker, settings) in AGENT_TOOL_SETTINGS {
        if set(marker, "1") {
            for (name, value) in *settings {
                if set(name, value) {
                    command.env_remove(name);
                }
            }
        }
    }
    if set("CODEX_CI", "1") {
        for name in CODEX_LOCALE {
            if set(name, "C.UTF-8") {
                match desktop(name) {
                    Some(value) => {
                        command.env(name, value);
                    }
                    None if *name != "LANG" => {
                        command.env_remove(name);
                    }
                    None => {}
                }
            }
        }
    }
    for name in AGENT_SESSION_VARIABLES.iter().chain(&[
        super::generation::DELEGATION_MARKER,
        super::generation::ROUTE_MARKER,
    ]) {
        command.env_remove(name);
    }
    command
}

/// The desktop session's value of a locale name: the systemd user manager's
/// environment, read once per supervisor and only when a child needs it. Best effort;
/// without systemd, or without its answer within a second, there is none.
fn desktop_locale(name: &str) -> Option<OsString> {
    static ENVIRONMENT: OnceLock<String> = OnceLock::new();
    ENVIRONMENT
        .get_or_init(|| {
            user_manager_environment(OsStr::new("systemctl"), Duration::from_secs(1))
                .unwrap_or_default()
        })
        .lines()
        .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
        .map(OsString::from)
}

/// The most `systemctl --user show-environment` may print, far above a real user
/// manager's environment.
const MAX_USER_MANAGER_BYTES: usize = 4 << 20;

/// Runs `systemctl --user show-environment` in its own process group, draining its
/// output as it comes. The group is killed and reaped by the deadline whether or not it
/// answered, so a stalled bus leaves no process, thread or socket behind; output past
/// `MAX_USER_MANAGER_BYTES` counts as no answer.
fn user_manager_environment(program: &OsStr, timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(["--user", "--no-pager", "show-environment"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let mut output = child.stdout.take().expect("piped user manager output");
    let answer = (|| {
        let flags = rustix::fs::fcntl_getfl(&output).ok()?;
        rustix::fs::fcntl_setfl(&output, flags | rustix::fs::OFlags::NONBLOCK).ok()?;
        let deadline = Instant::now() + timeout;
        let mut bytes = Vec::new();
        let mut eof = false;
        while Instant::now() < deadline {
            let mut chunk = [0; 8192];
            match output.read(&mut chunk) {
                Ok(0) => eof = true,
                Ok(count) => {
                    bytes.extend_from_slice(&chunk[..count]);
                    if bytes.len() > MAX_USER_MANAGER_BYTES {
                        return None;
                    }
                    continue;
                }
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return None,
            }
            if eof && let Some(status) = child.try_wait().ok()? {
                return status
                    .success()
                    .then(|| String::from_utf8_lossy(&bytes).into_owned());
            }
            thread::sleep(Duration::from_millis(5));
        }
        None
    })();
    let _ = rustix::process::kill_process_group(
        rustix::process::Pid::from_child(&child),
        rustix::process::Signal::KILL,
    );
    let _ = child.wait();
    answer
}

pub(crate) fn session_path(prefix: &Path) -> Result<OsString, String> {
    prepend_path(prefix, env::var_os("PATH").as_deref(), "Eon terminal")
}

/// `quick` when the Files pair starts on quick search (EON-C18).
pub(crate) const FILES_START_VARIABLE: &str = "EON_FILES_START";
/// The socket the pair's quick search listens on, for the runtime's screen switches.
pub(crate) const PAIR_SOCKET_VARIABLE: &str = "EON_PAIR_SOCKET";
/// The startup pair's "other windows" notice, its first quick search's header.
pub(crate) const FILES_NOTICE_VARIABLE: &str = "EON_FILES_NOTICE";

/// Applies the runtime's pair launch flags, ahead of Yazi's own arguments, to
/// the Files `command`, which inherits no other pair's, and returns the rest.
fn pair_launch_flags<'a>(command: &mut Command, mut arguments: &'a [OsString]) -> &'a [OsString] {
    command
        .env_remove(FILES_START_VARIABLE)
        .env_remove(PAIR_SOCKET_VARIABLE)
        .env_remove(FILES_NOTICE_VARIABLE);
    loop {
        match arguments {
            [flag, rest @ ..] if flag == "--eon-quick" => {
                command.env(FILES_START_VARIABLE, "quick");
                arguments = rest;
            }
            [flag, socket, rest @ ..] if flag == "--eon-pair-socket" => {
                command.env(PAIR_SOCKET_VARIABLE, socket);
                arguments = rest;
            }
            _ => return arguments,
        }
    }
}

pub(crate) fn command(
    tool: Tool,
    programs: &ManagedPrograms,
    config: &Path,
    mut arguments: &[OsString],
    defaults: &Defaults,
) -> Result<Command, String> {
    // The supervisor marks only the default shell it starts, after warning about unknown
    // keys in its log; stderr here is that terminal's scrollback. Every other launch warns.
    let warned = env::var_os(CONFIG_WARNED_VARIABLE).is_some();
    let shell = match tool {
        Tool::Nu | Tool::Bash if warned => Some(shell_config(
            parse_config(config, defaults)?.shell,
            defaults,
        )?),
        Tool::Nu | Tool::Bash => Some(read_shell_config(config, defaults, None)?),
        _ => None,
    };
    let mut command = Command::new(program(tool, programs)?);
    // Nothing started inside the terminal inherits the one-shot marker, or
    // the delegation and routing markers, which name one delegated attach or
    // routed command (EON-C11).
    command.env_remove(CONFIG_WARNED_VARIABLE);
    command.env_remove(super::generation::DELEGATION_MARKER);
    command.env_remove(super::generation::ROUTE_MARKER);
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
            // popup's tab directory and to open files in editor panes.
            let eon = env::current_exe()
                .map_err(|error| format!("cannot find the running Eon executable: {error}"))?;
            // Yazi prefers PWD even when it disagrees with the terminal's actual CWD.
            command
                .env("YAZI_CONFIG_HOME", files)
                .env("EON_FILES_CLI", eon)
                .env_remove("PWD");
            arguments = pair_launch_flags(&mut command, arguments);
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
                .env_remove("HELIX_RUNTIME");
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
    shell_config(read_config(root, defaults, log)?.shell, defaults)
}

fn shell_config(settings: ShellSettings, defaults: &Defaults) -> Result<ShellConfig, String> {
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

/// A configuration key a release deprecated, read as its replacement without
/// rewriting the file (eon-c8ytm). An entry may go, with its corpus fixtures,
/// only once the migration floor is above `deprecated_in`, never while it
/// equals it.
#[derive(Debug)]
struct Alias {
    old: &'static str,
    new: &'static str,
    /// The release that first warned about, or stopped accepting, `old`.
    deprecated_in: &'static str,
    /// The value for `new` given the value of `old`, or `None` to drop `old`
    /// without a replacement.
    translate: fn(&toml::Value, &Defaults) -> Result<Option<toml::Value>, String>,
}

#[rustfmt::skip]
const ALIASES: &[Alias] = &[
    Alias { old: "bar.show_version", new: "bar.widgets", deprecated_in: "v0.1.2", translate: show_version },
];

/// A configuration table a release renamed. The old table has no effect and
/// warns once, naming its replacement, rather than being read as an alias
/// (docs/AGENT-RULES.md#configurability). An entry goes with its corpus
/// fixture under the same migration floor rule as `Alias`.
#[derive(Debug)]
struct Tombstone {
    old: &'static str,
    new: &'static str,
    /// The release that stopped reading `old`.
    renamed_in: &'static str,
}

#[rustfmt::skip]
const TOMBSTONES: &[Tombstone] = &[
    Tombstone { old: "workspace", new: "session", renamed_in: "v0.1.20" },
    Tombstone { old: "popups.workspaces", new: "popups.sessions", renamed_in: "v0.1.20" },
];

/// Removes each tombstoned table from `table` and returns those it found.
fn remove_tombstoned(table: &mut toml::Table) -> Vec<&'static Tombstone> {
    TOMBSTONES
        .iter()
        .filter(|tombstone| {
            let (path, name) = tombstone
                .old
                .rsplit_once('.')
                .unwrap_or(("", tombstone.old));
            let mut parent = Some(&mut *table);
            for part in path.split('.').filter(|part| !part.is_empty()) {
                parent = parent
                    .and_then(|table| table.get_mut(part))
                    .and_then(|value| value.as_table_mut());
            }
            parent.is_some_and(|table| table.remove(name).is_some())
        })
        .collect()
}

/// `show_version = false` hid the version badge, which `[bar] widgets` now lists.
fn show_version(value: &toml::Value, defaults: &Defaults) -> Result<Option<toml::Value>, String> {
    let shown = value
        .as_bool()
        .ok_or("bar.show_version must be true or false")?;
    let widgets = defaults.bar_widgets.iter().copied();
    Ok(Some(
        widgets
            .filter(|widget| shown || *widget != "version")
            .collect::<Vec<_>>()
            .into(),
    ))
}

/// Reads each deprecated key in `table` as its replacement, unless the
/// replacement is set too, and returns the aliases it applied. A value that
/// cannot be translated fails even when the replacement wins.
fn translate_deprecated<'a>(
    table: &mut toml::Table,
    aliases: &'a [Alias],
    defaults: &Defaults,
) -> Result<Vec<(&'a Alias, bool)>, String> {
    // The table holding a dotted key, creating missing tables when `create`.
    fn parent<'a>(
        mut table: &'a mut toml::Table,
        key: &'static str,
        create: bool,
    ) -> Option<(&'a mut toml::Table, &'static str)> {
        let (path, name) = key.rsplit_once('.').unwrap_or(("", key));
        for part in path.split('.').filter(|part| !part.is_empty()) {
            if create {
                table
                    .entry(part)
                    .or_insert_with(|| toml::Table::new().into());
            }
            table = table.get_mut(part)?.as_table_mut()?;
        }
        Some((table, name))
    }
    let mut applied = Vec::new();
    for alias in aliases {
        let Some(value) =
            parent(table, alias.old, false).and_then(|(table, name)| table.remove(name))
        else {
            continue;
        };
        let value = (alias.translate)(&value, defaults)?;
        let replaced =
            parent(table, alias.new, false).is_some_and(|(table, name)| table.contains_key(name));
        if !replaced
            && let Some(value) = value
            && let Some((table, name)) = parent(table, alias.new, true)
        {
            table.insert(name.into(), value);
        }
        applied.push((alias, replaced));
    }
    Ok(applied)
}

/// Parses the configuration and warns once per unknown, deprecated or renamed key:
/// through `log`, which remembers what it has noted, else on stderr.
fn read_config(
    root: &Path,
    defaults: &Defaults,
    log: Option<&DiagnosticLog>,
) -> Result<EonConfig, String> {
    let config = parse_config(root, defaults)?;
    for warning in config_warnings(&config, &root.join(CONFIG_FILE), defaults) {
        if let Some(log) = log {
            log.note_once(&warning);
        } else {
            let _ = writeln!(io::stderr(), "{warning}");
        }
    }
    Ok(config)
}

/// The warnings the configuration gives, as `read_config` would print them, without printing
/// them or preparing any directory.
pub(super) fn configuration_warnings(defaults: &Defaults) -> Result<Vec<String>, String> {
    let root = super::supervisor::configuration_directory()?;
    let config = parse_config(&root, defaults)?;
    Ok(config_warnings(&config, &root.join(CONFIG_FILE), defaults))
}

/// One warning per unknown, deprecated or renamed key of the configuration read from `path`,
/// and per popup it configures on a focus key.
fn config_warnings(config: &EonConfig, path: &Path, defaults: &Defaults) -> Vec<String> {
    let path = path.to_string_lossy();
    let path = path.escape_debug();
    let unknown = config.unknown_keys().into_iter().map(|key| {
        format!(
            "eon: ignoring unknown setting {} in {path}",
            key.escape_debug()
        )
    });
    let deprecated = config.deprecated.iter().map(|(alias, replaced)| {
        if *replaced {
            format!(
                "eon: ignoring deprecated setting {} in {path}; {} is set",
                alias.old, alias.new
            )
        } else {
            format!(
                "eon: {} in {path} is deprecated since {}; use {}",
                alias.old, alias.deprecated_in, alias.new
            )
        }
    });
    let tombstoned = config.tombstoned.iter().map(|tombstone| {
        format!(
            "eon: ignoring [{}] in {path}: renamed to [{}] in {}",
            tombstone.old, tombstone.new, tombstone.renamed_in
        )
    });
    let widgets = config
        .bar
        .widgets
        .iter()
        .flatten()
        .filter(|name| !bar_widget(name))
        .map(|name| format!("eon: ignoring unknown bar.widgets name {name:?} in {path}"));
    // Files is always the packaged Yazi (EON-C18), which earlier releases let a
    // configuration replace, disable or stop when hidden.
    let files = config.popups.get("files").into_iter().flat_map(|settings| {
        [
            settings.command.is_some().then_some("command"),
            (settings.enabled == Some(false)).then_some("enabled"),
            (settings.keep_alive == Some(false)).then_some("keep_alive"),
        ]
        .into_iter()
        .flatten()
        .filter(|key| popup_field_owned("files", key, defaults))
        .map(|key| {
            format!(
                "eon: ignoring popups.files.{key} in {path}: Files is Eon's packaged Yazi; move a custom Yazi to its own popup, such as [popups.yazi] command = [\"eon-yazi\"]"
            )
        })
    });
    let editor = defaults
        .popups
        .entries
        .iter()
        .find(|entry| entry.command == PopupCommand::Editor)
        .and_then(|entry| config.popups.get(&entry.id).map(|settings| (&entry.id, settings)))
        .into_iter()
        .flat_map(|(id, settings)| {
            [
                settings.command.is_some().then_some("command"),
                settings.keep_alive.is_some().then_some("keep_alive"),
            ]
            .into_iter()
            .flatten()
            .filter(move |key| popup_field_owned(id, key, defaults))
            .map(|key| {
                format!(
                    "eon: ignoring popups.editor.{key} in {path}: Editor keeps running and uses [editor] command"
                )
            })
        });
    let focus = config.popups.iter().filter_map(|(id, settings)| {
        let keybinding = settings.keybinding.as_deref()?;
        ((settings.enabled != Some(false) || id == "files")
            && parse_shortcut("", keybinding).is_ok_and(|shortcut| {
                popup_policy(shortcut.modifiers, &shortcut.key) == PopupPolicy::Focus
            }))
        .then(|| {
            format!(
                "eon: popups.{id}.keybinding {} in {path} is reserved for moving focus; popup {id} loads without a key",
                keybinding.escape_debug(),
                id = id.escape_debug()
            )
        })
    });
    unknown
        .chain(widgets)
        .chain(deprecated)
        .chain(tombstoned)
        .chain(files)
        .chain(editor)
        .chain(focus)
        .collect()
}

/// Parses the configuration, reading deprecated keys as their replacements
/// and dropping tombstoned tables, without warning about any of them.
fn parse_config(root: &Path, defaults: &Defaults) -> Result<EonConfig, String> {
    let path = root.join(CONFIG_FILE);
    parse_configuration_text(&read_configuration(&path)?, &path, defaults)
}

/// Read without creating directories; a missing file selects the build's defaults.
pub(super) fn read_configuration(path: &Path) -> Result<String, String> {
    match fs::read_to_string(path) {
        Ok(source) => Ok(source),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("cannot read {}: {error}", path.display())),
    }
}

fn invalid_configuration_error(path: &Path, error: impl std::fmt::Display) -> String {
    format!("invalid Eon configuration {}: {error}", path.display())
}

/// Read `[session] number_keys` from `path` without creating directories or
/// printing unrelated configuration warnings (EON-C10).
pub(super) fn read_number_keys(
    path: &Path,
    defaults: &Defaults,
) -> Result<bool, (&'static str, String)> {
    let source = read_configuration(path).map_err(|detail| ("unreadable-configuration", detail))?;
    let config = parse_configuration_text(&source, path, defaults)
        .map_err(|detail| ("invalid-configuration", detail))?;
    let number_keys = config.session.number_keys.unwrap_or(defaults.number_keys);
    config.validate(defaults).map_err(|error| {
        (
            "invalid-configuration",
            invalid_configuration_error(path, error),
        )
    })?;
    Ok(number_keys)
}

/// Validate one candidate without reading a file, emitting warnings or starting children.
pub(super) fn validate_configuration_text(
    source: &str,
    path: &Path,
    defaults: &Defaults,
) -> Result<(), String> {
    parse_configuration_text(source, path, defaults)?
        .validate(defaults)
        .map_err(|error| invalid_configuration_error(path, error))
}

fn parse_configuration_text(
    source: &str,
    path: &Path,
    defaults: &Defaults,
) -> Result<EonConfig, String> {
    let invalid = |error: String| invalid_configuration_error(path, error);
    let mut table: toml::Table =
        toml::from_str(source).map_err(|error: toml::de::Error| invalid(error.to_string()))?;
    // A tombstoned table goes before any typed read, so nothing in it can fail.
    let tombstoned = remove_tombstoned(&mut table);
    // Parsing the text keeps each error's line and column; a deprecated key
    // lands among the unknown keys until the translation below replaces it.
    let parsed: Option<EonConfig> = if tombstoned.is_empty() {
        Some(toml::from_str(source).map_err(|error: toml::de::Error| invalid(error.to_string()))?)
    } else {
        None
    };
    let deprecated = translate_deprecated(&mut table, ALIASES, defaults).map_err(invalid)?;
    let mut config = match parsed {
        Some(config) if deprecated.is_empty() => config,
        // A dropped or translated key leaves the table to read, whose errors
        // have no line or column.
        _ => table
            .try_into()
            .map_err(|error: toml::de::Error| invalid(error.to_string()))?,
    };
    config.deprecated = deprecated;
    config.tombstoned = tombstoned;
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
        ALT, ManagedPrograms, PopupCommand, PopupDefinition, PopupKey, SHIFT, Shortcut, Tool,
        animate_working_badge, command as managed_command, disk_path, integration_mask,
        number_keys, pair_launch_flags, popup_catalog, prepare_popup_command, prepend_path,
        print_defaults, read_shell_config, remove_agent_environment, remove_agent_session,
        sidebar_enabled, startup_animation, terminal_presentation, tool, user_manager_environment,
        warning_close_last_tab,
    };
    use std::{
        ffi::{OsStr, OsString},
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        process::Command,
        thread,
        time::{Duration, Instant},
    };

    fn schema_set(document: &mut serde_json::Value, path: &str, value: serde_json::Value) {
        let path = path.replace("<id>", "schema_fixture");
        let mut parts = path.split('.').peekable();
        let mut table = document;
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                table[part] = value;
                return;
            }
            table = table
                .as_object_mut()
                .unwrap()
                .entry(part)
                .or_insert_with(|| serde_json::json!({}));
        }
    }

    fn schema_sample(key: &super::ConfigKey) -> serde_json::Value {
        if !key.default.is_null() {
            return key.default.clone();
        }
        match key.kind {
            "boolean" => serde_json::json!(true),
            "integer" | "number" => key
                .bounds
                .get("minimum")
                .cloned()
                .unwrap_or(serde_json::json!(1)),
            "string-array" | "command" => serde_json::json!(["fixture"]),
            "string" if key.path.ends_with("keybinding") => serde_json::json!(""),
            "string" if key.path.starts_with("updates.") => serde_json::json!("/fixture"),
            "string" if key.path.ends_with("color") => serde_json::json!("custom:#123456"),
            "string" => serde_json::json!("fixture"),
            kind => panic!("unsupported schema kind {kind}"),
        }
    }

    #[test]
    fn configuration_schema_matches_derived_defaults_and_every_key_parses() {
        fn flatten(
            path: &str,
            value: &serde_json::Value,
            fields: &mut std::collections::BTreeMap<String, serde_json::Value>,
        ) {
            if let Some(table) = value.as_object() {
                for (key, value) in table {
                    flatten(
                        &if path.is_empty() {
                            key.clone()
                        } else {
                            format!("{path}.{key}")
                        },
                        value,
                        fields,
                    );
                }
            } else {
                fields.insert(path.into(), value.clone());
            }
        }
        let mut defaults = crate::fixtures::inputs().defaults;
        // Include both owned Editor fields and a build without a default editor.
        defaults.popups.entries.push(super::PopupDefinition {
            id: "editor".into(),
            label: "Editor".into(),
            shortcut: super::PopupKey::Unbound,
            command: super::PopupCommand::Editor,
            keep_alive: true,
        });
        for no_editor in [false, true] {
            if no_editor {
                defaults.editor = None;
                defaults.terminal.font_size = Some(19.0);
                defaults.shell.starship = false;
                defaults.custom_popup_keep_alive = false;
            }
            let keys = super::schema_keys(&defaults).unwrap();
            let mut paths = std::collections::HashSet::new();
            let mut actual = std::collections::BTreeMap::new();
            let mut editable = std::collections::BTreeMap::new();
            let mut document = serde_json::json!({});
            for key in &keys {
                assert!(paths.insert(&key.path), "duplicate {}", key.path);
                assert!(
                    !key.description.is_empty() && !key.takes_effect.is_empty(),
                    "{}",
                    key.path
                );
                if !key.path.contains("<id>") {
                    actual.insert(key.path.clone(), key.default.clone());
                    if !key.eon_owned {
                        editable.insert(key.path.clone(), key.default.clone());
                    }
                }
                if !key.eon_owned {
                    schema_set(&mut document, &key.path, schema_sample(key));
                }
            }
            assert_eq!(
                keys.iter().filter(|key| key.path.contains("<id>")).count(),
                5
            );
            let mut expected = std::collections::BTreeMap::new();
            flatten(
                "",
                &super::all_defaults_config(&defaults).unwrap(),
                &mut expected,
            );
            assert_eq!(actual, expected);
            expected.clear();
            flatten(
                "",
                &super::defaults_config(&defaults).unwrap(),
                &mut expected,
            );
            assert_eq!(editable, expected);
            let source = toml::to_string(&document).unwrap();
            let path = Path::new("/absent/config.toml");
            let parsed = super::parse_configuration_text(&source, path, &defaults).unwrap();
            assert!(super::config_warnings(&parsed, path, &defaults).is_empty());
            // Both installation targets cannot coexist, but every key must parse.
            document["updates"]
                .as_object_mut()
                .unwrap()
                .remove("bundle_prefix");
            super::validate_configuration_text(
                &toml::to_string(&document).unwrap(),
                path,
                &defaults,
            )
            .unwrap();
        }
    }

    #[test]
    fn project_configuration_errors_preserve_required_popup_priority() {
        let error = super::validate_configuration_text(
            "[popups.project]\nenabled = false\nkeep_alive = true",
            Path::new("/absent/config.toml"),
            &crate::fixtures::inputs().defaults,
        )
        .unwrap_err();
        assert!(
            error.contains("cannot disable Eon's required quick search"),
            "{error}"
        );
    }

    #[test]
    fn read_number_keys_validates_semantic_configuration_errors() {
        let root = crate::supervisor::temporary_directory();
        let config_path = root.join("config.toml");
        let defaults = crate::fixtures::inputs().defaults;

        assert_eq!(
            super::read_number_keys(&config_path, &defaults).unwrap(),
            defaults.number_keys
        );

        fs::write(&config_path, "[session]\nnumber_keys = false\n").unwrap();
        assert!(!super::read_number_keys(&config_path, &defaults).unwrap());

        for invalid in [
            "[terminal]\nfont_size = 0\n",
            "[popups.project]\nenabled = false\n",
        ] {
            fs::write(&config_path, invalid).unwrap();
            let (code, detail) = super::read_number_keys(&config_path, &defaults).unwrap_err();
            assert_eq!(code, "invalid-configuration", "{invalid}: {detail}");
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_schema_custom_popup_defaults_match_resolution() {
        let mut defaults = crate::fixtures::inputs().defaults;
        let id = "schema_fixture";
        let source = format!("[popups.{id}]\ncommand = ['fixture']\nkeybinding = ''");
        let path = Path::new("/absent/config.toml");
        for keep_alive in [false, true] {
            defaults.custom_popup_keep_alive = keep_alive;
            let keys = super::schema_keys(&defaults).unwrap();
            let key = |field: &str| {
                keys.iter()
                    .find(|key| key.path == format!("popups.<id>.{field}"))
                    .unwrap()
            };
            let parsed = super::parse_configuration_text(&source, path, &defaults).unwrap();
            let resolved = super::resolve_popups(parsed.popups, &defaults).unwrap();
            let popup = resolved.entries.iter().find(|popup| popup.id == id);
            assert_eq!(key("enabled").default, serde_json::json!(popup.is_some()));
            let popup = popup.unwrap();
            assert_eq!(
                key("keep_alive").default,
                serde_json::json!(popup.keep_alive)
            );
            assert_eq!(key("label").default.as_str().unwrap_or(id), popup.label);
            // A minimal custom popup supplies these fields because neither has a default.
            for (field, line) in [
                ("command", "command = ['fixture']\n"),
                ("keybinding", "keybinding = ''"),
            ] {
                assert!(key(field).default.is_null());
                let without = source.replace(line, "");
                let error =
                    super::validate_configuration_text(&without, path, &defaults).unwrap_err();
                assert!(
                    error.contains(&format!("popups.{id}.{field} is required")),
                    "{error}"
                );
            }
        }
    }

    #[test]
    fn configuration_schema_choices_cover_supported_values() {
        let defaults = crate::fixtures::inputs().defaults;
        let schema: serde_json::Value =
            serde_json::from_str(&super::print_schema(&defaults).unwrap()).unwrap();
        let widgets = schema["keys"]
            .as_array()
            .unwrap()
            .iter()
            .find(|key| key["path"] == "bar.widgets")
            .unwrap();
        assert_eq!(
            widgets["element_forms"],
            serde_json::json!([
                {"prefix":"google_quota:","suffix":":5h","min_length":1,"max_length":32,"control_free":true},
                {"prefix":"google_quota:","suffix":":7d","min_length":1,"max_length":32,"control_free":true}
            ])
        );
        fn variants<T: serde::Deserialize<'static>>() -> &'static [&'static str] {
            #[derive(Debug)]
            struct Variants(&'static [&'static str]);
            impl std::fmt::Display for Variants {
                fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str("enum variant probe")
                }
            }
            impl std::error::Error for Variants {}
            impl serde::de::Error for Variants {
                fn custom<T: std::fmt::Display>(message: T) -> Self {
                    panic!("unexpected enum deserialization error: {message}")
                }
                fn unknown_variant(_: &str, expected: &'static [&'static str]) -> Self {
                    Self(expected)
                }
            }
            // Capture serde's accepted variants without parsing diagnostic text or copying enums.
            match T::deserialize(serde::de::value::StrDeserializer::<Variants>::new(
                "__schema_unknown_variant__",
            )) {
                Err(Variants(expected)) if !expected.is_empty() => expected,
                _ => panic!("enum probe did not return its supported variants"),
            }
        }
        let defaults = crate::fixtures::inputs().defaults;
        let keys = super::schema_keys(&defaults).unwrap();
        for (path, supported) in [
            ("terminal.padding_color", variants::<super::PaddingColor>()),
            ("panels.heading", variants::<super::PanelHeading>()),
            (
                "agents.terminal_access",
                variants::<super::TerminalAccess>(),
            ),
            ("editor.kind", variants::<super::EditorKind>()),
            ("editor.open_in", variants::<super::EditorOpenIn>()),
            ("bar.widgets", &super::BAR_WIDGETS[..]),
            ("popups.agent.command", &["auto"] as &[&str]),
        ] {
            let key = keys.iter().find(|key| key.path == path).unwrap();
            let mut actual: Vec<_> = key
                .choices
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect();
            let mut expected = supported.to_vec();
            actual.sort_unstable();
            expected.sort_unstable();
            assert_eq!(actual, expected, "{path}");
        }
        let mut reduced = defaults;
        reduced.bar_widgets = &["clock"];
        let reduced_keys = super::schema_keys(&reduced).unwrap();
        let widgets = keys.iter().find(|key| key.path == "bar.widgets").unwrap();
        let reduced_widgets = reduced_keys
            .iter()
            .find(|key| key.path == "bar.widgets")
            .unwrap();
        assert_eq!(widgets.choices, reduced_widgets.choices);
        assert_eq!(widgets.element_forms, reduced_widgets.element_forms);
    }

    #[test]
    fn configuration_schema_bounds_and_choices_match_validation() {
        use serde_json::{Value, json};
        let defaults = crate::fixtures::inputs().defaults;
        let probe = |key: &super::ConfigKey, value: Value| {
            let mut document = json!({});
            if key.path.starts_with("popups.<id>.") {
                document["popups"] =
                    json!({"schema_fixture":{"command":["fixture"],"keybinding":""}});
            }
            if key.path.starts_with("bar.cpu_temperature_") {
                document["bar"] = json!({"cpu_temperature_warning_celsius":30,"cpu_temperature_critical_celsius":120});
            }
            if matches!(
                key.path.as_str(),
                "updates.nix_profile" | "updates.nix_entry"
            ) {
                document["updates"] = json!({"nix_profile":"/profile","nix_entry":"entry"});
            }
            schema_set(&mut document, &key.path, value);
            let source = toml::to_string(&document).unwrap();
            let path = Path::new("/absent/config.toml");
            super::validate_configuration_text(&source, path, &defaults).and_then(|()| {
                let parsed = super::parse_configuration_text(&source, path, &defaults)?;
                let warnings = super::config_warnings(&parsed, path, &defaults);
                if warnings.is_empty() {
                    Ok(())
                } else {
                    Err(warnings.join("\n"))
                }
            })
        };
        for key in super::schema_keys(&defaults)
            .unwrap()
            .iter()
            .filter(|key| !key.eon_owned)
        {
            for (bound, endpoint) in key.bounds.as_object().unwrap() {
                let lower = bound.contains("min");
                let limit = endpoint.as_f64().unwrap();
                let outside = if key.kind == "number" {
                    let endpoint = limit as f32;
                    f64::from(if lower {
                        endpoint.next_down()
                    } else {
                        endpoint.next_up()
                    })
                } else {
                    limit + if lower { -1.0 } else { 1.0 }
                };
                for (number, valid) in [(limit, true), (outside, false)] {
                    if number < 0.0 && key.kind != "number" && bound != "minimum" {
                        continue;
                    }
                    let value = match bound.as_str() {
                        "minimum" | "maximum" if key.kind == "integer" => json!(number as i64),
                        "minimum" | "maximum" => json!(number),
                        "min_length" | "max_length" => json!(if key.path.starts_with("updates.")
                            && key.path != "updates.nix_entry"
                        {
                            "/".repeat(number as usize)
                        } else {
                            "x".repeat(number as usize)
                        }),
                        "min_items" | "max_items" => json!(vec!["x"; number as usize]),
                        "item_min_length" | "item_max_length" => {
                            json!(["x".repeat(number as usize)])
                        }
                        bound => panic!("unknown bound {bound}"),
                    };
                    let result = probe(key, value.clone());
                    assert_eq!(
                        result.is_ok(),
                        valid,
                        "{} {bound} {value}: {result:?}",
                        key.path
                    );
                }
            }
            for value in &key.choices {
                let value = if key.kind == "string-array" {
                    json!([value])
                } else {
                    value.clone()
                };
                assert!(probe(key, value).is_ok(), "{}", key.path);
            }
            if key.path == "bar.widgets" {
                let cases: Value = serde_json::from_str(include_str!(
                    "../../eon-workspace-protocol/testdata/google-quota-selectors.json"
                ))
                .unwrap();
                for case in cases.as_array().unwrap() {
                    let name = case["selector"].as_str().unwrap();
                    let declared = key.choices.contains(&json!(name))
                        || key.element_forms.iter().any(|form| {
                            name.strip_prefix(form["prefix"].as_str().unwrap())
                                .and_then(|rest| {
                                    rest.strip_suffix(form["suffix"].as_str().unwrap())
                                })
                                .is_some_and(|scope| {
                                    scope.len() >= form["min_length"].as_u64().unwrap() as usize
                                        && scope.len()
                                            <= form["max_length"].as_u64().unwrap() as usize
                                        && (!form["control_free"].as_bool().unwrap()
                                            || !scope.chars().any(char::is_control))
                                })
                        });
                    assert_eq!(declared, case["scope"].is_string(), "{case}");
                    assert_eq!(probe(key, json!([name])).is_ok(), declared, "{case}");
                }
            }
            if !key.choices.is_empty() {
                let outside = if key.kind == "string-array" {
                    json!(["schema_unknown_choice"])
                } else {
                    json!("schema_unknown_choice")
                };
                assert!(probe(key, outside).is_err(), "{}", key.path);
            }
        }
        let key = super::schema_keys(&defaults)
            .unwrap()
            .into_iter()
            .find(|key| key.path == "terminal.cursor_trail_duration")
            .unwrap();
        assert!(probe(&key, json!(0.0)).is_ok());
        assert!(probe(&key, json!(0.1)).is_err());
        assert!(probe(&key, json!(0.25)).is_ok());
    }

    #[test]
    fn configuration_text_validation_covers_all_resolvers_without_reading_disk() {
        let root = crate::supervisor::temporary_directory();
        let path = root.join("config.toml");
        let defaults = crate::fixtures::inputs().defaults;
        fs::write(&path, "broken = [").unwrap();
        for source in [
            "",
            "[terminal]\nfont_size = 18\n[shell]\ncommand = ['custom']",
        ] {
            super::validate_configuration_text(source, &path, &defaults).unwrap();
        }
        assert!(super::parse_config(&root, &defaults).is_err());
        fs::write(&path, "").unwrap();
        for (source, key) in [
            (
                "[bar]\ncpu_warning_percent = 101",
                "bar.cpu_warning_percent",
            ),
            ("[bar]\nwidgets = ['cpu', 'cpu']", "bar.widgets"),
            ("[terminal]\nfont_size = 0", "terminal.font_size"),
            (
                "[popups.project]\nenabled = false",
                "popups.project.enabled",
            ),
            (
                "[popups.one]\ncommand = ['one']\nkeybinding = 'Alt+Shift+B'\n\
                 [popups.two]\ncommand = ['two']\nkeybinding = 'Alt+Shift+B'",
                "conflicts with popups.one.keybinding",
            ),
            ("[shell]\ncommand = []", "shell.command"),
            (
                "[agents]\nsend_enter_delay_ms = 65535",
                "agents.send_enter_delay_ms",
            ),
            ("[editor]\ncommand = []", "editor.command"),
            ("[anima]\nduration_seconds = 0", "anima.duration_seconds"),
            ("[updates]\nbundle_prefix = 'relative'", "updates"),
        ] {
            let error = super::validate_configuration_text(source, &path, &defaults).unwrap_err();
            assert!(error.contains(key), "{source}: {error}");
            assert!(error.contains(path.to_str().unwrap()), "{error}");
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn update_target_probe_leaves_other_settings_for_the_receiving_build() {
        for source in [
            "",
            "[updates]",
            "[terminal]\nfont_size = 'future'",
            "[updates]\nallowed_signers = 7",
        ] {
            assert!(!super::update_target_present(source).unwrap(), "{source}");
        }
        for source in [
            "[updates]\nnix_entry = ''",
            "[updates]\nnix_profile = false",
            "[updates]\nbundle_prefix = []",
        ] {
            assert!(super::update_target_present(source).unwrap(), "{source}");
        }
        for source in ["[updates", "updates = 5", "updates = []"] {
            assert!(super::update_target_present(source).is_err(), "{source}");
        }
    }

    #[test]
    fn update_targets_and_trust_are_resolved_independently() {
        let parse = |text: &str| toml::from_str::<super::UpdateSettings>(text).unwrap();
        let empty = parse("").resolve(false).unwrap();
        assert!(empty.bundle_prefix.is_none() && empty.allowed_signers.is_none());
        let settings =
            parse("bundle_prefix = '/private/prefix'\nallowed_signers = '/private/signers'")
                .resolve(false)
                .unwrap();
        assert_eq!(
            settings.bundle_prefix.as_deref(),
            Some(Path::new("/private/prefix"))
        );
        for text in [
            "bundle_prefix = 'relative'",
            "bundle_prefix = '/a/../b'",
            "nix_entry = 'eon'",
            "nix_profile = '/p'\nnix_entry = 'eon'\nbundle_prefix = '/b'",
        ] {
            assert!(parse(text).resolve(false).is_err(), "{text}");
            assert!(parse(text).resolve(true).unwrap().bundle_prefix.is_none());
        }
        assert!(parse("allowed_signers = 'relative'").resolve(true).is_err());
        let settings = parse("bundle_prefix = '/p'\nallowed_signers = '/s'")
            .resolve(true)
            .unwrap();
        assert!(settings.bundle_prefix.is_none());
        assert_eq!(settings.allowed_signers.as_deref(), Some(Path::new("/s")));
    }

    #[test]
    fn chip_percent_defaults_follow_explicit_levels() {
        let root = crate::supervisor::temporary_directory();
        let defaults = crate::fixtures::inputs().defaults;
        for chip in ["cpu", "ram", "disk"] {
            for (notice, warning, critical, expected) in [
                (None, Some(50_u8), None, [50, 50, 90]),
                (None, Some(95), None, [60, 95, 95]),
                (Some(80), None, None, [80, 80, 90]),
                (Some(95), None, None, [95, 95, 95]),
                (None, None, Some(40), [40, 40, 40]),
                (Some(50), None, Some(85), [50, 75, 85]),
                (Some(80), None, Some(85), [80, 80, 85]),
                (None, Some(50), Some(85), [50, 50, 85]),
                (Some(1), None, Some(100), [1, 75, 100]),
            ] {
                let mut source = String::from("[bar]\n");
                for (tier, value) in ["notice", "warning", "critical"]
                    .into_iter()
                    .zip([notice, warning, critical])
                {
                    if let Some(value) = value {
                        source.push_str(&format!("{chip}_{tier}_percent = {value}\n"));
                    }
                }
                fs::write(root.join("config.toml"), &source).unwrap();
                let load = super::bar(&root, &defaults, None).unwrap().load;
                let actual = match chip {
                    "cpu" => [
                        load.cpu_notice_percent,
                        load.cpu_warning_percent,
                        load.cpu_critical_percent,
                    ],
                    "ram" => [
                        load.ram_notice_percent,
                        load.ram_warning_percent,
                        load.ram_critical_percent,
                    ],
                    _ => [
                        load.disk_notice_percent,
                        load.disk_warning_percent,
                        load.disk_critical_percent,
                    ],
                };
                assert_eq!(actual, expected, "{source}");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn chip_use_levels_validate_all_three_tiers() {
        let root = crate::supervisor::temporary_directory();
        let defaults = crate::fixtures::inputs().defaults;
        let read = |settings: &str| {
            fs::write(root.join("config.toml"), format!("[bar]\n{settings}")).unwrap();
            super::bar(&root, &defaults, None)
        };
        for chip in ["cpu", "ram", "disk"] {
            let load = read(&format!("{chip}_notice_percent = 20\n{chip}_warning_percent = 40\n{chip}_critical_percent = 80")).unwrap().load;
            let values = match chip {
                "cpu" => (
                    load.cpu_notice_percent,
                    load.cpu_warning_percent,
                    load.cpu_critical_percent,
                ),
                "ram" => (
                    load.ram_notice_percent,
                    load.ram_warning_percent,
                    load.ram_critical_percent,
                ),
                _ => (
                    load.disk_notice_percent,
                    load.disk_warning_percent,
                    load.disk_critical_percent,
                ),
            };
            assert_eq!(values, (20, 40, 80));
            for value in [1, 100] {
                assert!(read(&format!("{chip}_notice_percent = {value}\n{chip}_warning_percent = {value}\n{chip}_critical_percent = {value}")).is_ok());
            }
            for tier in ["notice", "warning", "critical"] {
                for value in ["0", "101", "256", "-1", "60.5", "'high'"] {
                    assert!(
                        read(&format!("{chip}_{tier}_percent = {value}")).is_err(),
                        "{chip} {tier} {value}"
                    );
                }
            }
            for (settings, error) in [
                (
                    format!("{chip}_notice_percent = 76\n{chip}_warning_percent = 75"),
                    format!(
                        "bar.{chip}_warning_percent must not be below bar.{chip}_notice_percent"
                    ),
                ),
                (
                    format!("{chip}_warning_percent = 75\n{chip}_critical_percent = 74"),
                    format!(
                        "bar.{chip}_critical_percent must not be below bar.{chip}_warning_percent"
                    ),
                ),
                (
                    format!("{chip}_notice_percent = 95\n{chip}_critical_percent = 90"),
                    format!(
                        "bar.{chip}_critical_percent must not be below bar.{chip}_notice_percent"
                    ),
                ),
            ] {
                assert_eq!(read(&settings).unwrap_err(), error);
            }
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cpu_temperature_levels_validate_range_order_and_types() {
        let root = crate::supervisor::temporary_directory();
        let defaults = crate::fixtures::inputs().defaults;
        let read = |settings: &str| {
            fs::write(root.join("config.toml"), format!("[bar]\n{settings}")).unwrap();
            super::bar(&root, &defaults, None)
        };
        assert_eq!(read("").unwrap().load.cpu_temperature_warning_celsius, 85);
        assert_eq!(read("").unwrap().load.cpu_temperature_critical_celsius, 95);
        let valid =
            read("cpu_temperature_warning_celsius = 30\ncpu_temperature_critical_celsius = 120\n")
                .unwrap()
                .load;
        assert_eq!(
            (
                valid.cpu_temperature_warning_celsius,
                valid.cpu_temperature_critical_celsius
            ),
            (30, 120)
        );
        assert!(
            read("cpu_temperature_warning_celsius = 95\ncpu_temperature_critical_celsius = 95\n")
                .is_ok()
        );
        for settings in [
            "cpu_temperature_warning_celsius = 29",
            "cpu_temperature_critical_celsius = 121",
            "cpu_temperature_warning_celsius = 256",
            "cpu_temperature_critical_celsius = 0",
            "cpu_temperature_warning_celsius = 96",
            "cpu_temperature_critical_celsius = 84",
            "cpu_temperature_warning_celsius = -1",
            "cpu_temperature_critical_celsius = 90.5",
            "cpu_temperature_warning_celsius = 'hot'",
        ] {
            assert!(read(settings).is_err(), "accepted {settings}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disk_path_expands_home_and_keeps_absolute_paths() {
        let home = || Some(PathBuf::from("/home/test"));
        for (value, path) in [
            ("~", "/home/test"),
            ("~/", "/home/test"),
            ("~/work/eon", "/home/test/work/eon"),
            ("~//work", "/home/test/work"),
            ("/srv/data", "/srv/data"),
        ] {
            assert_eq!(disk_path(value, home()), Ok(Some(path.into())), "{value}");
        }
        // Under an unset or relative HOME, `~` names no path and the chip has no reading.
        for home in [None, Some(PathBuf::from("home/test"))] {
            assert_eq!(disk_path("~/work", home.clone()), Ok(None));
            assert_eq!(disk_path("/srv", home), Ok(Some("/srv".into())));
        }
        for value in ["", "work", "./work", "~user", "~user/work"] {
            assert_eq!(
                disk_path(value, home()),
                Err("bar.disk_path must be ~, absolute, or start with ~/".into()),
                "{value}"
            );
        }
    }

    #[test]
    fn defaults_formatter_keeps_new_sections_and_unset_keys() {
        let mut defaults = crate::fixtures::inputs().defaults;
        defaults.editor = None;
        let mut config = super::defaults_config(&defaults).unwrap();
        for path in [
            "/terminal/font_size",
            "/editor/command",
            "/updates/allowed_signers",
        ] {
            assert_eq!(
                config.pointer(path),
                Some(&serde_json::Value::Null),
                "{path}"
            );
        }
        // Model additions to parser structs without keeping a second key list.
        config["workspace"] = serde_json::json!({"future_value": true});
        config["session"]["future_option"] = serde_json::Value::Null;
        config["popups"]["git"]["future_option"] = serde_json::Value::Null;
        config["updates"]["future_option"] = serde_json::Value::Null;
        let toml = super::format_defaults(config.clone(), false).unwrap();
        let parsed: toml::Value = toml::from_str(&toml).unwrap();
        assert_eq!(parsed["workspace"]["future_value"].as_bool(), Some(true));
        assert_eq!(
            toml.matches("# future_option: unset. Built-in setting.")
                .count(),
            3
        );
        let json: serde_json::Value =
            serde_json::from_str(&super::format_defaults(config.clone(), true).unwrap()).unwrap();
        assert_eq!(json["workspace"]["future_value"], true);
        assert!(json["session"].get("future_option").is_none());
        assert!(json["popups"]["git"].get("future_option").is_none());
        assert!(json.get("updates").is_none());
        assert_eq!(
            json["editor"],
            serde_json::json!({"kind":"auto","open_in":"popup"})
        );
        // A future configured update default must no longer hide its section.
        config["updates"]["future_value"] = true.into();
        let toml = super::format_defaults(config.clone(), false).unwrap();
        let parsed: toml::Value = toml::from_str(&toml).unwrap();
        assert_eq!(parsed["updates"]["future_value"].as_bool(), Some(true));
        let json: serde_json::Value =
            serde_json::from_str(&super::format_defaults(config, true).unwrap()).unwrap();
        assert_eq!(json["updates"], serde_json::json!({"future_value": true}));
    }

    #[test]
    fn neovim_recognition_uses_the_original_name_and_explicit_wrapper_setting() {
        use super::super::editor::ListenerKind;
        let root = crate::supervisor::temporary_directory();
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        let wrapper = bin.join("my-editor");
        fs::write(&wrapper, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&wrapper, bin.join("nvim")).unwrap();
        let mut inputs = crate::fixtures::inputs();
        inputs.managed.helix = None;
        inputs.managed.shell_bin = Some(bin.clone());
        let prepare = |source: &str| {
            fs::write(root.join("config.toml"), source).unwrap();
            prepare_popup_command(
                &PopupCommand::Editor,
                &inputs.managed,
                &root,
                &root,
                &inputs.defaults,
                None,
            )
        };
        for command in ["nvim".to_owned(), bin.join("nvim").display().to_string()] {
            let prepared = prepare(&format!("[editor]\ncommand = [{command:?}]\n")).unwrap();
            assert_eq!(prepared.listener, Some((ListenerKind::Neovim, 1)));
            assert_eq!(prepared.argv[0], bin.join("nvim"));
        }
        assert_eq!(
            prepare("[editor]\ncommand = ['my-editor', 'nvim']\n")
                .unwrap()
                .listener,
            None
        );
        assert_eq!(
            prepare("[editor]\ncommand = ['my-editor']\nkind = 'neovim'\n")
                .unwrap()
                .listener,
            Some((ListenerKind::Neovim, 1))
        );
        let source = "[editor]\ncommand = ['nvim', '--headless']\n";
        assert_eq!(prepare(source).unwrap().listener, None);
        assert!(prepare(&format!("{source}kind = 'neovim'\n")).is_err());
        let pane = format!("{source}kind = 'neovim'\nopen_in = 'pane'\n");
        fs::write(root.join("config.toml"), pane).unwrap();
        let edit = super::prepare_edit(
            &root,
            &inputs.defaults,
            &inputs.managed,
            &root,
            Path::new("/a"),
            None,
            true,
            None,
        )
        .unwrap();
        let super::super::editor::EditCommand::Pane(command) = edit else {
            panic!("pane opt-out");
        };
        assert_eq!(command.listener, None);
        assert_eq!(
            command.argv,
            [bin.join("nvim").into_os_string(), "--headless".into()]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_kind_is_printed_and_rejects_unknown_values() {
        let defaults = crate::fixtures::inputs().defaults;
        let json: serde_json::Value =
            serde_json::from_str(&print_defaults(&defaults, true).unwrap()).unwrap();
        assert_eq!(json["editor"]["kind"], "auto");
        assert!(
            super::validate_configuration_text(
                "[editor]\nkind = 'unknown'\n",
                Path::new("config.toml"),
                &defaults,
            )
            .is_err()
        );
    }

    #[test]
    fn config_defaults_roundtrip_through_the_resolver_and_json() {
        let empty = crate::supervisor::temporary_directory();
        let printed = crate::supervisor::temporary_directory();
        let mut defaults = crate::fixtures::inputs().defaults;
        for explicit in [false, true] {
            if explicit {
                defaults.terminal.panel_heading = super::PanelHeading::Notch;
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
                defaults.animate_working_badge = false;
                defaults.number_keys = false;
                defaults.terminal_access = super::TerminalAccess::None;
                defaults.send_enter_delay_ms = 0;
                defaults.send_typing_quiet_ms = 0;
                // A build without Helix has no default editor.
                defaults.editor = None;
                defaults.editor_kind = super::EditorKind::Neovim;
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
            assert_eq!(
                json["panels"]["heading"],
                defaults.terminal.panel_heading.as_str()
            );
            assert!(json["terminal"].get("panel_heading").is_none());
            assert!(json["popups"]["project"].get("command").is_none());
            // Files is Eon's packaged Yazi: only its key and label are settings.
            for key in ["command", "enabled", "keep_alive"] {
                assert!(json["popups"]["files"].get(key).is_none(), "{key}");
            }
            assert!(toml.contains(
                "[popups.files]\n# Files is required: it is the tab's directory pair.\n"
            ));
            assert!(json["popups"]["anima"].get("command").is_none());
            assert!(json["popups"]["anima"].get("keep_alive").is_none());
            assert_eq!(json["terminal"].get("font_family").is_some(), explicit);
            assert_eq!(json["editor"].get("command").is_none(), explicit);
            assert_eq!(
                toml.contains(
                    "\n[editor]\n# command: unset. Direct argv that `eon edit` and Files run \
                     with the file appended. Unset, each edit runs $VISUAL, then $EDITOR.\n"
                ),
                explicit
            );
            fs::write(printed.join("config.toml"), toml).unwrap();
            assert!(
                super::read_config(&printed, &defaults, None)
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
                    super::bar(root, &defaults, None).unwrap(),
                    warning_close_last_tab(root, &defaults, None).unwrap(),
                    animate_working_badge(root, &defaults, None).unwrap(),
                    number_keys(root, &defaults, None).unwrap(),
                    sidebar_enabled(root, &defaults, None).unwrap(),
                    super::agents(root, &defaults, None).unwrap(),
                    super::editor_setting(root, &defaults, None).unwrap(),
                    super::read_config(root, &defaults, None)
                        .unwrap()
                        .editor
                        .kind
                        .unwrap_or(defaults.editor_kind),
                )
            };
            assert_eq!(policy(&empty), policy(&printed));
        }
        fs::remove_dir_all(empty).unwrap();
        fs::remove_dir_all(printed).unwrap();
    }

    /// The configuration `root` resolves to, in `eon config defaults --json` shape
    /// over the test fixture defaults rather than Eon's, and its warnings.
    fn resolved_config(
        root: &Path,
        defaults: &super::Defaults,
    ) -> Result<(serde_json::Value, Vec<String>), String> {
        let warnings = super::config_warnings(
            &super::parse_config(root, defaults)?,
            Path::new("config.toml"),
            defaults,
        );
        let bar = super::bar(root, defaults, None)?;
        let agents = super::agents(root, defaults, None)?;
        let anima = startup_animation(root, defaults, None)?;
        let resolved = super::Defaults {
            // `Defaults` holds a static widget list.
            bar_widgets: Box::leak(
                bar.widgets
                    .into_iter()
                    .map(|name| &*Box::leak(name.into_boxed_str()))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            bar_load: bar.load,
            // The configured path, as `eon config defaults` prints it, not its expansion.
            bar_disk_path: super::read_config(root, defaults, None)?
                .bar
                .disk_path
                .map_or(defaults.bar_disk_path, |path| {
                    Box::leak(path.into_boxed_str())
                }),
            shell: read_shell_config(root, defaults, None)?,
            terminal: terminal_presentation(root, defaults, None)?,
            anima: anima.clone().unwrap_or_else(|| defaults.anima.clone()),
            warning_close_last_tab: warning_close_last_tab(root, defaults, None)?,
            animate_working_badge: animate_working_badge(root, defaults, None)?,
            number_keys: number_keys(root, defaults, None)?,
            sidebar_enabled: sidebar_enabled(root, defaults, None)?,
            terminal_access: agents.terminal_access,
            send_enter_delay_ms: agents.send_enter_delay_ms,
            send_typing_quiet_ms: agents.send_typing_quiet_ms,
            popups: popup_catalog(root, defaults, None)?,
            editor: super::editor_setting(root, defaults, None)?.map(|argv| {
                &*Box::leak(
                    argv.into_iter()
                        .map(|argument| &*argument.leak())
                        .collect::<Box<[_]>>(),
                )
            }),
            editor_kind: super::read_config(root, defaults, None)?
                .editor
                .kind
                .unwrap_or(defaults.editor_kind),
            editor_open_in: super::read_config(root, defaults, None)?
                .editor
                .open_in
                .unwrap_or(defaults.editor_open_in),
            custom_popup_keep_alive: defaults.custom_popup_keep_alive,
            ansi_palette: defaults.ansi_palette,
            agent_commands: defaults.agent_commands,
        };
        let mut resolved: serde_json::Value =
            serde_json::from_str(&super::print_defaults(&resolved, true)?).unwrap();
        if anima.is_none() {
            resolved["anima"] = serde_json::json!({ "enabled": false });
        }
        Ok((resolved, warnings))
    }

    /// The migration floor: the oldest release whose configuration and durable
    /// state Eon keeps reading. Migration code and corpus fixtures below it may be
    /// deleted.
    const MIGRATION_FLOOR: &str = "v0.1.1";

    // Fails when a configuration that an earlier release at or above the migration
    // floor accepted stops loading or changes meaning (eon-c8ytm). Each
    // `testdata/config-corpus/<release>[-<case>]` holds a `config.toml` and an
    // `expected.json`: its resolved values and warnings, or, for a file this build
    // still rejects, the error and the reason it is expected.
    #[test]
    fn configuration_corpus_loads_from_the_migration_floor() {
        let release = |name: &str| -> Vec<u64> {
            name.strip_prefix('v')
                .unwrap()
                .split('.')
                .map(|part| part.parse().unwrap())
                .collect()
        };
        let floor = release(MIGRATION_FLOOR);
        let defaults = crate::fixtures::inputs().defaults;
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/config-corpus");
        let mut fixtures = fs::read_dir(&corpus)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        fixtures.sort();
        assert!(fixtures.len() >= 3, "{fixtures:?}");
        for root in fixtures {
            let name = root.file_name().unwrap().to_str().unwrap();
            assert!(
                release(name.split('-').next().unwrap()) >= floor,
                "{name} is below the migration floor; delete it with its migration code"
            );
            let expected: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(root.join("expected.json")).unwrap())
                    .unwrap();
            let actual = match resolved_config(&root, &defaults) {
                Ok((resolved, warnings)) => {
                    serde_json::json!({ "resolved": resolved, "warnings": warnings })
                }
                Err(error) => {
                    assert!(expected["expected_failure"].is_string(), "{name}: {error}");
                    serde_json::json!({
                        "error": error.replace(&format!("{}/", root.display()), ""),
                        "expected_failure": expected["expected_failure"],
                    })
                }
            };
            assert!(
                actual == expected,
                "{name} resolves to\n{}",
                serde_json::to_string_pretty(&actual).unwrap()
            );
        }
    }

    #[test]
    fn typing_quiet_configuration_bounds_and_defaults() {
        let root = crate::supervisor::temporary_directory();
        let defaults = crate::fixtures::inputs().defaults;
        for (source, expected) in [
            ("", 2000),
            ("[agents]\nsend_typing_quiet_ms = 0\n", 0),
            ("[agents]\nsend_typing_quiet_ms = 60000\n", 60000),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            let (resolved, warnings) = resolved_config(&root, &defaults).unwrap();
            assert!(warnings.is_empty(), "{warnings:?}");
            assert_eq!(resolved["agents"]["send_typing_quiet_ms"], expected);
        }
        for value in [
            "60001",
            "65536",
            "4294967296",
            "-1",
            "1.5",
            "true",
            "\"2000\"",
        ] {
            fs::write(
                root.join("config.toml"),
                format!("[agents]\nsend_typing_quiet_ms = {value}\n"),
            )
            .unwrap();
            assert!(resolved_config(&root, &defaults).is_err(), "{value}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn agent_settings_take_window_or_none_and_a_bounded_enter_delay() {
        let root = crate::supervisor::temporary_directory();
        let defaults = crate::fixtures::inputs().defaults;
        let agents = |source: &str| {
            fs::write(root.join("config.toml"), source).unwrap();
            super::agents(&root, &defaults, None)
        };
        assert_eq!(
            agents("[agents]\nterminal_access = \"none\"\nsend_enter_delay_ms = 5000\n"),
            Ok(super::Agents {
                terminal_access: super::TerminalAccess::None,
                send_enter_delay_ms: 5000,
                send_typing_quiet_ms: 2000,
            })
        );
        assert_eq!(
            agents("[agents]\nterminal_access = \"window\"\nsend_enter_delay_ms = 0\n"),
            Ok(super::Agents {
                terminal_access: super::TerminalAccess::Window,
                send_enter_delay_ms: 0,
                send_typing_quiet_ms: 2000,
            })
        );
        for source in [
            "[agents]\nterminal_access = \"all\"\n",
            "[agents]\nsend_enter_delay_ms = 5001\n",
            "[agents]\nsend_enter_delay_ms = 4294967296\n",
            "[agents]\nsend_enter_delay_ms = -1\n",
        ] {
            assert!(agents(source).is_err(), "{source}");
        }
        fs::remove_dir_all(root).unwrap();
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
[panels]
heading = "connected"
future_fill = true
[bar]
widgets = ["cpu"]
new_field = true
[anima]
enabled = false
new_field = true
[session]
warning_close_last_tab = false
animate_working_badge = false
number_keys = false
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
        let defaults = crate::fixtures::inputs().defaults;
        let config = super::read_config(&root, &defaults, Some(&log)).unwrap();
        assert_eq!(config.shell.starship, Some(false));
        assert_eq!(config.terminal.background_opacity, Some(0.37));
        assert_eq!(config.panels.heading, Some(super::PanelHeading::Connected));
        assert_eq!(
            config.bar.widgets.as_deref(),
            Some(["cpu".to_owned()].as_slice())
        );
        assert_eq!(config.anima.enabled, Some(false));
        assert_eq!(config.session.warning_close_last_tab, Some(false));
        assert_eq!(config.session.animate_working_badge, Some(false));
        assert_eq!(config.session.number_keys, Some(false));
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
            "panels.future_fill",
            "popup",
            "popups.agent.new_field",
            "session.new_field",
            "shell.new_field",
            "sidebar.new_field",
            "terminal.future_border_width",
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
            super::read_config(&root, &defaults, None)
                .unwrap_err()
                .contains("background_opacity")
        );
        fs::remove_dir_all(root).unwrap();
    }

    // Fails when every reread of an unchanged file warns again, as the supervisor's
    // several reads per start and per new terminal once did (EON-C9: one warning).
    #[test]
    fn rereads_through_one_log_warn_once_per_unknown_or_deprecated_key() {
        let root = crate::supervisor::temporary_directory();
        let log = super::DiagnosticLog::open_in(
            Ok(root.join("logs")),
            "g1-0123456789abcdef0123456789abcdef",
            "workspace",
        );
        let path = root.join("config.toml");
        fs::write(&path, "[popup]\nside_margin = 13.0\n").unwrap();
        let inputs = crate::fixtures::inputs();
        for _ in 0..3 {
            terminal_presentation(&root, &inputs.defaults, Some(&log)).unwrap();
            super::shell_command(&root, &inputs.defaults, Some(&log)).unwrap();
        }
        fs::write(
            &path,
            "[popup]\nside_margin = 13.0\n[terminal]\nnew_field = 1\n[bar]\nshow_version = false\n",
        )
        .unwrap();
        for _ in 0..2 {
            terminal_presentation(&root, &inputs.defaults, Some(&log)).unwrap();
            super::bar(&root, &inputs.defaults, Some(&log)).unwrap();
        }
        let log_path = fs::read_dir(root.join("logs"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_file())
            .unwrap();
        let logged = fs::read_to_string(log_path).unwrap();
        for key in ["popup", "terminal.new_field"] {
            let warning = format!("eon: ignoring unknown setting {key} in {}", path.display());
            assert_eq!(
                logged
                    .lines()
                    .filter(|line| line.ends_with(&warning))
                    .count(),
                1,
                "{logged}"
            );
        }
        assert_eq!(logged.matches("ignoring unknown setting").count(), 2);
        assert_eq!(
            logged.matches(" is deprecated since ").count(),
            1,
            "{logged}"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn google_quota_selector_boundaries() {
        let mut defaults = crate::fixtures::inputs().defaults;
        defaults.bar_widgets = &["clock"];
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../eon-workspace-protocol/testdata/google-quota-selectors.json"
        ))
        .unwrap();
        for case in cases.as_array().unwrap() {
            let name = case["selector"].as_str().unwrap();
            let bar = super::resolve_bar(
                super::BarSettings {
                    widgets: Some(vec![name.into()]),
                    ..Default::default()
                },
                &defaults,
            )
            .unwrap();
            assert_eq!(!bar.widgets.is_empty(), case["scope"].is_string(), "{case}");
            if !bar.widgets.is_empty() {
                assert_eq!(bar.widgets, [name]);
            }
            let source = toml::to_string(&serde_json::json!({"bar":{"widgets":[name]}})).unwrap();
            let path = Path::new("config.toml");
            let config = super::parse_configuration_text(&source, path, &defaults).unwrap();
            let warnings = super::config_warnings(&config, path, &defaults);
            if case["scope"].is_string() {
                assert!(warnings.is_empty(), "{case}: {warnings:?}");
            } else {
                assert_eq!(
                    warnings,
                    [format!(
                        "eon: ignoring unknown bar.widgets name {name:?} in config.toml"
                    )]
                );
            }
        }
        let bar = super::resolve_bar(
            super::BarSettings {
                widgets: Some(vec!["ram".into()]),
                ..Default::default()
            },
            &defaults,
        )
        .unwrap();
        assert_eq!(bar.widgets, ["ram"]);
    }

    #[test]
    fn quota_window_widgets_and_unknown_names() {
        let root = crate::supervisor::temporary_directory();
        let defaults = crate::fixtures::inputs().defaults;
        fs::write(root.join("config.toml"),
            "[bar]\nwidgets = ['claude_quota_7d', 'cpu', 'claude_quota_5h', 'claude_quota_scoped', 'codex_quota_5h', 'codex_quota_7d', 'codex_quota', 'claude_quota', 'future_quota']\n"
        ).unwrap();
        let bar = super::bar(&root, &defaults, None).unwrap();
        assert_eq!(
            bar.widgets,
            [
                "claude_quota_7d",
                "cpu",
                "claude_quota_5h",
                "claude_quota_scoped",
                "codex_quota_5h",
                "codex_quota_7d"
            ]
        );
        assert_eq!(
            super::config_warnings(
                &super::parse_config(&root, &defaults).unwrap(),
                Path::new("config.toml"),
                &defaults,
            ),
            [
                "eon: ignoring unknown bar.widgets name \"codex_quota\" in config.toml",
                "eon: ignoring unknown bar.widgets name \"claude_quota\" in config.toml",
                "eon: ignoring unknown bar.widgets name \"future_quota\" in config.toml",
            ]
        );
        fs::write(
            root.join("config.toml"),
            "[bar]\nwidgets = ['claude_quota_5h', 'claude_quota_5h']\n",
        )
        .unwrap();
        assert!(
            super::bar(&root, &defaults, None)
                .unwrap_err()
                .contains("repeats")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deprecated_show_version_reads_as_bar_widgets() {
        let root = crate::supervisor::temporary_directory();
        let defaults = crate::fixtures::inputs().defaults;
        let read = |source: &str| {
            fs::write(root.join("config.toml"), source).unwrap();
            let warnings = super::config_warnings(
                &super::parse_config(&root, &defaults)?,
                Path::new("config.toml"),
                &defaults,
            );
            Ok::<_, String>((super::bar(&root, &defaults, None)?.widgets, warnings))
        };
        let (widgets, warnings) = read("[bar]\nshow_version = false\n").unwrap();
        let mut expected = defaults.bar_widgets.to_vec();
        expected.retain(|widget| *widget != "version");
        assert_eq!(widgets, expected);
        assert_eq!(
            warnings,
            ["eon: bar.show_version in config.toml is deprecated since v0.1.2; use bar.widgets"]
        );
        let (widgets, warnings) = read("[bar]\nshow_version = true\n").unwrap();
        assert_eq!(widgets, defaults.bar_widgets);
        assert_eq!(warnings.len(), 1);
        let (widgets, warnings) =
            read("[bar]\nshow_version = false\nwidgets = ['version']\n").unwrap();
        assert_eq!(widgets, ["version"]);
        assert_eq!(
            warnings,
            [
                "eon: ignoring deprecated setting bar.show_version in config.toml; bar.widgets is set"
            ]
        );
        for source in [
            "[bar]\nshow_version = 'no'\n",
            "[bar]\nshow_version = 0\n",
            "[bar]\nwidgets = ['cpu']\nshow_version = 'no'\n",
        ] {
            let error = read(source).unwrap_err();
            assert!(
                error.ends_with("config.toml: bar.show_version must be true or false"),
                "{error}"
            );
        }
        // Errors keep their position, and a deprecated key changes no other check.
        for (source, position) in [
            ("[bar]\nwidgets = 'version'\n", "line 2, column 11"),
            (
                "[bar]\nshow_version = false\n[terminal]\nbackground_opacity = 'x'\n",
                "line 4, column 22",
            ),
            (
                "[bar]\nshow_version = false\n[terminal]\nfont_family = 2026-10-31\n",
                "line 4, column 15",
            ),
        ] {
            let error = read(source).unwrap_err();
            assert!(error.contains(position), "{error}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    // Fails when an alias that drops its key inserts one, or one that moves a key
    // into a missing table does not create it.
    #[test]
    fn deprecated_keys_drop_or_move_into_missing_tables() {
        let defaults = crate::fixtures::inputs().defaults;
        let aliases = [
            super::Alias {
                old: "dropped",
                new: "kept",
                deprecated_in: "v0",
                translate: |_, _| Ok(None),
            },
            super::Alias {
                old: "a.moved",
                new: "b.c.moved",
                deprecated_in: "v0",
                translate: |value, _| Ok(Some(value.clone())),
            },
        ];
        let mut table: toml::Table = toml::from_str("dropped = 1\n[a]\nmoved = 2\n").unwrap();
        let applied = super::translate_deprecated(&mut table, &aliases, &defaults).unwrap();
        assert_eq!(applied.len(), 2);
        assert_eq!(table, toml::from_str("a = {}\nb.c.moved = 2\n").unwrap());
    }

    // Fails when an alias names a replacement `eon config defaults --json` does
    // not print, or still prints the key it replaces.
    #[test]
    fn deprecated_keys_alias_current_settings() {
        let defaults = crate::fixtures::inputs().defaults;
        let json: serde_json::Value =
            serde_json::from_str(&super::print_defaults(&defaults, true).unwrap()).unwrap();
        let pointer = |key: &str| format!("/{}", key.replace('.', "/"));
        for alias in super::ALIASES {
            assert!(json.pointer(&pointer(alias.new)).is_some(), "{alias:?}");
            assert!(json.pointer(&pointer(alias.old)).is_none(), "{alias:?}");
        }
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
            "[session]\nwarning_close_last_tab = false\n",
        )
        .unwrap();
        assert!(!warning_close_last_tab(&root, &inputs.defaults, None).unwrap());
        fs::write(
            root.join("config.toml"),
            "[session]\nwarning_close_last_tab = 'no'\n",
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
    fn the_working_badge_setting_reads_its_value_and_rejects_non_booleans() {
        let inputs = crate::fixtures::inputs();
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-working-badge",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        assert!(animate_working_badge(&root, &inputs.defaults, None).unwrap());
        fs::write(
            root.join("config.toml"),
            "[session]\nanimate_working_badge = false\n",
        )
        .unwrap();
        assert!(!animate_working_badge(&root, &inputs.defaults, None).unwrap());
        fs::write(
            root.join("config.toml"),
            "[session]\nanimate_working_badge = 'no'\n",
        )
        .unwrap();
        assert!(
            animate_working_badge(&root, &inputs.defaults, None)
                .unwrap_err()
                .contains("animate_working_badge")
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
    fn cursor_trail_duration_accepts_off_and_keeps_other_bounds() {
        let root = crate::supervisor::temporary_directory();
        let mut defaults = crate::fixtures::inputs().defaults;
        defaults.terminal.cursor_trail_duration = Some(1.5);
        assert_eq!(
            terminal_presentation(&root, &defaults, None)
                .unwrap()
                .cursor_trail_duration,
            Some(1.5)
        );
        for value in ["0", "0.0", "0.25", "4.0"] {
            fs::write(
                root.join("config.toml"),
                format!("[terminal]\ncursor_trail_duration = {value}\n"),
            )
            .unwrap();
            assert_eq!(
                terminal_presentation(&root, &defaults, None)
                    .unwrap()
                    .cursor_trail_duration,
                Some(value.parse().unwrap())
            );
        }
        for value in ["-0.01", "0.24", "4.01", "nan", "inf", "'0'"] {
            fs::write(
                root.join("config.toml"),
                format!("[terminal]\ncursor_trail_duration = {value}\n"),
            )
            .unwrap();
            assert!(
                terminal_presentation(&root, &defaults, None)
                    .unwrap_err()
                    .contains("cursor_trail_duration"),
                "accepted {value}"
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
            ["project", "git", "agent", "files", "anima"]
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
keybinding = "Alt+Shift+F"
label = "Browse"
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
            ["project", "agent", "files", "anima"]
        );
        let files = &configured.entries[2];
        assert_eq!(
            (files.label.as_str(), &files.command, files.keep_alive),
            (
                "Browse",
                &PopupCommand::Argv(vec!["fixture-files".into()]),
                true
            )
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

        // A popup on a focus key loads without a key, with a warning naming it.
        for keybinding in ["Alt+H", "alt+j", "Alt+K", "Alt+L"] {
            fs::write(
                root.join("config.toml"),
                format!(
                    "[popups.extra]\ncommand = [\"tool\"]\nkeybinding = \"{keybinding}\"\n[popups.project]\nkeybinding = \"{keybinding}\"\n"
                ),
            )
            .unwrap();
            let catalog = popup_catalog(&root, &inputs.defaults, None).unwrap();
            for id in ["extra", "project"] {
                assert_eq!(
                    catalog
                        .entries
                        .iter()
                        .find(|entry| entry.id == id)
                        .unwrap()
                        .shortcut,
                    PopupKey::Unbound,
                    "{keybinding}"
                );
            }
            assert_eq!(
                super::config_warnings(
                    &super::parse_config(&root, &inputs.defaults).unwrap(),
                    Path::new("config.toml"),
                    &inputs.defaults,
                ),
                ["extra", "project"].map(|id| format!(
                    "eon: popups.{id}.keybinding {keybinding} in config.toml is reserved for moving focus; popup {id} loads without a key"
                ))
            );
        }

        for (source, expected) in [
            (
                "[popups.extra]\ncommand = [\"tool\"]\nkeybinding = \"Alt+Shift+F\"\n[popups.other]\ncommand = [\"tool\"]\nkeybinding = \"Alt+Shift+F\"\n",
                "popups.other.keybinding conflicts with popups.extra.keybinding",
            ),
            (
                "[popups.agent]\nkeybinding = \"Alt+Shift+F\"\n[popups.extra]\ncommand = [\"tool\"]\nkeybinding = \"Alt+Shift+F\"\n",
                "popups.extra.keybinding conflicts with popups.agent.keybinding",
            ),
            (
                "[popups.project]\nkeybinding = \"Alt+5\"\n",
                "popups.project.keybinding is reserved for an Eon shortcut",
            ),
            (
                "[popups.extra]\ncommand = [\"tool\"]\n",
                "popups.extra.keybinding is required",
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

    /// Every tab's Files pair is Eon's packaged Yazi (EON-C18): the settings
    /// earlier releases let replace, disable or stop it are ignored with a
    /// warning, and its key and label stay configurable.
    #[test]
    fn files_ignores_command_enabled_and_keep_alive_with_a_warning() {
        let inputs = crate::fixtures::inputs();
        let root = crate::supervisor::temporary_directory();
        let packaged = PopupCommand::Argv(vec!["fixture-files".into()]);
        let warning = |key: &str| {
            format!(
                "eon: ignoring popups.files.{key} in config.toml: Files is Eon's packaged Yazi; move a custom Yazi to its own popup, such as [popups.yazi] command = [\"eon-yazi\"]"
            )
        };
        for (source, ignored) in [
            (
                "[popups.files]\ncommand = ['eon-yazi']\nenabled = false\nkeep_alive = false\nlabel = 'Browse'\n",
                &["command", "enabled", "keep_alive"][..],
            ),
            (
                "[popups.files]\ncommand = ['eon-yazi']\nlabel = 'Browse'\n",
                &["command"][..],
            ),
            (
                "[popups.files]\nenabled = false\nlabel = 'Browse'\n",
                &["enabled"][..],
            ),
            (
                "[popups.files]\nkeep_alive = false\nlabel = 'Browse'\n",
                &["keep_alive"][..],
            ),
            // Values that keep Files as it is need no warning.
            (
                "[popups.files]\nenabled = true\nkeep_alive = true\nlabel = 'Browse'\n",
                &[][..],
            ),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            let catalog = popup_catalog(&root, &inputs.defaults, None).unwrap();
            let files = catalog
                .entries
                .iter()
                .find(|entry| entry.id == "files")
                .expect("Files stays enabled");
            assert_eq!(
                (files.label.as_str(), &files.command, files.keep_alive),
                ("Browse", &packaged, true),
                "{source}"
            );
            assert_eq!(
                super::config_warnings(
                    &super::parse_config(&root, &inputs.defaults).unwrap(),
                    Path::new("config.toml"),
                    &inputs.defaults,
                ),
                ignored.iter().map(|key| warning(key)).collect::<Vec<_>>(),
                "{source}"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn editor_popup_settings_round_trip_and_count_against_the_catalog_limit() {
        use eon_workspace_protocol::v44::MAX_ENTRIES;

        let mut defaults = crate::fixtures::inputs().defaults;
        let editor = PopupDefinition {
            id: "editor".into(),
            label: "Editor".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT | SHIFT,
                key: "KeyO".into(),
            }),
            command: PopupCommand::Editor,
            keep_alive: true,
        };
        defaults.popups.entries.push(editor.clone());
        let root = crate::supervisor::temporary_directory();
        let path = root.join("config.toml");
        let printed = print_defaults(&defaults, false).unwrap();
        fs::write(&path, printed).unwrap();
        assert_eq!(
            popup_catalog(&root, &defaults, None).unwrap(),
            defaults.popups
        );
        let json: serde_json::Value =
            serde_json::from_str(&print_defaults(&defaults, true).unwrap()).unwrap();
        assert_eq!(
            json["popups"]["editor"],
            serde_json::json!({
                "enabled": true, "keybinding": "Alt+Shift+O", "label": "Editor",
            })
        );

        fs::write(&path, "[popups.editor]\ncommand = ['hx']\nkeep_alive = false\nkeybinding = 'Alt+Shift+F'\nlabel = 'Code'\n").unwrap();
        let catalog = popup_catalog(&root, &defaults, None).unwrap();
        let configured = catalog.entries.last().unwrap();
        assert_eq!(
            configured,
            &PopupDefinition {
                label: "Code".into(),
                shortcut: PopupKey::Bound(Shortcut {
                    modifiers: ALT | SHIFT,
                    key: "KeyF".into()
                }),
                ..editor
            },
        );
        let warnings = super::config_warnings(
            &super::parse_config(&root, &defaults).unwrap(),
            Path::new("config.toml"),
            &defaults,
        );
        assert_eq!(warnings.len(), 2);
        for (warning, setting) in warnings.iter().zip(["command", "keep_alive"]) {
            assert!(
                warning.contains(&format!("popups.editor.{setting}")),
                "{warning}"
            );
            assert!(warning.contains("[editor] command"), "{warning}");
        }

        let custom = (0..=MAX_ENTRIES - defaults.popups.entries.len())
            .map(|index| format!("[popups.extra-{index}]\ncommand = ['tool']\nkeybinding = ''\n"))
            .collect::<String>();
        fs::write(&path, &custom).unwrap();
        assert_eq!(
            popup_catalog(&root, &defaults, None).unwrap_err(),
            format!("popups accepts at most {MAX_ENTRIES} enabled entries")
        );
        fs::write(
            &path,
            format!("{custom}\n[popups.editor]\nenabled = false\n"),
        )
        .unwrap();
        let catalog = popup_catalog(&root, &defaults, None).unwrap();
        assert_eq!(catalog.entries.len(), MAX_ENTRIES);
        assert!(catalog.entries.iter().all(|entry| entry.id != "editor"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configured_popup_keys_take_built_in_defaults() {
        let mut inputs = crate::fixtures::inputs();
        inputs.defaults.popups.entries.push(PopupDefinition {
            id: "commands".into(),
            label: "Commands".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT | SHIFT,
                key: "KeyM".into(),
            }),
            command: PopupCommand::Argv(vec!["fixture-commands".into()]),
            keep_alive: false,
        });
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-popup-keys",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        let key = |modifiers, key: &str| Shortcut {
            modifiers,
            key: key.into(),
        };
        let overridden = |default, by: &str| PopupKey::Overridden {
            default,
            by: by.into(),
        };
        for (source, expected) in [
            // A custom popup on a built-in default takes it, wherever the
            // built-in sits in the catalog.
            (
                "[popups.editor]\ncommand = [\"hx\"]\nkeybinding = \"Alt+Shift+M\"\n",
                vec![
                    ("commands", overridden(key(ALT | SHIFT, "KeyM"), "editor")),
                    ("editor", PopupKey::Bound(key(ALT | SHIFT, "KeyM"))),
                ],
            ),
            (
                "[popups.extra]\ncommand = [\"tool\"]\nkeybinding = \"Alt+Z\"\n",
                vec![
                    ("project", overridden(key(ALT, "KeyZ"), "extra")),
                    ("extra", PopupKey::Bound(key(ALT, "KeyZ"))),
                ],
            ),
            // So does a built-in the user moved onto another's default.
            (
                "[popups.anima]\nkeybinding = \"Alt+Shift+J\"\n",
                vec![
                    ("git", overridden(key(ALT | SHIFT, "KeyJ"), "anima")),
                    ("anima", PopupKey::Bound(key(ALT | SHIFT, "KeyJ"))),
                ],
            ),
            // An empty keybinding leaves any popup without a key.
            (
                "[popups.project]\nkeybinding = \"\"\n[popups.extra]\ncommand = [\"tool\"]\nkeybinding = \"\"\n",
                vec![("project", PopupKey::Unbound), ("extra", PopupKey::Unbound)],
            ),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            let catalog = popup_catalog(&root, &inputs.defaults, None).unwrap();
            for (id, key) in expected {
                let entry = catalog.entries.iter().find(|entry| entry.id == id);
                assert_eq!(entry.map(|entry| &entry.shortcut), Some(&key), "{source}");
            }
        }
        // Two defaults on one chord stay an error even while a configured popup holds it.
        inputs.defaults.popups.entries.last_mut().unwrap().shortcut =
            PopupKey::Bound(key(ALT | SHIFT, "KeyJ"));
        fs::write(
            root.join("config.toml"),
            "[popups.extra]\ncommand = [\"tool\"]\nkeybinding = \"Alt+Shift+J\"\n",
        )
        .unwrap();
        assert!(
            popup_catalog(&root, &inputs.defaults, None)
                .unwrap_err()
                .contains("popups.commands.keybinding conflicts with popups.git.keybinding")
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Scrollback's command and lifetime are Eon's: its key, label and
    /// enabled flag are settings, and a configured popup on its default key
    /// takes the key without failing the load (EON-C10).
    #[test]
    fn scrollback_owns_its_command_and_yields_its_default_key() {
        let mut inputs = crate::fixtures::inputs();
        inputs.defaults.popups.entries.push(PopupDefinition {
            id: "scrollback".into(),
            label: "Scrollback".into(),
            shortcut: PopupKey::Bound(Shortcut {
                modifiers: ALT | SHIFT,
                key: "KeyE".into(),
            }),
            command: PopupCommand::Scrollback,
            keep_alive: false,
        });
        let json: serde_json::Value =
            serde_json::from_str(&super::print_defaults(&inputs.defaults, true).unwrap()).unwrap();
        assert_eq!(
            json["popups"]["scrollback"],
            serde_json::json!({
                "enabled": true,
                "keybinding": "Alt+Shift+E",
                "label": "Scrollback",
            })
        );
        assert!(
            super::print_defaults(&inputs.defaults, false)
                .unwrap()
                .contains("[popups.scrollback]\n# command and keep_alive are Eon-owned.\n")
        );
        let root = std::env::temp_dir().join(format!(
            "eon-managed-environment-test-{}-scrollback",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        for (source, expected) in [
            (
                "[popups.scrollback]\ncommand = ['less']\n",
                "popups.scrollback.command is Eon-owned",
            ),
            (
                "[popups.scrollback]\nkeep_alive = true\n",
                "popups.scrollback.keep_alive must remain false",
            ),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert_eq!(
                popup_catalog(&root, &inputs.defaults, None).unwrap_err(),
                expected
            );
        }
        let scrollback = |source: &str| {
            fs::write(root.join("config.toml"), source).unwrap();
            popup_catalog(&root, &inputs.defaults, None)
                .unwrap()
                .entries
                .into_iter()
                .find(|entry| entry.id == "scrollback")
        };
        let configured = scrollback(
            "[popups.scrollback]\nkeybinding = 'Alt+Shift+V'\nlabel = 'History'\nkeep_alive = false\n",
        )
        .unwrap();
        assert_eq!(
            (
                configured.label.as_str(),
                &configured.shortcut,
                &configured.command,
                configured.keep_alive
            ),
            (
                "History",
                &PopupKey::Bound(Shortcut {
                    modifiers: ALT | SHIFT,
                    key: "KeyV".into(),
                }),
                &PopupCommand::Scrollback,
                false
            )
        );
        assert_eq!(scrollback("[popups.scrollback]\nenabled = false\n"), None);
        assert_eq!(
            scrollback("[popups.less]\ncommand = ['less']\nkeybinding = 'Alt+Shift+E'\n")
                .unwrap()
                .shortcut,
            PopupKey::Overridden {
                default: Shortcut {
                    modifiers: ALT | SHIFT,
                    key: "KeyE".into(),
                },
                by: "less".into(),
            }
        );
        assert_eq!(
            prepare_popup_command(
                &PopupCommand::Scrollback,
                &inputs.managed,
                &root,
                &root,
                &inputs.defaults,
                None
            )
            .unwrap_err(),
            "the supervisor builds Scrollback's command for each press"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn popup_executable_preflight_uses_the_session_context() {
        let mut inputs = crate::fixtures::inputs();
        inputs.managed.shell_bin = None;
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
            prepare_popup_command(
                &command,
                &inputs.managed,
                &launch,
                &root,
                &inputs.defaults,
                None
            )
            .unwrap()
            .argv,
            [OsString::from("./tool")]
        );
        assert!(
            prepare_popup_command(
                &command,
                &inputs.managed,
                &root,
                &root,
                &inputs.defaults,
                None
            )
            .is_err()
        );
        if crate::supervisor::effective_uid() != 0 {
            // The owner cannot execute this file, even though the group bit is set.
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o010)).unwrap();
            assert!(
                prepare_popup_command(
                    &command,
                    &inputs.managed,
                    &launch,
                    &root,
                    &inputs.defaults,
                    None
                )
                .is_err()
            );
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::create_dir(launch.join("bin")).unwrap();
        let path_tool = launch.join("bin/path-tool");
        fs::write(&path_tool, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path_tool, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            prepare_popup_command(
                &PopupCommand::Argv(vec!["path-tool".into()]),
                &ManagedPrograms {
                    shell_bin: Some("bin".into()),
                    ..crate::fixtures::inputs().managed
                },
                &launch,
                &root,
                &inputs.defaults,
                None
            )
            .unwrap()
            .argv,
            [OsString::from("path-tool")]
        );
        assert!(
            prepare_popup_command(
                &PopupCommand::Argv(vec![path_tool.into_os_string()]),
                &ManagedPrograms {
                    shell_bin: Some("invalid:path".into()),
                    ..crate::fixtures::inputs().managed
                },
                &launch,
                &root,
                &inputs.defaults,
                None
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
            prepare_popup_command(
                &PopupCommand::AgentAuto,
                &inputs.managed,
                &launch,
                &root,
                &defaults,
                None
            )
            .unwrap()
            .argv,
            [OsString::from("./tool"), OsString::from("--fallback")]
        );
        let preferred = launch.join("preferred-agent");
        fs::write(&preferred, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&preferred, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            prepare_popup_command(
                &PopupCommand::AgentAuto,
                &inputs.managed,
                &launch,
                &root,
                &defaults,
                None
            )
            .unwrap()
            .argv,
            [
                OsString::from("./preferred-agent"),
                OsString::from("--preferred")
            ]
        );

        // The configured editor runs on the file, reread for each edit, and an
        // unavailable or malformed one fails the edit rather than running Helix.
        let edit = |config: &str| {
            fs::write(root.join("config.toml"), config).unwrap();
            prepare_popup_command(
                &PopupCommand::Edit("/src/a b.rs".into()),
                &inputs.managed,
                &launch,
                &root,
                &inputs.defaults,
                None,
            )
        };
        assert_eq!(
            edit("[editor]\ncommand = [\"./tool\", \"--wait\"]\n")
                .unwrap()
                .argv,
            ["./tool", "--wait", "/src/a b.rs"].map(OsString::from)
        );
        assert!(
            edit("")
                .unwrap_err()
                .contains("\"fixture-editor\" is unavailable")
        );
        for (config, error) in [
            (
                "[editor]\ncommand = [\"./missing\"]\n",
                "\"./missing\" is unavailable",
            ),
            (
                "[editor]\ncommand = \"./tool\"\n",
                "editor.command must be a direct argv array, such as command = [\"nvim\"]",
            ),
            (
                "[editor]\ncommand = []\n",
                "editor.command requires a nonempty executable",
            ),
        ] {
            assert!(edit(config).unwrap_err().contains(error), "{config}");
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// The pair's launch flags become its Yazi's environment, never Yazi
    /// arguments, and a Files started inside another pair inherits none of
    /// that pair's (EON-C18).
    #[test]
    fn pair_launch_flags_set_the_pair_environment_and_pass_the_rest_to_yazi() {
        let arguments = [
            "--eon-quick",
            "--eon-pair-socket",
            "/run/eon-pair-1.sock",
            "--client-id",
            "4611686018427387905",
        ]
        .map(OsString::from);
        let mut command = Command::new("yazi");
        let rest = pair_launch_flags(&mut command, &arguments);
        assert_eq!(rest, &arguments[3..]);
        assert_eq!(
            command_environment(&command, "EON_FILES_START"),
            Some(Some(OsStr::new("quick")))
        );
        assert_eq!(
            command_environment(&command, "EON_PAIR_SOCKET"),
            Some(Some(OsStr::new("/run/eon-pair-1.sock")))
        );
        assert_eq!(
            command_environment(&command, "EON_FILES_NOTICE"),
            Some(None)
        );

        // A plain `eon-files DIR` clears what a parent pair exported.
        let plain = [OsString::from("dir")];
        let mut command = Command::new("yazi");
        assert_eq!(pair_launch_flags(&mut command, &plain), &plain[..]);
        for variable in ["EON_FILES_START", "EON_PAIR_SOCKET", "EON_FILES_NOTICE"] {
            assert_eq!(command_environment(&command, variable), Some(None));
        }
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
            helix: Some("/managed/hx".into()),
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
                &["CARGO_MANIFEST_DIR", "HELIX_RUNTIME"][..],
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
    fn a_build_without_helix_refuses_its_names() {
        // Running a bare `hx` instead would find this same name on the terminal PATH.
        let inputs = crate::fixtures::inputs();
        let programs = ManagedPrograms {
            helix: None,
            ..inputs.managed
        };
        let config = Path::new("/private/eon");
        let error =
            managed_command(Tool::Helix, &programs, config, &[], &inputs.defaults).unwrap_err();
        assert_eq!(
            error,
            "this Eon package includes no Helix, so it provides no hx or eon-hx"
        );
        assert!(managed_command(Tool::Yazi, &programs, config, &[], &inputs.defaults).is_ok());
    }

    #[test]
    fn an_unset_editor_comes_from_visual_then_editor_split_on_ascii_whitespace() {
        use std::os::unix::ffi::OsStringExt;

        let resolve = |visual: Option<&str>, editor: Option<&str>| {
            super::environment_editor(visual.map(Into::into), editor.map(Into::into))
        };
        let argv = |words: &[&str]| words.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            resolve(Some("vis --flag"), Some("ed")),
            Some(("$VISUAL", argv(&["vis", "--flag"])))
        );
        assert_eq!(resolve(None, Some("ed")), Some(("$EDITOR", argv(&["ed"]))));
        // No shell reads the value: only ASCII whitespace separates words.
        assert_eq!(
            resolve(None, Some(" my-editor  --wait\t-x\n")),
            Some(("$EDITOR", argv(&["my-editor", "--wait", "-x"])))
        );
        assert_eq!(
            resolve(None, Some("'my editor'\u{a0}-w")),
            Some(("$EDITOR", argv(&["'my", "editor'\u{a0}-w"])))
        );
        // A blank value names no editor.
        assert_eq!(
            resolve(Some(" \t"), Some("ed")),
            Some(("$EDITOR", argv(&["ed"])))
        );
        assert_eq!(resolve(Some(""), Some(" ")), None);
        assert_eq!(resolve(None, None), None);
        // A path that is not UTF-8 keeps its bytes.
        assert_eq!(
            super::environment_editor(
                None,
                Some(OsString::from_vec(b"/opt/\xffed --wait".to_vec()))
            ),
            Some((
                "$EDITOR",
                vec![OsString::from_vec(b"/opt/\xffed".to_vec()), "--wait".into()]
            ))
        );
    }

    #[test]
    fn managed_commands_never_pass_on_a_delegation_marker() {
        // `eon-bash -c 'eon window stop ID'` with an inherited marker would
        // narrow that Stop to one generation.
        let inputs = crate::fixtures::inputs();
        let programs = ManagedPrograms {
            nu: "/managed/nu".into(),
            bash: "/managed/bash".into(),
            helix: Some("/managed/hx".into()),
            yazi: "/managed/yazi".into(),
            ya: "/managed/ya".into(),
            lazygit: "/managed/lazygit".into(),
            nu_vendor_autoload: None,
            bash_rc: None,
            shell_bin: None,
        };
        let config = Path::new("/private/eon");
        for tool in [
            Tool::Nu,
            Tool::Bash,
            Tool::Helix,
            Tool::Yazi,
            Tool::Ya,
            Tool::LazyGit,
        ] {
            let command = managed_command(tool, &programs, config, &[], &inputs.defaults).unwrap();
            for marker in ["EON_ATTACH_DELEGATED", "EON_ROUTED"] {
                assert_eq!(
                    command_environment(&command, marker),
                    Some(None),
                    "{tool:?} passes on {marker}"
                );
            }
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

    #[test]
    fn children_never_inherit_a_delegation_marker() {
        // The marker names one delegated attach (EON-C11); a terminal that
        // inherited it would carry it into every `eon` run there.
        // So would the routing marker, which names one routed command.
        for marker in ["EON_ATTACH_DELEGATED", "EON_ROUTED"] {
            let mut command = Command::new("/managed/orbit");
            command.env(marker, "g1-00000000000000000000000000000001");
            remove_agent_session(&mut command);
            assert!(
                command
                    .get_envs()
                    .any(|(variable, value)| variable == marker && value.is_none()),
                "{marker} reaches the child"
            );
        }
    }

    #[test]
    fn children_drop_the_launching_agent_session_and_keep_agent_settings() {
        // What Claude Code 2.1.303, Codex 0.166.0, OpenCode 2.0.32, Pi 1.4.0 and Grok
        // 1.0.62 export to the commands they run (eon-geigu), and what stays: settings
        // a person sets on purpose and markers left out on purpose.
        let session = [
            "CLAUDECODE",
            "CLAUDE_PID",
            "CLAUDE_EFFORT",
            "CLAUDE_CODE_CHILD_SESSION",
            "CLAUDE_CODE_SESSION_ID",
            "CLAUDE_CODE_SESSION_ATTENDED",
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CODE_EXECPATH",
            "CLAUDE_CODE_MESSAGING_SOCKET",
            "CLAUDE_CODE_MESSAGING_TOKEN",
            "CLAUDE_CODE_SESSION_ACCESS_TOKEN",
            "CLAUDE_SESSION_INGRESS_TOKEN_FILE",
            "CLAUDE_CODE_REMOTE_SESSION_ID",
            "CLAUDE_CODE_REMOTE_SESSION_UUID",
            "CLAUDE_CODE_CLOUD_SESSION_ID",
            "CLAUDE_CODE_BRIDGE_SESSION_ID",
            "CLAUDE_CODE_HOST_SESSION_ID",
            "CLAUDE_CODE_HOST_WORKTREE",
            "CLAUDE_CODE_HOST_WORKTREE_FENCE",
            "AI_AGENT",
            "CODEX_SESSION_ID",
            "CODEX_THREAD_ID",
            "CODEX_TOOL_CALL_ID",
            "CODEX_VERSION",
            "CODEX_PERMISSION_PROFILE",
            "CODEX_CI",
            "OPENCODE",
            "OPENCODE_SESSION_ID",
            "OPENCODE_TERMINAL",
            "PI_CODING_AGENT",
            "PI_SESSION_ID",
            "PI_SESSION_FILE",
            "PI_REASONING_LEVEL",
            "GROK_SESSION_ID",
        ];
        let kept = [
            "CLAUDE_CONFIG_DIR",
            "CLAUDE_CODE_FORCE_SESSION_PERSISTENCE",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_CONTAINER_ID",
            "ANTHROPIC_API_KEY",
            "CODEX_HOME",
            "CODEX_SANDBOX",
            "OPENAI_API_KEY",
            "OPENCODE_CONFIG",
            "AGENT",
            "PI_CODING_AGENT_DIR",
            "PI_MODEL",
            "GROK_LEADER_SOCKET",
            "TRACEPARENT",
            "EDITOR",
        ];
        let mut command = Command::new("/managed/orbit");
        for name in session.iter().chain(&kept) {
            command.env(name, "parent");
        }
        remove_agent_session(&mut command);

        for name in session {
            assert!(
                !command
                    .get_envs()
                    .any(|(variable, value)| variable == name && value.is_some()),
                "{name} reaches the child"
            );
        }
        for name in kept {
            assert!(
                command
                    .get_envs()
                    .any(|(variable, value)| variable == name && value.is_some()),
                "{name} is removed"
            );
        }
    }

    #[test]
    fn children_drop_an_agents_tool_settings_only_under_its_marker() {
        // What Claude Code 2.1.303's Bash tool and Codex 0.166.0's unified exec give
        // every command they run besides its session (eon-yxo2y). Each is removed only
        // with its exact value and its own runner's marker inherited, and Codex's
        // locale is replaced by the desktop session's; a person's other value, or the
        // same value without the marker, passes through.
        let claude = [("GIT_EDITOR", "true"), ("COREPACK_ENABLE_AUTO_PIN", "0")];
        let codex = [
            ("NO_COLOR", "1"),
            ("PAGER", "cat"),
            ("GIT_PAGER", "cat"),
            ("GH_PAGER", "cat"),
        ];
        let locale = [
            ("LANG", "C.UTF-8"),
            ("LC_CTYPE", "C.UTF-8"),
            ("LC_ALL", "C.UTF-8"),
        ];
        let kept = [("EDITOR", "hx"), ("LC_TIME", "pt_BR.UTF-8")];
        let launched = |inherited: &[(&'static str, &'static str)],
                        desktop: &[(&'static str, &'static str)]| {
            let lookup = |pairs: &[(&'static str, &'static str)], name: &str| {
                pairs
                    .iter()
                    .find(|(variable, _)| *variable == name)
                    .map(|(_, value)| OsString::from(value))
            };
            let inherited = [inherited, kept.as_slice()].concat();
            let mut command = Command::new("/managed/orbit");
            remove_agent_environment(
                &mut command,
                |name| lookup(&inherited, name),
                |name| lookup(desktop, name),
            );
            command
        };
        let child = |command: &Command, name: &str| {
            command
                .get_envs()
                .find(|(variable, _)| *variable == name)
                .map(|(_, value)| value.map(OsStr::to_owned))
        };
        let removed = |command: &Command, name: &str| child(command, name) == Some(None);
        let untouched = |command: &Command, name: &str| child(command, name).is_none();
        let all = [claude.as_slice(), codex.as_slice(), locale.as_slice()].concat();
        let desktop = [("LANG", "pt_BR.UTF-8"), ("LC_MESSAGES", "en_US.UTF-8")];

        // Fails when a runner's settings need the other runner's marker too.
        let claude_alone = launched(&[[("CLAUDECODE", "1")].as_slice(), &all].concat(), &desktop);
        for (name, _) in claude {
            assert!(
                removed(&claude_alone, name),
                "Claude: {name} reaches the child"
            );
        }
        for (name, _) in codex.iter().chain(&locale) {
            assert!(untouched(&claude_alone, name), "Claude: {name} is changed");
        }
        let codex_alone = launched(&[[("CODEX_CI", "1")].as_slice(), &all].concat(), &desktop);
        for (name, _) in codex.iter().chain(&locale[1..]) {
            assert!(
                removed(&codex_alone, name),
                "Codex: {name} reaches the child"
            );
        }
        // Fails when Codex's forced locale survives a desktop session that has its own.
        assert_eq!(
            child(&codex_alone, "LANG"),
            Some(Some(OsString::from("pt_BR.UTF-8")))
        );
        for (name, _) in claude {
            assert!(untouched(&codex_alone, name), "Codex: {name} is changed");
        }
        let both = launched(
            &[[("CLAUDECODE", "1"), ("CODEX_CI", "1")].as_slice(), &all].concat(),
            &desktop,
        );
        for (name, _) in claude.iter().chain(&codex).chain(&locale[1..]) {
            assert!(removed(&both, name), "both: {name} reaches the child");
        }
        for command in [&claude_alone, &codex_alone, &both] {
            for (name, _) in kept {
                assert!(untouched(command, name), "{name} is changed");
            }
            assert!(untouched(command, "LC_MESSAGES"));
        }

        // Without a desktop locale, LANG stays UTF-8 and the overrides go.
        let headless = launched(&[[("CODEX_CI", "1")].as_slice(), &locale].concat(), &[]);
        assert!(untouched(&headless, "LANG"));
        assert!(removed(&headless, "LC_CTYPE"));
        assert!(removed(&headless, "LC_ALL"));

        // Fails when a setting is removed without its marker, or when a person's
        // other value is taken for the agent's.
        let unmarked = launched(&all, &desktop);
        let own = launched(
            &[
                ("CLAUDECODE", "1"),
                ("CODEX_CI", "1"),
                ("GIT_EDITOR", "hx"),
                ("NO_COLOR", ""),
                ("LANG", "pt_BR.UTF-8"),
                ("LC_ALL", "pt_BR.UTF-8"),
                ("PAGER", "less"),
            ],
            &[("LANG", "en_US.UTF-8")],
        );
        for (command, case) in [(&unmarked, "no marker"), (&own, "a person's own values")] {
            for (name, _) in &all {
                assert!(untouched(command, name), "{case}: {name} is changed");
            }
        }
    }

    #[test]
    fn the_user_manager_read_drains_its_output_and_leaves_nothing_by_its_deadline() {
        // Fails when the desktop-locale read only stops waiting (eon-yxo2y): a
        // systemctl stalled on a bus that accepts and never answers, and a helper it
        // started, must both be gone by the deadline, and the launch goes on without a
        // desktop locale.
        let root = crate::supervisor::temporary_directory();
        let program = root.join("systemctl");
        let pids = root.join("pids");
        let script =
            |body: &str| crate::sessions::tests::script(&program, &format!("#!/bin/sh\n{body}\n"));
        script(&format!(
            "sleep 30 &\necho \"$$ $!\" > '{}'\nwait",
            pids.display()
        ));
        let started = Instant::now();
        assert_eq!(
            user_manager_environment(program.as_os_str(), Duration::from_secs(1)),
            None
        );
        assert!(started.elapsed() < Duration::from_secs(3));
        let pids = fs::read_to_string(&pids).unwrap();
        let [child, helper] = pids.split_whitespace().collect::<Vec<_>>()[..] else {
            panic!("pids: {pids}");
        };
        // Fails when the read leaves its own child unreaped.
        assert!(
            !Path::new(&format!("/proc/{child}")).exists(),
            "{child} outlives the read"
        );
        // The killed helper may remain a zombie, holding nothing, until its new parent
        // reaps it.
        let gone = |pid: &str| {
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                let state = fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .and_then(|stat| {
                        Some(
                            stat.rsplit(')')
                                .next()?
                                .split_whitespace()
                                .next()?
                                .to_owned(),
                        )
                    });
                if state.as_deref().is_none_or(|state| state == "Z") {
                    return true;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                thread::sleep(Duration::from_millis(10));
            }
        };
        assert!(gone(helper), "{helper} outlives the read");

        // Fails when output is read only after the child exits: past the pipe's
        // capacity, the child blocks until the deadline and the locale is lost.
        script(&format!(
            "yes FILLER={} | head -n 20000\necho LANG=pt_BR.UTF-8",
            "x".repeat(64)
        ));
        let large = user_manager_environment(program.as_os_str(), Duration::from_secs(5)).unwrap();
        assert!(large.len() > 1 << 20);
        assert_eq!(large.lines().last(), Some("LANG=pt_BR.UTF-8"));
        // Fails without the cap: finite output past it, from a child that exits
        // successfully within the deadline, gives no answer.
        script(&format!(
            "yes FILLER=x | head -c {}\necho LANG=pt_BR.UTF-8",
            (4 << 20) + 1
        ));
        assert_eq!(
            user_manager_environment(program.as_os_str(), Duration::from_secs(5)),
            None
        );
        // Output without end stops at the cap and gives no answer.
        script("yes FILLER=x");
        assert_eq!(
            user_manager_environment(program.as_os_str(), Duration::from_secs(5)),
            None
        );

        script("printf 'HOME=/home/person\\nLANG=pt_BR.UTF-8\\n'");
        assert_eq!(
            user_manager_environment(program.as_os_str(), Duration::from_secs(5)).as_deref(),
            Some("HOME=/home/person\nLANG=pt_BR.UTF-8\n")
        );
        script("echo LANG=pt_BR.UTF-8; exit 1");
        assert_eq!(
            user_manager_environment(program.as_os_str(), Duration::from_secs(5)),
            None
        );
        assert_eq!(
            user_manager_environment(root.join("missing").as_os_str(), Duration::from_secs(5)),
            None
        );
        fs::remove_dir_all(root).unwrap();
    }
}
