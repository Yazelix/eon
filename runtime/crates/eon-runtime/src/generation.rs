use super::cli::Inputs;
use super::control::{
    CONTROL_TIMEOUT, ControlResponse, EndpointFailure, EndpointFailureKind, Error, connect_control,
    failure, probe_generation_runtime, probe_generation_runtime_from,
    probe_generation_runtime_until, remove_socket_if_identity, report_failure,
    send_action_from_until, send_action_on, socket_identity, socket_identity_from, write_stdout,
};
use super::sessions::TERMINAL_VARIABLE;
use super::supervisor::{
    base_runtime_directory, effective_uid, present_at, probe_supervisor, runtime_directory,
    supervisor_lock_path, try_lock_supervisor_lifecycle, validate_private_directory,
};
use super::windows::WINDOW_RUNTIME;
use super::workspace::{json_escape, terminal_text};
use eon_workspace_protocol::v30::{
    Action, Availability, BusyTerminals, Failure, LifecycleResponse, MAX_RUNNING_WINDOWS, Response,
    Stopped, WindowEntry, WindowState, WorkspaceAction,
};
use std::{
    env,
    ffi::OsString,
    fs,
    io::IsTerminal,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, symlink},
        net::UnixStream,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) fn current_generation(inputs: &Inputs) -> Result<String, String> {
    inputs.component_report()?;
    Ok(generation_id(inputs.assembly))
}

fn generation_id(input: &[u8]) -> String {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;
    let mut hash = OFFSET;
    for byte in input {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("g1-{hash:032x}")
}

pub(super) fn valid_generation(value: &str) -> bool {
    value.len() == 35
        && value.starts_with("g1-")
        && value[3..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(super) fn generation_directory(root: &Path, generation: &str) -> PathBuf {
    root.join("generations").join(generation)
}

/// The runtime root that holds a generation's directory.
fn generations_root(runtime: &Path) -> &Path {
    runtime.parent().and_then(Path::parent).unwrap_or(runtime)
}

/// The link in a generation's directory to the `eon` of the build that started
/// it, which a later build runs to present a generation it cannot present.
const LAUNCHER: &str = "eon";
/// Set on a delegated attach to the generation it presents.
const DELEGATED: &str = "EON_ATTACH_DELEGATED";
/// Set on a delegated Stop to the generation it stops.
const STOP_DELEGATED: &str = "EON_STOP_DELEGATED";

/// Records this build's `eon`, from its session bin, in the workspace
/// generation directory `runtime`.
pub(super) fn record_launcher(inputs: &Inputs, runtime: &Path) -> Result<(), String> {
    let Some(eon) = inputs.managed.shell_bin.as_ref().map(|bin| bin.join("eon")) else {
        return Ok(());
    };
    if !eon.is_absolute() || !eon.is_file() {
        return Ok(());
    }
    remove_launcher(runtime);
    let link = runtime.join(LAUNCHER);
    symlink(&eon, &link).map_err(|error| format!("cannot record {}: {error}", link.display()))
}

/// Removes a generation's launcher record so its directory can be removed.
pub(super) fn remove_launcher(runtime: &Path) {
    let link = runtime.join(LAUNCHER);
    if fs::symlink_metadata(&link).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        let _ = fs::remove_file(link);
    }
}

fn recorded_launcher(runtime: &Path) -> Option<PathBuf> {
    let link = runtime.join(LAUNCHER);
    let metadata = fs::symlink_metadata(&link).ok()?;
    if !metadata.file_type().is_symlink() || metadata.uid() != effective_uid() {
        return None;
    }
    fs::read_link(link).ok().filter(|eon| eon.is_absolute())
}

/// Why a recorded `eon` cannot run, or none while it is an executable file.
fn launcher_fault(eon: &Path) -> Option<&'static str> {
    match fs::metadata(eon) {
        Ok(metadata) if metadata.is_file() && metadata.mode() & 0o111 != 0 => None,
        Ok(_) => Some("is not an executable file"),
        Err(_) => Some("is missing"),
    }
}

/// What to run to stop `generation` through `eon` in its namespace: a window's
/// root, or none for the default namespace. It is one command when each word
/// has a form that POSIX shells and Nushell both read back, else one command
/// for each.
fn stop_advice(eon: &Path, window: Option<&Path>, generation: &str) -> String {
    let mut words = vec!["env".to_owned()];
    match window {
        Some(root) => words.push(format!("{WINDOW_RUNTIME}={}", root.display())),
        None => words.extend(["-u".to_owned(), WINDOW_RUNTIME.to_owned()]),
    }
    words.extend([eon.display().to_string(), "stop".into(), generation.into()]);
    if let Some(command) = words
        .iter()
        .map(|word| shared_word(word))
        .collect::<Option<Vec<_>>>()
    {
        return format!("then run {}", command.join(" "));
    }
    let command = |quote: fn(&str) -> String| {
        words
            .iter()
            .map(|word| quote(word))
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "then run, in sh:\n    {}\n  or in Nushell:\n    {}",
        command(posix_word),
        command(nu_word)
    )
}

fn bare_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character))
}

/// `word` in a form that POSIX shells and Nushell both read back as written:
/// bare, in single quotes, or, with an apostrophe, in double quotes with `\`,
/// `"` and `$` escaped. An apostrophe beside a backquote or `!`, or a control
/// character, has no such form.
fn shared_word(word: &str) -> Option<String> {
    if bare_word(word) {
        return Some(word.to_owned());
    }
    if word.chars().any(char::is_control) {
        return None;
    }
    if !word.contains('\'') {
        return Some(format!("'{word}'"));
    }
    if word.contains(['`', '!']) {
        return None;
    }
    let mut quoted = String::from('"');
    for character in word.chars() {
        if matches!(character, '\\' | '"' | '$') {
            quoted.push('\\');
        }
        quoted.push(character);
    }
    quoted.push('"');
    Some(quoted)
}

