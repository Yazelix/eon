use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Implement,
    Review,
    /// A lead launch: exempt from the quota gate, admitted only by `launch`.
    Lead,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Self::Implement => "implement",
            Self::Review => "review",
            Self::Lead => "lead",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Medium,
    High,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub state_file: PathBuf,
    pub log_file: PathBuf,
    pub fallback: Option<String>,
    pub popup_backend: Option<String>,
    #[serde(default)]
    pub run: RunEnv,
    #[serde(default)]
    pub slots: SlotConfig,
    pub backends: Vec<Backend>,
    #[serde(default)]
    pub projects: BTreeMap<String, crate::cleanup::Project>,
    #[serde(default)]
    pub cleanup: crate::cleanup::Cleanup,
    /// Non-fatal findings of validation, such as removed keys; callers print them.
    #[serde(skip)]
    pub warnings: Vec<Warning>,
}

/// A configuration key that loads but has no effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// The backend the key belongs to.
    pub backend: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SlotConfig {
    pub count: u16,
    pub nice: i32,
    pub env: BTreeMap<String, String>,
    pub classes: Vec<SlotClass>,
    pub lock_files: Vec<PathBuf>,
    pub legacy_dir: Option<PathBuf>,
    pub local_runner: Vec<String>,
    pub boat: BoatConfig,
}

impl Default for SlotConfig {
    fn default() -> Self {
        Self {
            count: 2,
            nice: 19,
            env: BTreeMap::new(),
            classes: Vec::new(),
            lock_files: Vec::new(),
            legacy_dir: None,
            local_runner: Vec::new(),
            boat: BoatConfig::default(),
        }
    }
}

/// A command shape that `slots.boat.allow_shapes` can permit on Boat. The slot route
/// classifies each request's argv into at most one shape.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    NixGate,
    CargoBuild,
    CargoClippy,
    EonTest,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BoatConfig {
    pub allow_shapes: Vec<Shape>,
    pub environment: String,
    pub reserve_hours: f64,
    pub timeout_seconds: u64,
    pub jobs: u16,
    pub snapshot: String,
    pub large_max_memory_gib: u8,
}

