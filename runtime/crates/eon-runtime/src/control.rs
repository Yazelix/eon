use super::{
    supervisor::{LaunchMode, SESSION_START_TIMEOUT, effective_uid, request_id},
    workspace,
};
use eon_workspace_protocol::v12::{
    Action, Error as ProtocolError, Failure, HEADER_BYTES, LifecycleResponse, MAX_DETAIL_BYTES,
    Request, Response, Runtime, VERSION, WorkspaceAction, declared_message_len,
    decode_lifecycle_response, decode_request, decode_response, encode_lifecycle_response,
    encode_request, encode_response,
};
use eon_workspace_protocol::{v2, v3, v4, v5, v6, v7, v8, v9, v10, v11};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::Duration,
};

const CONTROL_TIMEOUT: Duration = SESSION_START_TIMEOUT.saturating_add(Duration::from_secs(1));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EndpointFailureKind {
    Dead,
    Incompatible,
    UnsupportedVersion(u16),
    InvalidAction,
    Unreachable,
    Corrupt,
}

#[derive(Debug)]
pub(super) struct EndpointFailure {
    pub(super) kind: EndpointFailureKind,
    pub(super) detail: String,
}

impl EndpointFailure {
    pub(super) fn new(kind: EndpointFailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

pub(super) enum ControlResponse {
    Workspace(Response),
    Lifecycle(LifecycleResponse),
}

pub(super) fn send_action(
    socket: &Path,
    action: Action,
) -> Result<ControlResponse, EndpointFailure> {
    let action = prepare_action(action)?;
    send_prepared_action(connect_control(socket)?, action)
}

pub(super) fn connect_control(socket: &Path) -> Result<UnixStream, EndpointFailure> {
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

pub(super) fn send_action_on(
    stream: UnixStream,
    action: Action,
) -> Result<ControlResponse, EndpointFailure> {
    send_prepared_action(stream, prepare_action(action)?)
}

fn prepare_action(action: Action) -> Result<(bool, Vec<u8>, Duration), EndpointFailure> {
    let timeout = result_timeout(&action);
    let lifecycle = matches!(
        action,
        Action::Workspace(
            WorkspaceAction::InspectRuntime
                | WorkspaceAction::InspectPresentation
                | WorkspaceAction::Present { .. }
                | WorkspaceAction::Stop { .. }
        )
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
    Ok((lifecycle, request, timeout))
}

fn result_timeout(action: &Action) -> Duration {
    if matches!(action, Action::Workspace(WorkspaceAction::Stop { .. })) {
        // One in-flight operation may precede the bounded managed Stop.
        CONTROL_TIMEOUT.saturating_add(SESSION_START_TIMEOUT)
    } else {
        CONTROL_TIMEOUT
    }
}

fn send_prepared_action(
    stream: UnixStream,
    (lifecycle, request, timeout): (bool, Vec<u8>, Duration),
) -> Result<ControlResponse, EndpointFailure> {
    let response = exchange(stream, &request, timeout)?;
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

fn exchange(
    mut stream: UnixStream,
    request: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, EndpointFailure> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| io_endpoint_failure(error, "cannot bound Eon action result"))?;
    stream
        .write_all(request)
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
    Ok(response)
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
        match error {
            ProtocolError::UnsupportedVersion { version } => {
                EndpointFailureKind::UnsupportedVersion(version)
            }
            _ => EndpointFailureKind::Corrupt,
        },
        format!("invalid EONW response: {error}"),
    )
}

fn runtime_response(response: ControlResponse) -> Result<Runtime, EndpointFailure> {
    match response {
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

pub(super) fn probe_generation_runtime(socket: &Path) -> Result<Runtime, EndpointFailure> {
    match probe_presentable_runtime(socket) {
        Ok(runtime) if runtime.workspace_protocol == VERSION => Ok(runtime),
        Ok(runtime) => Err(EndpointFailure::new(
            EndpointFailureKind::Incompatible,
            format!(
                "supervisor reports EONW {}, current Eon requires EONW {}",
                runtime.workspace_protocol, VERSION
            ),
        )),
        Err(error) => {
            let EndpointFailureKind::UnsupportedVersion(version) = error.kind else {
                return Err(error);
            };
            if !(2..VERSION).contains(&version) {
                return Err(error);
            }
            let runtime = runtime_response(send_generation_action_on(
                connect_control(socket)?,
                version,
                WorkspaceAction::InspectRuntime,
            )?)?;
            if runtime.workspace_protocol != version {
                return Err(EndpointFailure::new(
                    EndpointFailureKind::Corrupt,
                    format!(
                        "supervisor reports EONW {} over EONW {version}",
                        runtime.workspace_protocol
                    ),
                ));
            }
            Ok(runtime)
        }
    }
}

pub(super) fn send_generation_action_on(
    stream: UnixStream,
    version: u16,
    action: WorkspaceAction,
) -> Result<ControlResponse, EndpointFailure> {
    if version == VERSION {
        return send_action_on(stream, Action::Workspace(action));
    }
    let timeout = result_timeout(&Action::Workspace(action.clone()));
    let id = request_id();
    let request = match version {
        2 => v2::encode_request(&v2::Request {
            id,
            action: match action {
                WorkspaceAction::InspectRuntime => v2::Action::InspectRuntime,
                WorkspaceAction::Stop { generation } => v2::Action::Stop { generation },
                _ => {
                    return Err(EndpointFailure::new(
                        EndpointFailureKind::InvalidAction,
                        "older EONW lifecycle only supports inspection and Stop",
                    ));
                }
            },
        }),
        3 => v3::encode_request(&v3::Request { id, action }),
        4 => v4::encode_request(&v4::Request { id, action }),
        5 => v5::encode_request(&v5::Request {
            id,
            action: v5::Action::Workspace(action),
        }),
        6 => v6::encode_request(&v6::Request {
            id,
            action: v6::Action::Workspace(action),
        }),
        7 => v7::encode_request(&v7::Request {
            id,
            action: v7::Action::Workspace(action),
        }),
        8 => v8::encode_request(&v8::Request {
            id,
            action: v8::Action::Workspace(action),
        }),
        9 => v9::encode_request(&v9::Request {
            id,
            action: v9::Action::Workspace(action),
        }),
        10 => v10::encode_request(&v10::Request {
            id,
            action: v10::Action::Workspace(action),
        }),
        11 => v11::encode_request(&v11::Request {
            id,
            action: v11::Action::Workspace(action),
        }),
        _ => {
            return Err(EndpointFailure::new(
                EndpointFailureKind::Incompatible,
                format!("EONW v{version} has no supported generation lifecycle codec"),
            ));
        }
    }
    .map_err(|error| {
        EndpointFailure::new(
            EndpointFailureKind::InvalidAction,
            format!("cannot encode Eon generation action: {error}"),
        )
    })?;
    let response = exchange(stream, &request, timeout)?;
    let decoded = match version {
        2 => v2::decode_lifecycle_response(&response),
        3 => v3::decode_lifecycle_response(&response),
        4 => v4::decode_lifecycle_response(&response),
        5 => v5::decode_lifecycle_response(&response),
        6 => v6::decode_lifecycle_response(&response),
        7 => v7::decode_lifecycle_response(&response),
        8 => v8::decode_lifecycle_response(&response),
        9 => v9::decode_lifecycle_response(&response),
        10 => v10::decode_lifecycle_response(&response),
        11 => v11::decode_lifecycle_response(&response),
        _ => unreachable!(),
    };
    decoded
        .map(ControlResponse::Lifecycle)
        .map_err(protocol_endpoint_failure)
}

pub(super) fn probe_presentable_runtime(socket: &Path) -> Result<Runtime, EndpointFailure> {
    runtime_response(send_action(
        socket,
        Action::Workspace(WorkspaceAction::InspectPresentation),
    )?)
}

pub(super) fn probe_launch_mode(socket: &Path) -> Result<LaunchMode, EndpointFailure> {
    match send_action(socket, Action::Workspace(WorkspaceAction::Inspect))? {
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

pub(super) struct ControlListener {
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
}

impl ControlListener {
    pub(super) fn bind(path: &Path) -> Result<Self, String> {
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
            Ok(metadata)
                if metadata.file_type().is_socket() && metadata.uid() == effective_uid() =>
            {
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
        let listener = UnixListener::bind(path).map_err(|error| {
            format!("cannot bind Eon control socket {}: {error}", path.display())
        })?;
        let identity = socket_identity(path)?
            .ok_or_else(|| format!("Eon control socket {} disappeared", path.display()))?;
        let control = Self {
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

    pub(super) fn accept(
        &self,
        dispatch: impl FnOnce(Request) -> (ControlResponse, bool),
    ) -> Result<bool, String> {
        match self.listener.accept() {
            Ok((stream, _)) => Ok(handle_client(stream, dispatch)),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
            Err(error) => Err(format!("cannot accept Eon control client: {error}")),
        }
    }
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

fn handle_client(
    mut stream: UnixStream,
    dispatch: impl FnOnce(Request) -> (ControlResponse, bool),
) -> bool {
    let timeout = Some(Duration::from_millis(250));
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return false;
    }
    let (response, shutdown) = match read_request(&mut stream) {
        Ok(request) => dispatch(request),
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
        let _ = stream.write_all(&encoded);
    }
    shutdown
}

fn read_request(stream: &mut impl Read) -> Result<Request, Failure> {
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

pub(super) fn report_failure(failure: &Failure, json: bool) -> Result<i32, String> {
    if json {
        write_stdout(workspace::failure_json(failure))?;
    } else {
        eprint!("{}", workspace::failure_human(failure));
    }
    Ok(2)
}

pub(super) fn write_stdout(output: impl AsRef<[u8]>) -> Result<(), String> {
    match std::io::stdout().lock().write_all(output.as_ref()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(error) => Err(format!("cannot write stdout: {error}")),
    }
}

pub(super) fn failure(code: impl Into<String>, detail: impl Into<String>) -> Failure {
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

pub(super) type SocketIdentity = (u64, u64, i64, i64);

pub(super) fn socket_identity_from(metadata: &fs::Metadata) -> SocketIdentity {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

pub(super) fn socket_identity(socket: &Path) -> Result<Option<SocketIdentity>, String> {
    match fs::symlink_metadata(socket) {
        Ok(metadata) => Ok(Some(socket_identity_from(&metadata))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "cannot inspect socket {}: {error}",
            socket.display()
        )),
    }
}

pub(super) fn remove_socket_if_identity(path: &Path, identity: SocketIdentity) {
    if socket_identity(path).is_ok_and(|current| current == Some(identity)) {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::{ControlListener, remove_socket_if_identity, socket_identity};
    use crate::supervisor::temporary_directory;
    use eon_workspace_protocol::v12::{Availability, Runtime, WorkspaceAction};
    use std::{
        fs,
        io::Write,
        os::unix::net::{UnixListener, UnixStream},
    };

    #[test]
    fn previous_supervisors_keep_inspection_and_stop_without_presentation() {
        use super::{ControlResponse, send_generation_action_on};
        use eon_workspace_protocol::{v7, v8};
        use std::io::Read;
        for version in [7, 8] {
            let generation = "g1-0123456789abcdef0123456789abcdef".to_string();
            let runtime = Runtime {
                generation: generation.clone(),
                eon_version: "0.1.0".into(),
                workspace_protocol: version,
                component_report: "old components".into(),
                sessions: vec!["session-1".into()],
                attach: Availability {
                    available: true,
                    reason: "original executable".into(),
                },
                stop: Availability {
                    available: true,
                    reason: "generation-aware".into(),
                },
            };
            for (action, expected) in [
                (
                    WorkspaceAction::InspectRuntime,
                    v7::LifecycleResponse::Runtime(runtime),
                ),
                (
                    WorkspaceAction::Stop {
                        generation: generation.clone(),
                    },
                    v7::LifecycleResponse::Stopped(v7::Stopped {
                        generation: generation.clone(),
                        sessions: vec!["session-1".into()],
                    }),
                ),
            ] {
                let (client, mut server) = UnixStream::pair().unwrap();
                server
                    .set_read_timeout(Some(std::time::Duration::from_secs(1)))
                    .unwrap();
                let bytes = if version == 7 {
                    v7::encode_lifecycle_response(&expected)
                } else {
                    v8::encode_lifecycle_response(&expected)
                }
                .unwrap();
                server.write_all(&bytes).unwrap();
                let response = send_generation_action_on(client, version, action.clone())
                    .expect("v7 lifecycle remains usable");
                assert!(
                    matches!(response, ControlResponse::Lifecycle(actual) if actual == expected)
                );
                let mut header = [0; v7::HEADER_BYTES];
                server.read_exact(&mut header).unwrap();
                let mut bytes = vec![0; v7::declared_message_len(&header).unwrap()];
                bytes[..header.len()].copy_from_slice(&header);
                server.read_exact(&mut bytes[header.len()..]).unwrap();
                assert_eq!(
                    if version == 7 {
                        v7::decode_request(&bytes)
                    } else {
                        v8::decode_request(&bytes)
                    }
                    .unwrap()
                    .action,
                    v7::Action::Workspace(action)
                );
            }
        }
    }

    #[test]
    fn listener_preserves_a_replacement_socket() {
        let root = temporary_directory();
        let socket = root.join("eon.sock");
        let control = ControlListener::bind(&socket).unwrap();
        fs::remove_file(&socket).unwrap();
        let _replacement = UnixListener::bind(&socket).unwrap();

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

        assert!(UnixStream::connect(&socket).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
}