/// `word` for a POSIX shell: bare, else in single quotes.
fn posix_word(word: &str) -> String {
    if bare_word(word) {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// `word` for Nushell: bare, else a raw string with more `#` than any run
/// after an apostrophe in it.
fn nu_word(word: &str) -> String {
    if bare_word(word) {
        return word.to_owned();
    }
    let hashes = word
        .split('\'')
        .skip(1)
        .map(|after| after.len() - after.trim_start_matches('#').len())
        .max()
        .unwrap_or(0)
        + 1;
    let hashes = "#".repeat(hashes);
    format!("r{hashes}'{word}'{hashes}")
}

fn is_default_namespace(root: &Path) -> bool {
    root == base_runtime_directory("eon")
}

const MAX_GENERATIONS: usize = 256;

#[derive(Default)]
pub(super) struct WorkspaceTarget {
    pub(super) generation: Option<String>,
    pub(super) window: Option<String>,
}

pub(super) struct ResolvedWorkspace {
    pub(super) generation: String,
    pub(super) window: Option<String>,
    pub(super) socket: PathBuf,
}

pub(super) fn resolve_workspace(
    inputs: &Inputs,
    target: WorkspaceTarget,
    caller: Option<&str>,
) -> Result<ResolvedWorkspace, Failure> {
    let root = match &target.window {
        Some(id) => super::windows::selected_window(id.as_ref()).map_err(|error| {
            failure(
                if error.status == 3 {
                    "unknown-window"
                } else {
                    "supervisor-unavailable"
                },
                error.message,
            )
        })?,
        None => runtime_directory("eon"),
    };
    let window = super::windows::runtime_window(&root);
    let current =
        current_generation(inputs).map_err(|detail| failure("supervisor-unavailable", detail))?;
    let selected = |generation: String| ResolvedWorkspace {
        socket: generation_directory(&root, &generation).join("eon.sock"),
        generation,
        window: window.clone(),
    };
    let unavailable = |generation: &str, detail: String| {
        failure(
            "supervisor-unavailable",
            format!(
                "generation {generation}: {detail}; use the terminal's own `eon` from its PATH"
            ),
        )
    };
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    let probe = |id: &str| {
        let path = generation_directory(&root, id);
        validate_private_directory(&path)
            .map_err(|detail| (EndpointFailureKind::Corrupt, unavailable(id, detail)))?;
        let socket = path.join("eon.sock");
        let runtime = probe_generation_runtime_until(&socket, deadline)
            .map_err(|error| (error.kind, unavailable(id, error.detail)));
        match runtime {
            Ok(runtime) if runtime.generation == id => Ok(()),
            Ok(runtime) => Err((
                EndpointFailureKind::Corrupt,
                unavailable(
                    id,
                    format!("supervisor reports generation {}", runtime.generation),
                ),
            )),
            Err(error) => Err(error),
        }
    };
    // A plain outside-Eon command keeps its current-generation default and existing failures.
    if caller.is_none() && target.generation.is_none() && target.window.is_none() {
        if !workspace_runtime_exists(&root, &current)
            .map_err(|detail| unavailable(&current, detail))?
        {
            return Err(failure(
                "missing-supervisor",
                format!("generation {current} has no supervisor; run `eon` first"),
            ));
        }
        probe(&current).map_err(|(_, error)| error)?;
        return Ok(selected(current));
    }

    let generation = if let Some(id) = target.generation {
        let present =
            workspace_runtime_exists(&root, &id).map_err(|detail| unavailable(&id, detail))?;
        if !present {
            return Err(failure(
                "unknown-generation",
                format!("generation {id} was not found"),
            ));
        }
        probe(&id).map_err(|(_, error)| error)?;
        id
    } else {
        let candidates = generation_candidates(&root)
            .map_err(|detail| failure("supervisor-unavailable", detail))?;
        let mut live = Vec::new();
        let mut failures = Vec::new();
        let mut owner = None;
        for (id, path) in candidates {
            let id = id
                .to_str()
                .filter(|id| valid_generation(id))
                .ok_or_else(|| {
                    failure(
                        "supervisor-unavailable",
                        format!("invalid generation directory {}", path.display()),
                    )
                })?;
            let verified = match probe(id) {
                Ok(_) => true,
                Err((EndpointFailureKind::Dead, _)) => continue,
                Err((_, error)) => {
                    failures.push((id.to_owned(), error));
                    false
                }
            };
            live.push(id.to_owned());
            if verified
                && target.window.is_none()
                && let Some(caller) = caller
            {
                match validates_caller(&path.join("eon.sock"), caller, deadline) {
                    Ok(true) if owner.is_some() => {
                        return Err(unavailable(
                            id,
                            "more than one supervisor validates the caller".into(),
                        ));
                    }
                    Ok(true) => owner = Some(id.to_owned()),
                    Ok(false) => {}
                    Err(error) => failures.push((id.to_owned(), unavailable(id, error.detail))),
                }
            }
        }
        if target.window.is_some() {
            match preferred_window_generation(&current, &live) {
                Some(id) => {
                    if let Some(index) = failures.iter().position(|(failed, _)| failed == id) {
                        return Err(failures.swap_remove(index).1);
                    }
                    id.to_owned()
                }
                None if live.is_empty() => {
                    return Err(failure(
                        "supervisor-unavailable",
                        "the selected Eon window has ended; no supervisor runs in it",
                    ));
                }
                None => {
                    return Err(failure(
                        "ambiguous-window",
                        format!(
                            "Eon window {} runs generations {}; select --generation",
                            window.as_deref().unwrap_or("?"),
                            live.join(", ")
                        ),
                    ));
                }
            }
        } else {
            if Instant::now() >= deadline {
                return Err(failure(
                    "supervisor-unavailable",
                    "caller discovery exceeded the supervisor inspection deadline; no action sent",
                ));
            }
            // A validated complete identity is authoritative; another generation's failure is not its owner.
            return owner.map(selected).ok_or_else(|| {
                failures
                    .into_iter()
                    .next()
                    .map_or_else(unknown_caller, |(_, error)| error)
            });
        }
    };
    let resolved = selected(generation);
    if let Some(caller) = caller
        && !validates_caller(&resolved.socket, caller, deadline)
            .map_err(|error| unavailable(&resolved.generation, error.detail))?
    {
        return Err(unknown_caller());
    }
    Ok(resolved)
}

fn unknown_caller() -> Failure {
    failure(
        "unknown-caller",
        "the calling terminal is not live in this Eon runtime namespace",
    )
}

fn workspace_runtime_exists(root: &Path, id: &str) -> Result<bool, String> {
    Ok(private_directory_exists(root, "runtime directory")?
        && private_directory_exists(&root.join("generations"), "generation directory")?
        && private_directory_exists(
            &generation_directory(root, id),
            "generation runtime directory",
        )?)
}

fn validates_caller(
    socket: &Path,
    caller: &str,
    deadline: Instant,
) -> Result<bool, EndpointFailure> {
    match send_action_from_until(
        socket,
        Some(caller.into()),
        Action::Workspace(WorkspaceAction::Inspect),
        deadline,
    )? {
        ControlResponse::Workspace(Response::Snapshot(_)) => Ok(true),
        ControlResponse::Workspace(Response::Failure(error)) if error.code == "unknown-caller" => {
            Ok(false)
        }
        ControlResponse::Workspace(Response::Failure(error)) => Err(EndpointFailure::new(
            EndpointFailureKind::Unreachable,
            format!(
                "supervisor rejected caller inspection: {}: {}",
                error.code, error.detail
            ),
        )),
        _ => Err(EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            "supervisor returned the wrong result for caller inspection",
        )),
    }
}

fn generation_candidates(root: &Path) -> Result<Vec<(OsString, PathBuf)>, String> {
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(Vec::new());
    }
    let parent = root.join("generations");
    let mut candidates = Vec::new();
    if private_directory_exists(&parent, "generation directory")? {
        for entry in fs::read_dir(&parent).map_err(|error| {
            format!(
                "cannot list generation directory {}: {error}",
                parent.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "cannot read generation entry in {}: {error}",
                    parent.display()
                )
            })?;
            candidates.push((entry.file_name(), entry.path()));
            if candidates.len() > MAX_GENERATIONS {
                return Err(format!(
                    "generation directory {} exceeds the {MAX_GENERATIONS}-entry inspection limit",
                    parent.display()
                ));
            }
        }
    }
    candidates.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
    Ok(candidates)
}
#[derive(Debug)]
pub(super) struct GenerationRecord {
    pub(super) id: String,
    kind: &'static str,
    pub(super) state: &'static str,
    runtime: PathBuf,
    name: Option<String>,
    started_at: Option<u64>,
    eon_version: Option<String>,
    workspace_protocol: Option<u16>,
    component_report: Option<String>,
    pub(super) sessions: Vec<String>,
    /// Its busy terminals (EON-C11), known for a live full-Eon window.
    busy: Option<BusyTerminals>,
    attach: Availability,
    pub(super) stop: Availability,
    detail: String,
    /// The recorded `eon` of the other build that started it, which presents
    /// or stops it when this build cannot.
    launcher: Option<PathBuf>,
}

fn discover_generations(
    inputs: &Inputs,
    root: &Path,
    current: &str,
) -> Result<Vec<GenerationRecord>, String> {
    let component_report = inputs.component_report()?;
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(vec![unstarted_current_generation(root, current)]);
    }

    let candidates = generation_candidates(root)?;

    let current_path = generation_directory(root, current);
    let mut records = vec![inspect_generation(
        current,
        "current",
        current_path,
        &supervisor_lock_path(root, current),
        component_report,
        true,
    )];
    for (id, path) in candidates {
        let id = id.to_string_lossy();
        if id != current {
            records.push(inspect_generation(
                &id,
                "previous",
                path,
                &supervisor_lock_path(root, &id),
                component_report,
                true,
            ));
        }
    }
    Ok(records)
}