impl Default for BoatConfig {
    fn default() -> Self {
        Self {
            allow_shapes: Vec::new(),
            environment: "eon-builds".into(),
            reserve_hours: 2.0,
            timeout_seconds: 3600,
            jobs: 4,
            snapshot: "eon-warm".into(),
            large_max_memory_gib: 0,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotClass {
    pub name: String,
    #[serde(default)]
    pub match_argv: Vec<Vec<String>>,
    pub slots: Vec<u16>,
}

impl SlotConfig {
    fn validate(&self) -> Result<(), String> {
        let boat = &self.boat;
        if boat.allow_shapes.iter().collect::<BTreeSet<_>>().len() != boat.allow_shapes.len()
            || boat.environment.is_empty()
            || boat.environment == "base"
            || boat.environment.len() > 63
            || !boat
                .environment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
            || !boat.reserve_hours.is_finite()
            || boat.reserve_hours < 0.0
            || !(120..=7200).contains(&boat.timeout_seconds)
            || !(1..=4).contains(&boat.jobs)
            || !(boat.large_max_memory_gib == 0 || (8..=12).contains(&boat.large_max_memory_gib))
            || boat.snapshot.len() > 63
            || boat.snapshot.starts_with('-')
            || !boat
                .snapshot
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("invalid slots.boat policy: use unique build shapes, a non-base environment, finite reserve, timeout 120..7200 and jobs 1..4".into());
        }
        if self.count == 0 || self.count == u16::MAX || !(0..=19).contains(&self.nice) {
            return Err("slots need a positive count below 65535 and nice from 0 to 19".into());
        }
        let paths = self.lock_files.iter().collect::<BTreeSet<_>>();
        if (self.legacy_dir.is_some() && paths.is_empty())
            || (!paths.is_empty()
                && (paths.len() != self.lock_files.len()
                    || !(self.count as usize..=self.count as usize + 1).contains(&paths.len())
                    || paths.iter().any(|p| !p.is_absolute())))
        {
            return Err("slots.lock_files needs one unique absolute path per slot, with an optional extra slot".into());
        }
        if self.legacy_dir.as_ref().is_some_and(|p| !p.is_absolute()) {
            return Err("slots.legacy_dir must be absolute".into());
        }
        if !self.local_runner.is_empty() && !command_valid(&self.local_runner) {
            return Err("slots.local_runner must be a nonempty command prefix".into());
        }
        if self
            .env
            .iter()
            .any(|(key, value)| key.is_empty() || key.contains(['=', '\0']) || value.contains('\0'))
        {
            return Err("invalid slots.env entry".into());
        }
        let mut names = BTreeSet::new();
        for (i, class) in self.classes.iter().enumerate() {
            if class.name.is_empty()
                || !class
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || !names.insert(&class.name)
                || class.slots.is_empty()
                || class.slots.iter().any(|n| !(1..=self.count).contains(n))
                || class.slots.iter().collect::<BTreeSet<_>>().len() != class.slots.len()
                || class
                    .match_argv
                    .iter()
                    .any(|pattern| !command_valid(pattern))
                || (class.match_argv.is_empty() != (i + 1 == self.classes.len()))
            {
                return Err(format!(
                    "invalid slot class {:?}: use unique names and valid slots, with a final class without match_argv",
                    class.name
                ));
            }
        }
        Ok(())
    }
}

/// Environment changes for headless runs only; checks never see them.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunEnv {
    #[serde(default)]
    pub unset_env: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub commands: BTreeMap<Role, Vec<String>>,
    pub stdin: Stdin,
    pub efforts: Vec<String>,
    pub default_effort: String,
    pub add_dir: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Stdin {
    Brief,
    Null,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backend {
    pub name: String,
    /// The subscription account this backend draws on; backends that share one account
    /// declare the same name. Defaults to the backend's own name.
    pub account: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    pub roles: Vec<Role>,
    pub commands: BTreeMap<Level, Vec<String>>,
    pub popup_command: Option<Vec<String>>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub unset_env: Vec<String>,
    pub guard: Vec<String>,
    #[serde(default)]
    pub required_files: Vec<PathBuf>,
    #[serde(default)]
    pub excluded_roots: Vec<PathBuf>,
    pub account_check: Option<Vec<String>>,
    pub quota: Option<Reader>,
    #[serde(default)]
    pub allow_unknown_quota: bool,
    #[serde(default)]
    pub limits: BTreeMap<Role, Limits>,
    pub cap: Option<Cap>,
    pub trusted_settings: Option<PathBuf>,
    #[serde(default)]
    pub prepare: Vec<Vec<String>>,
    pub run: Option<Run>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reader {
    pub command: Option<Vec<String>>,
    pub parser: Parser,
    pub agy_group: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub refresh: Option<Vec<String>>,
    #[serde(default = "refresh_timeout")]
    pub refresh_timeout_seconds: u64,
    /// Windows this account does not have: a limit naming one does not hold when it is missing.
    #[serde(default)]
    pub absent_windows: Vec<String>,
    /// Maximum age in seconds for limited backends and CodexBar readers.
    /// Uses [`DEFAULT_MAX_AGE_SECONDS`] when unset.
    pub max_age_seconds: Option<u64>,
}

/// Default maximum age for limited backends and CodexBar readers.
pub const DEFAULT_MAX_AGE_SECONDS: u64 = 600;

/// How long a stale Claude reading (a newer poll failed) still admits a limited backend, in
/// place of `max_age_seconds`. Fixed: Lucca's approved rule of 2026-11-19 names it (EON-C40).
pub const STALE_BOUND_SECONDS: u64 = 1800;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Parser {
    Agy,
    ClaudeText,
    ClaudeJson,
    Codexbar,
    EonCodex,
    EonClaude,
    RemainingJson,
}

impl Parser {
    pub fn native(self) -> bool {
        matches!(self, Self::EonCodex | Self::EonClaude)
    }
}

impl Reader {
    pub fn timeout_seconds(&self) -> u64 {
        self.timeout_seconds.unwrap_or_else(timeout)
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    #[serde(default)]
    pub min_remaining: BTreeMap<String, f64>,
    #[serde(default)]
    pub max_used: BTreeMap<String, f64>,
    /// Removed: parsed so older files load, ignored, and reported as a [`Warning`].
    #[serde(default)]
    pub allow_missing: Option<bool>,
    #[serde(default)]
    pub strictly_above_min: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cap {
    pub max: usize,
    pub executable: String,
    #[serde(default)]
    pub runtime_executables: Vec<PathBuf>,
    pub script: Option<PathBuf>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub exclude_args: Vec<String>,
}

fn yes() -> bool {
    true
}
fn refresh_timeout() -> u64 {
    90
}
fn timeout() -> u64 {
    30
}

fn command_valid(command: &[String]) -> bool {
    command.first().is_some_and(|s| !s.is_empty()) && command.iter().all(|s| !s.contains('\0'))
}

impl Backend {
    /// The account this backend's quota describes.
    pub fn account(&self) -> &str {
        self.account.as_deref().unwrap_or(&self.name)
    }

    /// Whether this backend sets limits, so its admissions consume quota observations.
    pub fn limited(&self) -> bool {
        !self.limits.is_empty()
    }

    pub(crate) fn uses_codexbar(&self) -> bool {
        self.quota
            .as_ref()
            .is_some_and(|reader| matches!(reader.parser, Parser::Codexbar))
    }

    /// Maximum observation age for limited backends and CodexBar readers.
    pub fn max_age_seconds(&self) -> Option<u64> {
        (self.limited() || self.uses_codexbar()).then(|| {
            self.quota
                .as_ref()
                .and_then(|reader| reader.max_age_seconds)
                .unwrap_or(DEFAULT_MAX_AGE_SECONDS)
        })
    }

    /// The windows its reader declares absent.
    pub fn absent_windows(&self) -> &[String] {
        self.quota
            .as_ref()
            .map_or(&[], |reader| reader.absent_windows.as_slice())
    }

    pub fn popup_command(&self) -> Option<Vec<String>> {
        if let Some(command) = &self.popup_command {
            return Some(command.clone());
        }
        let executable = self.commands.get(&Level::High)?.first()?;
        let tail: &[&str] = match Path::new(executable).file_name()?.to_str()? {
            "codex" => &["resume", "--no-daemon"],
            "claude" => &["--resume"],
            "grok" | "opencode" | "pi" => &[],
            _ => return None,
        };
        Some(
            std::iter::once(executable.clone())
                .chain(tail.iter().map(|s| s.to_string()))
                .collect(),
        )
    }
}

fn run_valid(run: &Run) -> bool {
    let most_add_dirs = usize::from(run.add_dir.is_some());
    !run.commands.is_empty()
        && run.commands.values().all(|c| {
            command_valid(c)
                && c[0] != "{add_dirs}"
                && c.iter()
                    .filter(|a| a.contains("{add_dirs}"))
                    .all(|a| a == "{add_dirs}")
                && c.iter().filter(|a| *a == "{add_dirs}").count() <= most_add_dirs
                && (run.stdin == Stdin::Brief || c.iter().any(|a| a.contains("{brief}")))
        })
        && run.efforts.iter().all(|e| {
            !e.is_empty()
                && e.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        && run.efforts.contains(&run.default_effort)
        && run.add_dir.as_ref().is_none_or(|f| {
            f.iter().filter(|a| *a == "{dir}").count() == 1 && f.iter().all(|a| !a.contains('\0'))
        })
}

impl Config {
    /// Parses and validates a configuration; `program` names the host in error messages.
    pub fn parse(text: &str, program: &str) -> Result<Self, String> {
        let mut config: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        config.warnings = config.validate(program)?;
        Ok(config)
    }

    fn validate(&self, program: &str) -> Result<Vec<Warning>, String> {
        let mut warnings = Vec::new();
        self.slots.validate()?;
        crate::cleanup::validate(self)?;
        if self.state_file.as_os_str().is_empty()
            || self.log_file.as_os_str().is_empty()
            || self.state_file == self.log_file
            || self.backends.is_empty()
        {
            return Err(
                "distinct state_file/log_file and at least one backend are required".into(),
            );
        }
        let mut names = BTreeSet::new();
        for b in &self.backends {
            let fail = |message| Err(format!("{}: {message}", b.name));
            if b.name.is_empty()
                || !b
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || !names.insert(&b.name)
            {
                return fail("backend names must be unique identifiers");
            }
            if !command_valid(&b.guard)
                || [Level::Medium, Level::High].iter().any(|level| {
                    !b.commands.get(level).is_some_and(|c| {
                        command_valid(c) && c.iter().filter(|a| *a == "{prompt}").count() == 1
                    })
                })
            {
                return fail(
                    "guard and both level commands (with one {prompt} argument) are required",
                );
            }
            if b.roles.iter().collect::<BTreeSet<_>>().len() != b.roles.len() {
                return fail("duplicate role");
            }
            if b.popup_command.as_ref().is_some_and(|command| {
                !command_valid(command) || command.iter().any(|arg| arg.contains("{prompt}"))
            }) {
                return fail("popup_command needs a nonempty executable, no NUL or {prompt}");
            }
            if b.account_check.as_ref().is_some_and(|c| !command_valid(c))
                || b.prepare.iter().any(|c| !command_valid(c))
                || b.quota.as_ref().is_some_and(|q| {
                    (!q.parser.native() && q.command.as_ref().is_none_or(|c| !command_valid(c)))
                        || q.timeout_seconds == Some(0)
                        || q.max_age_seconds == Some(0)
                        || q.refresh_timeout_seconds == 0
                        || q.refresh.as_ref().is_some_and(|c| !command_valid(c))
                })
            {
                return fail(
                    "invalid check, preparation or reader command/timeout/max_age_seconds",
                );
            }
            if b.quota.as_ref().is_some_and(|q| {
                q.agy_group.as_ref().is_some_and(|g| {
                    !matches!(q.parser, Parser::Agy)
                        || g.is_empty()
                        || g.trim() != g
                        || g.contains(['\t', '\n', '\r'])
                })
            }) {
                return fail(
                    "agy_group needs parser = \"agy\" and a non-empty single-line name without surrounding spaces",
                );
            }
            if b.cap.as_ref().is_some_and(|c| {
                c.max == 0
                    || c.executable.is_empty()
                    || c.executable.contains('/')
                    || c.runtime_executables
                        .iter()
                        .chain(c.script.iter())
                        .any(|p| !p.is_absolute())
            }) {
                return fail(
                    "cap needs positive max, an executable basename and absolute runtime/script paths",
                );
            }
            if b.env
                .keys()
                .chain(&b.unset_env)
                .chain(b.cap.iter().flat_map(|c| c.env.keys()))
                .any(|k| k.is_empty() || k.contains(['=', '\0']))
                || b.env
                    .values()
                    .chain(b.cap.iter().flat_map(|c| c.env.values()))
                    .any(|v| v.contains('\0'))
                || b.unset_env.iter().any(|k| b.env.contains_key(k))
            {
                return fail("invalid or conflicting environment");
            }
            if b.run.as_ref().is_some_and(|r| !run_valid(r)) {
                return fail(
                    "run needs commands, efforts containing default_effort, the brief on stdin or as {brief}, and add_dir with one {dir} for a whole {add_dirs} argument",
                );
            }
            for (role, limit) in &b.limits {
                if limit
                    .min_remaining
                    .values()
                    .chain(limit.max_used.values())
                    .any(|v| !v.is_finite() || !(0.0..=100.0).contains(v))
                {
                    return fail("quota limits must be finite percentages");
                }
                if limit.allow_missing.is_some() {
                    warnings.push(Warning {
                        backend: b.name.clone(),
                        text: format!(
                            "backends.{}.limits.{}.allow_missing is removed and ignored; \
                             declare the account's missing windows in [backends.quota] absent_windows",
                            b.name,
                            role.name()
                        ),
                    });
                }
            }
            if b.limits.contains_key(&Role::Lead) {
                return fail("limits.lead is invalid: lead launches check no limits");
            }
            if b.limited() && b.allow_unknown_quota {
                return fail("allow_unknown_quota is invalid on a backend that sets limits");
            }
            if b.limited() && b.account.is_none() {
                return fail("a backend that sets limits must declare account");
            }
            if b.run
                .as_ref()
                .is_some_and(|r| r.commands.contains_key(&Role::Lead))
            {
                return fail("run.commands.lead is invalid: lead launches run only through launch");
            }
            let absent = b.absent_windows();
            if absent
                .iter()
                .any(|w| w.is_empty() || w == "*" || w.contains('\0'))
                || absent.iter().collect::<BTreeSet<_>>().len() != absent.len()
            {
                return fail(
                    "absent_windows entries must be unique window names, not empty or \"*\"",
                );
            }
        }
        if self.run.unset_env.iter().any(|p| {
            let name = p.strip_suffix('*').unwrap_or(p);
            name.is_empty() || name.contains(['=', '\0', '*'])
        }) || self
            .run
            .env
            .iter()
            .any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.contains('\0'))
        {
            return Err(
                "[run] needs valid variable names; an unset_env name may end in one *".into(),
            );
        }
        if crate::run::MANAGER_ENV
            .iter()
            .any(|key| self.run.env.contains_key(*key) || crate::run::scrubbed(&self.run, key))
        {
            return Err(format!(
                "[run] must keep XDG_RUNTIME_DIR and DBUS_SESSION_BUS_ADDRESS: \
                 {program} run reaches the systemd user manager through them"
            ));
        }
        if let Some(fallback) = &self.fallback {
            let Some(b) = self.backends.iter().find(|b| &b.name == fallback) else {
                return Err("fallback backend does not exist".into());
            };
            if !b.roles.is_empty() {
                return Err("fallback backend must have no normal routing roles".into());
            }
        }
        if let Some(pin) = &self.popup_backend
            && !self.backends.iter().any(|backend| {
                &backend.name == pin
                    && (backend.roles.contains(&Role::Implement)
                        || self.fallback.as_ref() == Some(pin))
            })
        {
            return Err(
                "popup_backend must name an implement backend or the configured fallback".into(),
            );
        }
        Ok(warnings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const EXAMPLE: &str = include_str!("../config.example.toml");

    #[test]
    fn example_is_valid() {
        Config::parse(EXAMPLE, "rota").unwrap();
    }

    #[test]
    fn native_readers_accept_shared_commands_and_custom_readers_require_commands() {
        for parser in ["eon_codex", "eon_claude"] {
            let native = EXAMPLE.replace("parser = \"eon_codex\"", &format!("parser = '{parser}'"));
            Config::parse(&native, "rota").unwrap();
            for field in [
                "command = ['eon', 'session', '--json']",
                "command = []",
                "timeout_seconds = 10",
            ] {
                Config::parse(
                    &native.replace(
                        &format!("parser = '{parser}'"),
                        &format!("parser = '{parser}'\n{field}"),
                    ),
                    "rota",
                )
                .unwrap();
            }
        }
        for field in [
            "",
            "command = []",
            "command = ['printf', '{}']\ntimeout_seconds = 0",
            "command = ['printf', '{}']\nmax_age_seconds = 0",
        ] {
            let text = EXAMPLE.replace(
                "parser = \"eon_codex\"",
                &format!("parser = 'remaining_json'\n{field}"),
            );
            assert!(Config::parse(&text, "rota").is_err(), "{field}");
        }
        let custom = EXAMPLE.replace(
            "parser = \"eon_codex\"",
            "parser = 'remaining_json'\ncommand = ['printf', '{}']",
        );
        let config = Config::parse(&custom, "rota").unwrap();
        assert_eq!(
            config.backends[2].quota.as_ref().unwrap().timeout_seconds(),
            30
        );
        assert_eq!(
            config.backends[0].quota.as_ref().unwrap().timeout_seconds(),
            60
        );
    }

    #[test]
    fn popup_pin_requires_an_implement_backend_or_the_configured_fallback() {
        for name in ["alpha", "main"] {
            Config::parse(&format!("popup_backend = '{name}'\n{EXAMPLE}"), "rota").unwrap();
        }
        for name in ["", "missing", "alpha"] {
            let config = format!("popup_backend = '{name}'\n{EXAMPLE}").replace(
                "roles = [\"implement\", \"review\"]",
                "roles = [\"review\"]",
            );
            let error = Config::parse(&config, "rota").unwrap_err();
            assert!(
                error.contains("implement backend or the configured fallback"),
                "{error}"
            );
        }
    }

    #[test]
    fn rejects_invalid_config() {
        for (from, to) in [
            ("name = \"alpha\"", "name = \"bad name\""),
            (
                "guard = [\"test\", \"-f\", \"/PATH/TO/guard\"]",
                "guard = []",
            ),
            ("max = 1", "max = 0"),
            ("max = 1", "max = 1\nruntime_executables = ['relative']"),
            ("five_hour = 60", "five_hour = 101"),
            ("fallback = \"main\"", "fallback = \"absent\""),
            ("roles = []", "roles = [\"implement\"]"),
            ("{prompt}", "missing-prompt"),
            ("parser = \"agy\"", "parser = \"agy\"\nagy_group = \"\""),
            (
                "parser = \"agy\"",
                "parser = \"agy\"\nagy_group = \" Gemini Models\"",
            ),
            (
                "parser = \"agy\"",
                "parser = \"agy\"\nagy_group = \"Gemini\\nModels\"",
            ),
            (
                "parser = \"claude_text\"",
                "parser = \"claude_text\"\nagy_group = \"Gemini Models\"",
            ),
            ("default_effort = \"high\"", "default_effort = \"ultra\""),
            ("stdin = \"brief\"", "stdin = \"null\""),
            ("add_dir = [\"--add-dir\", \"{dir}\"]", ""),
            (
                "\"--add-dir\", \"{dir}\"]",
                "\"--add-dir\", \"{dir}\", \"{dir}\"]",
            ),
            (
                "\"workspace-write\", \"{add_dirs}\"",
                "\"workspace-write\", \"x{add_dirs}\"",
            ),
            (
                "efforts = [\"medium\", \"high\"",
                "efforts = [\"medium\", \"hi gh\"",
            ),
            ("\"TERMINAL_*\"", "\"TERM*INAL\""),
            ("\"TERMINAL_*\"", "\"*\""),
        ] {
            assert_ne!(EXAMPLE, EXAMPLE.replace(from, to));
            assert!(
                Config::parse(&EXAMPLE.replace(from, to), "rota").is_err(),
                "{from}"
            );
        }
        assert!(Config::parse(&format!("typo = true\n{EXAMPLE}"), "rota").is_err());
    }

    #[test]
    fn boat_shapes_accept_each_known_name_once_and_refuse_unknown_or_repeated_names() {
        let with = |shapes: &str| {
            EXAMPLE.replace("allow_shapes = []", &format!("allow_shapes = {shapes}"))
        };
        let config = Config::parse(
            &with("['nix-gate', 'cargo-build', 'cargo-clippy', 'eon-test']"),
            "rota",
        )
        .unwrap();
        assert_eq!(
            config.slots.boat.allow_shapes,
            [
                Shape::NixGate,
                Shape::CargoBuild,
                Shape::CargoClippy,
                Shape::EonTest
            ]
        );
        let error = Config::parse(&with("['bogus']"), "rota").unwrap_err();
        assert!(error.contains("unknown variant `bogus`"), "{error}");
        let error = Config::parse(&with("['cargo-clippy', 'cargo-clippy']"), "rota").unwrap_err();
        assert!(error.contains("invalid slots.boat policy"), "{error}");
    }

    #[test]
    fn example_run_settings_parse() {
        let config = Config::parse(EXAMPLE, "rota").unwrap();
        assert_eq!(config.run.unset_env.len(), 3);
        assert_eq!(config.run.env.len(), 1);
        let run = config.backends[2].run.as_ref().unwrap();
        assert_eq!(run.stdin, Stdin::Brief);
        assert_eq!(run.default_effort, "high");
        assert!(run.commands.contains_key(&Role::Review));
        assert!(!config.backends[2].roles.contains(&Role::Review));
        assert!(config.backends[0].run.is_none());
    }

    #[test]
    fn rejects_add_dirs_as_the_program() {
        let text = EXAMPLE
            .replace("\"workspace-write\", \"{add_dirs}\"", "\"workspace-write\"")
            .replace("implement = [\"codex\"", "implement = [\"{add_dirs}\"");
        assert_ne!(text.matches("{add_dirs}").count(), 0);
        assert!(Config::parse(&text, "rota").is_err());
        let text = EXAMPLE.replace("\"workspace-write\", \"{add_dirs}\"", "\"workspace-write\"");
        Config::parse(&text, "rota").unwrap();
    }

    #[test]
    fn run_settings_keep_the_manager_environment() {
        let unset = "unset_env = [\"DISPLAY\", \"WAYLAND_DISPLAY\", \"TERMINAL_*\"]";
        for bad in ["\"XDG_RUNTIME_DIR\"", "\"XDG_*\"", "\"DBUS_*\""] {
            let text = EXAMPLE.replace(unset, &format!("unset_env = [{bad}]"));
            assert!(Config::parse(&text, "rota").is_err(), "{bad}");
        }
        let text = EXAMPLE.replace(
            "TERMINAL_RUNTIME_DIR = ",
            "DBUS_SESSION_BUS_ADDRESS = \"unix:path=/x\"\nTERMINAL_RUNTIME_DIR = ",
        );
        assert!(Config::parse(&text, "rota").is_err());
        Config::parse(
            &EXAMPLE.replace(unset, "unset_env = [\"XDG_SESSION_ID\"]"),
            "rota",
        )
        .unwrap();
    }

    #[test]
    fn the_quota_gate_rejects_contradictory_settings() {
        let limited = "name = \"secondary\"\naccount = \"secondary\"\n";
        assert_eq!(EXAMPLE.matches(limited).count(), 1);
        for (from, to, error) in [
            (
                limited,
                "name = \"secondary\"\n",
                "secondary: a backend that sets limits must declare account",
            ),
            (
                limited,
                "name = \"secondary\"\naccount = \"secondary\"\nallow_unknown_quota = true\n",
                "secondary: allow_unknown_quota is invalid on a backend that sets limits",
            ),
            (
                "[backends.limits.review.max_used]\n",
                "[backends.limits.lead.max_used]\nfive_hour = 1\n[backends.limits.review.max_used]\n",
                "secondary: limits.lead is invalid: lead launches check no limits",
            ),
            (
                "[backends.run.commands]\n",
                "[backends.run.commands]\nlead = [\"codex\", \"exec\", \"-\"]\n",
                "code: run.commands.lead is invalid: lead launches run only through launch",
            ),
        ] {
            assert_eq!(EXAMPLE.matches(from).count(), 1, "{from}");
            assert_eq!(
                Config::parse(&EXAMPLE.replace(from, to), "rota").unwrap_err(),
                error
            );
        }
        let absent = "absent_windows = [\"five_hour\"]";
        assert_eq!(EXAMPLE.matches(absent).count(), 1);
        for bad in ["[\"\"]", "[\"*\"]", "[\"five_hour\", \"five_hour\"]"] {
            let error = Config::parse(
                &EXAMPLE.replace(absent, &format!("absent_windows = {bad}")),
                "rota",
            )
            .unwrap_err();
            assert!(error.contains("absent_windows entries"), "{bad}: {error}");
        }
        // Backends without limits keep allow_unknown_quota and need no account.
        let config = Config::parse(EXAMPLE, "rota").unwrap();
        assert!(config.warnings.is_empty());
        assert!(config.backends[4].allow_unknown_quota && !config.backends[4].limited());
        assert_eq!(config.backends[1].absent_windows(), ["five_hour"]);
        assert!(config.backends[1].roles.contains(&Role::Lead));
        // The example's limited reader has a 600 s default, while its unlimited backend has no bound.
        assert_eq!(config.backends[1].max_age_seconds(), Some(600));
        assert_eq!(config.backends[4].max_age_seconds(), None);
        let text = EXAMPLE.replace(absent, &format!("{absent}\nmax_age_seconds = 2000"));
        let config = Config::parse(&text, "rota").unwrap();
        assert_eq!(config.backends[1].max_age_seconds(), Some(2000));
        let error = Config::parse(
            &EXAMPLE.replace(absent, &format!("{absent}\nmax_age_seconds = 0")),
            "rota",
        )
        .unwrap_err();
        assert!(error.contains("max_age_seconds"), "{error}");
    }

    #[test]
    fn allow_missing_is_a_warned_tombstone() {
        let text = EXAMPLE.replace(
            "[backends.limits.review.max_used]\n",
            "[backends.limits.review]\nallow_missing = false\n[backends.limits.review.max_used]\n",
        );
        let config = Config::parse(&text, "rota").unwrap();
        assert_eq!(
            config.warnings,
            [Warning {
                backend: "secondary".into(),
                text: "backends.secondary.limits.review.allow_missing is removed and ignored; \
                       declare the account's missing windows in [backends.quota] absent_windows"
                    .into(),
            }]
        );
    }

    #[test]
    fn accepts_agy_group_on_agy_parser() {
        let config = Config::parse(
            &EXAMPLE.replace(
                "parser = \"agy\"",
                "parser = \"agy\"\nagy_group = \"Gemini Models\"",
            ),
            "rota",
        )
        .unwrap();
        assert_eq!(
            config.backends[0]
                .quota
                .as_ref()
                .unwrap()
                .agy_group
                .as_deref(),
            Some("Gemini Models")
        );
    }
}
