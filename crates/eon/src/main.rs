mod workspace;

use eon_workspace_protocol::{
    Action, Availability, Direction, Error as ProtocolError, Failure, HEADER_BYTES,
    LifecycleResponse, MAX_DETAIL_BYTES, Request, Response, Runtime, Stopped, VERSION,
    declared_message_len, decode_lifecycle_response, decode_request, decode_response,
    encode_lifecycle_response, encode_request, encode_response,
};
use serde::Deserialize;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use workspace::{
    Workspace, failure_human, failure_json, human as human_output, json as json_output, json_escape,
};

const MANIFEST: &str = include_str!("../../../components/eon-alpha-v1.json");
const USAGE: &str = "usage: eon [run [-- COMMAND...]] | terminal -- COMMAND... | attach [GENERATION] | generations [--json] | stop GENERATION [--json] | workspace [--json] | tab create [--json] | pane create [--json] | focus <ID|left|right|up|down> [--json] | versions | config-path";
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(0);

fn current_generation() -> String {
    generation_id(&[
        include_bytes!("main.rs"),
        include_bytes!("workspace.rs"),
        include_bytes!("../../eon-workspace-protocol/src/lib.rs"),
        include_bytes!("../../eon-workspace-protocol/Cargo.toml"),
        include_bytes!("../../eon-manifest/src/lib.rs"),
        include_bytes!("../../eon-manifest/Cargo.toml"),
        include_bytes!("../Cargo.toml"),
        include_bytes!("../../../Cargo.toml"),
        include_bytes!("../../../Cargo.lock"),
        MANIFEST.as_bytes(),
    ])
}

fn generation_id(inputs: &[&[u8]]) -> String {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;
    let mut hash = OFFSET;
    for input in inputs {
        for byte in (input.len() as u64).to_le_bytes().iter().chain(*input) {
            hash ^= u128::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
    }
    format!("g1-{hash:032x}")
}

fn valid_generation(value: &str) -> bool {
    value.len() == 35
        && value.starts_with("g1-")
        && value[3..]
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn generation_directory(root: &Path, generation: &str) -> PathBuf {
    root.join("generations").join(generation)
}

const MAX_GENERATIONS: usize = 256;
const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EndpointFailureKind {
    Dead,
    Incompatible,
    Unreachable,
    Corrupt,
}

#[derive(Debug)]
struct EndpointFailure {
    kind: EndpointFailureKind,
    detail: String,
}

impl EndpointFailure {
    fn new(kind: EndpointFailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
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
}

enum ControlResponse {
    Workspace(Response),
    Lifecycle(LifecycleResponse),
}

fn send_action(socket: &Path, action: Action) -> Result<ControlResponse, EndpointFailure> {
    let metadata = match fs::symlink_metadata(socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(EndpointFailure::new(
                EndpointFailureKind::Dead,
                format!("endpoint {} is missing", socket.display()),
            ));
        }
        Err(error) => {
            return Err(EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                format!("cannot inspect endpoint {}: {error}", socket.display()),
            ));
        }
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            format!(
                "endpoint {} must be an owned mode-0600 Unix socket",
                socket.display()
            ),
        ));
    }

    let mut stream = UnixStream::connect(socket).map_err(|error| {
        let kind = match error.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                EndpointFailureKind::Dead
            }
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                EndpointFailureKind::Unreachable
            }
            _ => EndpointFailureKind::Corrupt,
        };
        EndpointFailure::new(
            kind,
            format!(
                "cannot connect to supervisor at {}: {error}",
                socket.display()
            ),
        )
    })?;
    stream
        .set_read_timeout(Some(CONTROL_TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(CONTROL_TIMEOUT)))
        .map_err(|error| {
            EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                format!("cannot configure supervisor connection: {error}"),
            )
        })?;
    let lifecycle = matches!(action, Action::InspectRuntime | Action::Stop { .. });
    let request = encode_request(&Request {
        id: request_id(),
        action,
    })
    .map_err(|error| {
        EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            format!("cannot encode Eon action: {error}"),
        )
    })?;
    stream
        .write_all(&request)
        .map_err(|error| io_endpoint_failure(error, "cannot send Eon action"))?;

    let mut response = vec![0; HEADER_BYTES];
    stream
        .read_exact(&mut response)
        .map_err(|error| io_endpoint_failure(error, "cannot read Eon action result"))?;
    let length = declared_message_len(&response).map_err(protocol_endpoint_failure)?;
    response.resize(length, 0);
    stream
        .read_exact(&mut response[HEADER_BYTES..])
        .map_err(|error| io_endpoint_failure(error, "cannot read complete Eon result"))?;
    if lifecycle {
        decode_lifecycle_response(&response)
            .map(ControlResponse::Lifecycle)
            .map_err(protocol_endpoint_failure)
    } else {
        decode_response(&response)
            .map(ControlResponse::Workspace)
            .map_err(protocol_endpoint_failure)
    }
}

fn io_endpoint_failure(error: std::io::Error, context: &str) -> EndpointFailure {
    let kind = if matches!(
        error.kind(),
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
    ) {
        EndpointFailureKind::Unreachable
    } else {
        EndpointFailureKind::Corrupt
    };
    EndpointFailure::new(kind, format!("{context}: {error}"))
}

fn protocol_endpoint_failure(error: ProtocolError) -> EndpointFailure {
    EndpointFailure::new(
        if matches!(error, ProtocolError::UnsupportedVersion { .. }) {
            EndpointFailureKind::Incompatible
        } else {
            EndpointFailureKind::Corrupt
        },
        format!("invalid EONW response: {error}"),
    )
}

fn probe_runtime(socket: &Path) -> Result<Runtime, EndpointFailure> {
    match send_action(socket, Action::InspectRuntime)? {
        ControlResponse::Lifecycle(LifecycleResponse::Runtime(runtime)) => Ok(runtime),
        ControlResponse::Lifecycle(LifecycleResponse::Failure(failure))
            if matches!(
                failure.code.as_str(),
                "unsupported-version" | "malformed-action"
            ) =>
        {
            Err(EndpointFailure::new(
                EndpointFailureKind::Incompatible,
                format!(
                    "supervisor does not support generation inspection: {}",
                    failure.detail
                ),
            ))
        }
        ControlResponse::Lifecycle(LifecycleResponse::Failure(failure)) => {
            Err(EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                format!(
                    "supervisor rejected generation inspection: {}",
                    failure.detail
                ),
            ))
        }
        _ => Err(EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            "supervisor returned the wrong EONW result for generation inspection",
        )),
    }
}

fn probe_launch_mode(socket: &Path) -> Result<LaunchMode, EndpointFailure> {
    match send_action(socket, Action::Inspect)? {
        ControlResponse::Workspace(Response::Snapshot(_)) => Ok(LaunchMode::Workspace),
        ControlResponse::Workspace(Response::Failure(failure))
            if failure.code == "workspace-unavailable" =>
        {
            Ok(LaunchMode::Terminal)
        }
        ControlResponse::Workspace(Response::Failure(failure)) => Err(EndpointFailure::new(
            if failure.code == "unsupported-version" {
                EndpointFailureKind::Incompatible
            } else {
                EndpointFailureKind::Corrupt
            },
            format!(
                "supervisor rejected launch-mode inspection: {}",
                failure.detail
            ),
        )),
        ControlResponse::Lifecycle(_) => Err(EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            "supervisor returned the wrong EONW result for launch-mode inspection",
        )),
    }
}

