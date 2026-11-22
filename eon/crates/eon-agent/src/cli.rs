use crate::config::{Config, Role};
use crate::{decision, launch, ps, readings, run as headless, slots};
use std::io::{IsTerminal, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::{env, fs, process::ExitCode};

/// What the embedding program supplies. `rota` and `eon agent` differ only here; every other
/// behaviour is the core's.
pub struct Host<'a> {
    /// How usage and every message line name the program: "rota" or "eon agent".
    pub program: &'a str,
    /// Arguments after the executable that re-invoke this same program (the slot delegate).
    pub reinvoke: &'a [&'a str],
    /// The host's own configuration file, read before rota's; Err when it cannot be located.
    pub config: Result<Option<PathBuf>, String>,
    /// How usage names that file.
    pub config_label: Option<&'a str>,
    /// `slot --config FILE` is accepted.
    pub slot_config_flag: bool,
    /// `status` names the configuration file it read.
    pub report_config_file: bool,
    /// Quota facts for the native parsers (see [`readings::SessionReader`]).
    pub session: &'a readings::SessionReader<'a>,
    /// The remote route for `slot run` (Eon's Boat); `None` runs every command locally.
    pub route: Option<&'a dyn slots::Route>,
}

impl<'a> Host<'a> {
    /// The standalone `rota` program: rota's own configuration file only, no host facts.
    pub fn rota(session: &'a readings::SessionReader<'a>) -> Self {
        Host {
            program: "rota",
            reinvoke: &[],
            config: Ok(None),
            config_label: None,
            slot_config_flag: false,
            report_config_file: false,
            session,
            route: None,
        }
    }
}

const COMMANDS: [&str; 8] = [
    "launch [--role implement|review|lead] [--level medium|high] [--only BACKEND] [--project NAME] [--name NAME] [--add-dir DIR]... [--wait DURATION] [--dry-run|--json] <prompt-file>",
    "launch --popup [--only BACKEND] [--dry-run]",
    "run --brief FILE --name NAME --timeout DURATION [--role implement|review] [--only BACKEND] [--project NAME] [--cwd DIR] [--effort LEVEL] [--add-dir DIR]... [--wait DURATION] [--json]",
    "ps [--json] [--all]",
    "slot run [--class NAME] [--quiet-exempt] [--front] -- COMMAND...",
    "slot ls [--json]",
    "retire NAME [--project P] [--bead ID] [--target-only] [--dry-run] [--json]",
    "status [--json]",
];
const ROTA_CONFIG_HELP: &str =
    "$ROTA_CONFIG or $XDG_CONFIG_HOME/rota/config.toml (default ~/.config/rota/config.toml)";

fn usage(host: &Host<'_>) -> String {
    let mut text = String::new();
    for (index, command) in COMMANDS.iter().enumerate() {
        text += if index == 0 { "Usage: " } else { "\n       " };
        text += host.program;
        text += " ";
        match command.strip_prefix("slot ") {
            Some(rest) if host.slot_config_flag => {
                text += "slot [--config FILE] ";
                text += rest;
            }
            _ => text += command,
        }
    }
    match host.config_label {
        Some(label) => format!("{text}\nConfig: {label}; when absent, {ROTA_CONFIG_HELP}"),
        None => format!("{text}\nConfig: {ROTA_CONFIG_HELP}"),
    }
}

fn fallback_config_path() -> Result<PathBuf, String> {
    env::var_os("ROTA_CONFIG")
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
                .map(|root| root.join("rota/config.toml"))
        })
        .ok_or_else(|| "cannot locate config: set ROTA_CONFIG".into())
}

/// Locates configuration without parsing it or running any backend checks.
/// Relative candidates use `directory`; reported paths keep their literal spelling.
pub fn config_source(primary: Option<&Path>, directory: &Path) -> Result<Option<PathBuf>, String> {
    let exists = |path: &Path| match fs::metadata(directory.join(path)) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot read config: {error}")),
    };
    if let Some(path) = primary
        && exists(path)?
    {
        return Ok(Some(path.to_owned()));
    }
    let Ok(path) = fallback_config_path() else {
        return Ok(None);
    };
    Ok(exists(&path)?.then_some(path))
}