pub(super) struct SupervisorInventory {
    pub entries: Vec<WindowEntry>,
    /// Include dormant generations: recovery can reuse their existing directory.
    pub observed: Vec<(PathBuf, String)>,
}

pub(super) const RUNNING_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

/// Read-only, off-loop inventory with at most 32 concurrent peer probes.
pub(super) fn live_supervisors(
    base: &Path,
    self_root: &Path,
    generation: &str,
    log: &super::diagnostics::DiagnosticLog,
) -> Result<(SupervisorInventory, super::running::Fingerprint), String> {
    let (roots, _) = super::running::window_roots(base, Some(log))?;
    let mut candidates = Vec::new();
    let mut observed = Vec::new();
    for root in roots {
        let generations = match generation_candidates(&root) {
            Ok(generations) => generations,
            Err(error) => {
                log.note_once(&format!(
                    "skipping running Eon namespace {}: {error}",
                    root.display()
                ));
                continue;
            }
        };
        for (id, runtime) in generations {
            let Some(id) = id.to_str().filter(|id| valid_generation(id)) else {
                continue;
            };
            if root == self_root && id == generation {
                continue;
            }
            observed.push((root.clone(), id.to_owned()));
            let lock = supervisor_lock_path(&root, id);
            let held = match super::running::supervisor_held(&lock) {
                Ok(held) => held,
                Err(error) => {
                    log.note_once(&format!("skipping running Eon generation: {error}"));
                    continue;
                }
            };
            if held {
                let window = (root != base)
                    .then(|| root.file_name().unwrap().to_string_lossy().into_owned());
                candidates.push((
                    WindowEntry {
                        window,
                        generation: id.into(),
                        name: None,
                        started_at: None,
                        live_terminals: None,
                        state: WindowState::Unreachable,
                    },
                    runtime,
                    lock,
                ));
            }
        }
    }
    let fingerprint = super::running::Fingerprint::read(base, &observed)?;
    let mut candidates = candidates.into_iter();
    let mut entries = Vec::new();
    loop {
        let batch: Vec<_> = candidates.by_ref().take(MAX_RUNNING_WINDOWS).collect();
        if batch.is_empty() {
            break;
        }
        let inspected = thread::scope(|scope| {
            let probes: Vec<_> = batch
                .into_iter()
                .map(|(entry, runtime, lock)| {
                    let id = entry.generation.clone();
                    let probe = thread::Builder::new()
                        .spawn_scoped(scope, move || {
                            inspect_generation(&id, "previous", runtime, &lock, "", false)
                        })
                        .map_err(|error| {
                            format!("cannot start running Eon window inspection: {error}")
                        })?;
                    Ok((entry, probe))
                })
                .collect::<Result<_, String>>()?;
            probes
                .into_iter()
                .map(|(mut entry, probe)| {
                    let record = probe
                        .join()
                        .map_err(|_| "running Eon window inspection panicked".to_string())?;
                    match record.state {
                        // Full Eon always supplies its derived workspace name; eonbase has none.
                        "live" if record.name.is_none() => return Ok(None),
                        "live" => {
                            entry.name = record.name;
                            entry.started_at = record.started_at;
                            entry.live_terminals = Some(record.sessions.len() as u16);
                            entry.state = WindowState::Live;
                        }
                        "incompatible" => entry.state = WindowState::Incompatible,
                        _ => {}
                    }
                    Ok(Some(entry))
                })
                .filter_map(Result::transpose)
                .collect::<Result<Vec<_>, String>>()
        })?;
        entries.extend(inspected);
    }
    Ok((SupervisorInventory { entries, observed }, fingerprint))
}

fn inspect_selected_generation(
    inputs: &Inputs,
    root: &Path,
    current: &str,
    target: &str,
) -> Result<Option<GenerationRecord>, String> {
    let missing_current =
        || (target == current).then(|| unstarted_current_generation(root, current));
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(missing_current());
    }
    let parent = root.join("generations");
    if !private_directory_exists(&parent, "generation directory")? {
        return Ok(missing_current());
    }
    let runtime = generation_directory(root, target);
    if target != current
        && matches!(
            fs::symlink_metadata(&runtime),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        )
    {
        return Ok(None);
    }
    let component_report = inputs.component_report()?;
    Ok(Some(inspect_generation(
        target,
        if target == current {
            "current"
        } else {
            "previous"
        },
        runtime,
        &supervisor_lock_path(root, target),
        component_report,
        true,
    )))
}

fn inspect_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
    lifecycle_lock: &Path,
    current_components: &str,
    cleanup: bool,
) -> GenerationRecord {
    if !valid_generation(id) {
        return failed_generation(
            id,
            kind,
            runtime,
            EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                "generation directory name is not a valid Eon identity",
            ),
        );
    }
    if let Err(detail) = validate_private_directory(&runtime) {
        return if matches!(
            fs::symlink_metadata(&runtime),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound
        ) {
            dead_generation(id, kind, runtime, "generation has not started")
        } else {
            failed_generation(
                id,
                kind,
                runtime,
                EndpointFailure::new(EndpointFailureKind::Corrupt, detail),
            )
        };
    }

    let socket = runtime.join("eon.sock");
    let probe = if cleanup {
        probe_generation_runtime(&socket)
    } else {
        probe_generation_runtime_until(&socket, Instant::now() + RUNNING_PROBE_TIMEOUT)
    };
    let mut record = match probe {
        Ok(info) if info.generation != id => failed_generation(
            id,
            kind,
            runtime,
            EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                format!(
                    "supervisor reports generation {} from directory {id}",
                    info.generation
                ),
            ),
        ),
        Ok(mut info) => {
            // A supervisor that refuses presentation itself keeps its reason.
            let other_build =
                cleanup && info.component_report != current_components && info.attach.available;
            if other_build {
                info.attach = Availability {
                    available: false,
                    reason: "component graph differs from the current Eon build; \
                             the Eon build that started it can attach or stop it"
                        .into(),
                };
            }
            GenerationRecord {
                id: id.into(),
                kind,
                state: "live",
                name: info.name,
                started_at: Some(info.started_at),
                eon_version: Some(info.eon_version),
                workspace_protocol: Some(info.workspace_protocol),
                component_report: Some(info.component_report),
                sessions: info.sessions,
                busy: info.busy,
                attach: info.attach,
                stop: info.stop,
                detail: "live supervisor validated its generation identity".into(),
                launcher: other_build.then(|| recorded_launcher(&runtime)).flatten(),
                runtime,
            }
        }
        Err(error) => {
            let dead = error.kind == EndpointFailureKind::Dead;
            let mut record = failed_generation(id, kind, runtime.clone(), error);
            if cleanup && record.state == "incompatible" {
                record.launcher = recorded_launcher(&runtime);
                if let Some(eon) = &record.launcher {
                    let root = generations_root(&runtime);
                    let window = (!is_default_namespace(root)).then_some(root);
                    record.stop = match launcher_fault(eon) {
                        None => Availability {
                            available: true,
                            reason: "stop runs the Eon build that started it".into(),
                        },
                        Some(fault) => Availability {
                            available: false,
                            reason: format!(
                                "{}; its recorded eon {} {fault}; restore that build, {}",
                                record.detail,
                                eon.display(),
                                stop_advice(eon, window, id)
                            ),
                        },
                    };
                }
            }
            if cleanup
                && dead
                && let Ok(Some(_lifecycle_lock)) = try_lock_supervisor_lifecycle(lifecycle_lock)
            {
                remove_dead_socket(&socket);
                remove_launcher(&runtime);
                let _ = fs::remove_dir(&runtime);
            }
            record
        }
    };
    if record.launcher.is_some() {
        record.attach = Availability {
            available: true,
            reason: "attach runs the Eon build that started it".into(),
        };
    }
    record
}

