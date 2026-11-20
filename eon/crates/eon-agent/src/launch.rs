use crate::command::{self, Part};
use crate::config::{Backend, Config, Level, Role};
use crate::projects::{self, Selected};
use crate::readings::QuotaMode;
use crate::{admissions, decision, readings};
use fs2::FileExt;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub type Failure = (u8, String);

#[derive(Debug, PartialEq, Eq)]
pub struct Options {
    pub role: Role,
    pub level: Level,
    pub only: Option<String>,
    pub dry_run: bool,
    pub popup: bool,
    pub prompt_file: PathBuf,
    pub project: Option<String>,
    pub name: Option<String>,
    pub add_dirs: Vec<PathBuf>,
}

pub fn parse(args: &[String]) -> Result<Options, Failure> {
    let bad = |message: &str| (2, message.to_string());
    let mut options = Options {
        role: Role::Implement,
        level: Level::High,
        only: None,
        dry_run: false,
        popup: false,
        prompt_file: PathBuf::new(),
        project: None,
        name: None,
        add_dirs: Vec::new(),
    };
    let mut i = 0;
    let mut role_or_level = false;
    while let Some(arg) = args.get(i) {
        match arg.as_str() {
            "--role" | "--level" | "--only" | "--project" | "--name" | "--add-dir" => {
                i += 1;
                let value = args.get(i).ok_or_else(|| bad("option needs a value"))?;
                match arg.as_str() {
                    "--role" => {
                        role_or_level = true;
                        options.role = match value.as_str() {
                            "implement" => Role::Implement,
                            "review" => Role::Review,
                            "lead" => Role::Lead,
                            _ => return Err(bad("bad --role")),
                        }
                    }
                    "--level" => {
                        role_or_level = true;
                        options.level = match value.as_str() {
                            "medium" => Level::Medium,
                            "high" => Level::High,
                            _ => return Err(bad("bad --level")),
                        }
                    }
                    "--project" => options.project = Some(value.clone()),
                    "--name" => {
                        if !command::valid_name(value) {
                            return Err(bad(command::BAD_NAME));
                        }
                        options.name = Some(value.clone());
                    }
                    "--add-dir" => options.add_dirs.push(PathBuf::from(value)),
                    _ => options.only = Some(value.clone()),
                }
            }
            "--dry-run" => options.dry_run = true,
            "--popup" if !options.popup => options.popup = true,
            "--" => {
                i += 1;
                break;
            }
            s if s.starts_with('-') => return Err(bad("unknown launch option")),
            _ => break,
        }
        i += 1;
    }
    if options.popup {
        if args.len() != i
            || role_or_level
            || options.project.is_some()
            || options.name.is_some()
            || !options.add_dirs.is_empty()
        {
            return Err(bad(
                "--popup takes no prompt file, --role, --level, --project, --name or --add-dir",
            ));
        }
    } else if args.len() != i + 1 {
        return Err(bad("launch needs exactly one prompt file"));
    } else {
        options.prompt_file = PathBuf::from(&args[i]);
    }
    Ok(options)
}

pub fn ensure_parent(path: &Path) -> std::io::Result<&Path> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    Ok(parent)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = ensure_parent(path)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    match fs::metadata(path) {
        Ok(metadata) => temp.as_file().set_permissions(metadata.permissions())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(e),
    }
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    File::open(parent)?.sync_all()
}