/// A loaded configuration: the file rota parsed, where it came from, and the warnings the
/// host's route returned for its own keys (slot commands print them).
struct Loaded {
    config: Config,
    path: PathBuf,
    warnings: Vec<String>,
}

/// Parses a configuration, then lets the host's route check its own keys in the same text.
fn parse_config(text: &str, host: &Host<'_>) -> Result<(Config, Vec<String>), String> {
    let config = Config::parse(text, host.program)?;
    let warnings = match host.route {
        Some(route) => route.check_config(text)?,
        None => Vec::new(),
    };
    Ok((config, warnings))
}

fn load_config(primary: Option<&Path>, host: &Host<'_>) -> Result<Loaded, String> {
    let path = match config_source(primary, Path::new("."))? {
        Some(path) => path,
        None => fallback_config_path()?,
    };
    let (config, warnings) = parse_config(
        &fs::read_to_string(&path).map_err(|e| format!("cannot read config: {e}"))?,
        host,
    )?;
    Ok(Loaded {
        config,
        path,
        warnings,
    })
}

fn run(
    arguments: impl Iterator<Item = std::ffi::OsString>,
    host: &Host<'_>,
) -> Result<u8, (u8, String)> {
    let usage = usage(host);
    let load = || load_config(host.config.as_ref().map_err(Clone::clone)?.as_deref(), host);
    // Each admission attempt reads the configuration again (EON-C40 clause 7).
    let load_admission = || load().map(|loaded| loaded.config).map_err(|e| (2, e));
    let args: Vec<_> = arguments
        .map(|arg| {
            arg.into_string()
                .map_err(|_| (2, "arguments must be UTF-8".into()))
        })
        .collect::<Result<_, _>>()?;
    if args == ["--help"] || args == ["-h"] {
        println!("{usage}");
        return Ok(0);
    }
    if let [internal, rest @ ..] = args.as_slice()
        && internal == "_heavy-memory"
    {
        return slots::memory::helper(rest);
    }
    if let [internal, path, separator, argv @ ..] = args.as_slice()
        && internal == "_slot-local-exec"
        && separator == "--"
    {
        return slots::delegate::execute(Path::new(path), argv, host.route);
    }
    if args.first().is_some_and(|s| s == "retire") {
        if args[1..] == ["--help"] || args[1..] == ["-h"] {
            println!("{}", crate::cleanup::help(host.program));
            return Ok(0);
        }
        let options = crate::cleanup::parse(&args[1..])?;
        let config = load().map_err(|e| (2, e))?.config;
        return crate::cleanup::run(&config, &options);
    }
    if args.first().is_some_and(|s| s == "launch") {
        let options = launch::parse(&args[1..])?;
        let result = launch::run(&options, host, &load_admission).map(|()| 0);
        if options.popup
            && !options.dry_run
            && std::io::stdin().is_terminal()
            && std::io::stderr().is_terminal()
            && let Err((code, message)) = &result
        {
            print_error(host.program, message);
            let _ = write!(std::io::stderr(), "\nPress Enter to close");
            let _ = std::io::stderr().flush();
            let _ = std::io::stdin().read_line(&mut String::new());
            return Ok(*code);
        }
        return result;
    }
    if args.first().is_some_and(|s| s == "run") {
        let options = headless::parse(&args[1..])?;
        return headless::run(&options, host, &load_admission).map(|()| 0);
    }
    if args.first().is_some_and(|s| s == "ps") {
        let options = ps::parse(&args[1..])?;
        let config = load().map_err(|e| (2, e))?.config;
        return ps::run(&config, &options, host.program).map(|()| 0);
    }
    if args.first().is_some_and(|s| s == "slot") {
        let ((config, warnings), rest) =
            if host.slot_config_flag && args.get(1).is_some_and(|s| s == "--config") {
                let path = args
                    .get(2)
                    .ok_or_else(|| (2, "slot --config needs a file".into()))?;
                let text = fs::read_to_string(path).map_err(|e| (2, e.to_string()))?;
                (parse_config(&text, host).map_err(|e| (2, e))?, &args[3..])
            } else {
                let loaded = load().map_err(|e| (2, e))?;
                ((loaded.config, loaded.warnings), &args[1..])
            };
        for text in warnings {
            let _ = writeln!(std::io::stderr(), "{}: warning: {text}", host.program);
        }
        let keys = config.slots.route_keys();
        if host.route.is_none() && !keys.is_empty() {
            let program = host.program;
            let need = if keys.len() == 1 { "needs" } else { "need" };
            let _ = writeln!(
                std::io::stderr(),
                "{program}: warning: {} {need} eon agent; {program} runs every slot command locally",
                keys.join(" and ")
            );
        }
        let options = slots::parse(rest, host.program)?;
        return slots::run(&config, options, host);
    }
    if args != ["status"] && args != ["status", "--json"] {
        return Err((2, usage));
    }
    let Loaded {
        config,
        path: config_path,
        ..
    } = load().map_err(|e| (2, e))?;
    let cwd = env::current_dir().map_err(|e| (1, e.to_string()))?;
    let indices: Vec<_> = (0..config.backends.len()).collect();
    let ledger = crate::admissions::Ledger::load(&config.state_file);
    let readings = readings::collect(
        &config,
        &indices,
        &cwd,
        readings::QuotaMode::Read,
        &ledger,
        host.session,
    );
    if args.len() == 2 {
        let status: Vec<_> = config.backends.iter().zip(&readings).map(|(b,r)| {
            let mut value = serde_json::json!({ "backend": b.name, "enabled": b.enabled, "reading": r,
                "implement": status_for(&config, b, r, Role::Implement, host.program), "review": status_for(&config, b, r, Role::Review, host.program),
                "lead": status_for(&config, b, r, Role::Lead, host.program) });
            if let Some(identity) = &r.identity {
                value["reading"]["source"]["identity"] = serde_json::json!(identity);
            }
            if host.report_config_file {
                value["config_file"] = match config_path.to_str() {
                    Some(path) => serde_json::json!(path),
                    None => serde_json::json!(config_path.as_os_str().as_bytes()),
                };
            }
            value
        }).collect();
        println!("{}", serde_json::to_string_pretty(&status).unwrap());
    } else {
        if host.report_config_file {
            println!("config: {}", config_path.display());
        }
        for (b, r) in config.backends.iter().zip(&readings) {
            println!(
                "{}: implement={} review={} lead={} running={} {}",
                b.name,
                status_for(&config, b, r, Role::Implement, host.program),
                status_for(&config, b, r, Role::Review, host.program),
                status_for(&config, b, r, Role::Lead, host.program),
                r.running,
                r.detail
            );
        }
    }
    Ok(0)
}
fn status_for(
    config: &Config,
    b: &crate::config::Backend,
    r: &decision::Reading,
    role: Role,
    program: &str,
) -> String {
    if role == Role::Implement && config.fallback.as_deref() == Some(&b.name) {
        return match decision::availability(b, r, role) {
            Ok(()) => "fallback available".into(),
            Err(problem) => format!("fallback unavailable: {problem}"),
        };
    }
    if !b.roles.contains(&role) {
        if b.run
            .as_ref()
            .is_some_and(|run| run.commands.contains_key(&role))
        {
            return match decision::availability(b, r, role) {
                Ok(()) => format!("available for {program} run"),
                Err(problem) => format!("unavailable for {program} run: {problem}"),
            };
        }
        return "not allowed for role".into();
    }
    decision::availability(b, r, role)
        .err()
        .unwrap_or_else(|| "available".into())
}
pub fn main(arguments: impl Iterator<Item = std::ffi::OsString>, host: &Host<'_>) -> ExitCode {
    match run(arguments, host) {
        Ok(code) => ExitCode::from(code),
        Err((code, message)) => {
            print_error(host.program, &message);
            ExitCode::from(code)
        }
    }
}