fn dead_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
    detail: impl Into<String>,
) -> GenerationRecord {
    failed_generation(
        id,
        kind,
        runtime,
        EndpointFailure::new(EndpointFailureKind::Dead, detail),
    )
}

fn unstarted_current_generation(root: &Path, current: &str) -> GenerationRecord {
    dead_generation(
        current,
        "current",
        generation_directory(root, current),
        "current generation has not started",
    )
}

fn failed_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
    error: EndpointFailure,
) -> GenerationRecord {
    let state = match error.kind {
        EndpointFailureKind::Dead => "dead",
        EndpointFailureKind::Incompatible => "incompatible",
        EndpointFailureKind::InvalidAction | EndpointFailureKind::Corrupt => "corrupt",
        EndpointFailureKind::Unreachable => "unreachable",
    };
    GenerationRecord {
        id: id.into(),
        kind,
        state,
        runtime,
        name: None,
        started_at: None,
        eon_version: None,
        workspace_protocol: None,
        component_report: None,
        sessions: Vec::new(),
        busy: None,
        attach: Availability {
            available: false,
            reason: error.detail.clone(),
        },
        stop: Availability {
            available: false,
            reason: error.detail.clone(),
        },
        detail: error.detail,
        launcher: None,
    }
}

pub(super) fn private_directory_exists(path: &Path, description: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => validate_private_directory(path).map(|()| true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "cannot inspect {description} {}: {error}",
            path.display()
        )),
    }
}

fn remove_dead_socket(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
        return;
    }
    let identity = socket_identity_from(&metadata);
    if matches!(
        UnixStream::connect(path),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
    ) {
        remove_socket_if_identity(path, identity);
    }
}

pub(super) fn generations_command(
    inputs: &Inputs,
    json: bool,
    product: &str,
) -> Result<i32, String> {
    let records = discover_generations(
        inputs,
        &runtime_directory(product),
        &current_generation(inputs)?,
    )?;
    write_stdout(if json {
        generations_json(&records)
    } else {
        generations_human(&records)
    })?;
    Ok(0)
}

fn generations_human(records: &[GenerationRecord]) -> String {
    let mut output = String::new();
    for record in records {
        output.push_str(&format!(
            "{} {} {} terminals={}\n",
            record.kind,
            record.id.escape_debug(),
            record.state,
            record.sessions.len()
        ));
        output.push_str(&format!(
            "  attach={} {}\n  stop={} {}\n  {}\n",
            record.attach.available,
            record.attach.reason.escape_debug(),
            record.stop.available,
            terminal_text(&record.stop.reason),
            record.detail.escape_debug()
        ));
        if let (Some(version), Some(protocol)) = (&record.eon_version, record.workspace_protocol) {
            output.push_str(&format!(
                "  eon={} eonw={protocol}\n",
                version.escape_debug()
            ));
        }
        if let Some(eon) = &record.launcher {
            output.push_str(&format!("  executable {}\n", eon.display()));
        }
        if let Some(started_at) = record.started_at {
            output.push_str(&format!(
                "  name={} started_at={started_at}\n",
                record.name.as_deref().unwrap_or("unknown").escape_debug()
            ));
        }
        for session in &record.sessions {
            output.push_str(&format!("  terminal {session}\n"));
        }
        for line in super::workspace::human_busy(record.busy.as_ref()).lines() {
            output.push_str(&format!("  {line}\n"));
        }
        if let Some(report) = &record.component_report {
            for line in report.lines() {
                output.push_str(&format!("  component {}\n", line.escape_debug()));
            }
        }
    }
    output
}

fn generations_json(records: &[GenerationRecord]) -> String {
    let mut output = String::from("{\"generations\":[");
    for (index, record) in records.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!(
            "{{\"id\":\"{}\",\"kind\":\"{}\",\"state\":\"{}\",\"name\":{},\"started_at\":{},\"eon_version\":{},\"workspace_protocol\":{},\"component_report\":{},\"executable\":{},\"sessions\":[",
            json_escape(&record.id),
            record.kind,
            record.state,
            json_option(record.name.as_deref()),
            record.started_at.map_or_else(|| "null".into(), |value| value.to_string()),
            json_option(record.eon_version.as_deref()),
            record.workspace_protocol.map_or_else(|| "null".into(), |value| value.to_string()),
            json_option(record.component_report.as_deref()),
            json_option(
                record
                    .launcher
                    .as_ref()
                    .map(|eon| eon.to_string_lossy())
                    .as_deref()
            ),
        ));
        for (session_index, session) in record.sessions.iter().enumerate() {
            if session_index != 0 {
                output.push(',');
            }
            output.push_str(&format!("\"{}\"", json_escape(session)));
        }
        output.push_str("],\"busy\":");
        super::workspace::json_busy(&mut output, record.busy.as_ref());
        output.push_str(&format!(
            ",\"attach\":{{\"available\":{},\"reason\":\"{}\"}},\"stop\":{{\"available\":{},\"reason\":\"{}\"}},\"detail\":\"{}\"}}",
            record.attach.available,
            json_escape(&record.attach.reason),
            record.stop.available,
            json_escape(&record.stop.reason),
            json_escape(&record.detail),
        ));
    }
    output.push_str("]}\n");
    output
}

fn json_option(value: Option<&str>) -> String {
    value.map_or_else(
        || "null".into(),
        |value| format!("\"{}\"", json_escape(value)),
    )
}

pub(super) fn attach_generation(
    inputs: &Inputs,
    target: Option<&str>,
    product: &str,
) -> Result<i32, Error> {
    let root = runtime_directory(product);
    let current = current_generation(inputs)?;
    let target = target.unwrap_or(&current);
    let record = inspect_selected_generation(inputs, &root, &current, target)?
        .ok_or_else(|| Error::missing(format!("generation {target} was not found")))?;
    if !record.attach.available {
        let versions = match (&record.eon_version, record.workspace_protocol) {
            (Some(version), Some(protocol)) => format!(" (Eon {version}, EONW {protocol})"),
            _ => String::new(),
        };
        return Err(format!(
            "generation {target}{versions} is not attachable: {}",
            record.attach.reason
        )
        .into());
    }
    // That build presents its own generation with its own components. The
    // marker stops a record that names this build from delegating again.
    if let Some(eon) = &record.launcher
        && env::var_os(DELEGATED).is_none_or(|delegated| delegated != target)
    {
        let error = Command::new(eon)
            .env(DELEGATED, target)
            .arg("attach")
            .arg(target)
            .exec();
        return Err(format!("cannot run {}: {error}", eon.display()).into());
    }
    let (mode, supervisor) = probe_supervisor(inputs, &record.runtime.join("eon.sock"), target)
        .map_err(|error| error.detail)?;
    present_at(inputs, &record.runtime, target, mode, supervisor).map_err(Error::from)
}

/// The generations of the window at `root` whose supervisor has not ended,
/// current first.
pub(super) fn running_generations(
    inputs: &Inputs,
    root: &Path,
) -> Result<Vec<GenerationRecord>, String> {
    let mut records = discover_generations(inputs, root, &current_generation(inputs)?)?;
    records.retain(|record| record.state != "dead");
    Ok(records)
}

/// The generation `eon window attach` presents: current unless it has ended,
/// else the window's one other running generation.
pub(super) fn window_generation(inputs: &Inputs, id: &str, root: &Path) -> Result<String, Error> {
    let records = running_generations(inputs, root)?;
    if let Some(selected) = preferred_window_generation(
        &current_generation(inputs)?,
        &records
            .iter()
            .map(|record| record.id.clone())
            .collect::<Vec<_>>(),
    ) {
        return Ok(selected.to_owned());
    }
    match records.as_slice() {
        [] => Err(Error::unreachable(format!(
            "Eon window {id} has ended; no supervisor runs in it"
        ))),
        records => Err(format!(
            "Eon window {id} runs generations {}; run `eon window attach {id}` with the `eon` of the one to present",
            records
                .iter()
                .map(|record| match &record.launcher {
                    Some(eon) => format!("{} (its eon: {})", record.id, eon.display()),
                    None => record.id.clone(),
                })
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into()),
    }
}

fn preferred_window_generation<'a>(current: &str, live: &'a [String]) -> Option<&'a str> {
    live.iter()
        .find(|id| *id == current)
        .or_else(|| (live.len() == 1).then(|| &live[0]))
        .map(String::as_str)
}