fn discover_generations(root: &Path, current: &str) -> Result<Vec<GenerationRecord>, String> {
    let component_report =
        eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?;
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(vec![unstarted_current_generation(root, current)]);
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
            candidates.push((
                entry.file_name().as_bytes().to_vec(),
                entry.file_name().to_string_lossy().into_owned(),
                entry.path(),
            ));
            if candidates.len() > MAX_GENERATIONS {
                return Err(format!(
                    "generation directory {} exceeds the {MAX_GENERATIONS}-entry inspection limit",
                    parent.display()
                ));
            }
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0));

    let current_path = generation_directory(root, current);
    let mut records = vec![inspect_generation(
        current,
        "current",
        current_path,
        &component_report,
    )];
    for (_, id, path) in candidates {
        if id != current {
            records.push(inspect_generation(&id, "previous", path, &component_report));
        }
    }
    if path_exists(&root.join("eon.sock")) || path_exists(&root.join("orbit.sock")) {
        records.push(inspect_legacy(root));
    }
    Ok(records)
}

fn inspect_selected_generation(
    root: &Path,
    current: &str,
    target: &str,
) -> Result<Option<GenerationRecord>, String> {
    let missing_current =
        || (target == current).then(|| unstarted_current_generation(root, current));
    if !private_directory_exists(root, "runtime directory")? {
        return Ok(missing_current());
    }
    if target == "legacy" {
        return Ok(
            (path_exists(&root.join("eon.sock")) || path_exists(&root.join("orbit.sock")))
                .then(|| inspect_legacy(root)),
        );
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
    let component_report =
        eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?;
    Ok(Some(inspect_generation(
        target,
        if target == current {
            "current"
        } else {
            "previous"
        },
        runtime,
        &component_report,
    )))
}

fn inspect_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
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
    match probe_runtime(&socket) {
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
            if info.workspace_protocol != VERSION {
                return failed_generation(
                    id,
                    kind,
                    runtime,
                    EndpointFailure::new(
                        EndpointFailureKind::Incompatible,
                        format!(
                            "supervisor reports EONW {}, current Eon requires EONW {VERSION}",
                            info.workspace_protocol
                        ),
                    ),
                );
            }
            if info.component_report != current_components {
                info.attach = Availability {
                    available: false,
                    reason: "component graph differs from the current Eon build".into(),
                };
            }
            GenerationRecord {
                id: id.into(),
                kind,
                state: "live",
                runtime,
                eon_version: Some(info.eon_version),
                workspace_protocol: Some(info.workspace_protocol),
                component_report: Some(info.component_report),
                sessions: info.sessions,
                attach: info.attach,
                stop: info.stop,
                detail: "live supervisor validated its generation identity".into(),
            }
        }
        Err(error) => {
            let dead = error.kind == EndpointFailureKind::Dead;
            let record = failed_generation(id, kind, runtime.clone(), error);
            if dead {
                remove_dead_socket(&socket);
                let _ = fs::remove_dir(&runtime);
            }
            record
        }
    }
}

fn inspect_legacy(root: &Path) -> GenerationRecord {
    let runtime = root.to_path_buf();
    let socket = root.join("eon.sock");
    match send_action(&socket, Action::Inspect) {
        Ok(ControlResponse::Workspace(Response::Snapshot(snapshot))) => GenerationRecord {
            id: "legacy".into(),
            kind: "legacy",
            state: "live",
            runtime,
            eon_version: None,
            workspace_protocol: Some(VERSION),
            component_report: None,
            sessions: snapshot
                .tabs
                .iter()
                .flat_map(|tab| tab.panes.iter().map(|pane| pane.session.clone()))
                .collect(),
            attach: Availability {
                available: true,
                reason: "legacy supervisor returned a valid EONW v1 workspace".into(),
            },
            stop: Availability {
                available: false,
                reason: "legacy supervisor has no authoritative stop action".into(),
            },
            detail: "live fixed-namespace supervisor; component identity unavailable".into(),
        },
        Ok(ControlResponse::Workspace(Response::Failure(failure))) => failed_generation(
            "legacy",
            "legacy",
            runtime,
            EndpointFailure::new(
                if failure.code == "unsupported-version" {
                    EndpointFailureKind::Incompatible
                } else {
                    EndpointFailureKind::Corrupt
                },
                format!(
                    "legacy supervisor rejected EONW inspection: {}",
                    failure.detail
                ),
            ),
        ),
        Ok(_) => failed_generation(
            "legacy",
            "legacy",
            runtime,
            EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                "legacy supervisor returned the wrong EONW result",
            ),
        ),
        Err(error) => {
            let dead = error.kind == EndpointFailureKind::Dead;
            let record = failed_generation("legacy", "legacy", runtime, error);
            if dead {
                remove_dead_socket(&socket);
            }
            record
        }
    }
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
        EndpointFailureKind::Unreachable => "unreachable",
        EndpointFailureKind::Corrupt => "corrupt",
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
    }
}

fn validate_private_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect directory {}: {error}", path.display()))?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(format!(
            "directory {} must be an owned private directory",
            path.display()
        ));
    }
    Ok(())
}

fn private_directory_exists(path: &Path, description: &str) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Ok(_) => validate_private_directory(path).map(|()| true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "cannot inspect {description} {}: {error}",
            path.display()
        )),
    }
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn remove_dead_socket(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if !metadata.file_type().is_socket() || metadata.uid() != effective_uid() {
        return;
    }
    if matches!(
        UnixStream::connect(path),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            )
    ) {
        let _ = fs::remove_file(path);
    }
}

fn write_stdout(output: impl AsRef<[u8]>) -> Result<(), String> {
    match std::io::stdout().lock().write_all(output.as_ref()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(format!("cannot write stdout: {error}")),
    }
}