pub fn trust_folder(path: &Path, cwd: &Path) -> Result<(), String> {
    let mut json: serde_json::Value = match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| "invalid trusted settings JSON")?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e.to_string()),
    };
    let object = json
        .as_object_mut()
        .ok_or("trusted settings must be a JSON object")?;
    let trusted = object
        .entry("trustedWorkspaces")
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .ok_or("trustedWorkspaces must be an array")?;
    if trusted.iter().any(|v| v.as_str().is_none()) {
        return Err("trustedWorkspaces entries must be strings".into());
    }
    let folder = cwd.to_str().ok_or("working directory must be UTF-8")?;
    if !trusted.iter().any(|v| v.as_str() == Some(folder)) {
        trusted.push(folder.into());
        let mut bytes = serde_json::to_vec_pretty(&json).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        atomic_write(path, &bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn shell_quote(text: &str) -> String {
    if !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-/.=:".contains(c))
    {
        text.into()
    } else {
        format!("'{}'", text.replace('\'', "'\\''"))
    }
}

pub fn display_command(backend: &Backend, level: Level, cwd: &str, prompt_file: &str) -> String {
    let values = command::Values {
        fills: &[("{cwd}", cwd)],
        template: None,
        add_dirs: &[],
        overlay: &[],
    };
    let parts = command::expand(&backend.commands[&level], &values);
    display(backend, &parts, prompt_file)
}

/// A shell line for dry runs: the prompt is shown as `"$(cat FILE)"`, never its content.
fn display(backend: &Backend, parts: &[Part], prompt_file: &str) -> String {
    let mut args = Vec::new();
    if !backend.env.is_empty() || !backend.unset_env.is_empty() {
        args.push("env".into());
        for key in &backend.unset_env {
            args.extend(["-u".into(), shell_quote(key)]);
        }
        for (key, value) in &backend.env {
            args.push(shell_quote(&format!("{key}={value}")));
        }
    }
    for part in parts {
        args.push(match part {
            Part::Prompt => format!("\"$(cat {})\"", shell_quote(prompt_file)),
            Part::Text(arg) => shell_quote(arg),
        });
    }
    args.join(" ")
}

/// What a decision record says about the request.
pub struct Entry<'a> {
    pub role: Role,
    pub level: Option<Level>,
    pub cwd: &'a Path,
    /// The selected project, when there is one.
    pub project: Option<&'a str>,
}

pub fn log(
    config: &Config,
    entry: &Entry<'_>,
    readings: &[decision::Reading],
    pick: Option<&decision::Pick>,
    hold: Option<&str>,
) -> Result<(), Failure> {
    let Entry {
        role,
        level,
        cwd,
        project,
    } = *entry;
    ensure_parent(&config.log_file).map_err(io_failure)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&config.log_file)
        .map_err(io_failure)?;
    let mut entry = serde_json::json!({
        "time": SystemTime::now().duration_since(UNIX_EPOCH).map_err(io_failure)?.as_secs(),
        "folder": cwd, "role": role, "level": level,
        "pick": pick.map(|p| &config.backends[p.index].name),
        "decision": match pick { Some(p) if p.fallback => "FALLBACK", Some(_) => "pick", None => "REFUSAL" },
        "readings": config.backends.iter().zip(readings).map(|(b,r)| serde_json::json!({"backend": b.name, "reading": r, "availability": decision::availability(b,r,role).err()})).collect::<Vec<_>>()
    });
    if let Some(project) = project {
        entry["project"] = project.into();
    }
    if let Some(hold) = hold {
        entry["hold"] = hold.into();
    }
    writeln!(file, "{entry}").map_err(io_failure)?;
    file.sync_data().map_err(io_failure)
}

pub fn io_failure(error: impl std::fmt::Display) -> Failure {
    (1, error.to_string())
}

/// Serializes rota's backend selection; the descriptor closes on exec.
pub fn lock(config: &Config, wait: bool, program: &str) -> Result<File, Failure> {
    let mut lock_path = config.state_file.as_os_str().to_os_string();
    lock_path.push(".lock");
    let lock_path = PathBuf::from(lock_path);
    ensure_parent(&lock_path).map_err(io_failure)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)
        .map_err(io_failure)?;
    let locked = if wait {
        lock.lock_exclusive()
    } else {
        lock.try_lock_exclusive()
    };
    locked.map_err(|e| {
        if e.kind() == std::io::ErrorKind::WouldBlock {
            (
                3,
                format!("another {program} command is selecting a backend"),
            )
        } else {
            io_failure(e)
        }
    })?;
    Ok(lock)
}

/// Spends the admitted reading's observation on its account, for a backend that sets limits.
/// A failed write refuses the admission before anything runs.
pub fn record_admission(
    ledger: &mut admissions::Ledger,
    backend: &Backend,
    role: Role,
    reading: &decision::Reading,
) -> Result<(), Failure> {
    if role == Role::Lead || !backend.limited() {
        return Ok(());
    }
    // A limited backend admits only a timed reading; refuse rather than record nothing.
    let observed_at = reading
        .source
        .as_ref()
        .and_then(|source| source.observed_at_ms)
        .ok_or_else(|| (1, format!("{}: admitted without observed_at", backend.name)))?;
    ledger.record(backend.account(), observed_at).map_err(|e| {
        (
            1,
            format!(
                "cannot record the admission in {}: {e}",
                ledger.path().display()
            ),
        )
    })
}

