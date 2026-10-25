use super::{
    supervisor::{
        LaunchMode, PRESENTATION_ADMISSION_TIMEOUT, SESSION_START_TIMEOUT, effective_uid,
        request_id,
    },
    workspace,
};
use eon_workspace_protocol::v20::{
    Action, Error as ProtocolError, Failure, HEADER_BYTES, LifecycleResponse, MAX_DETAIL_BYTES,
    Request, Response, Runtime, VERSION, WorkspaceAction, declared_message_len,
    decode_lifecycle_response, decode_request, decode_response, encode_lifecycle_response,
    encode_request, encode_response,
};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

// A reply can wait behind one in-flight operation, the longest being startup: presentation
// admission, then the first terminal's start.
pub(super) const CONTROL_TIMEOUT: Duration = PRESENTATION_ADMISSION_TIMEOUT
    .saturating_add(SESSION_START_TIMEOUT)
    .saturating_add(Duration::from_secs(1));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EndpointFailureKind {
    Dead,
    Incompatible,
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
    send_action_from(Ok(socket), None, action)
}

/// Sends an action on behalf of the calling terminal `caller`, if any. The
/// action is validated before the endpoint, which may already be known missing.
pub(super) fn send_action_from(
    socket: Result<&Path, EndpointFailure>,
    caller: Option<String>,
    action: Action,
) -> Result<ControlResponse, EndpointFailure> {
    let action = prepare_action(caller, action)?;
    send_prepared_action(connect_control(socket?)?, action, None)
}

pub(super) fn validate_action(
    caller: Option<String>,
    action: Action,
) -> Result<(), EndpointFailure> {
    prepare_action(caller, action).map(|_| ())
}

/// Read-only discovery shares one deadline across connection, framing and decoding.
pub(super) fn send_action_from_until(
    socket: &Path,
    caller: Option<String>,
    action: Action,
    deadline: Instant,
) -> Result<ControlResponse, EndpointFailure> {
    let action = prepare_action(caller, action)?;
    let stream = connect_control_with_timeout(socket, time_left(deadline)?)?;
    send_prepared_action(stream, action, Some(deadline))
}

pub(super) fn missing_endpoint(socket: &Path) -> EndpointFailure {
    EndpointFailure::new(
        EndpointFailureKind::Dead,
        format!("endpoint {} is missing", socket.display()),
    )
}

pub(super) fn connect_control(socket: &Path) -> Result<UnixStream, EndpointFailure> {
    connect_control_with_timeout(socket, CONTROL_TIMEOUT)
}