fn generations_command(arguments: &[OsString]) -> Result<i32, String> {
    let json = match arguments {
        [] => false,
        [flag] if flag == "--json" => true,
        _ => return Err(USAGE.into()),
    };
    let records = discover_generations(&runtime_directory(), &current_generation())?;
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
            "{} {} {} sessions={}\n",
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
        for session in &record.sessions {
            output.push_str(&format!("  session {session}\n"));
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
            "{{\"id\":\"{}\",\"kind\":\"{}\",\"state\":\"{}\",\"eon_version\":{},\"workspace_protocol\":{},\"component_report\":{},\"sessions\":[",
            json_escape(&record.id),
            record.kind,
            record.state,
            json_option(record.eon_version.as_deref()),
            record.workspace_protocol.map_or_else(|| "null".into(), |value| value.to_string()),
            json_option(record.component_report.as_deref()),
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

fn attach_generation(target: &str) -> Result<i32, String> {
    let root = runtime_directory();
    let current = current_generation();
    let record = inspect_selected_generation(&root, &current, target)?
        .ok_or_else(|| format!("generation {target} was not found"))?;
    if !record.attach.available {
        return Err(format!(
            "generation {target} is not attachable: {}",
            record.attach.reason
        ));
    }
    let mode = if target == "legacy" {
        LaunchMode::Workspace
    } else {
        probe_launch_mode(&record.runtime.join("eon.sock")).map_err(|error| error.detail)?
    };
    attach_at(&record.runtime, mode)
}

fn stop_generation(target: &str, json: bool) -> Result<i32, String> {
    let root = runtime_directory();
    let current = current_generation();
    let record = match inspect_selected_generation(&root, &current, target)? {
        Some(record) => record,
        None => {
            return report_failure(
                &failure(
                    "unknown-generation",
                    format!("generation {target} was not found"),
                ),
                json,
            );
        }
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
    if !json {
        eprint!(
            "Stop generation {target} and {} live Session{} [{}]? [y/N] ",
            record.sessions.len(),
            if record.sessions.len() == 1 { "" } else { "s" },
            record.sessions.join(", ")
        );
        let mut answer = String::new();
        std::io::stdin()
            .read_line(&mut answer)
            .map_err(|error| format!("cannot read stop confirmation: {error}"))?;
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "YES") {
            write_stdout("cancelled; no Sessions stopped\n")?;
            return Ok(0);
        }
    }

    let response = match send_action(
        &record.runtime.join("eon.sock"),
        Action::Stop {
            generation: target.into(),
        },
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

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct EonConfig {
    shell: ShellConfig,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
struct ShellConfig {
    command: Vec<String>,
    starship: bool,
    zoxide: bool,
    atuin: bool,
    carapace: bool,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            command: vec!["eon-nu".into()],
            starship: true,
            zoxide: true,
            atuin: true,
            carapace: true,
        }
    }
}

struct Programs {
    orbit: PathBuf,
    venus: PathBuf,
    session_bin: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaunchMode {
    Workspace,
    Terminal,
}

impl LaunchMode {
    fn name(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Terminal => "terminal",
        }
    }
}

struct ManagedPrograms {
    nu: PathBuf,
    bash: PathBuf,
    zsh: PathBuf,
    fish: PathBuf,
    helix: PathBuf,
    yazi: PathBuf,
    ya: PathBuf,
    lazygit: PathBuf,
    nu_vendor_autoload: Option<PathBuf>,
    bash_rc: Option<PathBuf>,
    zsh_config: Option<PathBuf>,
    fish_init: Option<PathBuf>,
    shell_bin: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ManagedTool {
    Nu,
    Bash,
    Zsh,
    Fish,
    Helix,
    Yazi,
    Ya,
    LazyGit,
}

impl ManagedTool {
    fn is_shell(self) -> bool {
        matches!(self, Self::Nu | Self::Bash | Self::Zsh | Self::Fish)
    }
}

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let invocation = arguments.next().unwrap_or_default();
    let arguments = arguments.collect();
    let result = match managed_tool(&invocation) {
        Some(tool) => launch_managed(tool, arguments),
        None => execute(arguments),
    };
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("eon: {error}");
            ExitCode::FAILURE
        }
    }
}

fn managed_tool(invocation: &OsStr) -> Option<ManagedTool> {
    let name = Path::new(invocation).file_name()?.to_str()?;
    match name {
        "eon-nu" | "nu" => Some(ManagedTool::Nu),
        "eon-bash" | "bash" => Some(ManagedTool::Bash),
        "eon-zsh" | "zsh" => Some(ManagedTool::Zsh),
        "eon-fish" | "fish" => Some(ManagedTool::Fish),
        "eon-hx" | "hx" => Some(ManagedTool::Helix),
        "eon-yazi" | "yazi" => Some(ManagedTool::Yazi),
        "eon-ya" | "ya" => Some(ManagedTool::Ya),
        "eon-lazygit" | "eon-lg" | "lazygit" => Some(ManagedTool::LazyGit),
        _ => None,
    }
}

fn launch_managed(tool: ManagedTool, arguments: Vec<OsString>) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let programs = managed_programs();
    let mut command = managed_command(tool, &programs, &config, &arguments)?;
    let program = command.get_program().to_string_lossy().into_owned();
    let error = command.exec();
    Err(format!("cannot launch {program}: {error}"))
}