/// The `eon windows` inventory of the window at `root`: its running
/// generations, or `None` once the window has ended.
pub(super) fn window_inventory(
    inputs: &Inputs,
    root: &Path,
    json: bool,
) -> Result<Option<String>, String> {
    let records = running_generations(inputs, root)?;
    Ok((!records.is_empty()).then(|| {
        if json {
            generations_json(&records)
        } else {
            generations_human(&records)
        }
    }))
}

/// How a human-mode Stop asks first.
pub(super) enum Ask {
    /// Always, naming the generation and its terminals.
    Stop,
    /// Only while a terminal other than `caller`, the terminal running the
    /// command, is busy (EON-C11), naming the window: "this window" when
    /// `this_window`, else its name.
    Quit {
        caller: Option<String>,
        this_window: bool,
    },
}

/// Quit's question and its decline, or none while nothing is busy. A
/// window whose busy terminals are unknown names every live terminal.
fn quit_question(
    record: &GenerationRecord,
    sessions: &[String],
    busy: Option<&BusyTerminals>,
    this_window: bool,
) -> Option<(String, String)> {
    let (names, omitted) = match busy {
        Some(busy) if busy.terminals.is_empty() && busy.omitted == 0 => return None,
        Some(busy) => (
            busy.terminals
                .iter()
                .map(|terminal| terminal.name.clone())
                .collect::<Vec<_>>(),
            usize::from(busy.omitted),
        ),
        None => (sessions.to_vec(), 0),
    };
    let window = if this_window {
        "this window".to_owned()
    } else {
        // EONW carries names without control characters.
        format!("window {}", record.name.as_deref().unwrap_or(&record.id))
    };
    let plural = |count: usize| if count == 1 { "" } else { "s" };
    let running = names.len() + omitted;
    let mut question = format!(
        "Quit {window}? It stops {} terminal{}; {running} still running: {}",
        sessions.len(),
        plural(sessions.len()),
        names.join(", ")
    );
    if omitted > 0 {
        question.push_str(&format!(" and {omitted} more"));
    }
    question.push_str(". [y/N] ");
    Some((question, format!("cancelled; {window} keeps running\n")))
}

pub(super) fn stop_generation(
    inputs: &Inputs,
    root: &Path,
    target: &str,
    json: bool,
    ask: &Ask,
) -> Result<i32, String> {
    let current = current_generation(inputs)?;
    let control = generation_directory(root, target).join("eon.sock");
    let observed_supervisor = socket_identity(&control);
    let Some(record) = inspect_selected_generation(inputs, root, &current, target)? else {
        return report_failure(
            &failure(
                "unknown-generation",
                format!("generation {target} was not found"),
            ),
            json,
        );
    };
    // That build stops its own generation; a delegated Stop never delegates
    // the same generation again.
    if record.state == "incompatible"
        && record.stop.available
        && let Some(eon) = &record.launcher
    {
        if env::var_os(STOP_DELEGATED).is_some_and(|delegated| delegated == target) {
            return report_failure(
                &failure(
                    "stop-unavailable",
                    format!("generation {target}: {}", record.detail),
                ),
                json,
            );
        }
        return delegated_stop(&record, eon, root, json, ask);
    }
    if !record.stop.available {
        return report_failure(
            &failure(
                "stop-unavailable",
                format!("generation {target}: {}", record.stop.reason),
            ),
            json,
        );
    }
    let supervisor = match observed_supervisor {
        Ok(Some(supervisor))
            if socket_identity(&control).is_ok_and(|current| current == Some(supervisor)) =>
        {
            supervisor
        }
        _ => {
            return report_failure(
                &failure(
                    "stop-failed",
                    "supervisor endpoint changed while stop was being validated",
                ),
                json,
            );
        }
    };
    let question = match ask {
        _ if json => None,
        Ask::Stop => Some((
            format!(
                "Stop generation {target} and {} live terminal{} [{}]? [y/N] ",
                record.sessions.len(),
                if record.sessions.len() == 1 { "" } else { "s" },
                record.sessions.join(", ")
            ),
            format!("cancelled; no Stop sent for generation {target}\n"),
        )),
        Ask::Quit {
            caller,
            this_window,
        } => {
            // Inspect again as the caller, whose own terminal is not one to
            // ask about; a failed inspection leaves every terminal busy.
            let runtime = probe_generation_runtime_from(&control, caller.clone())
                .ok()
                .filter(|runtime| runtime.generation == target);
            let (sessions, busy) = runtime.map_or((record.sessions.clone(), None), |runtime| {
                (runtime.sessions, runtime.busy)
            });
            quit_question(&record, &sessions, busy.as_ref(), *this_window)
        }
    };
    if let Some((question, cancelled)) = question
        && let Some(status) = confirm(&question, &cancelled)?
    {
        return Ok(status);
    }

    let stream = match connect_control(&control) {
        Ok(stream) => stream,
        Err(error) => return report_failure(&failure("stop-failed", error.detail), json),
    };
    if !socket_identity(&control).is_ok_and(|current| current == Some(supervisor)) {
        return report_failure(
            &failure(
                "stop-failed",
                "supervisor endpoint changed before stop could be sent",
            ),
            json,
        );
    }
    let response = match send_action_on(
        stream,
        Action::Workspace(WorkspaceAction::Stop {
            generation: target.into(),
        }),
    ) {
        Ok(response) => response,
        Err(error) => {
            return report_failure(&failure("stop-failed", error.detail), json);
        }
    };
    match response {
        ControlResponse::Lifecycle(LifecycleResponse::Stopped(stopped))
            if stopped.generation == target =>
        {
            report_stopped(&stopped, json)
        }
        ControlResponse::Lifecycle(LifecycleResponse::Failure(failure)) => {
            report_failure(&failure, json)
        }
        _ => report_failure(
            &failure(
                "stop-failed",
                "supervisor returned the wrong EONW result for stop",
            ),
            json,
        ),
    }
}

fn report_stopped(stopped: &Stopped, json: bool) -> Result<i32, String> {
    if json {
        write_stdout(stopped_json(stopped))?;
    } else {
        write_stdout(format!(
            "stopped generation {}: {}\n",
            stopped.generation,
            stopped.sessions.join(", ")
        ))?;
    }
    Ok(0)
}

/// Stops a generation of another EONW version by running its recorded `eon`
/// as `stop GENERATION --json`, without input, in the generation's namespace
/// (EON-C11). This build asks any question first and reports the result as
/// its own.
fn delegated_stop(
    record: &GenerationRecord,
    eon: &Path,
    root: &Path,
    json: bool,
    ask: &Ask,
) -> Result<i32, String> {
    let target = record.id.as_str();
    if !json {
        let (question, cancelled) = delegated_question(record, ask);
        if let Some(status) = confirm(&question, &cancelled)? {
            return Ok(status);
        }
    }
    let mut command = Command::new(eon);
    command
        .args(["stop", target, "--json"])
        .env(STOP_DELEGATED, target)
        .env_remove(TERMINAL_VARIABLE)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit());
    if is_default_namespace(root) {
        command.env_remove(WINDOW_RUNTIME);
    } else {
        command.env(WINDOW_RUNTIME, root);
    }
    let result = match command.output() {
        Ok(output) => {
            delegated_result(&output.stdout, output.status, target).unwrap_or_else(|| {
                Err(failure(
                    "stop-failed",
                    format!(
                        "{} stop {target} ended with {} and no complete Stop result",
                        eon.display(),
                        output.status
                    ),
                ))
            })
        }
        Err(error) => Err(failure(
            "stop-failed",
            format!("cannot run {}: {error}", eon.display()),
        )),
    };
    match result {
        Ok(stopped) => report_stopped(&stopped, json),
        Err(failure) => report_failure(&failure, json),
    }
}

