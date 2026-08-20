mod managed_environment;
mod workspace;

use eon_workspace_protocol::{
    Action, Availability, Direction, Error as ProtocolError, Failure, HEADER_BYTES,
    LifecycleResponse, MAX_DETAIL_BYTES, MAX_PANES, Request, Response, Runtime, Stopped, VERSION,
    declared_message_len, decode_lifecycle_response, decode_request, decode_response,
    encode_lifecycle_response, encode_request, encode_response,
};
use orbit_protocol::management::{
    self as management, ClientMessage as ManagementClientMessage, EndpointIdentity, LiveIdentity,
    ObjectIdentity, ProcessOutcome, Record as ManagementRecord,
    ServerMessage as ManagementServerMessage, TerminationReason, Tombstone,
};
use std::{
    collections::HashSet,
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    os::fd::{AsRawFd, OwnedFd},
    os::unix::{
        ffi::{OsStrExt, OsStringExt},
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::{UnixListener, UnixStream},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, ExitStatus, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use workspace::{
    Workspace, failure_human, failure_json, human as human_output, json as json_output, json_escape,
};

const MANIFEST: &str = include_str!("../../../components/eon-alpha-v3.json");
const EON_ANSI_PALETTE: &str = "000000,cd0000,00cd00,cdcd00,1093f5,cd00cd,00cdcd,faebd7,404040,ff0000,00ff00,ffff00,11b5f6,ff00ff,00ffff,ffffff";
const EON_USAGE: &str = "usage: eon [run [-- COMMAND...]] | attach [GENERATION] | generations [--json] | stop GENERATION [--json] | workspace [--json] | tab create [--json] | pane create [--json] | focus <ID|left|right|up|down> [--json] | versions | config-path";
const EONTERM_USAGE: &str = "usage: eonterm [--no-decorations] -- COMMAND... | attach [GENERATION] | generations [--json] | stop GENERATION [--json]";
static NEXT_REQUEST: AtomicU64 = AtomicU64::new(0);

fn current_generation() -> Result<String, String> {
    eon_manifest::parse_and_validate(MANIFEST).map_err(|error| error.to_string())?;
    Ok(generation_id(&[
        include_bytes!("main.rs"),
        include_bytes!("managed_environment.rs"),
        include_bytes!("workspace.rs"),
        include_bytes!("../../eon-workspace-protocol/src/lib.rs"),
        include_bytes!("../../eon-workspace-protocol/Cargo.toml"),
        include_bytes!("../../eon-manifest/src/lib.rs"),
        include_bytes!("../../eon-manifest/Cargo.toml"),
        include_bytes!("../Cargo.toml"),
        include_bytes!("../../../Cargo.toml"),
        include_bytes!("../../../Cargo.lock"),
        include_bytes!("../../../flake.nix"),
        include_bytes!("../../../flake.lock"),
        MANIFEST.as_bytes(),
    ]))
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

fn session_number(value: &str) -> Option<usize> {
    let number = value.strip_prefix("session-")?;
    if number.is_empty() || number == "0" || number.starts_with('0') {
        return None;
    }
    number
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| number.parse().ok())
        .flatten()
}

fn generation_directory(root: &Path, generation: &str) -> PathBuf {
    root.join("generations").join(generation)
}

const MAX_GENERATIONS: usize = 256;
const SESSION_START_TIMEOUT: Duration = Duration::from_secs(5);
const CONTROL_TIMEOUT: Duration = SESSION_START_TIMEOUT.saturating_add(Duration::from_secs(1));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EndpointFailureKind {
    Dead,
    Incompatible,
    InvalidAction,
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
    let action = prepare_action(action)?;
    send_prepared_action(connect_control(socket)?, action)
}

fn connect_control(socket: &Path) -> Result<UnixStream, EndpointFailure> {
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

    let stream = UnixStream::connect(socket).map_err(|error| {
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
    Ok(stream)
}

fn send_action_on(stream: UnixStream, action: Action) -> Result<ControlResponse, EndpointFailure> {
    send_prepared_action(stream, prepare_action(action)?)
}

fn prepare_action(action: Action) -> Result<(bool, Vec<u8>), EndpointFailure> {
    let lifecycle = matches!(
        action,
        Action::InspectRuntime
            | Action::InspectPresentation
            | Action::Present { .. }
            | Action::Stop { .. }
    );
    let request = encode_request(&Request {
        id: request_id(),
        action,
    })
    .map_err(|error| {
        EndpointFailure::new(
            EndpointFailureKind::InvalidAction,
            format!("cannot encode Eon action: {error}"),
        )
    })?;
    Ok((lifecycle, request))
}

fn send_prepared_action(
    mut stream: UnixStream,
    (lifecycle, request): (bool, Vec<u8>),
) -> Result<ControlResponse, EndpointFailure> {
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

fn probe_runtime_action(socket: &Path, action: Action) -> Result<Runtime, EndpointFailure> {
    match send_action(socket, action)? {
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

fn probe_presentable_runtime(socket: &Path) -> Result<Runtime, EndpointFailure> {
    match probe_runtime_action(socket, Action::InspectPresentation) {
        Ok(runtime) => Ok(runtime),
        Err(error) if error.kind == EndpointFailureKind::Incompatible => {
            let mut runtime = probe_runtime_action(socket, Action::InspectRuntime)?;
            runtime.attach = Availability {
                available: false,
                reason: "supervisor does not support presentation requests".into(),
            };
            Ok(runtime)
        }
        Err(error) => Err(error),
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

fn probe_supervisor(
    socket: &Path,
    generation: &str,
) -> Result<(LaunchMode, SocketIdentity), EndpointFailure> {
    let identity = socket_identity(socket)
        .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?
        .ok_or_else(|| {
            EndpointFailure::new(
                EndpointFailureKind::Dead,
                format!("endpoint {} is missing", socket.display()),
            )
        })?;
    let info = probe_presentable_runtime(socket)?;
    if !info.attach.available {
        return Err(EndpointFailure::new(
            EndpointFailureKind::Incompatible,
            info.attach.reason,
        ));
    }
    validate_runtime(&info, generation)
        .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?;
    let mode = probe_launch_mode(socket)?;
    if socket_identity(socket)
        .map_err(|detail| EndpointFailure::new(EndpointFailureKind::Corrupt, detail))?
        != Some(identity)
    {
        return Err(EndpointFailure::new(
            EndpointFailureKind::Dead,
            "supervisor endpoint changed while it was being validated",
        ));
    }
    Ok((mode, identity))
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
    let startup_lock = root.join("startup.lock");
    let mut records = vec![inspect_generation(
        current,
        "current",
        current_path,
        &startup_lock,
        &component_report,
    )];
    for (_, id, path) in candidates {
        if id != current {
            records.push(inspect_generation(
                &id,
                "previous",
                path,
                &startup_lock,
                &component_report,
            ));
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
        &root.join("startup.lock"),
        &component_report,
    )))
}

fn inspect_generation(
    id: &str,
    kind: &'static str,
    runtime: PathBuf,
    startup_lock: &Path,
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
    match probe_presentable_runtime(&socket) {
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
            if dead && let Some(_startup_lock) = try_lock_supervisor_startup(startup_lock) {
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
        Err(error) => failed_generation("legacy", "legacy", runtime, error),
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
        EndpointFailureKind::InvalidAction => "corrupt",
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

fn write_stdout(output: impl AsRef<[u8]>) -> Result<(), String> {
    match std::io::stdout().lock().write_all(output.as_ref()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(format!("cannot write stdout: {error}")),
    }
}

fn generations_command(json: bool, product: &str) -> Result<i32, String> {
    let records = discover_generations(&runtime_directory(product), &current_generation()?)?;
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

fn attach_generation(target: &str, product: &str) -> Result<i32, String> {
    let root = runtime_directory(product);
    let current = current_generation()?;
    let record = inspect_selected_generation(&root, &current, target)?
        .ok_or_else(|| format!("generation {target} was not found"))?;
    if !record.attach.available {
        return Err(format!(
            "generation {target} is not attachable: {}",
            record.attach.reason
        ));
    }
    if target == "legacy" {
        return attach_legacy(&record.runtime);
    }
    let (mode, supervisor) =
        probe_supervisor(&record.runtime.join("eon.sock"), target).map_err(|error| error.detail)?;
    present_at(&record.runtime, target, mode, supervisor)
}

fn stop_generation(target: &str, json: bool, product: &str) -> Result<i32, String> {
    let root = runtime_directory(product);
    let current = current_generation()?;
    let control = generation_directory(&root, target).join("eon.sock");
    let observed_supervisor = socket_identity(&control);
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

struct Programs {
    orbit: PathBuf,
    venus: PathBuf,
    session_bin: Option<PathBuf>,
    venus_decorations: bool,
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
            Self::Terminal => "EonTerm",
        }
    }
}

fn main() -> ExitCode {
    let mut arguments = env::args_os();
    let invocation = arguments.next().unwrap_or_default();
    let arguments = arguments.collect();
    let eonterm = Path::new(&invocation).file_name() == Some(OsStr::new("eonterm"));
    let result = if eonterm {
        execute_eonterm(arguments)
    } else {
        match managed_environment::tool(&invocation) {
            Some(tool) => launch_managed(tool, arguments),
            None => execute(arguments),
        }
    };
    match result {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(error) => {
            eprintln!("{}: {error}", if eonterm { "eonterm" } else { "eon" });
            ExitCode::FAILURE
        }
    }
}

fn launch_managed(
    tool: managed_environment::Tool,
    arguments: Vec<OsString>,
) -> Result<i32, String> {
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let mut command = managed_environment::command(tool, &config, &arguments)?;
    let program = command.get_program().to_string_lossy().into_owned();
    let error = command.exec();
    Err(format!("cannot launch {program}: {error}"))
}

fn execute(arguments: Vec<OsString>) -> Result<i32, String> {
    if let Some(code) = lifecycle_command(&arguments, "eon")? {
        return Ok(code);
    }
    match arguments.as_slice() {
        [] => launch_current(LaunchMode::Workspace, &[], true, true),
        [command] if command == "run" => launch_current(LaunchMode::Workspace, &[], false, true),
        [command, separator, child @ ..]
            if command == "run" && separator == "--" && !child.is_empty() =>
        {
            launch_current(LaunchMode::Workspace, child, false, true)
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
                current_generation()?,
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
        _ => Err(EON_USAGE.into()),
    }
}

fn execute_eonterm(arguments: Vec<OsString>) -> Result<i32, String> {
    if let Some(code) = lifecycle_command(&arguments, "eonterm")? {
        return Ok(code);
    }
    match arguments.as_slice() {
        [separator, child @ ..] if separator == "--" && !child.is_empty() => {
            launch_current(LaunchMode::Terminal, child, true, true)
        }
        [flag, separator, child @ ..]
            if flag == "--no-decorations" && separator == "--" && !child.is_empty() =>
        {
            launch_current(LaunchMode::Terminal, child, true, false)
        }
        _ => Err(EONTERM_USAGE.into()),
    }
}

fn lifecycle_command(arguments: &[OsString], product: &str) -> Result<Option<i32>, String> {
    match arguments {
        [command] if command == "attach" => attach_generation(&current_generation()?, product),
        [command, generation] if command == "attach" => {
            attach_generation(generation_argument(generation)?, product)
        }
        [command] if command == "generations" => generations_command(false, product),
        [command, flag] if command == "generations" && flag == "--json" => {
            generations_command(true, product)
        }
        [command, generation] if command == "stop" => {
            stop_generation(generation_argument(generation)?, false, product)
        }
        [command, generation, flag] | [command, flag, generation]
            if command == "stop" && flag == "--json" =>
        {
            stop_generation(generation_argument(generation)?, true, product)
        }
        _ => return Ok(None),
    }
    .map(Some)
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
    decorations: bool,
) -> Result<i32, String> {
    let generation = current_generation()?;
    let root = runtime_directory(if mode == LaunchMode::Terminal {
        "eonterm"
    } else {
        "eon"
    });
    prepare_runtime(&root)?;
    // ponytail: one root-wide startup lock; partition if cross-generation starts contend.
    let startup_lock = lock_supervisor_startup(&root.join("startup.lock"))?;
    let runtime = generation_directory(&root, &generation);
    let socket = runtime.join("eon.sock");
    match probe_supervisor(&socket, &generation) {
        Ok((active_mode, supervisor)) => {
            if active_mode != mode {
                return Err(format!(
                    "generation {generation} already has a live {} mode; requested {} mode",
                    active_mode.name(),
                    mode.name()
                ));
            }
            drop(startup_lock);
            return if attach_existing {
                present_at(&runtime, &generation, mode, supervisor)
            } else {
                Err(format!(
                    "generation {generation} already has a live Eon supervisor"
                ))
            };
        }
        Err(error) if error.kind == EndpointFailureKind::Dead => {}
        Err(error) => return Err(error.detail),
    }
    let config = configuration_directory()?;
    let terminal = managed_environment::terminal_presentation(&config)?;
    prepare_generation_runtime(&root, &generation)?;
    prepare_configuration(&config)?;
    let programs = programs(decorations);
    match supervise(
        &programs,
        &config,
        terminal,
        startup_lock,
        &runtime.join("orbit.sock"),
        child,
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
        match probe_supervisor(socket, generation) {
            Ok((active_mode, supervisor)) => {
                if active_mode != mode {
                    return Err(format!(
                        "{launch_error}; competing supervisor started in {} mode, requested {} mode",
                        active_mode.name(),
                        mode.name()
                    ));
                }
                return present_at(runtime, generation, mode, supervisor);
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

fn validate_runtime(info: &Runtime, generation: &str) -> Result<(), String> {
    if info.generation != generation {
        return Err(format!(
            "runtime directory contains generation {}, expected {generation}",
            info.generation
        ));
    }
    if info.workspace_protocol != VERSION {
        return Err(format!(
            "supervisor uses EONW {}, expected EONW {VERSION}",
            info.workspace_protocol
        ));
    }
    let components = eon_manifest::version_report(MANIFEST).map_err(|error| error.to_string())?;
    if info.component_report != components {
        return Err("supervisor reports a different component graph".into());
    }
    Ok(())
}

fn control(arguments: &[OsString]) -> Result<i32, String> {
    let (action, json) = parse_control_arguments(arguments)?;
    let generation = current_generation()?;
    let root = runtime_directory("eon");
    let runtime = prepare_generation_runtime(&root, &generation)?;
    let socket = runtime.join("eon.sock");
    let response = match send_action(&socket, action) {
        Ok(response) => response,
        Err(error) if error.kind == EndpointFailureKind::InvalidAction => {
            return report_failure(&failure("malformed-action", error.detail), json);
        }
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
                return Err(EON_USAGE.into());
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
        _ => return Err(EON_USAGE.into()),
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

fn attach_legacy(runtime: &Path) -> Result<i32, String> {
    let config = configuration_directory()?;
    let terminal = managed_environment::terminal_presentation(&config)?;
    prepare_configuration(&config)?;
    let programs = programs(true);
    venus_command(
        &programs,
        &config,
        &runtime.join("orbit.sock"),
        LaunchMode::Workspace,
        terminal,
    )
    .spawn()
    .map_err(|error| format!("cannot launch Eon Desktop: {error}"))?
    .wait()
    .map(status_code)
    .map_err(|error| format!("cannot observe Eon Desktop: {error}"))
}

fn present_at(
    runtime: &Path,
    generation: &str,
    mode: LaunchMode,
    supervisor: SocketIdentity,
) -> Result<i32, String> {
    let control = runtime.join("eon.sock");
    let stream = connect_control(&control).map_err(|error| error.detail)?;
    if socket_identity(&control)? != Some(supervisor) {
        return Err("supervisor changed before Eon Desktop could present".into());
    }
    match send_action_on(
        stream,
        Action::Present {
            workspace: mode == LaunchMode::Workspace,
        },
    )
    .map_err(|error| error.detail)?
    {
        ControlResponse::Lifecycle(LifecycleResponse::Runtime(info)) => {
            validate_runtime(&info, generation)?;
            Ok(0)
        }
        ControlResponse::Lifecycle(LifecycleResponse::Failure(failure)) => {
            Err(format!("cannot present Eon Desktop: {}", failure.detail))
        }
        _ => Err("supervisor returned the wrong EONW result for presentation".into()),
    }
}

fn venus_command(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    mode: LaunchMode,
    terminal: managed_environment::TerminalConfig,
) -> Command {
    let mut command = Command::new(&programs.venus);
    if !programs.venus_decorations {
        command.arg("--no-decorations");
    }
    command
        .arg("--background-opacity")
        .arg(terminal.background_opacity.to_string());
    if terminal.background_blur {
        command.arg("--background-blur");
    }
    command.arg(socket);
    if mode == LaunchMode::Workspace {
        command.arg(socket.with_file_name("eon.sock"));
    }
    command.env("XDG_CONFIG_HOME", config);
    command
}

struct PresentationProcess {
    child: Child,
    control: UnixStream,
}

impl PresentationProcess {
    fn start(mut command: Command) -> Result<Self, String> {
        let (control, input) = UnixStream::pair()
            .map_err(|error| format!("cannot create Eon Desktop presentation control: {error}"))?;
        control
            .set_write_timeout(Some(Duration::from_millis(250)))
            .map_err(|error| format!("cannot bound Eon Desktop presentation control: {error}"))?;
        command
            .env("EON_VENUS_PRESENTATION_CONTROL", "stdin")
            .stdin(Stdio::from(OwnedFd::from(input)));
        let child = command
            .spawn()
            .map_err(|error| format!("cannot launch Eon Desktop: {error}"))?;
        Ok(Self { child, control })
    }

    fn present(&self) -> Result<(), String> {
        (&self.control)
            .write_all(b"present\n")
            .map_err(|error| format!("cannot signal Eon Desktop presentation: {error}"))
    }
}

fn close_presentation(process: PresentationProcess) {
    let PresentationProcess { mut child, control } = process;
    drop(control);
    let deadline = Instant::now() + SESSION_START_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            _ => {
                stop(&mut child);
                return;
            }
        }
    }
}

fn start_venus(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    mode: LaunchMode,
    terminal: managed_environment::TerminalConfig,
) -> Result<PresentationProcess, String> {
    PresentationProcess::start(venus_command(programs, config, socket, mode, terminal))
}

fn programs(venus_decorations: bool) -> Programs {
    Programs {
        orbit: configured_program("EON_ORBIT", "yazelix-orbit"),
        venus: configured_program("EON_VENUS", "yazelix-venus"),
        session_bin: nonempty_environment_path("EON_SESSION_BIN"),
        venus_decorations,
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

fn runtime_directory(product: &str) -> PathBuf {
    nonempty_environment_path("EON_RUNTIME_DIR")
        .or_else(|| {
            xdg_path(nonempty_environment_path("XDG_RUNTIME_DIR")).map(|path| path.join(product))
        })
        .unwrap_or_else(|| {
            eprintln!(
                "{product}: warning: XDG_RUNTIME_DIR is unset; using a private temporary runtime root"
            );
            env::temp_dir().join(format!("{product}-{}", effective_uid()))
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

struct RecordSnapshot {
    object: ObjectIdentity,
    record: ManagementRecord,
}

struct ManagedCandidate {
    number: usize,
    endpoint: PathBuf,
    record_path: PathBuf,
    record: ObjectIdentity,
    identity: LiveIdentity,
}

fn artifact_path(endpoint: &Path, suffix: &str) -> PathBuf {
    let mut path = endpoint.as_os_str().to_os_string();
    path.push(suffix);
    path.into()
}

fn object_identity(metadata: &fs::Metadata) -> ObjectIdentity {
    ObjectIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

fn read_management_record(path: &Path) -> Result<Option<RecordSnapshot>, String> {
    let mut file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot open Sessions record {}: {error}",
                path.display()
            ));
        }
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect Sessions record {}: {error}", path.display()))?;
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.len() > management::MAX_RECORD_BYTES as u64
    {
        return Err(format!(
            "Sessions record {} must be an owned mode-0600 regular file no larger than {} bytes",
            path.display(),
            management::MAX_RECORD_BYTES
        ));
    }
    let object = object_identity(&metadata);
    let mut bytes = Vec::with_capacity(management::MAX_RECORD_BYTES.saturating_add(1));
    Read::by_ref(&mut file)
        .take(management::MAX_RECORD_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read Sessions record {}: {error}", path.display()))?;
    if bytes.len() > management::MAX_RECORD_BYTES {
        return Err(format!(
            "Sessions record {} exceeds {} bytes",
            path.display(),
            management::MAX_RECORD_BYTES
        ));
    }
    let current = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot revalidate Sessions record {}: {error}",
            path.display()
        )
    })?;
    if !current.file_type().is_file()
        || current.uid() != effective_uid()
        || current.mode() & 0o7777 != 0o600
        || object_identity(&current) != object
    {
        return Err(format!(
            "Sessions record {} changed while it was being read",
            path.display()
        ));
    }
    let record = management::decode_record(&bytes)
        .map_err(|error| format!("invalid Sessions record {}: {error}", path.display()))?;
    Ok(Some(RecordSnapshot { object, record }))
}

fn endpoint_matches(path: &Path, expected: &EndpointIdentity) -> Result<(), String> {
    if expected.path != path.as_os_str().as_bytes() {
        return Err(format!(
            "Sessions identity names endpoint {}, expected {}",
            PathBuf::from(OsString::from_vec(expected.path.clone())).display(),
            path.display()
        ));
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect Sessions endpoint {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || object_identity(&metadata) != expected.object
    {
        return Err(format!(
            "Sessions endpoint {} does not match its owned mode-0600 socket identity",
            path.display()
        ));
    }
    Ok(())
}

fn expected_session_endpoint(runtime: &Path, number: usize) -> PathBuf {
    if number == 1 {
        runtime.join("orbit.sock")
    } else {
        runtime.join(format!("session-{number}.sock"))
    }
}

fn validate_management_identity(
    identity: &LiveIdentity,
    component_generation: &str,
    expected_run: Option<&str>,
    require_live_endpoints: bool,
) -> Result<(usize, PathBuf), String> {
    let number = session_number(&identity.session_id)
        .ok_or_else(|| format!("invalid Sessions identity {:?}", identity.session_id))?;
    if expected_run.is_some_and(|expected| identity.run_id != expected) {
        return Err("Sessions record reports a different run identity".into());
    }
    if identity.component_generation != component_generation {
        return Err(format!(
            "Sessions record reports component generation {}, expected {component_generation}",
            identity.component_generation
        ));
    }
    if identity.record_generation != management::RECORD_GENERATION
        || identity.management_generation != management::VERSION
    {
        return Err("Sessions record reports an unsupported management generation".into());
    }
    if identity.uid != effective_uid() {
        return Err(format!(
            "Sessions record reports UID {}, expected {}",
            identity.uid,
            effective_uid()
        ));
    }
    let presentation = PathBuf::from(OsString::from_vec(identity.presentation.path.clone()));
    let runtime = presentation
        .parent()
        .ok_or_else(|| "Sessions presentation endpoint has no parent".to_string())?;
    let expected_presentation = expected_session_endpoint(runtime, number);
    if presentation != expected_presentation {
        return Err(format!(
            "Sessions identity {} uses unexpected presentation endpoint {}",
            identity.session_id,
            presentation.display()
        ));
    }
    let management_path = artifact_path(&presentation, ".management");
    if identity.management.path != management_path.as_os_str().as_bytes() {
        return Err(format!(
            "Sessions identity {} uses an unexpected management endpoint",
            identity.session_id
        ));
    }
    if require_live_endpoints {
        endpoint_matches(&presentation, &identity.presentation)?;
        endpoint_matches(&management_path, &identity.management)?;
    }
    Ok((number, presentation))
}

fn operation_timeout(deadline: Instant, operation: &str) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("{operation} exceeded five seconds"))
}

fn read_management_response(stream: &mut UnixStream) -> Result<ManagementServerMessage, String> {
    let mut bytes = vec![0; management::HEADER_BYTES];
    stream
        .read_exact(&mut bytes)
        .map_err(|error| format!("cannot read Sessions management result: {error}"))?;
    let length = management::server_message_len(&bytes)
        .map_err(|error| format!("invalid Sessions management result: {error}"))?
        .ok_or("incomplete Sessions management header")?;
    bytes.resize(length, 0);
    stream
        .read_exact(&mut bytes[management::HEADER_BYTES..])
        .map_err(|error| format!("cannot read complete Sessions management result: {error}"))?;
    management::decode_server_message(&bytes)
        .map_err(|error| format!("invalid Sessions management result: {error}"))
}

fn validate_management_peer(stream: &UnixStream, identity: &LiveIdentity) -> Result<(), String> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials points to writable ucred storage of the declared length.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &raw mut length,
        )
    } == -1
        || length as usize != std::mem::size_of::<libc::ucred>()
    {
        return Err("cannot validate Sessions management peer credentials".into());
    }
    if u32::try_from(credentials.pid).ok() != Some(identity.process_id)
        || credentials.uid != identity.uid
    {
        return Err("Sessions management peer differs from its Ready process identity".into());
    }
    let stat = fs::read_to_string(format!("/proc/{}/stat", identity.process_id))
        .map_err(|error| format!("cannot validate Sessions process identity: {error}"))?;
    let start = stat
        .rsplit_once(')')
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or("invalid Sessions process identity")?;
    if start != identity.process_start {
        return Err("Sessions process start differs from its Ready identity".into());
    }
    Ok(())
}

fn acquire_management(
    candidate: ManagedCandidate,
    deadline: Instant,
) -> Result<RunningSession, String> {
    let current = read_management_record(&candidate.record_path)?.ok_or_else(|| {
        format!(
            "Sessions record {} disappeared",
            candidate.record_path.display()
        )
    })?;
    if current.object != candidate.record
        || current.record != ManagementRecord::Live(candidate.identity.clone())
    {
        return Err(format!(
            "Sessions record {} changed before lease acquisition",
            candidate.record_path.display()
        ));
    }
    let management_path = PathBuf::from(OsString::from_vec(
        candidate.identity.management.path.clone(),
    ));
    endpoint_matches(&management_path, &candidate.identity.management)?;
    let mut stream = UnixStream::connect(&management_path).map_err(|error| {
        format!(
            "cannot connect to Sessions management endpoint {}: {error}",
            management_path.display()
        )
    })?;
    validate_management_peer(&stream, &candidate.identity)?;
    let timeout = operation_timeout(deadline, "Sessions lease acquisition")?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|error| format!("cannot bound Sessions lease acquisition: {error}"))?;
    let request = management::encode_client_message(&ManagementClientMessage::Acquire {
        expected: candidate.identity.clone(),
        record: candidate.record,
    })
    .map_err(|error| format!("cannot encode Sessions lease request: {error}"))?;
    stream
        .write_all(&request)
        .map_err(|error| format!("cannot send Sessions lease request: {error}"))?;
    match read_management_response(&mut stream)? {
        ManagementServerMessage::Lease(identity) if identity == candidate.identity => {}
        ManagementServerMessage::Busy => {
            return Err(format!(
                "Sessions run {} is already managed",
                candidate.identity.run_id
            ));
        }
        ManagementServerMessage::Failure(failure) => {
            return Err(format!(
                "Sessions rejected lease acquisition: {}",
                failure.detail
            ));
        }
        _ => return Err("Sessions returned the wrong lease result".into()),
    }
    stream
        .set_read_timeout(None)
        .and_then(|()| stream.set_write_timeout(None))
        .and_then(|()| stream.set_nonblocking(true))
        .map_err(|error| format!("cannot configure Sessions management lease: {error}"))?;
    Ok(RunningSession {
        number: candidate.number,
        id: candidate.identity.session_id.clone(),
        endpoint: candidate.endpoint,
        record: candidate.record_path,
        identity: candidate.identity,
        lease: stream,
        child: None,
    })
}

fn cleanup_record(path: &Path, expected: ObjectIdentity) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect ended Sessions record {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || object_identity(&metadata) != expected
    {
        return Err(format!(
            "ended Sessions record {} changed before cleanup",
            path.display()
        ));
    }
    fs::remove_file(path).map_err(|error| {
        format!(
            "cannot remove ended Sessions record {}: {error}",
            path.display()
        )
    })
}

fn recover_sessions(
    runtime: &Path,
    mode: LaunchMode,
    component_generation: &str,
    deadline: Instant,
) -> Result<Vec<RunningSession>, String> {
    let mut paths = fs::read_dir(runtime)
        .map_err(|error| {
            format!(
                "cannot list Sessions runtime {}: {error}",
                runtime.display()
            )
        })?
        .filter_map(|entry| match entry {
            Ok(entry) if entry.file_name().as_bytes().ends_with(b".record") => {
                Some(Ok(entry.path()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(format!(
                "cannot inspect Sessions entry in {}: {error}",
                runtime.display()
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.len() > MAX_PANES {
        return Err(format!(
            "Sessions runtime {} exceeds the {MAX_PANES}-record recovery limit",
            runtime.display()
        ));
    }
    paths.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });

    let mut candidates = Vec::new();
    let mut numbers = HashSet::new();
    for record_path in paths {
        operation_timeout(deadline, "Sessions recovery")?;
        let snapshot = read_management_record(&record_path)?
            .ok_or_else(|| format!("Sessions record {} disappeared", record_path.display()))?;
        let identity = match &snapshot.record {
            ManagementRecord::Live(identity) => identity,
            ManagementRecord::Tombstone(tombstone) => {
                let (_, endpoint) = validate_management_identity(
                    &tombstone.identity,
                    component_generation,
                    None,
                    false,
                )?;
                if artifact_path(&endpoint, ".record") != record_path {
                    return Err(format!(
                        "ended Sessions record {} has the wrong identity path",
                        record_path.display()
                    ));
                }
                let management_path = PathBuf::from(OsString::from_vec(
                    tombstone.identity.management.path.clone(),
                ));
                loop {
                    if endpoint_removed(&endpoint, tombstone.identity.presentation.object)?
                        && endpoint_removed(&management_path, tombstone.identity.management.object)?
                    {
                        break;
                    }
                    operation_timeout(deadline, "ended Sessions cleanup")?;
                    thread::sleep(Duration::from_millis(25));
                }
                cleanup_record(&record_path, snapshot.object)?;
                continue;
            }
        };
        let (number, endpoint) =
            validate_management_identity(identity, component_generation, None, true)?;
        if artifact_path(&endpoint, ".record") != record_path {
            return Err(format!(
                "Sessions record {} has the wrong presentation identity",
                record_path.display()
            ));
        }
        if !numbers.insert(number) {
            return Err(format!("duplicate live Sessions identity session-{number}"));
        }
        candidates.push(ManagedCandidate {
            number,
            endpoint,
            record_path,
            record: snapshot.object,
            identity: identity.clone(),
        });
    }
    candidates.sort_by_key(|candidate| candidate.number);
    if mode == LaunchMode::Terminal && candidates.len() > 1 {
        return Err("EonTerm cannot recover more than one live Session".into());
    }

    let mut sessions = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        sessions.push(acquire_management(candidate, deadline)?);
    }
    Ok(sessions)
}

#[allow(clippy::too_many_arguments)]
fn supervise(
    programs: &Programs,
    config: &Path,
    terminal: managed_environment::TerminalConfig,
    startup_lock: fs::File,
    socket: &Path,
    child: &[OsString],
    generation: &str,
    mode: LaunchMode,
) -> Result<i32, String> {
    let runtime = socket
        .parent()
        .ok_or_else(|| format!("Sessions socket {} has no parent", socket.display()))?;
    let component_generation =
        eon_manifest::component_revision(MANIFEST, "orbit").map_err(|error| error.to_string())?;
    let deadline = Instant::now() + SESSION_START_TIMEOUT;
    let mut sessions = recover_sessions(runtime, mode, &component_generation, deadline)?;
    if sessions.is_empty() {
        sessions.push(start_orbit(
            programs,
            config,
            socket,
            "session-1",
            &component_generation,
            child,
            deadline,
        )?);
    }
    let workspace = (mode == LaunchMode::Workspace)
        .then(|| {
            Workspace::with_recovered_sessions(
                runtime.to_path_buf(),
                sessions
                    .iter()
                    .map(|session| {
                        (
                            session.number,
                            workspace::Session {
                                id: session.id.clone(),
                                endpoint: session.endpoint.clone(),
                            },
                        )
                    })
                    .collect(),
            )
        })
        .transpose()?;
    let control_listener = create_control_listener(&runtime.join("eon.sock"))?;
    let first_endpoint = sessions[0].endpoint.clone();
    let venus = match start_venus(programs, config, &first_endpoint, mode, terminal) {
        Ok(venus) => venus,
        Err(error) => {
            let stopped = stop_managed_sessions(&mut sessions, SESSION_START_TIMEOUT)
                .map_err(|stop_error| format!("{error}; cannot roll back Sessions: {stop_error}"));
            drop(control_listener);
            if stopped.is_ok() {
                let _ = fs::remove_dir(runtime);
            }
            return stopped.and(Err(error));
        }
    };
    let mut state = SupervisorState {
        workspace,
        sessions,
        venus: Some(venus),
    };
    drop(startup_lock);
    let mut initial_status = None;

    loop {
        let mut index = 0;
        while index < state.sessions.len() {
            if let Some(code) = session_finished(&mut state.sessions[index])? {
                let session = state.sessions.remove(index);
                if let Some(workspace) = &mut state.workspace {
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

        if state.sessions.is_empty() {
            if let Some(process) = state.venus.take() {
                close_presentation(process);
            }
            drop(control_listener);
            let _ = fs::remove_dir(runtime);
            return Ok(initial_status.unwrap_or(0));
        }

        if reap_desktop(&mut state.venus)? {
            match mode {
                LaunchMode::Workspace => eprintln!(
                    "Eon Desktop exited; Sessions remains active. Run `eon attach {generation}` to reconnect."
                ),
                LaunchMode::Terminal => eprintln!(
                    "Eon Desktop exited; Session remains active. Run `eonterm attach {generation}` to reconnect."
                ),
            }
        }

        if accept_control_client(
            &control_listener.listener,
            &mut state,
            programs,
            config,
            SESSION_START_TIMEOUT,
            generation,
        )? {
            if let Some(process) = state.venus.take() {
                close_presentation(process);
            }
            drop(control_listener);
            let _ = fs::remove_dir(runtime);
            return Ok(0);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

struct SupervisorState {
    workspace: Option<Workspace>,
    sessions: Vec<RunningSession>,
    venus: Option<PresentationProcess>,
}

fn reap_desktop(desktop: &mut Option<PresentationProcess>) -> Result<bool, String> {
    let exited = match desktop.as_mut() {
        Some(process) => process
            .child
            .try_wait()
            .map_err(|error| format!("cannot observe Eon Desktop: {error}"))?
            .is_some(),
        None => false,
    };
    if exited {
        *desktop = None;
    }
    Ok(exited)
}

struct RunningSession {
    number: usize,
    id: String,
    endpoint: PathBuf,
    record: PathBuf,
    identity: LiveIdentity,
    lease: UnixStream,
    child: Option<Child>,
}

fn start_orbit(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    session_id: &str,
    component_generation: &str,
    child: &[OsString],
    deadline: Instant,
) -> Result<RunningSession, String> {
    let run_id = request_id();
    let record_path = artifact_path(socket, ".record");
    let mut orbit_command = orbit_command(
        programs,
        config,
        socket,
        session_id,
        &run_id,
        component_generation,
        child,
    )?;
    let mut orbit = orbit_command
        .spawn()
        .map_err(|error| format!("cannot launch Sessions: {error}"))?;
    loop {
        if let Some(status) = orbit
            .try_wait()
            .map_err(|error| format!("cannot observe Sessions startup: {error}"))?
        {
            return Err(format!(
                "Sessions exited before publishing Ready (status {})",
                status_code(status)
            ));
        }
        match read_management_record(&record_path) {
            Ok(Some(RecordSnapshot {
                object,
                record: ManagementRecord::Live(identity),
            })) => {
                let (number, endpoint) = validate_management_identity(
                    &identity,
                    component_generation,
                    Some(&run_id),
                    true,
                )?;
                if identity.session_id != session_id || endpoint != socket {
                    return Err("Sessions Ready identity does not match its launch".into());
                }
                let mut session = acquire_management(
                    ManagedCandidate {
                        number,
                        endpoint,
                        record_path,
                        record: object,
                        identity,
                    },
                    deadline,
                )?;
                session.child = Some(orbit);
                return Ok(session);
            }
            Ok(Some(RecordSnapshot {
                record: ManagementRecord::Tombstone(_),
                ..
            })) => return Err("Sessions ended before its management lease was acquired".into()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                stop(&mut orbit);
                return Err("Sessions did not publish Ready within five seconds".into());
            }
            Err(error) => {
                stop(&mut orbit);
                return Err(error);
            }
        }
    }
}

fn orbit_command(
    programs: &Programs,
    config: &Path,
    socket: &Path,
    session_id: &str,
    run_id: &str,
    component_generation: &str,
    child: &[OsString],
) -> Result<Command, String> {
    let mut orbit_command = Command::new(&programs.orbit);
    orbit_command
        .arg("serve")
        .arg(socket)
        .arg("--management-v1")
        .arg(session_id)
        .arg(run_id)
        .arg(component_generation)
        .arg("--ansi-palette-v1")
        .arg(EON_ANSI_PALETTE)
        .arg("--")
        .env("EON_CONFIG_HOME", config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(session_bin) = &programs.session_bin {
        orbit_command.env("PATH", managed_environment::session_path(session_bin)?);
    }
    if child.is_empty() {
        orbit_command.args(managed_environment::shell_command(config)?);
    } else {
        orbit_command.args(child);
    }
    Ok(orbit_command)
}

fn endpoint_removed(path: &Path, expected: ObjectIdentity) -> Result<bool, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Ok(metadata) if object_identity(&metadata) == expected => Ok(false),
        Ok(_) => Err(format!(
            "Sessions endpoint {} was replaced during cleanup",
            path.display()
        )),
        Err(error) => Err(format!(
            "cannot observe Sessions endpoint cleanup {}: {error}",
            path.display()
        )),
    }
}

fn wait_and_finalize_tombstone(
    session: &mut RunningSession,
    reason: TerminationReason,
    response: Option<&Tombstone>,
    deadline: Instant,
) -> Result<ProcessOutcome, String> {
    let (tombstone, record_object) = loop {
        let snapshot = read_management_record(&session.record)?.ok_or_else(|| {
            format!(
                "ended Sessions record {} disappeared",
                session.record.display()
            )
        })?;
        match snapshot.record {
            ManagementRecord::Live(identity) if identity == session.identity => {
                operation_timeout(deadline, "Sessions tombstone reconciliation")?;
                thread::sleep(Duration::from_millis(25));
            }
            ManagementRecord::Tombstone(tombstone)
                if tombstone.identity == session.identity && tombstone.reason == reason =>
            {
                if response.is_some_and(|response| response != &tombstone) {
                    return Err("Sessions stop result differs from its terminal record".into());
                }
                break (tombstone, snapshot.object);
            }
            _ => return Err("Sessions terminal record does not match the acquired run".into()),
        }
    };
    validate_management_identity(
        &tombstone.identity,
        &session.identity.component_generation,
        Some(&session.identity.run_id),
        false,
    )?;
    let management_path =
        PathBuf::from(OsString::from_vec(session.identity.management.path.clone()));
    loop {
        if endpoint_removed(&session.endpoint, session.identity.presentation.object)?
            && endpoint_removed(&management_path, session.identity.management.object)?
        {
            break;
        }
        operation_timeout(deadline, "Sessions endpoint cleanup")?;
        thread::sleep(Duration::from_millis(25));
    }
    if let Some(child) = &mut session.child {
        loop {
            if child
                .try_wait()
                .map_err(|error| format!("cannot reap ended Sessions: {error}"))?
                .is_some()
            {
                break;
            }
            operation_timeout(deadline, "Sessions process reaping")?;
            thread::sleep(Duration::from_millis(25));
        }
    }
    cleanup_record(&session.record, record_object)?;
    Ok(tombstone.outcome)
}

fn session_finished(session: &mut RunningSession) -> Result<Option<i32>, String> {
    let ended = match session.lease.read(&mut [0]) {
        Ok(0) => true,
        Ok(_) => return Err("Sessions sent an unsolicited management result".into()),
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => false,
        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return Ok(None),
        Err(_) => true,
    };
    if !ended {
        return Ok(None);
    }
    let outcome = wait_and_finalize_tombstone(
        session,
        TerminationReason::NaturalExit,
        None,
        Instant::now() + SESSION_START_TIMEOUT,
    )?;
    Ok(Some(match outcome {
        ProcessOutcome::ExitCode(code) => code,
        ProcessOutcome::Signal(_) => 1,
    }))
}

fn stop_managed_sessions(
    sessions: &mut [RunningSession],
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let deadline = Instant::now() + timeout;
    let request = management::encode_client_message(&ManagementClientMessage::Stop)
        .map_err(|error| format!("cannot encode Sessions stop: {error}"))?;
    let mut writes = Vec::with_capacity(sessions.len());
    for session in sessions.iter_mut() {
        let result = (|| {
            let remaining = operation_timeout(deadline, "Sessions stop")?;
            session
                .lease
                .set_nonblocking(false)
                .and_then(|()| session.lease.set_read_timeout(Some(remaining)))
                .and_then(|()| session.lease.set_write_timeout(Some(remaining)))
                .map_err(|error| format!("cannot bound Sessions stop: {error}"))?;
            session
                .lease
                .write_all(&request)
                .map_err(|error| format!("cannot send Sessions stop: {error}"))
        })();
        writes.push(result);
    }

    let mut errors = Vec::new();
    for (session, write) in sessions.iter_mut().zip(writes) {
        let response = if write.is_ok() {
            let remaining = operation_timeout(deadline, "Sessions stop response");
            match remaining.and_then(|remaining| {
                session
                    .lease
                    .set_read_timeout(Some(remaining))
                    .map_err(|error| format!("cannot bound Sessions stop response: {error}"))?;
                read_management_response(&mut session.lease)
            }) {
                Ok(ManagementServerMessage::Stopped(tombstone))
                    if tombstone.identity == session.identity
                        && tombstone.reason == TerminationReason::ExplicitStop =>
                {
                    Some(Ok(tombstone))
                }
                Ok(ManagementServerMessage::Failure(failure)) => Some(Err(format!(
                    "Sessions rejected stop for {}: {}",
                    session.id, failure.detail
                ))),
                Ok(_) => Some(Err(format!(
                    "Sessions returned the wrong stop result for {}",
                    session.id
                ))),
                Err(_) => None,
            }
        } else {
            None
        };
        let result = match response {
            Some(Ok(tombstone)) => wait_and_finalize_tombstone(
                session,
                TerminationReason::ExplicitStop,
                Some(&tombstone),
                deadline,
            ),
            Some(Err(error)) => Err(error),
            None => wait_and_finalize_tombstone(
                session,
                TerminationReason::ExplicitStop,
                None,
                deadline,
            ),
        };
        if let Err(error) = result {
            errors.push(format!("{}: {error}", session.id));
        }
    }
    if errors.is_empty() {
        Ok(sessions.iter().map(|session| session.id.clone()).collect())
    } else {
        Err(errors.join("; "))
    }
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

fn open_supervisor_startup_lock(path: &Path) -> Result<fs::File, String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| format!("cannot open Eon startup lock {}: {error}", path.display()))?;
    let metadata = file.metadata().map_err(|error| {
        format!(
            "cannot inspect Eon startup lock {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file() || metadata.uid() != effective_uid() {
        return Err(format!(
            "Eon startup lock {} must be an owned regular file",
            path.display()
        ));
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| {
            format!(
                "cannot protect Eon startup lock {}: {error}",
                path.display()
            )
        })?;
    Ok(file)
}

fn lock_supervisor_startup(path: &Path) -> Result<fs::File, String> {
    let file = open_supervisor_startup_lock(path)?;
    loop {
        match file.lock() {
            Ok(()) => break,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(format!(
                    "cannot lock Eon supervisor startup {}: {error}",
                    path.display()
                ));
            }
        }
    }
    Ok(file)
}

fn try_lock_supervisor_startup(path: &Path) -> Option<fs::File> {
    let file = open_supervisor_startup_lock(path).ok()?;
    file.try_lock().ok()?;
    Some(file)
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
    state: &mut SupervisorState,
    programs: &Programs,
    config: &Path,
    socket_timeout: Duration,
    generation: &str,
) -> Result<bool, String> {
    match listener.accept() {
        Ok((stream, _)) => Ok(handle_control_client(
            stream,
            state,
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
    state: &mut SupervisorState,
    programs: &Programs,
    config: &Path,
    socket_timeout: Duration,
    generation: &str,
) -> bool {
    let mode = if state.workspace.is_some() {
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
            action: Action::InspectRuntime | Action::InspectPresentation,
            ..
        }) => match runtime_status(generation, &state.sessions, mode) {
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
            action: Action::Present {
                workspace: expected,
            },
            ..
        }) if expected != (mode == LaunchMode::Workspace) => (
            ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                "launch-mode-mismatch",
                format!("supervisor owns {} mode", mode.name()),
            ))),
            false,
        ),
        Ok(Request {
            action: Action::Present { .. },
            ..
        }) => {
            let result = reap_desktop(&mut state.venus).and_then(|_| {
                if let Some(venus) = &state.venus {
                    return venus.present();
                }
                let session = state
                    .sessions
                    .first()
                    .ok_or_else(|| "supervisor has no live Session to present".to_string())?;
                let terminal = managed_environment::terminal_presentation(config)?;
                state.venus = Some(start_venus(
                    programs,
                    config,
                    &session.endpoint,
                    mode,
                    terminal,
                )?);
                Ok(())
            });
            match result.and_then(|()| runtime_status(generation, &state.sessions, mode)) {
                Ok(runtime) => (
                    ControlResponse::Lifecycle(LifecycleResponse::Runtime(runtime)),
                    false,
                ),
                Err(detail) => (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "presentation-unavailable",
                        detail,
                    ))),
                    false,
                ),
            }
        }
        Ok(Request {
            action: Action::Stop { generation: target },
            ..
        }) if target == generation => {
            match stop_managed_sessions(&mut state.sessions, socket_timeout) {
                Ok(sessions) => (
                    ControlResponse::Lifecycle(LifecycleResponse::Stopped(Stopped {
                        generation: generation.into(),
                        sessions,
                    })),
                    true,
                ),
                Err(detail) => (
                    ControlResponse::Lifecycle(LifecycleResponse::Failure(failure(
                        "stop-failed",
                        detail,
                    ))),
                    false,
                ),
            }
        }
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
        Ok(request) => match state.workspace.as_mut() {
            Some(workspace) => match workspace.dispatch(&request.id, request.action, |session| {
                let component_generation = state
                    .sessions
                    .first()
                    .ok_or("supervisor has no component generation")?
                    .identity
                    .component_generation
                    .clone();
                state.sessions.push(start_orbit(
                    programs,
                    config,
                    &session.endpoint,
                    &session.id,
                    &component_generation,
                    &[],
                    Instant::now() + socket_timeout,
                )?);
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
                    "EonTerm has no Eon workspace",
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
                LaunchMode::Workspace => "supervisor accepts EONW v1 presentation requests",
                LaunchMode::Terminal => "supervisor owns one EonTerm Session",
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

fn socket_identity_from(metadata: &fs::Metadata) -> SocketIdentity {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

fn socket_identity(socket: &Path) -> Result<Option<SocketIdentity>, String> {
    match fs::symlink_metadata(socket) {
        Ok(metadata) => Ok(Some(socket_identity_from(&metadata))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot inspect socket {}: {error}",
            socket.display()
        )),
    }
}

fn remove_socket_if_identity(path: &Path, identity: SocketIdentity) {
    if socket_identity(path).is_ok_and(|current| current == Some(identity)) {
        let _ = fs::remove_file(path);
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
        Action, Availability, EON_ANSI_PALETTE, Failure, LaunchMode, LifecycleResponse, Programs,
        Response, Runtime, VERSION, create_control_listener, current_generation,
        discover_generations, effective_uid, encode_lifecycle_response, encode_response,
        generation_directory, generation_id, lock_supervisor_startup, orbit_command,
        prepare_configuration, prepare_runtime, probe_presentable_runtime, read_control_request,
        remove_socket_if_identity, session_number, socket_identity, valid_generation,
        venus_command, xdg_path,
    };
    use std::{
        ffi::{OsStr, OsString},
        fs,
        io::Write,
        os::unix::{
            fs::{MetadataExt, PermissionsExt, symlink},
            net::UnixListener,
        },
        path::{Path, PathBuf},
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
        thread,
        time::Duration,
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

    fn terminal_presentation(
        background_opacity: f32,
        background_blur: bool,
    ) -> super::managed_environment::TerminalConfig {
        super::managed_environment::TerminalConfig {
            background_opacity,
            background_blur,
        }
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
    fn session_identity_is_positive_canonical_decimal() {
        assert_eq!(session_number("session-1"), Some(1));
        assert_eq!(session_number("session-256"), Some(256));
        for invalid in [
            "session-0",
            "session-01",
            "session-",
            "session-a",
            "Session-1",
            "pane-1",
            "session-184467440737095516160",
        ] {
            assert_eq!(session_number(invalid), None, "accepted {invalid}");
        }
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

        let held = lock_supervisor_startup(&root.join("startup.lock")).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let inspection_root = root.clone();
        let inspection = thread::spawn(move || {
            sender
                .send(discover_generations(
                    &inspection_root,
                    &current_generation().unwrap(),
                ))
                .unwrap();
        });
        let result = receiver.recv_timeout(Duration::from_secs(2));
        let preserved = dead.exists();
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

        discover_generations(&root, &current_generation().unwrap()).unwrap();
        assert!(!dead.exists());
        assert!(outside.is_dir());

        fs::remove_file(generations.join(linked_id)).unwrap();
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn old_supervisor_remains_inspectable_but_not_presentable() {
        let root = temporary_directory();
        let socket = root.join("eon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let server = thread::spawn(move || {
            let responses = [
                (
                    Action::InspectPresentation,
                    encode_response(&Response::Failure(Failure {
                        code: "malformed-action".into(),
                        detail: "unknown EONW action tag 11".into(),
                    }))
                    .unwrap(),
                ),
                (
                    Action::InspectRuntime,
                    encode_lifecycle_response(&LifecycleResponse::Runtime(Runtime {
                        generation: "g1-0123456789abcdef0123456789abcdef".into(),
                        eon_version: "0.1.0".into(),
                        workspace_protocol: VERSION,
                        component_report: "old components".into(),
                        sessions: vec!["session-1".into()],
                        attach: Availability {
                            available: true,
                            reason: "older client attachment".into(),
                        },
                        stop: Availability {
                            available: true,
                            reason: "generation-aware supervisor".into(),
                        },
                    }))
                    .unwrap(),
                ),
            ];
            for (action, response) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                assert_eq!(read_control_request(&mut stream).unwrap().action, action);
                stream.write_all(&response).unwrap();
            }
        });

        let runtime = probe_presentable_runtime(&socket).unwrap();
        assert_eq!(runtime.sessions, ["session-1"]);
        assert!(!runtime.attach.available);
        assert_eq!(
            runtime.attach.reason,
            "supervisor does not support presentation requests"
        );
        assert!(runtime.stop.available);

        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn command_environment<'a>(command: &'a Command, name: &str) -> Option<Option<&'a OsStr>> {
        command
            .get_envs()
            .find(|(variable, _)| *variable == name)
            .map(|(_, value)| value)
    }

    #[test]
    fn orbit_uses_configured_argv_only_for_default_sessions() {
        let root = temporary_directory();
        let programs = Programs {
            orbit: "/managed/orbit".into(),
            venus: "/managed/venus".into(),
            session_bin: Some("/managed/bin".into()),
            venus_decorations: true,
        };
        let config = root.as_path();
        let socket = Path::new("/runtime/orbit.sock");
        fs::write(
            root.join("config.toml"),
            "[shell]\ncommand = [\"eon-fish\", \"--no-config\"]\n",
        )
        .unwrap();

        let default = orbit_command(
            &programs,
            config,
            socket,
            "session-1",
            "run-1",
            "component-1",
            &[],
        )
        .unwrap();
        assert_eq!(
            default.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "serve",
                "/runtime/orbit.sock",
                "--management-v1",
                "session-1",
                "run-1",
                "component-1",
                "--ansi-palette-v1",
                EON_ANSI_PALETTE,
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
            "session-2",
            "run-2",
            "component-2",
            &["codex".into(), "--model".into(), "test".into()],
        )
        .unwrap();
        assert_eq!(
            explicit.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "serve",
                "/runtime/orbit.sock",
                "--management-v1",
                "session-2",
                "run-2",
                "component-2",
                "--ansi-palette-v1",
                EON_ANSI_PALETTE,
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
        let mut programs = Programs {
            orbit: "/managed/orbit".into(),
            venus: "/managed/venus".into(),
            session_bin: None,
            venus_decorations: true,
        };
        let config = Path::new("/config/eon");
        let socket = Path::new("/runtime/orbit.sock");

        let workspace = venus_command(
            &programs,
            config,
            socket,
            LaunchMode::Workspace,
            terminal_presentation(0.88, true),
        );
        assert_eq!(
            workspace.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "--background-opacity",
                "0.88",
                "--background-blur",
                "/runtime/orbit.sock",
                "/runtime/eon.sock",
            ]
            .map(OsString::from)
        );

        let terminal = venus_command(
            &programs,
            config,
            socket,
            LaunchMode::Terminal,
            terminal_presentation(1.0, false),
        );
        assert_eq!(
            terminal.get_args().map(OsString::from).collect::<Vec<_>>(),
            ["--background-opacity", "1", "/runtime/orbit.sock"].map(OsString::from)
        );

        programs.venus_decorations = false;
        let undecorated = venus_command(
            &programs,
            config,
            socket,
            LaunchMode::Terminal,
            terminal_presentation(0.0, true),
        );
        assert_eq!(
            undecorated
                .get_args()
                .map(OsString::from)
                .collect::<Vec<_>>(),
            [
                "--no-decorations",
                "--background-opacity",
                "0",
                "--background-blur",
                "/runtime/orbit.sock",
            ]
            .map(OsString::from)
        );
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

    #[test]
    fn socket_removal_is_bound_to_the_observed_identity() {
        let root = temporary_directory();
        let socket = root.join("eon.sock");
        let stale = UnixListener::bind(&socket).unwrap();
        let observed = socket_identity(&socket).unwrap().unwrap();
        drop(stale);
        fs::remove_file(&socket).unwrap();
        let _replacement = UnixListener::bind(&socket).unwrap();

        remove_socket_if_identity(&socket, observed);

        assert!(std::os::unix::net::UnixStream::connect(&socket).is_ok());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_lock_serializes_stale_cleanup_and_bind() {
        let root = temporary_directory();
        let socket = root.join("eon.sock");
        let startup_lock = root.join("startup.lock");
        drop(std::os::unix::net::UnixListener::bind(&socket).unwrap());
        let held = lock_supervisor_startup(&startup_lock).unwrap();
        let contender = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&startup_lock)
            .unwrap();
        assert!(matches!(
            contender.try_lock(),
            Err(fs::TryLockError::WouldBlock)
        ));

        fs::remove_file(&socket).unwrap();
        let replacement = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        drop(held);
        contender.lock().unwrap();

        let error = create_control_listener(&socket).err().unwrap();
        assert!(error.contains("already active"));
        assert!(std::os::unix::net::UnixStream::connect(&socket).is_ok());
        drop(replacement);
        fs::remove_dir_all(root).unwrap();
    }
}