fn managed_command(
    tool: ManagedTool,
    programs: &ManagedPrograms,
    config: &Path,
    arguments: &[OsString],
) -> Result<Command, String> {
    let shell = tool
        .is_shell()
        .then(|| read_shell_config(config))
        .transpose()?;
    let program = match tool {
        ManagedTool::Nu => &programs.nu,
        ManagedTool::Bash => &programs.bash,
        ManagedTool::Zsh => &programs.zsh,
        ManagedTool::Fish => &programs.fish,
        ManagedTool::Helix => &programs.helix,
        ManagedTool::Yazi => &programs.yazi,
        ManagedTool::Ya => &programs.ya,
        ManagedTool::LazyGit => &programs.lazygit,
    };
    let mut command = Command::new(program);
    match tool {
        ManagedTool::Nu => {
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
        ManagedTool::Bash => {
            if let Some(path) = &programs.bash_rc {
                command.args([OsStr::new("--rcfile"), path.as_os_str()]);
            }
        }
        ManagedTool::Zsh => {
            if let Some(path) = &programs.zsh_config {
                command.env("ZDOTDIR", path);
                if let Some(user) = env::var_os("EON_USER_ZDOTDIR")
                    .or_else(|| env::var_os("ZDOTDIR"))
                    .or_else(|| env::var_os("HOME"))
                {
                    command.env("EON_USER_ZDOTDIR", user);
                }
            }
        }
        ManagedTool::Fish => {
            if let Some(path) = &programs.fish_init {
                command
                    .env("EON_FISH_INIT", path)
                    .args(["-C", "source \"$EON_FISH_INIT\""]);
            }
        }
        ManagedTool::Yazi | ManagedTool::Ya => {
            command.env_remove("YAZI_CONFIG_HOME");
        }
        ManagedTool::LazyGit => {
            command
                .env_remove("CONFIG_DIR")
                .env_remove("LG_CONFIG_FILE")
                .env("XDG_CONFIG_DIRS", config);
        }
        ManagedTool::Helix => {
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
    if !tool.is_shell() {
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

fn execute(arguments: Vec<OsString>) -> Result<i32, String> {
    match arguments.as_slice() {
        [] => launch_current(LaunchMode::Workspace, &[], true),
        [command] if command == "run" => launch_current(LaunchMode::Workspace, &[], false),
        [command, separator, child @ ..]
            if command == "run" && separator == "--" && !child.is_empty() =>
        {
            launch_current(LaunchMode::Workspace, child, false)
        }
        [command, separator, child @ ..]
            if command == "terminal" && separator == "--" && !child.is_empty() =>
        {
            launch_current(LaunchMode::Terminal, child, true)
        }
        [command] if command == "attach" => attach_generation(&current_generation()),
        [command, generation] if command == "attach" => {
            attach_generation(generation_argument(generation)?)
        }
        [command, rest @ ..] if command == "generations" => generations_command(rest),
        [command, generation] if command == "stop" => {
            stop_generation(generation_argument(generation)?, false)
        }
        [command, generation, flag] if command == "stop" && flag == "--json" => {
            stop_generation(generation_argument(generation)?, true)
        }
        [command, flag, generation] if command == "stop" && flag == "--json" => {
            stop_generation(generation_argument(generation)?, true)
        }
        [command, ..]
            if command == "workspace"
                || command == "tab"
                || command == "pane"
                || command == "focus" =>
        {
            control(&arguments)
        }
        [command] if command == "versions" => {
            write_stdout(format!(
                "eon {} {}\neonw {}\n{}\n",
                env!("CARGO_PKG_VERSION"),
                current_generation(),
                VERSION,
                eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?
            ))?;
            Ok(0)
        }
        [command] if command == "config-path" => {
            let path = configuration_directory()?;
            prepare_configuration(&path)?;
            write_stdout(format!("{}\n", path.display()))?;
            Ok(0)
        }
        _ => Err(USAGE.into()),
    }
}

fn generation_argument(argument: &OsStr) -> Result<&str, String> {
    let generation = argument
        .to_str()
        .ok_or_else(|| "Eon generation identities must be UTF-8".to_string())?;
    if generation != "legacy" && !valid_generation(generation) {
        return Err(format!("invalid Eon generation identity {generation:?}"));
    }
    Ok(generation)
}

fn launch_current(
    mode: LaunchMode,
    child: &[OsString],
    attach_existing: bool,
) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let root = runtime_directory();
    let generation = current_generation();
    let runtime = prepare_generation_runtime(&root, &generation)?;
    let socket = runtime.join("eon.sock");
    match probe_runtime(&socket) {
        Ok(info) => {
            validate_current_runtime(&info, &generation)?;
            let active_mode = probe_launch_mode(&socket).map_err(|error| error.detail)?;
            if active_mode != mode {
                return Err(format!(
                    "generation {generation} already has a live {} mode; requested {} mode",
                    active_mode.name(),
                    mode.name()
                ));
            }
            return if attach_existing {
                attach_at(&runtime, mode)
            } else {
                Err(format!(
                    "generation {generation} already has a live Eon supervisor"
                ))
            };
        }
        Err(error) if error.kind == EndpointFailureKind::Dead => {}
        Err(error) => return Err(error.detail),
    }
    match supervise(
        &programs(),
        &config,
        &runtime.join("orbit.sock"),
        child,
        Duration::from_secs(5),
        &generation,
        mode,
    ) {
        Ok(code) => Ok(code),
        Err(error) => {
            if attach_existing && path_exists(&socket) {
                attach_competing_supervisor(&socket, &runtime, &generation, mode, error)
            } else {
                Err(error)
            }
        }
    }
}

fn attach_competing_supervisor(
    socket: &Path,
    runtime: &Path,
    generation: &str,
    mode: LaunchMode,
    launch_error: String,
) -> Result<i32, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match probe_runtime(socket) {
            Ok(info) => {
                validate_current_runtime(&info, generation)?;
                let active_mode = probe_launch_mode(socket).map_err(|error| error.detail)?;
                if active_mode != mode {
                    return Err(format!(
                        "{launch_error}; competing supervisor started in {} mode, requested {} mode",
                        active_mode.name(),
                        mode.name()
                    ));
                }
                return attach_at(runtime, mode);
            }
            Err(error)
                if matches!(
                    error.kind,
                    EndpointFailureKind::Dead | EndpointFailureKind::Unreachable
                ) && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                return Err(format!(
                    "{launch_error}; competing supervisor did not become attachable: {}",
                    error.detail
                ));
            }
        }
    }
}

fn validate_current_runtime(info: &Runtime, generation: &str) -> Result<(), String> {
    if info.generation != generation {
        return Err(format!(
            "current runtime directory contains generation {}, expected {generation}",
            info.generation
        ));
    }
    if info.workspace_protocol != VERSION {
        return Err(format!(
            "current supervisor uses EONW {}, expected EONW {VERSION}",
            info.workspace_protocol
        ));
    }
    let components = eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?;
    if info.component_report != components {
        return Err("current supervisor reports a different component graph".into());
    }
    Ok(())
}

fn control(arguments: &[OsString]) -> Result<i32, String> {
    let (action, json) = parse_control_arguments(arguments)?;
    let root = runtime_directory();
    let runtime = prepare_generation_runtime(&root, &current_generation())?;
    let socket = runtime.join("eon.sock");
    let response = match send_action(&socket, action) {
        Ok(response) => response,
        Err(error) => {
            let code = if error.kind == EndpointFailureKind::Dead {
                "missing-supervisor"
            } else {
                "supervisor-unavailable"
            };
            return report_failure(
                &failure(code, format!("{}; run `eon` first", error.detail)),
                json,
            );
        }
    };
    match response {
        ControlResponse::Workspace(Response::Snapshot(snapshot)) => {
            write_stdout(if json {
                json_output(&snapshot)
            } else {
                human_output(&snapshot)
            })?;
            Ok(0)
        }
        ControlResponse::Workspace(Response::Failure(failure)) => report_failure(&failure, json),
        ControlResponse::Lifecycle(_) => {
            Err("Eon supervisor returned a lifecycle result for a workspace action".into())
        }
    }
}

fn parse_control_arguments(arguments: &[OsString]) -> Result<(Action, bool), String> {
    let mut json = false;
    let mut values = Vec::new();
    for argument in arguments {
        let argument = argument
            .to_str()
            .ok_or_else(|| "Eon workspace actions require UTF-8 arguments".to_string())?;
        if argument == "--json" {
            if json {
                return Err(USAGE.into());
            }
            json = true;
        } else {
            values.push(argument);
        }
    }
    let action = match values.as_slice() {
        ["workspace"] => Action::Inspect,
        ["tab", "create"] => Action::CreateTab,
        ["pane", "create"] => Action::CreatePane,
        ["focus", "left"] => Action::Focus(Direction::Left),
        ["focus", "right"] => Action::Focus(Direction::Right),
        ["focus", "up"] => Action::Focus(Direction::Up),
        ["focus", "down"] => Action::Focus(Direction::Down),
        ["focus", id] => Action::FocusId((*id).into()),
        _ => return Err(USAGE.into()),
    };
    Ok((action, json))
}

fn request_id() -> String {
    let sequence = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}-{sequence}", std::process::id())
}

fn attach_at(runtime: &Path, mode: LaunchMode) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    venus_command(&programs(), &config, &runtime.join("orbit.sock"), mode)
        .status()
        .map(status_code)
        .map_err(|error| format!("cannot launch Eon Desktop: {error}"))
}

fn venus_command(programs: &Programs, config: &Path, socket: &Path, mode: LaunchMode) -> Command {
    let mut command = Command::new(&programs.venus);
    command.arg(socket);
    if mode == LaunchMode::Workspace {
        command.arg(socket.with_file_name("eon.sock"));
    }
    command.env("XDG_CONFIG_HOME", config);
    command
}

fn programs() -> Programs {
    Programs {
        orbit: configured_program("EON_ORBIT", "yazelix-orbit"),
        venus: configured_program("EON_VENUS", "yazelix-venus"),
        session_bin: nonempty_environment_path("EON_SESSION_BIN"),
    }
}

fn managed_programs() -> ManagedPrograms {
    ManagedPrograms {
        nu: configured_program("EON_NU", "nu"),
        bash: configured_program("EON_BASH", "bash"),
        zsh: configured_program("EON_ZSH", "zsh"),
        fish: configured_program("EON_FISH", "fish"),
        helix: configured_program("EON_HX", "hx"),
        yazi: configured_program("EON_YAZI", "yazi"),
        ya: configured_program("EON_YA", "ya"),
        lazygit: configured_program("EON_LAZYGIT", "lazygit"),
        nu_vendor_autoload: nonempty_environment_path("EON_NU_VENDOR_AUTOLOAD"),
        bash_rc: nonempty_environment_path("EON_BASH_RC"),
        zsh_config: nonempty_environment_path("EON_ZSH_CONFIG"),
        fish_init: nonempty_environment_path("EON_FISH_INIT"),
        shell_bin: nonempty_environment_path("EON_SESSION_BIN"),
    }
}