/// A delegated Stop's question and its decline: its terminals are unknown
/// here, so it always asks.
fn delegated_question(record: &GenerationRecord, ask: &Ask) -> (String, String) {
    const UNKNOWN: &str = "It runs another Eon build, so its terminals are unknown. [y/N] ";
    let id = &record.id;
    match ask {
        Ask::Stop => (
            format!("Stop generation {id}? {UNKNOWN}"),
            format!("cancelled; no Stop sent for generation {id}\n"),
        ),
        Ask::Quit { this_window, .. } => {
            let window = if *this_window {
                "this window".to_owned()
            } else {
                format!("window {id}")
            };
            (
                format!("Quit {window}? {UNKNOWN}"),
                format!("cancelled; {window} keeps running\n"),
            )
        }
    }
}

/// The one Stop result for `target` that a delegated `eon stop --json`
/// printed and completed, else none: a Stop that a signal ended is not
/// complete, nor is a `stopped` result whose `eon` failed.
fn delegated_result(
    stdout: &[u8],
    status: ExitStatus,
    target: &str,
) -> Option<Result<Stopped, Failure>> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "lowercase")]
    enum Delegated {
        Stopped {
            generation: String,
            sessions: Vec<String>,
        },
        Error {
            code: String,
            detail: String,
        },
    }
    match serde_json::from_slice(stdout).ok()? {
        Delegated::Stopped {
            generation,
            sessions,
        } if generation == target && status.success() => Some(Ok(Stopped {
            generation,
            sessions,
        })),
        Delegated::Error { code, detail } if !code.is_empty() && status.code().is_some() => {
            Some(Err(failure(code, detail)))
        }
        _ => None,
    }
}