/// Keeps the candidates these options can use; each other one gets its reason as the
/// reading's problem, so status output and the decision record say why it was skipped.
/// Returns the kept indices and the `backend: reason` texts.
pub fn supported(
    config: &Config,
    indices: &[usize],
    readings: &mut [decision::Reading],
    reason: impl Fn(usize) -> Option<String>,
) -> (Vec<usize>, Vec<String>) {
    let mut kept = Vec::new();
    let mut reasons = Vec::new();
    for &index in indices {
        match reason(index) {
            None => kept.push(index),
            Some(why) => {
                reasons.push(format!("{}: {why}", config.backends[index].name));
                readings[index] = decision::Reading {
                    problem: Some(why),
                    ..decision::Reading::default()
                };
            }
        }
    }
    (kept, reasons)
}

/// The refusal when static reasons leave no candidate: waiting cannot change configuration.
pub fn unsupported_refusal(what: &str, reasons: &[String]) -> Failure {
    (
        2,
        format!("no backend can take this {what}: {}", reasons.join("; ")),
    )
}

/// The project's total and role caps and its role guard, checked before any backend is read
/// (EON-C40 clause 6). A hold is logged unless `dry_run`. Returns the scanner and live records
/// a work admission needs; a lead gets neither, since leads are not counted.
pub fn project_holds(
    config: &Config,
    project: Option<&Selected<'_>>,
    entry: &Entry<'_>,
    readings: &[decision::Reading],
    dry_run: bool,
) -> Result<(Option<projects::Scanner>, Vec<projects::Record>), Failure> {
    let Some(selected) = project else {
        return Ok((None, Vec::new()));
    };
    let held = |message: String| -> Failure {
        if !dry_run && let Err(e) = log(config, entry, readings, None, Some(&message)) {
            return e;
        }
        (3, message)
    };
    let mut found = (None, Vec::new());
    if entry.role != Role::Lead {
        let scanner = projects::Scanner::system()
            .map_err(|e| held(format!("cannot scan /proc for project caps: {e}")))?;
        let live = projects::scan(
            &projects::records_dir(&config.state_file),
            &scanner,
            !dry_run,
        )
        .map_err(held)?;
        if let Some(message) = projects::hold(selected, entry.role, &live) {
            return Err(held(message));
        }
        found = (Some(scanner), live);
    }
    projects::guard(selected, entry.role, entry.cwd).map_err(held)?;
    Ok(found)
}

pub fn print_readings(
    config: &Config,
    indices: &[usize],
    readings: &[decision::Reading],
    role: Role,
    program: &str,
) {
    for index in indices {
        let b = &config.backends[*index];
        let r = &readings[*index];
        let availability = decision::availability(b, r, role)
            .err()
            .unwrap_or_else(|| "available".into());
        let via = r
            .source
            .as_ref()
            .and_then(|source| source.via.as_deref())
            .map_or_else(String::new, |via| format!(" via {via}"));
        match r.detail.as_str() {
            "" => eprintln!("{program}: {} [{availability}]{via}", b.name),
            detail => eprintln!("{program}: {} [{availability}]: {detail}{via}", b.name),
        }
    }
}