fn configured_program(variable: &str, fallback: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| fallback.into())
}

fn configuration_directory() -> Result<PathBuf, String> {
    let path = nonempty_environment_path("EON_CONFIG_HOME")
        .or_else(|| {
            xdg_path(nonempty_environment_path("XDG_CONFIG_HOME")).map(|path| path.join("eon"))
        })
        .or_else(|| nonempty_environment_path("HOME").map(|path| path.join(".config/eon")))
        .ok_or_else(|| "HOME, XDG_CONFIG_HOME, and EON_CONFIG_HOME are unset".to_string())?;
    std::path::absolute(path)
        .map_err(|error| format!("cannot resolve Eon configuration root: {error}"))
}

fn prepare_configuration(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| format!("cannot create {}: {error}", path.display()))
}

fn read_shell_config(root: &Path) -> Result<ShellConfig, String> {
    let path = root.join("config.toml");
    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ShellConfig::default());
        }
        Err(error) => {
            return Err(format!("cannot read {}: {error}", path.display()));
        }
    };
    let config: EonConfig = toml::from_str(&source)
        .map_err(|error| format!("invalid Eon configuration {}: {error}", path.display()))?;
    if config.shell.command.is_empty() || config.shell.command[0].is_empty() {
        return Err("shell.command must not be empty".into());
    }
    if config
        .shell
        .command
        .iter()
        .any(|argument| argument.contains('\0'))
    {
        return Err("shell.command must not contain NUL".into());
    }
    Ok(config.shell)
}

fn runtime_directory() -> PathBuf {
    nonempty_environment_path("EON_RUNTIME_DIR")
        .or_else(|| {
            xdg_path(nonempty_environment_path("XDG_RUNTIME_DIR")).map(|path| path.join("eon"))
        })
        .unwrap_or_else(|| {
            eprintln!(
                "eon: warning: XDG_RUNTIME_DIR is unset; using a private temporary runtime root"
            );
            env::temp_dir().join(format!("eon-{}", effective_uid()))
        })
}

fn prepare_generation_runtime(root: &Path, generation: &str) -> Result<PathBuf, String> {
    prepare_runtime(root)?;
    prepare_runtime(&root.join("generations"))?;
    let runtime = generation_directory(root, generation);
    prepare_runtime(&runtime)?;
    Ok(runtime)
}

fn nonempty_environment_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn xdg_path(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|path| path.is_absolute())
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

fn prepare_runtime(path: &Path) -> Result<(), String> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|error| {
            format!(
                "cannot create runtime directory {}: {error}",
                path.display()
            )
        })?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect runtime directory {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err(format!(
            "runtime directory {} must be an owned private directory",
            path.display()
        ));
    }
    Ok(())
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and no failure state.
    unsafe { libc::geteuid() }
}

fn supervise(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    child: &[OsString],
    socket_timeout: Duration,
    generation: &str,
    mode: LaunchMode,
) -> Result<i32, String> {
    let runtime = socket
        .parent()
        .ok_or_else(|| format!("Sessions socket {} has no parent", socket.display()))?;
    let control_listener = create_control_listener(&runtime.join("eon.sock"))?;
    let mut initial = start_orbit(programs, config, socket, child, socket_timeout)?;

    let mut venus = match venus_command(programs, config, socket, mode).spawn() {
        Ok(venus) => Some(venus),
        Err(error) => {
            stop(&mut initial);
            remove_dead_socket(socket);
            drop(control_listener);
            let _ = fs::remove_dir(runtime);
            return Err(format!("cannot launch Eon Desktop: {error}"));
        }
    };
    let mut workspace = (mode == LaunchMode::Workspace)
        .then(|| Workspace::with_initial_session(runtime.to_path_buf(), socket.to_path_buf()));
    let mut sessions = vec![RunningSession {
        id: "session-1".into(),
        endpoint: socket.into(),
        child: initial,
    }];
    let mut initial_status = None;

    loop {
        let mut index = 0;
        while index < sessions.len() {
            if let Some(status) = sessions[index]
                .child
                .try_wait()
                .map_err(|error| format!("cannot observe Sessions: {error}"))?
            {
                let session = sessions.remove(index);
                let code = status_code(status);
                remove_dead_socket(&session.endpoint);
                if let Some(workspace) = &mut workspace {
                    workspace
                        .session_exited(&session.id)
                        .map_err(|error| error.detail)?;
                }
                if session.id == "session-1" {
                    initial_status = Some(code);
                }
            } else {
                index += 1;
            }
        }

        if sessions.is_empty() {
            if let Some(mut process) = venus.take() {
                stop(&mut process);
            }
            drop(control_listener);
            let _ = fs::remove_dir(runtime);
            return initial_status.ok_or("initial Sessions exit status is unavailable".into());
        }

        let venus_exited = if let Some(process) = venus.as_mut() {
            process
                .try_wait()
                .map_err(|error| format!("cannot observe Eon Desktop: {error}"))?
                .is_some()
        } else {
            false
        };
        if venus_exited {
            venus = None;
            eprintln!(
                "Eon Desktop exited; Sessions remains active. Run `eon attach` to reconnect."
            );
        }

        if accept_control_client(
            &control_listener.listener,
            workspace.as_mut(),
            &mut sessions,
            programs,
            config,
            socket_timeout,
            generation,
        )? {
            if let Some(mut process) = venus.take() {
                stop(&mut process);
            }
            for session in &mut sessions {
                stop(&mut session.child);
                remove_dead_socket(&session.endpoint);
            }
            drop(control_listener);
            let _ = fs::remove_dir(runtime);
            return Ok(0);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

struct RunningSession {
    id: String,
    endpoint: PathBuf,
    child: Child,
}

fn start_orbit(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    child: &[OsString],
    socket_timeout: Duration,
) -> Result<Child, String> {
    let previous_socket = socket_identity(socket)?;
    let mut orbit_command = orbit_command(programs, config, socket, child)?;
    let mut orbit = orbit_command
        .spawn()
        .map_err(|error| format!("cannot launch Sessions: {error}"))?;
    if let Err(error) = wait_for_socket(&mut orbit, socket, previous_socket, socket_timeout) {
        stop(&mut orbit);
        return Err(error);
    }
    Ok(orbit)
}

fn orbit_command(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    child: &[OsString],
) -> Result<Command, String> {
    let mut orbit_command = Command::new(&programs.orbit);
    orbit_command
        .arg("serve")
        .arg(socket)
        .arg("--")
        .env("EON_CONFIG_HOME", config);
    if let Some(session_bin) = &programs.session_bin {
        orbit_command.env(
            "PATH",
            prepend_path(session_bin, env::var_os("PATH").as_deref(), "Eon Session")?,
        );
    }
    if child.is_empty() {
        orbit_command.args(read_shell_config(config)?.command);
    } else {
        orbit_command.args(child);
    }
    Ok(orbit_command)
}

struct ControlListener {
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
}

impl Drop for ControlListener {
    fn drop(&mut self) {
        // The open listener prevents inode reuse; chmod may legitimately change ctime.
        if socket_identity(&self.path)
            .ok()
            .flatten()
            .is_some_and(|identity| (identity.0, identity.1) == self.identity)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn create_control_listener(path: &Path) -> Result<ControlListener, String> {
    match UnixStream::connect(path) {
        Ok(_) => {
            return Err(format!(
                "an Eon supervisor is already active at {}",
                path.display()
            ));
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.kind() == std::io::ErrorKind::ConnectionRefused => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect Eon control socket {}: {error}",
                path.display()
            ));
        }
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == effective_uid() => {
            fs::remove_file(path).map_err(|error| {
                format!(
                    "cannot remove stale Eon control socket {}: {error}",
                    path.display()
                )
            })?;
        }
        Ok(_) => {
            return Err(format!(
                "Eon control path {} must be an owned Unix socket",
                path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect Eon control path {}: {error}",
                path.display()
            ));
        }
    }
    let listener = UnixListener::bind(path)
        .map_err(|error| format!("cannot bind Eon control socket {}: {error}", path.display()))?;
    let identity = socket_identity(path)?
        .ok_or_else(|| format!("Eon control socket {} disappeared", path.display()))?;
    let control = ControlListener {
        listener,
        path: path.into(),
        identity: (identity.0, identity.1),
    };
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|error| {
        format!(
            "cannot protect Eon control socket {}: {error}",
            path.display()
        )
    })?;
    control
        .listener
        .set_nonblocking(true)
        .map_err(|error| format!("cannot configure Eon control socket: {error}"))?;
    Ok(control)
}

fn accept_control_client(
    listener: &UnixListener,
    workspace: Option<&mut Workspace>,
    sessions: &mut Vec<RunningSession>,
    programs: &Programs,
    config: &Path,
    socket_timeout: Duration,
    generation: &str,
) -> Result<bool, String> {
    match listener.accept() {
        Ok((stream, _)) => Ok(handle_control_client(
            stream,
            workspace,
            sessions,
            programs,
            config,
            socket_timeout,
            generation,
        )),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(format!("cannot accept Eon control client: {error}")),
    }
}

fn handle_control_client(
    mut stream: UnixStream,
    workspace: Option<&mut Workspace>,
    sessions: &mut Vec<RunningSession>,
    programs: &Programs,
    config: &Path,
    socket_timeout: Duration,
    generation: &str,
) -> bool {
    let mode = if workspace.is_some() {
        LaunchMode::Workspace
    } else {
        LaunchMode::Terminal
    };
    let timeout = Some(Duration::from_millis(250));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return false;
    }
    let (response, stop_requested) = match read_control_request(&mut stream) {
        Ok(Request {
            action: Action::InspectRuntime,
            ..
        }) => match runtime_status(generation, sessions, mode) {
            Ok(runtime) => (
                ControlResponse::Lifecycle(LifecycleResponse::Runtime(runtime)),
                false,
            ),
            Err(detail) => (
                ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                    "unrepresentable-state",
                    detail,
                ))),
                false,
            ),
        },
        Ok(Request {
            action: Action::Stop { generation: target },
            ..
        }) if target == generation => (
            ControlResponse::Lifecycle(LifecycleResponse::Stopped(Stopped {
                generation: generation.into(),
                sessions: sessions.iter().map(|session| session.id.clone()).collect(),
            })),
            true,
        ),
        Ok(Request {
            action: Action::Stop { generation: target },
            ..
        }) => (
            ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                "generation-mismatch",
                format!("supervisor owns generation {generation}, not {target}"),
            ))),
            false,
        ),
        Ok(request) => match workspace {
            Some(workspace) => match workspace.dispatch(&request.id, request.action, |session| {
                let child = start_orbit(programs, config, &session.endpoint, &[], socket_timeout)?;
                sessions.push(RunningSession {
                    id: session.id.clone(),
                    endpoint: session.endpoint.clone(),
                    child,
                });
                Ok(())
            }) {
                Ok(()) => (
                    ControlResponse::Workspace(Response::Snapshot(workspace.snapshot())),
                    false,
                ),
                Err(error) => (
                    ControlResponse::Workspace(Response::Failure(failure(
                        error.code,
                        error.detail,
                    ))),
                    false,
                ),
            },
            None => (
                ControlResponse::Workspace(Response::Failure(failure(
                    "workspace-unavailable",
                    "terminal mode has no Eon workspace",
                ))),
                false,
            ),
        },
        Err(error) => (ControlResponse::Workspace(Response::Failure(error)), false),
    };
    let encoded = match &response {
        ControlResponse::Workspace(response) => encode_response(response),
        ControlResponse::Lifecycle(response) => encode_lifecycle_response(response),
    };
    if let Ok(encoded) = encoded.or_else(|error| {
        encode_response(&Response::Failure(failure(
            "unrepresentable-state",
            format!("cannot encode Eon workspace result: {error}"),
        )))
    }) {
        let written = stream.write_all(&encoded).is_ok();
        return stop_requested && written;
    }
    false
}