fn print_error(program: &str, message: &str) {
    // A run may end after its caller closed stderr; never panic here.
    let _ = writeln!(std::io::stderr(), "{program}: {message}");
}

pub(crate) fn write_json(value: &serde_json::Value) {
    let _ = writeln!(std::io::stdout().lock(), "{value}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slots::{Listing, Local, Placement, Remote, Request, Route, Stage};

    /// A route that answers only for its own keys: `check_config` fails when the file
    /// names `refuse`, and warns when it names `warn`.
    struct Keys;

    impl Route for Keys {
        fn check_config(&self, text: &str) -> Result<Vec<String>, String> {
            if text.contains("refuse") {
                return Err("route refuses this file".into());
            }
            Ok(text
                .contains("warn")
                .then(|| "route warning".to_owned())
                .into_iter()
                .collect())
        }
        fn name(&self) -> &'static str {
            "keys"
        }
        fn place(&self, _: &Request<'_>, _: Stage) -> Placement {
            Placement::Local(None)
        }
        fn run_remote(
            &self,
            _: &Request<'_>,
            _: &mut dyn FnMut(),
            _: &mut Option<serde_json::Value>,
        ) -> Remote {
            Remote::Wait
        }
        fn run_local(&self, local: Local<'_>) -> std::io::Result<std::process::ExitStatus> {
            local.command.status()
        }
        fn verify(&self, _: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }
        fn listing(
            &self,
            _: &Path,
            _: &dyn Fn(&fs::Metadata) -> Option<Option<u32>>,
        ) -> Result<Listing, String> {
            Ok(Listing::default())
        }
    }

    fn file(directory: &Path, name: &str, extra: &str) -> PathBuf {
        let path = directory.join(name);
        fs::write(
            &path,
            format!(
                "state_file = '{root}/state/anchor'\nlog_file = '{root}/log.jsonl'\n{extra}\n\
                 [[backends]]\nname = 'fake'\nroles = ['implement']\nguard = ['false']\n\
                 allow_unknown_quota = true\n[backends.commands]\n\
                 medium = ['never-start-this', '{{prompt}}']\n\
                 high = ['never-start-this', '{{prompt}}']\n",
                root = directory.display()
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn the_route_checks_its_keys_after_every_load() {
        let directory = tempfile::tempdir().unwrap();
        let session = |_: std::time::Instant| Err("no session".into());
        let route = Keys;
        let host = |path: &Path| Host {
            config: Ok(Some(path.to_owned())),
            route: Some(&route),
            ..Host::rota(&session)
        };
        let words = |args: &[&str]| args.iter().map(Into::into).collect::<Vec<_>>();

        let refused = file(directory.path(), "refused.toml", "# refuse");
        let host_refused = host(&refused);
        let loaded = load_config(Some(&refused), &host_refused);
        assert_eq!(loaded.err().as_deref(), Some("route refuses this file"));
        for args in [&["slot", "ls"][..], &["ps"], &["status"]] {
            assert_eq!(
                run(words(args).into_iter(), &host_refused),
                Err((2, "route refuses this file".into())),
                "{args:?}"
            );
        }

        let warned = file(directory.path(), "warned.toml", "# warn");
        let loaded = load_config(Some(&warned), &host(&warned)).unwrap();
        assert_eq!(loaded.warnings, ["route warning"]);

        // Standalone rota has no route, so nothing checks the opaque table.
        let standalone = Host {
            config: Ok(Some(refused.clone())),
            ..Host::rota(&session)
        };
        let loaded = load_config(Some(&refused), &standalone).unwrap();
        assert!(loaded.warnings.is_empty());
    }
}