fn connect_control_with_timeout(
    socket: &Path,
    timeout: Duration,
) -> Result<UnixStream, EndpointFailure> {
    let metadata = match fs::symlink_metadata(socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(missing_endpoint(socket));
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

    let stream = super::sessions::unix_connect_with_timeout(socket, timeout).map_err(|error| {
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
    send_prepared_action(stream, prepare_action(None, action)?, None)
}

fn prepare_action(
    caller: Option<String>,
    action: Action,
) -> Result<(bool, Vec<u8>, Duration), EndpointFailure> {
    let timeout = if matches!(action, Action::Workspace(WorkspaceAction::Stop { .. })) {
        // One in-flight operation may precede the bounded managed Stop.
        CONTROL_TIMEOUT.saturating_add(SESSION_START_TIMEOUT)
    } else {
        CONTROL_TIMEOUT
    };
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
        caller,
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

fn send_prepared_action(
    mut stream: UnixStream,
    (lifecycle, request, timeout): (bool, Vec<u8>, Duration),
    deadline: Option<Instant>,
) -> Result<ControlResponse, EndpointFailure> {
    let deadline = deadline.unwrap_or_else(|| Instant::now() + timeout);
    let mut bytes = request.as_slice();
    while !bytes.is_empty() {
        stream
            .set_write_timeout(Some(time_left(deadline)?))
            .map_err(|error| io_endpoint_failure(error, "cannot bound Eon action"))?;
        match stream.write(bytes) {
            Ok(0) => {
                return Err(io_endpoint_failure(
                    std::io::ErrorKind::WriteZero.into(),
                    "cannot send Eon action",
                ));
            }
            Ok(written) => bytes = &bytes[written..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(io_endpoint_failure(error, "cannot send Eon action")),
        }
    }

    let mut read_exact = |mut bytes: &mut [u8]| -> Result<(), EndpointFailure> {
        while !bytes.is_empty() {
            stream
                .set_read_timeout(Some(time_left(deadline)?))
                .map_err(|error| io_endpoint_failure(error, "cannot bound Eon action result"))?;
            match stream.read(bytes) {
                Ok(0) => {
                    return Err(io_endpoint_failure(
                        std::io::ErrorKind::UnexpectedEof.into(),
                        "cannot read Eon action result",
                    ));
                }
                Ok(read) => bytes = &mut bytes[read..],
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => {
                    return Err(io_endpoint_failure(error, "cannot read Eon action result"));
                }
            }
        }
        Ok(())
    };

    let mut response = vec![0; HEADER_BYTES];
    read_exact(&mut response)?;
    let length = declared_message_len(&response).map_err(protocol_endpoint_failure)?;
    response.resize(length, 0);
    read_exact(&mut response[HEADER_BYTES..])?;
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

fn time_left(deadline: Instant) -> Result<Duration, EndpointFailure> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|left| !left.is_zero())
        .ok_or_else(|| {
            EndpointFailure::new(
                EndpointFailureKind::Unreachable,
                "supervisor inspection deadline exceeded",
            )
        })
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
    match error {
        ProtocolError::UnsupportedVersion { version } => EndpointFailure::new(
            EndpointFailureKind::Incompatible,
            format!(
                "supervisor uses EONW {version}, this Eon uses EONW {VERSION}; \
                 the Eon build that started it can attach or stop it"
            ),
        ),
        _ => EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            format!("invalid EONW response: {error}"),
        ),
    }
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
    validated_runtime(probe_presentable_runtime(socket)?)
}

pub(super) fn probe_generation_runtime_until(
    socket: &Path,
    deadline: Instant,
) -> Result<Runtime, EndpointFailure> {
    validated_runtime(runtime_response(send_action_from_until(
        socket,
        None,
        Action::Workspace(WorkspaceAction::InspectRuntime),
        deadline,
    )?)?)
}

fn validated_runtime(runtime: Runtime) -> Result<Runtime, EndpointFailure> {
    if runtime.workspace_protocol != VERSION {
        return Err(EndpointFailure::new(
            EndpointFailureKind::Incompatible,
            format!(
                "supervisor reports EONW {}, current Eon requires EONW {}",
                runtime.workspace_protocol, VERSION
            ),
        ));
    }
    Ok(runtime)
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
        ControlResponse::Workspace(Response::Ended(_)) | ControlResponse::Lifecycle(_) => {
            Err(EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                "supervisor returned the wrong EONW result for launch-mode inspection",
            ))
        }
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
    Ok(failure_status(&failure.code).into())
}

// Documented command exit statuses (EON-C10).
const FAILED: u8 = 1;
const USAGE: u8 = 2;
const MISSING: u8 = 3;
const UNREACHABLE: u8 = 4;

fn failure_status(code: &str) -> u8 {
    match code {
        "invalid-tab" | "malformed-action" => USAGE,
        "unknown-tab" | "unknown-pane" | "unknown-id" | "unknown-session" | "unknown-popup"
        | "stale-popup" | "stale-target" | "unknown-generation" | "unknown-caller"
        | "unknown-workspace" | "unknown-window" => MISSING,
        "missing-supervisor" | "supervisor-unavailable" => UNREACHABLE,
        _ => FAILED,
    }
}

/// A failed command: the message to report and its documented exit status.
#[derive(Debug)]
pub struct Error {
    pub status: u8,
    pub message: String,
}

impl Error {
    pub(super) fn usage(message: impl Into<String>) -> Self {
        Self {
            status: USAGE,
            message: message.into(),
        }
    }

    pub(super) fn missing(message: impl Into<String>) -> Self {
        Self {
            status: MISSING,
            message: message.into(),
        }
    }

    pub(super) fn unreachable(message: impl Into<String>) -> Self {
        Self {
            status: UNREACHABLE,
            message: message.into(),
        }
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self {
            status: FAILED,
            message,
        }
    }
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
    use std::{
        fs,
        io::Write,
        os::unix::fs::PermissionsExt,
        os::unix::net::{UnixListener, UnixStream},
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn discovery_deadline_covers_a_dribbling_response() {
        let root = temporary_directory();
        let socket = root.join("eon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            super::read_request(&mut stream).unwrap();
            let response = super::encode_response(&super::Response::Failure(super::failure(
                "unknown-caller",
                "not owned",
            )))
            .unwrap();
            for byte in response {
                if stream.write_all(&[byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let start = Instant::now();
        let error = super::send_action_from_until(
            &socket,
            Some("session-1.run".into()),
            super::Action::Workspace(super::WorkspaceAction::Inspect),
            start + Duration::from_millis(100),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind, super::EndpointFailureKind::Unreachable);
        assert!(start.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn workspace_refusals_use_the_existing_exit_status_classes() {
        for (code, expected) in [
            ("unknown-workspace", 3),
            ("ambiguous-workspace", 1),
            ("workspace-limit", 1),
            ("malformed-action", 2),
        ] {
            assert_eq!(
                super::report_failure(&super::failure(code, "refused"), true).unwrap(),
                expected
            );
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