fn runtime_status(
    generation: &str,
    sessions: &[RunningSession],
    mode: LaunchMode,
) -> Result<Runtime, String> {
    Ok(Runtime {
        generation: generation.into(),
        eon_version: env!("CARGO_PKG_VERSION").into(),
        workspace_protocol: VERSION,
        component_report: eon_manifest::version_report(MANIFEST)
            .map_err(|error| error.to_string())?,
        sessions: sessions.iter().map(|session| session.id.clone()).collect(),
        attach: Availability {
            available: true,
            reason: match mode {
                LaunchMode::Workspace => "supervisor accepts EONW v1 workspace clients",
                LaunchMode::Terminal => "supervisor owns one terminal host",
            }
            .into(),
        },
        stop: Availability {
            available: true,
            reason: "generation-aware supervisor owns these Sessions".into(),
        },
    })
}

fn read_control_request(stream: &mut impl Read) -> Result<Request, Failure> {
    let incomplete = |error| {
        failure(
            "malformed-action",
            format!("cannot read complete EONW request: {error}"),
        )
    };
    let mut message = vec![0; HEADER_BYTES];
    stream.read_exact(&mut message).map_err(&incomplete)?;
    let length = declared_message_len(&message).map_err(protocol_failure)?;
    message.resize(length, 0);
    stream
        .read_exact(&mut message[HEADER_BYTES..])
        .map_err(incomplete)?;
    decode_request(&message).map_err(protocol_failure)
}

fn protocol_failure(error: ProtocolError) -> Failure {
    let code = if matches!(error, ProtocolError::UnsupportedVersion { .. }) {
        "unsupported-version"
    } else {
        "malformed-action"
    };
    failure(code, error.to_string())
}

fn report_failure(failure: &Failure, json: bool) -> Result<i32, String> {
    if json {
        write_stdout(failure_json(failure))?;
    } else {
        eprint!("{}", failure_human(failure));
    }
    Ok(2)
}

fn failure(code: impl Into<String>, detail: impl Into<String>) -> Failure {
    let mut detail = detail.into();
    detail.truncate(detail.floor_char_boundary(MAX_DETAIL_BYTES));
    if detail.is_empty() {
        detail = "unspecified Eon workspace failure".into();
    }
    Failure {
        code: code.into(),
        detail,
    }
}

type SocketIdentity = (u64, u64, i64, i64);