pub(super) fn confirm(question: &str, cancelled: &str) -> Result<Option<i32>, String> {
    eprint!("{question}");
    let mut answer = Vec::new();
    loop {
        let mut byte = 0u8;
        // SAFETY: read writes at most one byte into the valid local buffer.
        let result = unsafe { libc::read(libc::STDIN_FILENO, (&mut byte as *mut u8).cast(), 1) };
        match result {
            0 => break,
            1 if byte == b'\n' => break,
            1 => answer.push(byte),
            _ => {
                let error = std::io::Error::last_os_error();
                if error.kind() != std::io::ErrorKind::Interrupted {
                    return Err(format!("cannot read stop confirmation: {error}"));
                }
            }
        }
    }
    match confirmation_status(&answer, std::io::stdin().is_terminal()) {
        Confirmation::Confirmed => Ok(None),
        Confirmation::Cancelled => {
            write_stdout(cancelled)?;
            Ok(Some(0))
        }
        Confirmation::Refused => report_failure(
            &failure(
                "not-confirmed",
                format!(
                    "{}; no yes was read and stdin is not a terminal",
                    cancelled.trim_end().trim_start_matches("cancelled; ")
                ),
            ),
            false,
        )
        .map(Some),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Confirmation {
    Confirmed,
    Cancelled,
    Refused,
}

fn confirmation_status(answer: &[u8], terminal: bool) -> Confirmation {
    match answer.trim_ascii() {
        b"y" | b"Y" | b"yes" | b"YES" => Confirmation::Confirmed,
        _ if terminal => Confirmation::Cancelled,
        _ => Confirmation::Refused,
    }
}

pub(super) fn stop_generations(
    inputs: &Inputs,
    root: &Path,
    include_current: bool,
    json: bool,
    ask: &Ask,
) -> Result<i32, String> {
    let mut records = discover_generations(inputs, root, &current_generation(inputs)?)?;
    // The CLI may itself be running inside current; stop that generation last.
    records.sort_by_key(|record| record.kind == "current");
    if json {
        write_stdout("[")?;
    }
    let mut separator = "";
    let mut status = 0;
    for record in records {
        if record.state == "dead" || (!include_current && record.kind == "current") {
            continue;
        }
        if json {
            write_stdout(separator)?;
            separator = ",";
        }
        let result = match stop_generation(inputs, root, &record.id, json, ask) {
            Ok(code) => code,
            Err(error) => report_failure(
                &failure("stop-failed", format!("generation {}: {error}", record.id)),
                json,
            )?,
        };
        status = status.max(result);
    }
    if json {
        write_stdout("]\n")?;
    }
    Ok(status)
}

fn stopped_json(stopped: &Stopped) -> String {
    let mut output = format!(
        "{{\"stopped\":{{\"generation\":\"{}\",\"sessions\":[",
        json_escape(&stopped.generation)
    );
    for (index, session) in stopped.sessions.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        output.push_str(&format!("\"{}\"", json_escape(session)));
    }
    output.push_str("]}}\n");
    output
}

#[cfg(test)]
mod tests {
    use super::{
        BusyTerminals, Confirmation, confirmation_status, current_generation, discover_generations,
        generation_directory, generation_id, valid_generation,
    };
    use crate::supervisor::{lock_supervisor_lifecycle, supervisor_lock_path, temporary_directory};
    use std::{
        fs,
        io::ErrorKind,
        os::unix::{
            fs::{PermissionsExt, symlink},
            net::{UnixListener, UnixStream},
        },
        path::Path,
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn read_only_discovery_skips_bad_entries() {
        use std::os::unix::fs::DirBuilderExt;
        let base = temporary_directory();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let log = crate::diagnostics::DiagnosticLog::open_in(Ok(base.join("logs")), "test", "test");
        let bad_id = "g1-11111111111111111111111111111111";
        let good_id = "g1-22222222222222222222222222222222";
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(generation_directory(&base, bad_id))
            .unwrap();
        let bad_lock = supervisor_lock_path(&base, bad_id);
        fs::create_dir(&bad_lock).unwrap();
        let good_root = base.join("w/00000001");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(generation_directory(&good_root, good_id))
            .unwrap();
        let held = lock_supervisor_lifecycle(&supervisor_lock_path(&good_root, good_id)).unwrap();
        let over_limit = base.join("w/00000002");
        for index in 0..=super::MAX_GENERATIONS {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(generation_directory(
                    &over_limit,
                    &format!("g1-{index:032x}"),
                ))
                .unwrap();
        }
        for _ in 0..2 {
            let (inventory, fingerprint) =
                super::live_supervisors(&base, &base, "self", &log).unwrap();
            assert_eq!(
                fingerprint,
                super::super::running::Fingerprint::read(&base, &inventory.observed).unwrap()
            );
            assert_eq!(inventory.entries.len(), 1);
            assert_eq!(inventory.entries[0].generation, good_id);
        }
        let notes = fs::read_dir(base.join("logs"))
            .unwrap()
            .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
            .collect::<String>();
        assert_eq!(
            notes
                .lines()
                .filter(|line| line.contains(bad_lock.to_str().unwrap()))
                .count(),
            1
        );
        assert_eq!(
            notes
                .lines()
                .filter(|line| line.contains(over_limit.to_str().unwrap()))
                .count(),
            1
        );
        held.unlock().unwrap();
        drop(log);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn read_only_discovery_excludes_terminal_only_supervisors() {
        use eon_workspace_protocol::v30::{
            Availability, HEADER_BYTES, LifecycleResponse, Runtime, declared_message_len,
            encode_lifecycle_response,
        };
        use std::io::{Read, Write};
        let base = temporary_directory();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let id = "g1-11111111111111111111111111111111";
        let directory = generation_directory(&base, id);
        fs::create_dir_all(&directory).unwrap();
        for path in [base.join("generations"), directory.clone()] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let socket = directory.join("eon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let held = lock_supervisor_lifecycle(&supervisor_lock_path(&base, id)).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = vec![0; HEADER_BYTES];
            stream.read_exact(&mut request).unwrap();
            request.resize(declared_message_len(&request).unwrap(), 0);
            stream.read_exact(&mut request[HEADER_BYTES..]).unwrap();
            let available = Availability {
                available: true,
                reason: "terminal-only supervisor".into(),
            };
            let response = LifecycleResponse::Runtime(Runtime {
                generation: id.into(),
                name: None,
                started_at: 123,
                eon_version: "fixture".into(),
                workspace_protocol: eon_workspace_protocol::v30::VERSION,
                component_report: "fixture".into(),
                sessions: vec!["session-1".into()],
                attach: available.clone(),
                stop: available,
                busy: None,
            });
            stream
                .write_all(&encode_lifecycle_response(&response).unwrap())
                .unwrap();
        });
        let log = crate::diagnostics::DiagnosticLog::open_in(Ok(base.join("logs")), "test", "test");
        let entries = super::live_supervisors(&base, &base.join("w/1234abcd"), id, &log)
            .unwrap()
            .0;
        server.join().unwrap();
        held.unlock().unwrap();
        assert!(
            entries.entries.is_empty(),
            "eonbase appeared as a window: {:?}",
            entries.entries
        );
        assert_eq!(entries.observed.len(), 1);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn read_only_inspection_keeps_dead_generation_residue() {
        let base = temporary_directory();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        let id = "g1-11111111111111111111111111111111";
        let directory = generation_directory(&base, id);
        fs::create_dir_all(&directory).unwrap();
        for path in [base.join("generations"), directory.clone()] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let socket = directory.join("eon.sock");
        drop(UnixListener::bind(&socket).unwrap());
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while UnixStream::connect(&socket).is_ok() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        symlink("/bin/false", directory.join("eon")).unwrap();
        let lock = supervisor_lock_path(&base, id);
        let record =
            super::inspect_generation(id, "previous", directory.clone(), &lock, "test", false);
        assert_eq!(record.state, "dead");
        assert!(
            super::live_supervisors(
                &base,
                &base,
                "other",
                &crate::diagnostics::DiagnosticLog::open_in(Ok(base.join("logs")), "test", "test")
            )
            .unwrap()
            .0
            .entries
            .is_empty()
        );
        assert!(!lock.exists(), "read-only discovery created a lock");
        assert!(fs::symlink_metadata(socket).is_ok());
        assert_eq!(
            fs::read_link(directory.join("eon")).unwrap(),
            Path::new("/bin/false")
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn generations_json_carries_name_and_started_at() {
        let mut record = super::dead_generation("g1", "previous", "/runtime".into(), "ended");
        let value: serde_json::Value =
            serde_json::from_str(&super::generations_json(std::slice::from_ref(&record))).unwrap();
        assert_eq!(
            value["generations"][0].get("name"),
            Some(&serde_json::Value::Null)
        );
        assert_eq!(
            value["generations"][0].get("started_at"),
            Some(&serde_json::Value::Null)
        );
        record.state = "live";
        record.name = Some("quoted \"name\"".into());
        record.started_at = Some(123);
        record.busy = Some(BusyTerminals {
            terminals: Vec::new(),
            omitted: 0,
        });
        let value: serde_json::Value =
            serde_json::from_str(&super::generations_json(std::slice::from_ref(&record))).unwrap();
        assert_eq!(value["generations"][0]["name"], "quoted \"name\"");
        assert_eq!(value["generations"][0]["started_at"], 123);
        assert_eq!(
            value["generations"][0]["busy"],
            serde_json::json!({"terminals": [], "omitted": 0})
        );
        record.busy = None;
        let value: serde_json::Value =
            serde_json::from_str(&super::generations_json(&[record])).unwrap();
        assert!(value["generations"][0]["busy"].is_null());
    }

    #[test]
    fn quit_asks_only_while_busy_in_window_language() {
        use eon_workspace_protocol::v30::{BusyTerminal, TargetId, TerminalTarget};
        let mut record = super::dead_generation("g1", "current", "/runtime".into(), "ended");
        let sessions = ["session-1".to_owned(), "session-2".to_owned()];
        let busy = |names: &[&str], omitted| BusyTerminals {
            terminals: names
                .iter()
                .enumerate()
                .map(|(index, name)| BusyTerminal {
                    target: TerminalTarget {
                        workspace: "w1".into(),
                        tab: "t1".into(),
                        id: TargetId::Pane(format!("p{index}")),
                        terminal: format!("session-{index}"),
                    },
                    name: (*name).into(),
                })
                .collect(),
            omitted,
        };
        assert_eq!(
            super::quit_question(&record, &sessions, Some(&busy(&[], 0)), true),
            None
        );
        assert_eq!(
            super::quit_question(
                &record,
                &sessions,
                Some(&busy(&["p2  vim", "Files"], 0)),
                true
            ),
            Some((
                "Quit this window? It stops 2 terminals; 2 still running: p2  vim, Files. [y/N] "
                    .into(),
                "cancelled; this window keeps running\n".into(),
            ))
        );
        // Unknown busy terminals name every live one; outside Eon the
        // question names the window, else its generation.
        assert_eq!(
            super::quit_question(&record, &sessions[..1], None, false),
            Some((
                "Quit window g1? It stops 1 terminal; 1 still running: session-1. [y/N] ".into(),
                "cancelled; window g1 keeps running\n".into(),
            ))
        );
        record.name = Some("eon".into());
        assert_eq!(
            super::quit_question(&record, &sessions, Some(&busy(&["Agent"], 4)), false)
                .unwrap()
                .0,
            "Quit window eon? It stops 2 terminals; 5 still running: Agent and 4 more. [y/N] "
        );
    }

    #[test]
    fn delegated_question_names_another_build() {
        let record = super::dead_generation("g1", "previous", "/runtime".into(), "ended");
        assert_eq!(
            super::delegated_question(&record, &super::Ask::Stop),
            (
                "Stop generation g1? It runs another Eon build, so its terminals are unknown. [y/N] "
                    .into(),
                "cancelled; no Stop sent for generation g1\n".into(),
            )
        );
        assert_eq!(
            super::delegated_question(
                &record,
                &super::Ask::Quit {
                    caller: None,
                    this_window: false,
                }
            ),
            (
                "Quit window g1? It runs another Eon build, so its terminals are unknown. [y/N] "
                    .into(),
                "cancelled; window g1 keeps running\n".into(),
            )
        );
    }

    #[test]
    fn stop_advice_runs_as_printed_in_sh_and_nushell() {
        use std::process::{Command, Stdio};
        let base = temporary_directory();
        let log = base.join("log");
        let window = base.join("e r").join("w").join("0000abcd");
        // Each directory names the recorded `eon`; the last two have no form
        // that both shells read, so their advice has one command per shell.
        for (directory, shared) in [
            ("e r", true),
            ("it's", true),
            ("bang!", true),
            ("it's!", false),
            ("it's`#'#", false),
        ] {
            let eon = base.join(directory).join("eon");
            fs::create_dir_all(eon.parent().unwrap()).unwrap();
            fs::write(
                &eon,
                format!(
                    "#!/bin/sh\nprintf '%s|%s\\n' \"$*\" \"${{EON_WINDOW_RUNTIME_DIR-unset}}\" >> '{}'\n",
                    log.display()
                ),
            )
            .unwrap();
            fs::set_permissions(&eon, fs::Permissions::from_mode(0o755)).unwrap();
            for namespace in [None, Some(window.as_path())] {
                let advice = super::stop_advice(&eon, namespace, "g1-x");
                let commands = match advice.strip_prefix("then run ") {
                    Some(command) => vec![("sh", command), ("nu", command)],
                    None => {
                        let lines: Vec<_> = advice.lines().collect();
                        assert_eq!(lines[0], "then run, in sh:", "{advice}");
                        assert_eq!(lines[2], "  or in Nushell:", "{advice}");
                        vec![("sh", lines[1].trim_start()), ("nu", lines[3].trim_start())]
                    }
                };
                assert_eq!(advice.starts_with("then run env"), shared, "{advice}");
                for (shell, command) in commands {
                    let mut run = if shell == "sh" {
                        Command::new("/bin/sh")
                    } else {
                        let mut nu = Command::new("nu");
                        nu.arg("--no-config-file");
                        nu
                    };
                    let output = run
                        .args(["-c", command])
                        .env("EON_WINDOW_RUNTIME_DIR", base.join("elsewhere"))
                        .stdin(Stdio::null())
                        .output()
                        .unwrap();
                    assert!(output.status.success(), "{shell}: {command}: {output:?}");
                    let expected = namespace
                        .map_or_else(|| "unset".to_owned(), |root| root.display().to_string());
                    assert_eq!(
                        fs::read_to_string(&log).unwrap(),
                        format!("stop g1-x|{expected}\n"),
                        "{shell}: {command}"
                    );
                    fs::remove_file(&log).unwrap();
                }
            }
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn delegated_result_accepts_one_stop_result_only() {
        use std::os::unix::process::ExitStatusExt;
        let exited = |code: i32| std::process::ExitStatus::from_raw(code << 8);
        let killed = std::process::ExitStatus::from_raw(libc::SIGKILL);
        for output in [
            b"{\"stopped\":{\"generation\":\"g1\",\"sessions\":[\"session-1\"]}}\n".as_slice(),
            b"{\"stopped\":{\"generation\":\"g1\",\"sessions\":[\"session-1\"],\"hint\":1}}",
        ] {
            let stopped = super::delegated_result(output, exited(0), "g1");
            assert!(
                matches!(&stopped, Some(Ok(stopped)) if stopped.sessions == ["session-1"]),
                "{stopped:?}"
            );
        }
        let failed = super::delegated_result(
            b"{\"error\":{\"code\":\"supervisor-unavailable\",\"detail\":\"d\"}}",
            exited(4),
            "g1",
        );
        assert!(
            matches!(&failed, Some(Err(failure)) if failure.code == "supervisor-unavailable" && failure.detail == "d"),
            "{failed:?}"
        );
        let stopped = b"{\"stopped\":{\"generation\":\"g1\",\"sessions\":[]}}".as_slice();
        let error = b"{\"error\":{\"code\":\"stop-failed\",\"detail\":\"d\"}}".as_slice();
        for (output, status) in [
            (stopped, exited(1)),
            (stopped, killed),
            (error, killed),
            (b"{\"stopped\":{\"generation\":\"g2\",\"sessions\":[]}}".as_slice(), exited(0)),
            (b"{\"stopped\":{\"generation\":\"g1\"}}", exited(0)),
            (b"{\"stopped\":{\"generation\":\"g1\",\"sessions\":[]},\"error\":{\"code\":\"c\",\"detail\":\"d\"}}", exited(0)),
            (b"{\"stopped\":{\"generation\":\"g1\",\"sessions\":[]}}{\"stopped\":{\"generation\":\"g1\",\"sessions\":[]}}", exited(0)),
            (b"[{\"stopped\":{\"generation\":\"g1\",\"sessions\":[]}}]", exited(0)),
            (b"{\"error\":{\"code\":\"\",\"detail\":\"d\"}}", exited(1)),
            (b"", exited(0)),
            (b"stopped generation g1: session-1\n", exited(0)),
        ] {
            assert!(
                super::delegated_result(output, status, "g1").is_none(),
                "{} {status}",
                String::from_utf8_lossy(output)
            );
        }
    }

    #[test]
    fn confirmation_requires_yes_and_only_terminal_declines_succeed() {
        for terminal in [false, true] {
            for answer in [b"y".as_slice(), b"Y", b"yes", b"YES", b" \tyes\r\n"] {
                assert_eq!(
                    confirmation_status(answer, terminal),
                    Confirmation::Confirmed
                );
            }
            for answer in [
                b"".as_slice(),
                b"n",
                b"no",
                b"Yes",
                b" yes please ",
                b"\xff",
                b"y\xff",
            ] {
                assert_eq!(
                    confirmation_status(answer, terminal),
                    if terminal {
                        Confirmation::Cancelled
                    } else {
                        Confirmation::Refused
                    }
                );
            }
        }
    }

    #[test]
    fn generation_identity_is_stable_bounded_and_namespaced() {
        let first = generation_id(b"abc");
        assert_eq!(first, generation_id(b"abc"));
        assert_ne!(first, generation_id(b"abd"));
        assert!(valid_generation(&first));
        assert!(!valid_generation("legacy"));
        assert!(!valid_generation("g1-0123456789ABCDEF0123456789ABCDEF"));
        assert_eq!(
            generation_directory(Path::new("/runtime/eon"), &first),
            Path::new("/runtime/eon/generations").join(first)
        );
    }

    #[test]
    fn current_generation_consumes_supplied_assembly() {
        let mut inputs = crate::fixtures::inputs();
        let first = current_generation(&inputs).unwrap();
        assert_eq!(first, current_generation(&inputs).unwrap());
        inputs.assembly = b"changed-assembly";
        assert_ne!(first, current_generation(&inputs).unwrap());
    }

    #[test]
    fn discovery_bounds_dead_cleanup_and_rejects_unsafe_metadata() {
        let root = temporary_directory();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let generations = root.join("generations");
        fs::create_dir(&generations).unwrap();
        fs::set_permissions(&generations, fs::Permissions::from_mode(0o700)).unwrap();

        let outside = temporary_directory();
        let linked_id = "g1-00000000000000000000000000000000";
        symlink(&outside, generations.join(linked_id)).unwrap();

        let dead_id = "g1-11111111111111111111111111111111";
        let dead = generations.join(dead_id);
        fs::create_dir(&dead).unwrap();
        fs::set_permissions(&dead, fs::Permissions::from_mode(0o700)).unwrap();
        let listener = UnixListener::bind(dead.join("eon.sock")).unwrap();
        fs::set_permissions(dead.join("eon.sock"), fs::Permissions::from_mode(0o600)).unwrap();
        drop(listener);
        // Until it execs, a child forked by a concurrent test shares this
        // test's open files: the dropped listener can briefly keep accepting.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !matches!(
            UnixStream::connect(dead.join("eon.sock")),
            Err(error) if error.kind() == ErrorKind::ConnectionRefused
        ) {
            assert!(Instant::now() < deadline, "dead socket kept accepting");
            thread::sleep(Duration::from_millis(1));
        }

        let held = lock_supervisor_lifecycle(&supervisor_lock_path(&root, dead_id)).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let inspection_root = root.clone();
        let inspection = thread::spawn(move || {
            sender
                .send(discover_generations(
                    &crate::fixtures::inputs(),
                    &inspection_root,
                    &current_generation(&crate::fixtures::inputs()).unwrap(),
                ))
                .unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));
        let preserved = dead.exists();
        // Unlock rather than only drop: a child forked meanwhile shares the
        // lock until it execs, and only an explicit unlock releases it there.
        held.unlock().unwrap();
        drop(held);
        inspection.join().unwrap();

        let records = result
            .expect("generation inspection waited for startup")
            .unwrap();
        assert!(records.iter().any(|record| {
            record.id == linked_id && record.kind == "previous" && record.state == "corrupt"
        }));
        assert!(records.iter().any(|record| {
            record.id == dead_id && record.kind == "previous" && record.state == "dead"
        }));
        assert!(
            preserved,
            "contended inspection removed dead control metadata"
        );

        discover_generations(
            &crate::fixtures::inputs(),
            &root,
            &current_generation(&crate::fixtures::inputs()).unwrap(),
        )
        .unwrap();
        assert!(!dead.exists());
        assert!(outside.is_dir());

        fs::remove_file(generations.join(linked_id)).unwrap();
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