pub fn run(config: &Config, options: &Options, host: &crate::cli::Host<'_>) -> Result<(), Failure> {
    let brief = if options.popup {
        None
    } else {
        let file = fs::canonicalize(&options.prompt_file)
            .map_err(|e| (2, format!("cannot read prompt file: {e}")))?;
        let prompt =
            fs::read_to_string(&file).map_err(|e| (2, format!("cannot read prompt file: {e}")))?;
        if prompt.contains('\0') {
            return Err((2, "prompt contains a NUL byte".into()));
        }
        Some((file, prompt))
    };
    let cwd = std::env::current_dir()
        .and_then(fs::canonicalize)
        .map_err(io_failure)?;
    let cwd_text = cwd
        .to_str()
        .ok_or_else(|| (2, "working directory must be UTF-8".into()))?;
    let prompt_path = brief
        .as_ref()
        .map(|(file, _)| {
            file.to_str()
                .ok_or_else(|| (2, "prompt path must be UTF-8".into()))
        })
        .transpose()?
        .unwrap_or("");
    let add_dirs = command::canonical_dirs(&options.add_dirs)?;
    let dirs: Vec<&str> = add_dirs
        .iter()
        .map(|d| {
            d.to_str()
                .ok_or_else(|| (2, "--add-dir must be UTF-8".to_string()))
        })
        .collect::<Result<_, _>>()?;
    // The person's Agent popup is never a project's agent.
    let project = if options.popup {
        None
    } else {
        projects::select(config, options.project.as_deref(), &cwd)?
    };
    if let Some(selected) = &project {
        eprintln!("{}: {}", host.program, selected.line());
    }
    let popup_backend = if options.popup {
        config.popup_backend.as_deref()
    } else {
        None
    };
    let mut candidates =
        decision::candidates(config, options.role, options.only.as_deref(), popup_backend)
            .map_err(|e| (e.code, e.message))?;
    let indices: Vec<_> = candidates
        .normal
        .iter()
        .copied()
        .chain(candidates.fallback)
        .collect();
    // Candidates are read in configured order until one admits; the rest are not checked.
    let mut readings: Vec<_> = config
        .backends
        .iter()
        .map(|_| readings::not_checked())
        .collect();
    let argv_of = |backend: &Backend| {
        if options.popup {
            backend.popup_command()
        } else {
            Some(backend.commands[&options.level].clone())
        }
    };
    let level_name = match options.level {
        Level::Medium => "medium",
        Level::High => "high",
    };
    // Reasons these options can never use a backend, decided before anything is read.
    let (kept, reasons) = supported(config, &indices, &mut readings, |index| {
        let backend = &config.backends[index];
        let argv = argv_of(backend)?;
        let overlay = match projects::overlay(project.as_ref(), options.role, &argv) {
            Ok(overlay) => overlay,
            Err(why) => return Some(why),
        };
        command::unsupported(
            &argv,
            overlay,
            !dirs.is_empty(),
            options.name.is_some(),
            level_name,
        )
    });
    if !reasons.is_empty() {
        candidates.normal.retain(|i| kept.contains(i));
        candidates.fallback = candidates.fallback.filter(|i| kept.contains(i));
        if candidates.normal.is_empty() && candidates.fallback.is_none() {
            return Err(unsupported_refusal("launch", &reasons));
        }
    }
    // Serialize rota launches from scan through exec.
    let _lock = if options.dry_run {
        None
    } else {
        Some(lock(config, false, host.program)?)
    };
    let entry = Entry {
        role: options.role,
        level: Some(options.level),
        cwd: &cwd,
        project: project.as_ref().map(|p| p.name),
    };
    let (scanner, live) =
        project_holds(config, project.as_ref(), &entry, &readings, options.dry_run)?;
    let mut ledger = admissions::Ledger::load(&config.state_file);
    let mode = match (options.role, options.dry_run) {
        (Role::Lead, _) => QuotaMode::Skip,
        (_, true) => QuotaMode::Read,
        (_, false) => QuotaMode::Admit,
    };
    let result = decision::select(config, &candidates, options.role, &mut readings, |index| {
        let backend = &config.backends[index];
        if options.popup && backend.popup_command().is_none() {
            decision::Reading {
                problem: Some("no popup_command for this executable".into()),
                ..decision::Reading::default()
            }
        } else if let Some(reason) = project
            .as_ref()
            .and_then(|p| projects::backend_hold(p, options.role, &backend.name, &live))
        {
            decision::Reading {
                problem: Some(reason),
                ..decision::Reading::default()
            }
        } else {
            readings::check(backend, &cwd, mode, &ledger, host.session)
        }
    });
    if !options.dry_run {
        log(config, &entry, &readings, result.as_ref().ok(), None)?;
    }
    print_readings(config, &indices, &readings, options.role, host.program);
    let pick = result.map_err(|e| (e.code, e.message))?;
    let backend = &config.backends[pick.index];
    if pick.fallback {
        eprintln!(
            "{}: FALLBACK {} (no normal backend available)",
            host.program, backend.name
        );
    }
    println!("pick: {}", backend.name);
    let command = argv_of(backend).expect("selection checked popup command");
    let overlay =
        projects::overlay(project.as_ref(), options.role, &command).map_err(|e| (2, e))?;
    let name = options.name.as_deref().unwrap_or_default();
    let values = command::Values {
        fills: &[("{cwd}", cwd_text), (command::NAME, name)],
        template: backend.add_dir.as_deref(),
        add_dirs: &dirs,
        overlay,
    };
    let parts = command::expand(&command, &values);
    if options.dry_run {
        println!("command: {}", display(backend, &parts, prompt_path));
        return Ok(());
    }
    if let Some(path) = &backend.trusted_settings {
        trust_folder(path, &cwd).map_err(io_failure)?;
    }
    for prepare in &backend.prepare {
        readings::probe(backend, prepare, 30, &cwd).map_err(io_failure)?;
    }
    let prompt = brief.as_ref().map_or("", |(_, prompt)| prompt.as_str());
    let argv = command::argv(parts, prompt);
    if let Err(problem) = readings::recheck_cap(backend, &mut readings[pick.index]) {
        log(config, &entry, &readings, None, None)?;
        return Err((3, problem));
    }
    // Written before exec, so the agent counts from the moment it is admitted; a failed
    // exec drops it.
    let _admitted = match (&project, &scanner) {
        (Some(selected), Some(scanner)) => Some(projects::Admitted::write(
            &config.state_file,
            selected.name,
            options.role,
            &backend.name,
            scanner,
        )?),
        _ => None,
    };
    record_admission(&mut ledger, backend, options.role, &readings[pick.index])?;
    let error = readings::command(backend, &argv).current_dir(&cwd).exec();
    Err(io_failure(format!(
        "could not exec {}: {error}",
        backend.name
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn launch_arguments() {
        let args = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let parsed = parse(&args(&[
            "--role",
            "review",
            "--level",
            "medium",
            "--only",
            "alpha",
            "--dry-run",
            "task",
        ]))
        .unwrap();
        assert_eq!(parsed.role, Role::Review);
        assert_eq!(parsed.level, Level::Medium);
        assert!(parsed.dry_run);
        let named = parse(&args(&[
            "--project",
            "tide",
            "--name",
            "tide-1",
            "--add-dir",
            "/a",
            "--add-dir",
            "/b",
            "task",
        ]))
        .unwrap();
        assert_eq!(named.project.as_deref(), Some("tide"));
        assert_eq!(named.name.as_deref(), Some("tide-1"));
        assert_eq!(named.add_dirs, [PathBuf::from("/a"), PathBuf::from("/b")]);
        let lead = parse(&["--role".to_string(), "lead".into(), "task".into()]).unwrap();
        assert_eq!(lead.role, Role::Lead);
        for args in [
            vec![],
            vec!["--role"],
            vec!["--role", "bad", "task"],
            vec!["--level", "bad", "task"],
            vec!["--wat", "task"],
            vec!["one", "two"],
            vec!["--name", "-bad", "task"],
            vec!["--popup", "--project", "p"],
            vec!["--popup", "--name", "n"],
            vec!["--popup", "--add-dir", "/x"],
        ] {
            assert_eq!(
                parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>())
                    .unwrap_err()
                    .0,
                2
            );
        }
    }
    #[test]
    fn atomic_trust_keeps_keys_mode_and_rejects_corrupt_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(
            &path,
            r#"{"other":{"keep":true},"trustedWorkspaces":["/fictional/old"]}"#,
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        trust_folder(&path, Path::new("/fictional/new")).unwrap();
        trust_folder(&path, Path::new("/fictional/new")).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["other"]["keep"], true);
        assert_eq!(json["trustedWorkspaces"].as_array().unwrap().len(), 2);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        fs::write(&path, "bad").unwrap();
        assert!(trust_folder(&path, Path::new("/fictional/new")).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "bad");
        let state = dir.path().join("nested/state");
        atomic_write(&state, b"alpha\n").unwrap();
        atomic_write(&state, b"secondary\n").unwrap();
        assert_eq!(fs::read(&state).unwrap(), b"secondary\n");
        assert_eq!(
            fs::metadata(&state).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn renders_shell_arguments_without_prompt_content() {
        let config = Config::parse(include_str!("../config.example.toml"), "rota").unwrap();
        let command = display_command(
            &config.backends[3],
            Level::High,
            "/fictional/folder",
            "/fictional/task's file",
        );
        assert!(command.contains("'PROVIDER/MODEL#HIGH'"));
        assert!(command.contains("\"$(cat '/fictional/task'\\''s file')\""));
        assert!(
            display_command(&config.backends[4], Level::High, ".", "task")
                .starts_with("env -u CLAUDE_CONFIG_DIR claude")
        );
    }
}