fn socket_identity(socket: &Path) -> Result<Option<SocketIdentity>, String> {
    match fs::symlink_metadata(socket) {
        Ok(metadata) => Ok(Some((
            metadata.dev(),
            metadata.ino(),
            metadata.ctime(),
            metadata.ctime_nsec(),
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot inspect socket {}: {error}",
            socket.display()
        )),
    }
}

fn wait_for_socket(
    child: &mut Child,
    socket: &Path,
    previous_socket: Option<SocketIdentity>,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("cannot observe Sessions startup: {error}"))?
        {
            return Err(format!(
                "Sessions exited before creating {} (status {})",
                socket.display(),
                status_code(status)
            ));
        }
        if socket_identity(socket)?.is_some_and(|identity| Some(identity) != previous_socket) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Sessions did not create {} within {} seconds",
                socket.display(),
                timeout.as_secs()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn stop(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::{
        LaunchMode, ManagedPrograms, ManagedTool, Programs, create_control_listener,
        current_generation, discover_generations, effective_uid, generation_directory,
        generation_id, integration_mask, managed_command, managed_tool, orbit_command,
        prepare_configuration, prepare_runtime, prepend_path, read_shell_config, supervise,
        valid_generation, venus_command, xdg_path,
    };
    use std::{
        ffi::{OsStr, OsString},
        fs,
        os::unix::{
            fs::{MetadataExt, PermissionsExt, symlink},
            net::UnixListener,
        },
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::{Duration, Instant},
    };

    static NEXT_TEST: AtomicU64 = AtomicU64::new(0);

    fn temporary_directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "eon-test-{}-{}",
            std::process::id(),
            NEXT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn generation_identity_is_stable_bounded_and_namespaced() {
        let first = generation_id(&[b"ab".as_slice(), b"c".as_slice()]);
        assert_eq!(first, generation_id(&[b"ab".as_slice(), b"c".as_slice()]));
        assert_ne!(first, generation_id(&[b"a".as_slice(), b"bc".as_slice()]));
        assert!(valid_generation(&first));
        assert!(!valid_generation("legacy"));
        assert!(!valid_generation("g1-0123456789ABCDEF0123456789ABCDEF"));
        assert_eq!(
            generation_directory(Path::new("/runtime/eon"), &first),
            Path::new("/runtime/eon/generations").join(first)
        );
    }

    #[test]
    fn discovery_rejects_symlinks_and_removes_only_dead_owned_control_metadata() {
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

        let records = discover_generations(&root, &current_generation()).unwrap();
        assert!(records.iter().any(|record| {
            record.id == linked_id && record.kind == "previous" && record.state == "corrupt"
        }));
        assert!(records.iter().any(|record| {
            record.id == dead_id && record.kind == "previous" && record.state == "dead"
        }));
        assert!(outside.is_dir());
        assert!(!dead.exists());

        fs::remove_file(generations.join(linked_id)).unwrap();
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    fn executable(path: &Path, source: &str) {
        fs::write(path, source).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn command_environment<'a>(command: &'a Command, name: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| value)
    }

    #[test]
    fn shell_configuration_is_strict_and_defaults_without_a_file() {
        let root = temporary_directory();
        let default = read_shell_config(&root).unwrap();
        assert_eq!(default.command, ["eon-nu"]);
        assert!(default.starship && default.zoxide && default.atuin && default.carapace);

        fs::write(
            root.join("config.toml"),
            "[shell]\ncommand = [\"eon-fish\", \"--no-config\"]\nstarship = false\nzoxide = false\natuin = false\ncarapace = false\n",
        )
        .unwrap();
        let configured = read_shell_config(&root).unwrap();
        assert_eq!(configured.command, ["eon-fish", "--no-config"]);
        assert!(
            !configured.starship && !configured.zoxide && !configured.atuin && !configured.carapace
        );
        assert_eq!(integration_mask(&configured, true), 0);
        assert_eq!(integration_mask(&default, false), 15);
        assert_eq!(integration_mask(&default, true), 31);

        for (source, expected) in [
            ("[shell]\ncommand = []\n", "shell.command must not be empty"),
            (
                "[shell]\ncommand = [\"eon-nu\"]\nunknown = true\n",
                "unknown field `unknown`",
            ),
            ("[shell\n", "invalid Eon configuration"),
        ] {
            fs::write(root.join("config.toml"), source).unwrap();
            assert!(read_shell_config(&root).unwrap_err().contains(expected));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn managed_invocation_names_are_bounded() {
        use ManagedTool::{Bash, Fish, Helix, LazyGit, Nu, Ya, Yazi, Zsh};

        for (name, expected) in [
            ("eon-nu", Some(Nu)),
            ("nu", Some(Nu)),
            ("eon-bash", Some(Bash)),
            ("bash", Some(Bash)),
            ("eon-zsh", Some(Zsh)),
            ("zsh", Some(Zsh)),
            ("eon-fish", Some(Fish)),
            ("fish", Some(Fish)),
            ("eon-hx", Some(Helix)),
            ("hx", Some(Helix)),
            ("eon-yazi", Some(Yazi)),
            ("yazi", Some(Yazi)),
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
            assert_eq!(managed_tool(Path::new(name).as_os_str()), expected);
        }
        assert_eq!(
            managed_tool(Path::new("/nix/store/example/bin/eon-nu").as_os_str()),
            Some(Nu)
        );
    }

    #[test]
    fn managed_commands_use_exact_programs_and_private_configuration() {
        use ManagedTool::{Bash, Fish, Helix, LazyGit, Nu, Ya, Yazi, Zsh};

        let programs = ManagedPrograms {
            nu: "/managed/nu".into(),
            bash: "/managed/bash".into(),
            zsh: "/managed/zsh".into(),
            fish: "/managed/fish".into(),
            helix: "/managed/hx".into(),
            yazi: "/managed/yazi".into(),
            ya: "/managed/ya".into(),
            lazygit: "/managed/lazygit".into(),
            nu_vendor_autoload: Some("/managed/autoload".into()),
            bash_rc: Some("/managed/bashrc".into()),
            zsh_config: Some("/managed/zsh-config".into()),
            fish_init: Some("/managed/fish-init".into()),
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
            let command = managed_command(tool, &programs, config, &["--version".into()]).unwrap();
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

        let command = managed_command(Nu, &programs, config, &["--version".into()]).unwrap();
        assert_eq!(command.get_program(), "/managed/nu");
        assert_eq!(
            command.get_args().map(OsString::from).collect::<Vec<_>>(),
            ["--version"].map(OsString::from)
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

        for (tool, program, arguments) in [
            (
                Bash,
                "/managed/bash",
                vec!["--rcfile", "/managed/bashrc", "--version"],
            ),
            (Zsh, "/managed/zsh", vec!["--version"]),
            (
                Fish,
                "/managed/fish",
                vec!["-C", "source \"$EON_FISH_INIT\"", "--version"],
            ),
        ] {
            let command = managed_command(tool, &programs, config, &["--version".into()]).unwrap();
            assert_eq!(command.get_program(), program);
            assert_eq!(
                command.get_args().map(OsString::from).collect::<Vec<_>>(),
                arguments
                    .into_iter()
                    .map(OsString::from)
                    .collect::<Vec<_>>()
            );
            for integration in ["STARSHIP", "ZOXIDE", "ATUIN", "CARAPACE"] {
                assert_eq!(
                    command_environment(&command, &format!("EON_SHELL_{integration}")),
                    Some(Some(OsStr::new("1")))
                );
            }
        }
        assert_eq!(
            command_environment(
                &managed_command(Zsh, &programs, config, &[]).unwrap(),
                "ZDOTDIR"
            ),
            Some(Some(OsStr::new("/managed/zsh-config")))
        );
        assert_eq!(
            command_environment(
                &managed_command(Fish, &programs, config, &[]).unwrap(),
                "EON_FISH_INIT"
            ),
            Some(Some(OsStr::new("/managed/fish-init")))
        );
    }

    #[test]
    fn orbit_uses_configured_argv_only_for_default_sessions() {
        let root = temporary_directory();
        let programs = Programs {
            orbit: "/managed/orbit".into(),
            venus: "/managed/venus".into(),
            session_bin: Some("/managed/bin".into()),
        };
        let config = root.as_path();
        let socket = Path::new("/runtime/orbit.sock");
        fs::write(
            root.join("config.toml"),
            "[shell]\ncommand = [\"eon-fish\", \"--no-config\"]\n",
        )
        .unwrap();

        let default = orbit_command(&programs, config, socket, &[]).unwrap();
        assert_eq!(
            default.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "serve",
                "/runtime/orbit.sock",
                "--",
                "eon-fish",
                "--no-config",
            ]
            .map(OsString::from)
        );
        assert_eq!(
            command_environment(&default, "EON_CONFIG_HOME"),
            Some(Some(config.as_os_str()))
        );
        assert_eq!(command_environment(&default, "XDG_CONFIG_HOME"), None);
        assert_eq!(
            std::env::split_paths(
                default
                    .get_envs()
                    .find(|(name, _)| *name == "PATH")
                    .unwrap()
                    .1
                    .unwrap(),
            )
            .next(),
            Some(PathBuf::from("/managed/bin"))
        );

        let explicit = orbit_command(
            &programs,
            config,
            socket,
            &["codex".into(), "--model".into(), "test".into()],
        )
        .unwrap();
        assert_eq!(
            explicit.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "serve",
                "/runtime/orbit.sock",
                "--",
                "codex",
                "--model",
                "test",
            ]
            .map(OsString::from)
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn venus_receives_workspace_endpoint_only_in_workspace_mode() {
        let programs = Programs {
            orbit: "/managed/orbit".into(),
            venus: "/managed/venus".into(),
            session_bin: None,
        };
        let config = Path::new("/config/eon");
        let socket = Path::new("/runtime/orbit.sock");

        let workspace = venus_command(&programs, config, socket, LaunchMode::Workspace);
        assert_eq!(
            workspace.get_args().map(OsString::from).collect::<Vec<_>>(),
            ["/runtime/orbit.sock", "/runtime/eon.sock"].map(OsString::from)
        );

        let terminal = venus_command(&programs, config, socket, LaunchMode::Terminal);
        assert_eq!(
            terminal.get_args().map(OsString::from).collect::<Vec<_>>(),
            ["/runtime/orbit.sock"].map(OsString::from)
        );
    }

    #[test]
    fn failed_terminal_surface_stops_its_new_session_and_control_endpoint() {
        let root = temporary_directory();
        let orbit = root.join("orbit");
        let socket = root.join("orbit.sock");
        let config = root.join("config");
        executable(
            &orbit,
            "#!/bin/sh\nprintf '%s' \"$$\" > \"$2\"\nwhile :; do sleep 0.01; done\n",
        );

        let error = supervise(
            &Programs {
                orbit,
                venus: root.join("missing-venus"),
                session_bin: None,
            },
            &config,
            &socket,
            &["ignored".into()],
            Duration::from_secs(2),
            "g1-0123456789abcdef0123456789abcdef",
            LaunchMode::Terminal,
        )
        .unwrap_err();

        assert!(error.contains("cannot launch Eon Desktop"));
        let pid = fs::read_to_string(&socket).unwrap();
        assert!(!Path::new("/proc").join(pid).exists());
        assert!(!root.join("eon.sock").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn venus_exit_leaves_orbit_and_its_command_running() {
        let root = temporary_directory();
        let orbit = root.join("orbit");
        let venus = root.join("venus");
        let socket = root.join("orbit.sock");
        let config = root.join("config");
        let orbit_log = root.join("orbit.log");
        let venus_log = root.join("venus.log");
        let venus_exited = root.join("venus-exited");
        let stop = root.join("stop");
        fs::write(&socket, "old").unwrap();

        executable(
            &orbit,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$EON_CONFIG_HOME\" \"$@\" > '{}'\nsleep 0.05\nmv \"$2\" \"$2.old\"\nprintf new > \"$2\"\nwhile test ! -e '{}'; do sleep 0.01; done\nexit 23\n",
                orbit_log.display(),
                stop.display()
            ),
        );
        executable(
            &venus,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$XDG_CONFIG_HOME\" \"$@\" \"$(cat \"$1\")\" > '{}'\n: > '{}'
",
                venus_log.display(),
                venus_exited.display()
            ),
        );

        let stop_after_venus = thread::spawn({
            let venus_exited = venus_exited.clone();
            let stop = stop.clone();
            move || {
                let deadline = Instant::now() + Duration::from_secs(2);
                while !venus_exited.exists() && Instant::now() < deadline {
                    thread::sleep(Duration::from_millis(10));
                }
                assert!(venus_exited.exists());
                thread::sleep(Duration::from_millis(100));
                fs::write(stop, "").unwrap();
            }
        });

        let status = supervise(
            &Programs {
                orbit,
                venus,
                session_bin: None,
            },
            &config,
            &socket,
            &["codex".into(), "--model".into(), "test".into()],
            Duration::from_secs(2),
            "g1-0123456789abcdef0123456789abcdef",
            LaunchMode::Workspace,
        )
        .unwrap();
        stop_after_venus.join().unwrap();

        assert_eq!(status, 23);
        assert_eq!(
            fs::read_to_string(orbit_log).unwrap(),
            format!(
                "{}\nserve\n{}\n--\ncodex\n--model\ntest\n",
                config.display(),
                socket.display()
            )
        );
        assert_eq!(
            fs::read_to_string(venus_log).unwrap(),
            format!(
                "{}\n{}\n{}\nnew\n",
                config.display(),
                socket.display(),
                root.join("eon.sock").display()
            )
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn runtime_directory_is_private() {
        let root = temporary_directory();
        let runtime = root.join("runtime");

        prepare_runtime(&runtime).unwrap();

        let metadata = fs::metadata(&runtime).unwrap();
        assert_eq!(metadata.uid(), effective_uid());
        assert_eq!(metadata.mode() & 0o777, 0o700);

        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            prepare_runtime(&runtime).unwrap_err(),
            format!(
                "runtime directory {} must be an owned private directory",
                runtime.display()
            )
        );
        assert_eq!(fs::metadata(&runtime).unwrap().mode() & 0o777, 0o777);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn configuration_directory_is_private_only_when_created() {
        let root = temporary_directory();
        let config = root.join("config");

        prepare_configuration(&config).unwrap();
        assert_eq!(fs::metadata(&config).unwrap().mode() & 0o777, 0o700);

        fs::set_permissions(&config, fs::Permissions::from_mode(0o755)).unwrap();
        prepare_configuration(&config).unwrap();
        assert_eq!(fs::metadata(&config).unwrap().mode() & 0o777, 0o755);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn xdg_paths_must_be_absolute() {
        assert_eq!(xdg_path(Some("relative".into())), None);

        let absolute = PathBuf::from("/absolute");
        assert_eq!(xdg_path(Some(absolute.clone())), Some(absolute));
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
    fn control_listener_preserves_a_replacement_socket() {
        let root = temporary_directory();
        let socket = root.join("eon.sock");
        let control = create_control_listener(&socket).unwrap();
        fs::remove_file(&socket).unwrap();
        let _replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();

        drop(control);

        assert!(socket.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
