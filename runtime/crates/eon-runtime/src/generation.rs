use super::cli::Inputs;
use super::control::{
    CONTROL_TIMEOUT, ControlResponse, EndpointFailure, EndpointFailureKind, Error, connect_control,
    failure, probe_generation_runtime, probe_generation_runtime_until, remove_socket_if_identity,
    report_failure, send_action_from_until, send_action_on, socket_identity, socket_identity_from,
    write_stdout,
};
use super::supervisor::{
    effective_uid, present_at, probe_supervisor, runtime_directory, supervisor_lock_path,
    try_lock_supervisor_lifecycle, validate_private_directory,
};
use super::workspace::json_escape;
use eon_workspace_protocol::v22::{
    Action, Availability, Failure, LifecycleResponse, Response, Stopped, WorkspaceAction,
};
use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, symlink},
        net::UnixStream,
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
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

/// The link in a generation's directory to the `eon` of the build that started
/// it, which a later build runs to present a generation it cannot present.
const LAUNCHER: &str = "eon";
/// Set on a delegated attach to the generation it presents.
const DELEGATED: &str = "EON_ATTACH_DELEGATED";

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
struct GenerationRecord {
    id: String,
    kind: &'static str,
    state: &'static str,
    runtime: PathBuf,
    eon_version: Option<String>,
    workspace_protocol: Option<u16>,
    component_report: Option<String>,
    sessions: Vec<String>,
    attach: Availability,
    stop: Availability,
    detail: String,
    /// The recorded `eon` of the other build that started it, which presents
    /// it when this build cannot.
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
            ));
        }
    }
    Ok(records)
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
    )))
}

fn inspect_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
    lifecycle_lock: &Path,
    current_components: &str,
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
    let mut record = match probe_generation_runtime(&socket) {
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
            let other_build = info.component_report != current_components && info.attach.available;
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
                eon_version: Some(info.eon_version),
                workspace_protocol: Some(info.workspace_protocol),
                component_report: Some(info.component_report),
                sessions: info.sessions,
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
            if record.state == "incompatible" {
                record.launcher = recorded_launcher(&runtime);
            }
            if dead && let Ok(Some(_lifecycle_lock)) = try_lock_supervisor_lifecycle(lifecycle_lock)
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
        eon_version: None,
        workspace_protocol: None,
        component_report: None,
        sessions: Vec::new(),
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
            record.stop.reason.escape_debug(),
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
        for session in &record.sessions {
            output.push_str(&format!("  terminal {session}\n"));
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
            "{{\"id\":\"{}\",\"kind\":\"{}\",\"state\":\"{}\",\"eon_version\":{},\"workspace_protocol\":{},\"component_report\":{},\"executable\":{},\"sessions\":[",
            json_escape(&record.id),
            record.kind,
            record.state,
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
        output.push_str(&format!(
            "],\"attach\":{{\"available\":{},\"reason\":\"{}\"}},\"stop\":{{\"available\":{},\"reason\":\"{}\"}},\"detail\":\"{}\"}}",
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
fn running_generations(inputs: &Inputs, root: &Path) -> Result<Vec<GenerationRecord>, String> {
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

pub(super) fn stop_generation(
    inputs: &Inputs,
    target: &str,
    json: bool,
    product: &str,
) -> Result<i32, String> {
    let root = runtime_directory(product);
    let current = current_generation(inputs)?;
    let control = generation_directory(&root, target).join("eon.sock");
    let observed_supervisor = socket_identity(&control);
    let Some(record) = inspect_selected_generation(inputs, &root, &current, target)? else {
        return report_failure(
            &failure(
                "unknown-generation",
                format!("generation {target} was not found"),
            ),
            json,
        );
    };
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
    if !json {
        eprint!(
            "Stop generation {target} and {} live terminal{} [{}]? [y/N] ",
            record.sessions.len(),
            if record.sessions.len() == 1 { "" } else { "s" },
            record.sessions.join(", ")
        );
        let answer = read_stop_confirmation()?;
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
            write_stdout(format!("cancelled; no Stop sent for generation {target}\n"))?;
            return Ok(0);
        }
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
            if json {
                write_stdout(stopped_json(&stopped))?;
            } else {
                write_stdout(format!(
                    "stopped generation {}: {}\n",
                    stopped.generation,
                    stopped.sessions.join(", ")
                ))?;
            }
            Ok(0)
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

pub(super) fn read_stop_confirmation() -> Result<String, String> {
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
    String::from_utf8(answer)
        .map_err(|_| "cannot read stop confirmation: stream did not contain valid UTF-8".into())
}

pub(super) fn stop_generations(
    inputs: &Inputs,
    include_current: bool,
    json: bool,
    product: &str,
) -> Result<i32, String> {
    let mut records = discover_generations(
        inputs,
        &runtime_directory(product),
        &current_generation(inputs)?,
    )?;
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
        let result = match stop_generation(inputs, &record.id, json, product) {
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
        current_generation, discover_generations, generation_directory, generation_id,
        valid_generation,
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
