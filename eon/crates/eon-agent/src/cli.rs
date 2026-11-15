use crate::config::{Config, Role};
use crate::{decision, launch, ps, readings, run as headless, slots};
use std::io::{IsTerminal, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::{env, fs, process::ExitCode};

const USAGE: &str = "Usage: rota launch [--role implement|review] [--level medium|high] [--only BACKEND] [--dry-run] <prompt-file>\n       rota launch --popup [--only BACKEND] [--dry-run]\n       rota run --brief FILE --name NAME --timeout DURATION [--role implement|review] [--only BACKEND] [--cwd DIR] [--effort LEVEL] [--add-dir DIR]...\n       rota ps [--json] [--all]\n       rota slot run [--class NAME] [--quiet-exempt] [--front] -- COMMAND...\n       rota slot ls [--json]\n       rota retire NAME [--project P] [--bead ID] [--target-only] [--dry-run] [--json]\n       rota status [--json]\nConfig: $ROTA_CONFIG or $XDG_CONFIG_HOME/rota/config.toml (default ~/.config/rota/config.toml)";

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
    if let Some(path) = primary {
        if exists(path)? {
            return Ok(Some(path.to_owned()));
        }
    }
    let Ok(path) = fallback_config_path() else {
        return Ok(None);
    };
    Ok(exists(&path)?.then_some(path))
}

fn load_config(primary: Option<&Path>) -> Result<(Config, PathBuf), String> {
    let path = match config_source(primary, Path::new("."))? {
        Some(path) => path,
        None => fallback_config_path()?,
    };
    let config =
        Config::parse(&fs::read_to_string(&path).map_err(|e| format!("cannot read config: {e}"))?)?;
    Ok((config, path))
}

fn run(
    arguments: impl Iterator<Item = std::ffi::OsString>,
    primary: Result<Option<PathBuf>, String>,
    session: &readings::SessionReader<'_>,
) -> Result<u8, (u8, String)> {
    let eon = !matches!(&primary, Ok(None));
    let usage = if eon {
        USAGE.replace("Usage: rota ", "Usage: eon agent ")
            .replace("       rota ", "       eon agent ")
            .replace("Config: $ROTA_CONFIG or $XDG_CONFIG_HOME/rota/config.toml (default ~/.config/rota/config.toml)",
                "Config: <eon config dir>/agent.toml; when absent, $ROTA_CONFIG or $XDG_CONFIG_HOME/rota/config.toml (default ~/.config/rota/config.toml)")
    } else {
        USAGE.to_owned()
    };
    let load = || load_config(primary.as_ref().map_err(Clone::clone)?.as_deref());
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
    if args.first().is_some_and(|s| s == "retire") {
        if args[1..] == ["--help"] || args[1..] == ["-h"] {
            let help = if eon {
                crate::cleanup::HELP.replacen("Usage: rota ", "Usage: eon agent ", 1)
            } else {
                crate::cleanup::HELP.to_owned()
            };
            println!("{help}");
            return Ok(0);
        }
        let options = crate::cleanup::parse(&args[1..])?;
        let (config, _) = load().map_err(|e| (2, e))?;
        return crate::cleanup::run(&config, &options);
    }
    if args.first().is_some_and(|s| s == "launch") {
        let options = launch::parse(&args[1..])?;
        let result = load()
            .map_err(|error| (2, error))
            .and_then(|(config, _)| launch::run(&config, &options, session))
            .map(|()| 0);
        if options.popup
            && !options.dry_run
            && std::io::stdin().is_terminal()
            && std::io::stderr().is_terminal()
            && let Err((code, message)) = &result
        {
            print_error(message);
            let _ = write!(std::io::stderr(), "\nPress Enter to close");
            let _ = std::io::stderr().flush();
            let _ = std::io::stdin().read_line(&mut String::new());
            return Ok(*code);
        }
        return result;
    }
    if args.first().is_some_and(|s| s == "run") {
        let options = headless::parse(&args[1..])?;
        let (config, _) = load().map_err(|e| (2, e))?;
        return headless::run(&config, &options, session).map(|()| 0);
    }
    if args.first().is_some_and(|s| s == "ps") {
        let options = ps::parse(&args[1..])?;
        let (config, _) = load().map_err(|e| (2, e))?;
        return ps::run(&config, &options).map(|()| 0);
    }
    if args.first().is_some_and(|s| s == "slot") {
        let options = slots::parse(&args[1..])?;
        let (config, _) = load().map_err(|e| (2, e))?;
        return slots::run(&config, options);
    }
    if args != ["status"] && args != ["status", "--json"] {
        return Err((2, usage));
    }
    let (config, config_path) = load().map_err(|e| (2, e))?;
    let cwd = env::current_dir().map_err(|e| (1, e.to_string()))?;
    let indices: Vec<_> = (0..config.backends.len()).collect();
    let readings = readings::collect(&config, &indices, &cwd, false, session);
    if args.len() == 2 {
        let status: Vec<_> = config.backends.iter().zip(&readings).map(|(b,r)| {
            let mut value = serde_json::json!({ "backend": b.name, "enabled": b.enabled, "reading": r,
                "implement": status_for(&config, b, r, Role::Implement), "review": status_for(&config, b, r, Role::Review) });
            if eon {
                value["config_file"] = match config_path.to_str() {
                    Some(path) => serde_json::json!(path),
                    None => serde_json::json!(config_path.as_os_str().as_bytes()),
                };
            }
            value
        }).collect();
        println!("{}", serde_json::to_string_pretty(&status).unwrap());
    } else {
        if eon {
            println!("config: {}", config_path.display());
        }
        for (b, r) in config.backends.iter().zip(&readings) {
            println!(
                "{}: implement={} review={} running={} {}",
                b.name,
                status_for(&config, b, r, Role::Implement),
                status_for(&config, b, r, Role::Review),
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
                Ok(()) => "available for rota run".into(),
                Err(problem) => format!("unavailable for rota run: {problem}"),
            };
        }
        return "not allowed for role".into();
    }
    decision::availability(b, r, role)
        .err()
        .unwrap_or_else(|| "available".into())
}
pub fn main(
    arguments: impl Iterator<Item = std::ffi::OsString>,
    primary: Result<Option<PathBuf>, String>,
    session: &readings::SessionReader<'_>,
) -> ExitCode {
    match run(arguments, primary, session) {
        Ok(code) => ExitCode::from(code),
        Err((code, message)) => {
            print_error(&message);
            ExitCode::from(code)
        }
    }
}

fn print_error(message: &str) {
    // A run may end after its caller closed stderr; never panic here.
    let _ = writeln!(std::io::stderr(), "rota: {message}");
}
