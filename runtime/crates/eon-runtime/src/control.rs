use super::{
    sessions::bound_io,
    subscriptions::Subscriber,
    supervisor::{LaunchMode, effective_uid, request_id},
    workspace,
};
use eon_workspace_protocol::stop;
use eon_workspace_protocol::v51::{
    Action, Error as ProtocolError, Failure, HEADER_BYTES, LifecycleResponse, MAX_DETAIL_BYTES,
    Request, Response, Runtime, VERSION, WorkspaceAction, declared_message_len,
    declared_response_len, decode_lifecycle_response, decode_request, decode_response,
    encode_lifecycle_response, encode_request, encode_response,
};
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::{UnixDatagram, UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) use eon_workspace_protocol::{CONTROL_TIMEOUT, STOP_TIMEOUT};

/// How long one client's request read or reply write may block the supervisor.
pub(super) const CLIENT_IO_TIMEOUT: Duration = Duration::from_millis(250);
/// How long the supervisor writes one reply in all before it drops the client.
/// A `History` reply is up to 8 MiB, so a client that drains it slowly would
/// otherwise hold the supervisor for as long as each write makes progress
/// within `CLIENT_IO_TIMEOUT`. Fixed, as the 2 s bound on a terminal read's
/// answer is: a performance budget for every pane's window actions.
pub(super) const REPLY_DEADLINE: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EndpointFailureKind {
    Starting,
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
    /// The EONW version an incompatible supervisor reported, when it did.
    pub(super) protocol: Option<u16>,
    /// Whether the whole request was written before the failure, so the
    /// supervisor may have carried it out.
    pub(super) dispatched: bool,
}

impl EndpointFailure {
    pub(super) fn new(kind: EndpointFailureKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
            protocol: None,
            dispatched: false,
        }
    }

    fn incompatible(protocol: u16, detail: String) -> Self {
        Self {
            protocol: Some(protocol),
            ..Self::new(EndpointFailureKind::Incompatible, detail)
        }
    }
}

// Snapshot payloads are already heap-owned; keep the direct value API.
#[allow(clippy::large_enum_variant)]
#[cfg_attr(test, derive(Debug, PartialEq))]
pub(super) enum ControlResponse {
    Workspace(Response),
    Lifecycle(LifecycleResponse),
}

/// What a control client sent: the Reserved Stop of a generation, or a
/// versioned request.
#[derive(Debug, PartialEq)]
pub(super) enum Incoming {
    Stop(String),
    Request(Request),
}

/// What the supervisor does with a request: reply and close the connection,
/// shutting down after a stopping reply, or keep the connection for an
/// event subscription (EON-C39).
#[allow(clippy::large_enum_variant)]
pub(super) enum Dispatched {
    Reply(ControlResponse, bool),
    /// The answer to a Reserved Stop, in its own framing.
    Stop(stop::Reply, bool),
    Subscribe(Box<Subscriber>),
    Deferred,
}

impl From<(ControlResponse, bool)> for Dispatched {
    fn from((response, shutdown): (ControlResponse, bool)) -> Self {
        Self::Reply(response, shutdown)
    }
}

/// A served control client.
pub(super) enum Accepted {
    /// No client was waiting.
    None,
    Replied {
        shutdown: bool,
    },
    /// The subscriber's connection, now its own.
    Subscribed(UnixStream, Box<Subscriber>),
    Deferred(UnixStream),
}

#[cfg(test)]
impl Accepted {
    pub(super) fn shutdown(&self) -> bool {
        matches!(self, Self::Replied { shutdown: true })
    }
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
            std::io::ErrorKind::NotFound => EndpointFailureKind::Dead,
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                EndpointFailureKind::Unreachable
            }
            // A live stream socket that fails a connect is starting (before
            // listen(2)) or busy (a full accept queue: macOS refuses, Linux
            // polls to POLLHUP with no error set).
            _ if matches!(
                probe_stream_socket(socket),
                Ok(StreamSocketProbe::LiveStream)
            ) =>
            {
                EndpointFailureKind::Unreachable
            }
            std::io::ErrorKind::ConnectionRefused => EndpointFailureKind::Dead,
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
    bound_io(&stream, stream.set_read_timeout(Some(CONTROL_TIMEOUT)))
        .and_then(|()| bound_io(&stream, stream.set_write_timeout(Some(CONTROL_TIMEOUT))))
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
    caller: Option<String>,
    action: Action,
) -> Result<ControlResponse, EndpointFailure> {
    send_prepared_action(stream, prepare_action(caller, action)?, None)
}

fn prepare_action(
    caller: Option<String>,
    action: Action,
) -> Result<(bool, Vec<u8>, Duration), EndpointFailure> {
    let timeout = if matches!(
        action,
        Action::Workspace(WorkspaceAction::ReloadPresentation { .. })
    ) {
        // Candidate, previous and own-build admission, plus bounded child teardown.
        eon_workspace_protocol::PRESENTATION_ADMISSION_TIMEOUT
            .saturating_mul(3)
            .saturating_add(eon_workspace_protocol::SESSION_CLEANUP_TIMEOUT.saturating_mul(4))
            .saturating_add(Duration::from_secs(1))
    } else {
        CONTROL_TIMEOUT
    };
    let lifecycle = matches!(
        action,
        Action::Workspace(
            WorkspaceAction::InspectRuntime
                | WorkspaceAction::InspectPresentation
                | WorkspaceAction::Present { .. }
                | WorkspaceAction::ReloadPresentation { .. }
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
    write_until(&mut stream, &request, deadline)?;
    // A supervisor carries out only a whole frame of its own version, so a
    // failure from here on may follow the request's effect.
    read_response(stream, lifecycle, deadline).map_err(|mut error| {
        error.dispatched = error.kind != EndpointFailureKind::Incompatible;
        error
    })
}

fn read_response(
    mut stream: UnixStream,
    lifecycle: bool,
    deadline: Instant,
) -> Result<ControlResponse, EndpointFailure> {
    let mut response = vec![0; HEADER_BYTES];
    read_until(&mut stream, &mut response, deadline)?;
    let length = declared_response_len(&response).map_err(protocol_endpoint_failure)?;
    response.resize(length, 0);
    read_until(&mut stream, &mut response[HEADER_BYTES..], deadline)?;
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

/// What a supervisor answered the Reserved Stop.
#[derive(Debug, PartialEq)]
pub(super) enum StopAnswer {
    Reply(stop::Reply),
    /// A reply in the framing of this EONW version, whose supervisor
    /// predates the Reserved Stop and stopped nothing.
    Predates(u16),
}

/// Sends the Reserved Stop of `generation` on `stream` (EON-C11). The reply's
/// first six bytes say whether its supervisor answered it.
pub(super) fn send_stop_on(
    stream: UnixStream,
    generation: &str,
) -> Result<StopAnswer, EndpointFailure> {
    send_stop_until(stream, generation, Instant::now() + STOP_TIMEOUT)
}

fn send_stop_until(
    mut stream: UnixStream,
    generation: &str,
    deadline: Instant,
) -> Result<StopAnswer, EndpointFailure> {
    let request = stop::encode_request(generation).map_err(|error| {
        EndpointFailure::new(
            EndpointFailureKind::InvalidAction,
            format!("cannot encode Eon Stop: {error}"),
        )
    })?;
    write_until(&mut stream, &request, deadline)?;
    let mut reply = vec![0; stop::HEADER_BYTES];
    read_until(&mut stream, &mut reply, deadline)?;
    let invalid = |error: stop::Error| {
        EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            format!("invalid Stop reply: {error}"),
        )
    };
    match stop::version(&reply) {
        Some(_) if stop::is_reserved(&reply) => {}
        Some(version) if version < stop::FIRST_VERSION => {
            return Ok(StopAnswer::Predates(version));
        }
        Some(version) => {
            return Err(EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                format!(
                    "supervisor of EONW {version} answered the reserved Stop in its \
                     versioned framing"
                ),
            ));
        }
        None => {
            return Err(EndpointFailure::new(
                EndpointFailureKind::Corrupt,
                "supervisor's Stop reply is not an EONW message",
            ));
        }
    }
    let length = stop::reply_len(&reply).map_err(invalid)?;
    reply.resize(length, 0);
    read_until(&mut stream, &mut reply[stop::HEADER_BYTES..], deadline)?;
    stop::decode_reply(&reply)
        .map(StopAnswer::Reply)
        .map_err(invalid)
}

fn write_until(
    stream: &mut UnixStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> Result<(), EndpointFailure> {
    while !bytes.is_empty() {
        bound_io(stream, stream.set_write_timeout(Some(time_left(deadline)?)))
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
    Ok(())
}

fn read_until(
    stream: &mut UnixStream,
    mut bytes: &mut [u8],
    deadline: Instant,
) -> Result<(), EndpointFailure> {
    while !bytes.is_empty() {
        bound_io(stream, stream.set_read_timeout(Some(time_left(deadline)?)))
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
        ProtocolError::UnsupportedVersion { version } => EndpointFailure::incompatible(
            version,
            if version < stop::FIRST_VERSION {
                format!(
                    "supervisor uses EONW {version}, this Eon uses EONW {VERSION}; it \
                     predates the reserved Stop, so the Eon build that started it can \
                     attach it and it is quit from inside its window"
                )
            } else {
                format!(
                    "supervisor uses EONW {version}, this Eon uses EONW {VERSION}; the \
                     Eon build that started it can attach it, and any Eon build of EONW \
                     {} or later can stop it",
                    stop::FIRST_VERSION
                )
            },
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

/// Inspects one generation for `caller`, the terminal running the command,
/// which its busy terminals then leave out (EON-C11).
pub(super) fn probe_generation_runtime_from(
    socket: &Path,
    caller: Option<String>,
) -> Result<Runtime, EndpointFailure> {
    validated_runtime(runtime_response(send_action_from(
        Ok(socket),
        caller,
        Action::Workspace(WorkspaceAction::InspectRuntime),
    )?)?)
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
        return Err(EndpointFailure::incompatible(
            runtime.workspace_protocol,
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
            if failure.code == "session-unavailable" =>
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
        ControlResponse::Workspace(
            Response::Ended(_)
            | Response::Confirm(_)
            | Response::Text(_)
            | Response::Sent(_)
            | Response::History(_)
            | Response::Editing(_),
        )
        | ControlResponse::Lifecycle(_) => Err(EndpointFailure::new(
            EndpointFailureKind::Corrupt,
            "supervisor returned the wrong EONW result for launch-mode inspection",
        )),
    }
}

/// Under the lifecycle lock, reclaim only an owned socket with no live peer.
pub(super) fn remove_stale_socket(path: &Path, what: &str) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() && metadata.uid() == effective_uid() => {
            metadata
        }
        Ok(_) => {
            return Err(format!(
                "{what} path {} must be an owned Unix socket",
                path.display()
            ));
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "cannot inspect {what} path {}: {error}",
                path.display()
            ));
        }
    };

    probe_stale_socket(path, what, socket_identity_from(&metadata))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StreamSocketProbe {
    Dead,
    Missing,
    LiveStream,
    LiveDatagram,
}

/// Probes `path` with a datagram connect so a live stream socket reports
/// `EPROTOTYPE` before any accept-queue check (on macOS a full stream
/// backlog refuses stream connects with `ECONNREFUSED`).
pub(super) fn probe_stream_socket(path: &Path) -> std::io::Result<StreamSocketProbe> {
    // Linux and macOS check the type before the accept queue: a live stream
    // gives EPROTOTYPE even when a stream connect would be refused.
    match UnixDatagram::unbound().and_then(|socket| socket.connect(path)) {
        Err(error) if error.kind() == ErrorKind::ConnectionRefused => Ok(StreamSocketProbe::Dead),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(StreamSocketProbe::Missing),
        Err(error) if error.raw_os_error() == Some(libc::EPROTOTYPE) => {
            Ok(StreamSocketProbe::LiveStream)
        }
        Ok(()) => Ok(StreamSocketProbe::LiveDatagram),
        Err(error) => Err(error),
    }
}

fn probe_stale_socket(path: &Path, what: &str, identity: SocketIdentity) -> Result<(), String> {
    match probe_stream_socket(path) {
        Ok(StreamSocketProbe::Dead) => {}
        Ok(StreamSocketProbe::Missing) => return Ok(()),
        Ok(StreamSocketProbe::LiveStream) => {
            return Err(format!(
                "an Eon supervisor is already active at {}",
                path.display()
            ));
        }
        Ok(StreamSocketProbe::LiveDatagram) => {
            return Err(format!(
                "{what} path {} is a Unix datagram socket",
                path.display()
            ));
        }
        Err(error) => {
            return Err(format!(
                "cannot inspect {what} socket {}: {error}",
                path.display()
            ));
        }
    }
    remove_socket_if_identity(path, identity).map_err(|error| {
        format!(
            "cannot remove stale {what} socket {}: {error}",
            path.display()
        )
    })
}

/// A private Unix listener at `path`: it refuses a live owner, replaces only
/// an owned dead socket, binds mode 0600 and nonblocking, and on drop removes
/// the socket only while it is still the one it bound. `eon.sock` and
/// `update.sock` share these steps.
pub(super) struct PrivateListener {
    listener: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
    cleanup: bool,
}

impl PrivateListener {
    /// `what` names the socket in errors, such as "Eon control".
    pub(super) fn bind(path: &Path, what: &str) -> Result<Self, String> {
        remove_stale_socket(path, what)?;
        let listener = UnixListener::bind(path)
            .map_err(|error| format!("cannot bind {what} socket {}: {error}", path.display()))?;
        let identity = socket_identity(path)?
            .ok_or_else(|| format!("{what} socket {} disappeared", path.display()))?;
        let private = Self {
            listener,
            path: path.into(),
            identity: (identity.0, identity.1),
            cleanup: true,
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("cannot protect {what} socket {}: {error}", path.display()))?;
        private
            .listener
            .set_nonblocking(true)
            .map_err(|error| format!("cannot configure {what} socket: {error}"))?;
        Ok(private)
    }

    pub(super) fn inherited(listener: UnixListener, path: PathBuf, identity: (u64, u64)) -> Self {
        Self {
            listener,
            path,
            identity,
            cleanup: false,
        }
    }

    pub(super) fn enable_cleanup(&mut self) {
        self.cleanup = true;
    }

    pub(super) fn identity(&self) -> (u64, u64) {
        self.identity
    }

    /// Where the listener is bound.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn listener(&self) -> &UnixListener {
        &self.listener
    }
}

impl Drop for PrivateListener {
    fn drop(&mut self) {
        // The open listener prevents inode reuse; chmod may legitimately change ctime.
        if self.cleanup
            && socket_identity(&self.path)
                .ok()
                .flatten()
                .is_some_and(|identity| (identity.0, identity.1) == self.identity)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

pub(super) struct ControlListener(pub(super) PrivateListener);

impl ControlListener {
    pub(super) fn bind(path: &Path) -> Result<Self, String> {
        PrivateListener::bind(path, "Eon control").map(Self)
    }

    /// Where the listener is bound.
    // An Update's import (S4b-1, eon-t3fyd.8) is its first caller.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn path(&self) -> &Path {
        self.0.path()
    }

    /// A second handle on the listening socket, for the start pump.
    pub(super) fn try_clone_listener(&self) -> Option<UnixListener> {
        self.0.listener().try_clone().ok()
    }

    pub(super) fn accept<D: Into<Dispatched>>(
        &self,
        dispatch: impl FnOnce(Incoming, Option<(libc::pid_t, libc::uid_t)>) -> D,
    ) -> Result<Accepted, String> {
        match self.0.listener().accept() {
            Ok((stream, _)) => {
                super::diagnostics::bench_trace("accept", "");
                Ok(handle_client(stream, dispatch))
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(Accepted::None),
            Err(error) => Err(format!("cannot accept Eon control client: {error}")),
        }
    }
}

fn handle_client<D: Into<Dispatched>>(
    stream: UnixStream,
    dispatch: impl FnOnce(Incoming, Option<(libc::pid_t, libc::uid_t)>) -> D,
) -> Accepted {
    match read_client(stream) {
        Some(client) => serve_client(client, dispatch),
        None => Accepted::Replied { shutdown: false },
    }
}

/// Dispatches a read client's request, or gives a request that could not be
/// read its one reply, and finishes the client.
pub(super) fn serve_client<D: Into<Dispatched>>(
    client: Client,
    dispatch: impl FnOnce(Incoming, Option<(libc::pid_t, libc::uid_t)>) -> D,
) -> Accepted {
    let dispatched = match client.incoming {
        Ok(incoming) => dispatch(incoming, client.peer).into(),
        Err(unread) => unread,
    };
    finish_client(client.stream, dispatched)
}

/// An accepted control client whose request has been read: the request, or
/// the one reply a request that could not be read gets.
pub(super) struct Client {
    pub(super) stream: UnixStream,
    pub(super) incoming: Result<Incoming, Dispatched>,
    pub(super) peer: Option<(libc::pid_t, libc::uid_t)>,
}

/// Reads one accepted client's whole request within `CLIENT_IO_TIMEOUT` of
/// its accept, however its bytes arrive (EON-C11); `None` when the
/// connection cannot be configured, which leaves it unserved.
pub(super) fn read_client(mut stream: UnixStream) -> Option<Client> {
    let deadline = Instant::now() + CLIENT_IO_TIMEOUT;
    let peer = super::platform::peer_identity(&stream).ok();
    // Accepted sockets inherit the listener's nonblocking mode on macOS.
    stream.set_nonblocking(false).ok()?;
    bound_io(&stream, stream.set_write_timeout(Some(CLIENT_IO_TIMEOUT))).ok()?;
    let incoming = read_incoming(&mut stream, deadline);
    Some(Client {
        stream,
        incoming,
        peer,
    })
}

/// Writes `dispatched`'s reply to a read client, or hands its connection
/// over to a subscription or a deferred completion.
pub(super) fn finish_client(mut stream: UnixStream, dispatched: Dispatched) -> Accepted {
    let unrepresentable = |error: String| {
        failure(
            "unrepresentable-state",
            format!("cannot encode Eon session result: {error}"),
        )
    };
    let (encoded, shutdown) = match dispatched {
        Dispatched::Reply(response, shutdown) => (
            match &response {
                ControlResponse::Workspace(response) => encode_response(response),
                ControlResponse::Lifecycle(response) => encode_lifecycle_response(response),
            }
            .or_else(|error| {
                encode_response(&Response::Failure(unrepresentable(error.to_string())))
            })
            .ok(),
            shutdown,
        ),
        Dispatched::Stop(reply, shutdown) => (
            stop::encode_reply(&reply)
                .or_else(|error| {
                    stop::encode_reply(&stop_failure(unrepresentable(error.to_string())))
                })
                .ok(),
            shutdown,
        ),
        // The supervisor writes to a subscriber without waiting.
        Dispatched::Subscribe(subscriber) => return Accepted::Subscribed(stream, subscriber),
        Dispatched::Deferred => return Accepted::Deferred(stream),
    };
    if let Some(encoded) = encoded {
        super::diagnostics::bench_trace("reply_begin", encoded.len());
        write_reply(&mut stream, &encoded, Instant::now() + REPLY_DEADLINE);
    }
    super::diagnostics::bench_trace("reply_end", "");
    Accepted::Replied { shutdown }
}

pub(super) fn complete(mut stream: UnixStream, response: ControlResponse) {
    let encoded = match response {
        ControlResponse::Workspace(response) => encode_response(&response),
        ControlResponse::Lifecycle(response) => encode_lifecycle_response(&response),
    }
    .or_else(|error| {
        encode_response(&Response::Failure(failure(
            "unrepresentable-state",
            format!("cannot encode Eon session result: {error}"),
        )))
    });
    if let Ok(encoded) = encoded {
        write_reply(&mut stream, &encoded, Instant::now() + REPLY_DEADLINE);
    }
}

/// `failure` as a Reserved Stop reply.
pub(super) fn stop_failure(failure: Failure) -> stop::Reply {
    stop::Reply::Failure {
        code: failure.code,
        detail: failure.detail,
    }
}

/// Writes `reply` until `deadline`, and drops the client once it has accepted
/// nothing for `CLIENT_IO_TIMEOUT`. A client that stalls or drains too slowly
/// gets a truncated reply and a closed connection. The writes do not block: a
/// blocking Unix socket write renews its timeout whenever the peer reads, so
/// one write of a large reply could last as long as the peer drains it.
pub(super) fn write_reply(stream: &mut UnixStream, mut reply: &[u8], deadline: Instant) {
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let mut progress = Instant::now();
    while !reply.is_empty() {
        // Checked before every write, so a client that keeps taking bytes
        // stops at the deadline too.
        if Instant::now() >= deadline {
            return;
        }
        match stream.write(reply) {
            Ok(0) => return,
            Ok(written) => {
                reply = &reply[written..];
                progress = Instant::now();
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }
        // Only a write that finds no room ends a stall: the client may have
        // read since the last poll without waking it.
        let Some(wait) = deadline
            .min(progress + CLIENT_IO_TIMEOUT)
            .checked_duration_since(Instant::now())
            .filter(|wait| !wait.is_zero())
        else {
            return;
        };
        let mut writable = [rustix::event::PollFd::new(
            stream,
            rustix::event::PollFlags::OUT,
        )];
        let millis = wait.as_millis().max(1) as i64;
        let timeout = rustix::event::Timespec {
            tv_sec: millis / 1000,
            tv_nsec: millis % 1000 * 1_000_000,
        };
        // Poll's readiness waits for more room than a write needs, so the
        // write is tried again however poll returns.
        let _ = rustix::event::poll(&mut writable, Some(&timeout));
    }
}

/// Reads one request. The Reserved Stop is recognised before any version
/// check and answered in its own framing, whatever this build's EONW
/// (EON-C11).
// The error is the one reply the connection gets, so it is not boxed.
#[allow(clippy::result_large_err)]
fn read_incoming(stream: &mut impl Source, deadline: Instant) -> Result<Incoming, Dispatched> {
    let incomplete = |error| {
        failure(
            "malformed-action",
            format!("cannot read complete EONW request: {error}"),
        )
    };
    let workspace = |failure| {
        Dispatched::Reply(
            ControlResponse::Workspace(Response::Failure(failure)),
            false,
        )
    };
    let reserved = |failure| Dispatched::Stop(stop_failure(failure), false);
    let mut read = |bytes: &mut [u8]| read_exact_before(stream, bytes, deadline, Instant::now);
    let mut message = vec![0; HEADER_BYTES];
    read(&mut message).map_err(|error| workspace(incomplete(error)))?;
    if stop::is_reserved(&message) {
        let malformed =
            |error: stop::Error| reserved(failure("malformed-action", error.to_string()));
        let length = stop::request_len(&message).map_err(malformed)?;
        message.resize(length, 0);
        read(&mut message[HEADER_BYTES..]).map_err(|error| reserved(incomplete(error)))?;
        return stop::decode_request(&message)
            .map(Incoming::Stop)
            .map_err(malformed);
    }
    let length =
        declared_message_len(&message).map_err(|error| workspace(protocol_failure(error)))?;
    message.resize(length, 0);
    read(&mut message[HEADER_BYTES..]).map_err(|error| workspace(incomplete(error)))?;
    decode_request(&message)
        .map(Incoming::Request)
        .map_err(|error| workspace(protocol_failure(error)))
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
const TYPING: u8 = 5;

pub(super) fn failure_status(code: &str) -> u8 {
    match code {
        "invalid-tab" | "malformed-action" | "invalid-text" | "invalid-key" | "input-too-large"
        | "unread-stdin" => USAGE,
        "unknown-tab" | "unknown-pane" | "unknown-id" | "unknown-terminal" | "unknown-popup"
        | "stale-popup" | "stale-target" | "unknown-generation" | "unknown-caller"
        | "unknown-session" | "unknown-window" | "unknown-target" => MISSING,
        "ambiguous-window-id" => MISSING,
        "missing-supervisor"
        | "supervisor-unavailable"
        | "reload-refused"
        | "reload-presentation-failed"
        | "reload-rollback-failed" => UNREACHABLE,
        "person-typing" => TYPING,
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
        detail = "unspecified Eon session failure".into();
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

fn remove_socket_if_identity(path: &Path, identity: SocketIdentity) -> Result<(), String> {
    crate::private_file::revalidate(path, Some(identity))?;
    fs::remove_file(path).map_err(|error| error.to_string())
}

/// A stream whose next read waits at most a given time.
pub(crate) trait Source {
    fn read_within(&mut self, buffer: &mut [u8], timeout: Duration) -> std::io::Result<usize>;
}

impl Source for UnixStream {
    fn read_within(&mut self, buffer: &mut [u8], timeout: Duration) -> std::io::Result<usize> {
        // A peer that wrote and closed may refuse the option on Darwin; its
        // bytes are still there to read.
        let read_timeout = super::update::option(
            self.set_read_timeout(Some(timeout)),
            "einval-on-read-timeout",
        );
        bound_io(self, read_timeout)?;
        self.read(buffer)
    }
}

/// Fills `buffer` before `deadline`, whatever the reads' pace: each read
/// waits only the time left, so a peer that trickles bytes cannot extend the
/// deadline. Times out with `TimedOut`, ends early with `UnexpectedEof`.
pub(crate) fn read_exact_before(
    source: &mut impl Source,
    buffer: &mut [u8],
    deadline: Instant,
    now: impl Fn() -> Instant,
) -> std::io::Result<()> {
    let mut filled = 0;
    while filled < buffer.len() {
        let left = deadline.saturating_duration_since(now());
        if left.is_zero() {
            return Err(ErrorKind::TimedOut.into());
        }
        match source.read_within(&mut buffer[filled..], left) {
            Ok(0) => return Err(ErrorKind::UnexpectedEof.into()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Err(ErrorKind::TimedOut.into());
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{ControlListener, remove_socket_if_identity, socket_identity};
    use crate::supervisor::temporary_directory;
    use std::{
        fs,
        io::{Read, Write},
        os::unix::fs::PermissionsExt,
        os::unix::net::{UnixListener, UnixStream},
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn client_deadlines_cover_admission_failed_files_and_failed_fallback() {
        use crate::{fixtures, supervisor::PANE_CLOSE_TIMEOUT, workspace::SessionOperation};
        use eon_workspace_protocol::{
            PRESENTATION_ADMISSION_TIMEOUT, SESSION_CLEANUP_TIMEOUT, TERMINAL_START_TIMEOUT,
        };
        const GENERATION: &str = "g1-0123456789abcdef0123456789abcdef";
        let startup = PRESENTATION_ADMISSION_TIMEOUT
            + TERMINAL_START_TIMEOUT * 2
            + SESSION_CLEANUP_TIMEOUT * 3;
        assert_eq!(super::CONTROL_TIMEOUT, startup + Duration::from_secs(1));
        assert_eq!(
            super::STOP_TIMEOUT,
            super::CONTROL_TIMEOUT + SESSION_CLEANUP_TIMEOUT
        );
        let (_, _, close_timeout) =
            super::prepare_action(None, super::Action::ClosePane { pane: "p1".into() }).unwrap();
        assert!(
            close_timeout > PANE_CLOSE_TIMEOUT + super::CLIENT_IO_TIMEOUT + super::REPLY_DEADLINE
        );
        // Exact budgets are checked above; allow fixture scheduling slack in the exchange.
        const SCALE: u32 = 100;
        for stopping in [false, true] {
            let (stream, peer) = UnixStream::pair().unwrap();
            let (entered, ready) = std::sync::mpsc::channel();
            let client = thread::spawn(move || {
                let timeout = if stopping {
                    super::STOP_TIMEOUT
                } else {
                    super::CONTROL_TIMEOUT
                } / SCALE;
                let deadline = Instant::now() + timeout + Duration::from_secs(2);
                entered.send(()).unwrap();
                if stopping {
                    assert_eq!(
                        super::send_stop_until(stream, GENERATION, deadline).unwrap(),
                        super::StopAnswer::Reply(eon_workspace_protocol::stop::Reply::Stopped {
                            generation: GENERATION.into(),
                            terminals: Vec::new(),
                        })
                    );
                } else {
                    let request = super::prepare_action(
                        None,
                        super::Action::Workspace(super::WorkspaceAction::Inspect),
                    )
                    .unwrap();
                    assert!(matches!(
                        super::send_prepared_action(stream, request, Some(deadline)).unwrap(),
                        super::ControlResponse::Workspace(super::Response::Failure(_))
                    ));
                }
            });
            ready.recv().unwrap();
            thread::sleep(PRESENTATION_ADMISSION_TIMEOUT / SCALE);
            let root = crate::supervisor::temporary_directory();
            let mut starts = 0;
            let workspace = crate::workspace::WorkspaceSet::pending(
                root.clone(),
                root.clone(),
                None,
                fixtures::inputs().defaults.popups,
                |_, _| {
                    Ok(super::super::managed_environment::PreparedCommand {
                        argv: vec!["fixture-files".into()],
                        listener: None,
                    })
                },
                |operation| {
                    assert!(matches!(operation, SessionOperation::Start { .. }));
                    starts += 1;
                    thread::sleep(TERMINAL_START_TIMEOUT / SCALE);
                    thread::sleep(SESSION_CLEANUP_TIMEOUT / SCALE);
                    Err("injected start and rollback failure".into())
                },
            );
            assert!(workspace.is_err());
            assert_eq!(
                starts, 2,
                "the Files failure must attempt the fallback pane"
            );
            // A failed bootstrap also closes its already admitted presentation.
            thread::sleep(SESSION_CLEANUP_TIMEOUT / SCALE);
            if stopping {
                thread::sleep(SESSION_CLEANUP_TIMEOUT / SCALE);
            }
            super::handle_client(peer, |incoming, _peer| match incoming {
                super::Incoming::Stop(generation) => super::Dispatched::Stop(
                    eon_workspace_protocol::stop::Reply::Stopped {
                        generation,
                        terminals: Vec::new(),
                    },
                    true,
                ),
                super::Incoming::Request(_) => (
                    super::ControlResponse::Workspace(super::Response::Failure(super::failure(
                        "terminal-start",
                        "injected failure",
                    ))),
                    false,
                )
                    .into(),
            });
            client.join().unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    /// EON-C36: only a failure after the whole request was written may follow
    /// its effect; an unwritten request or a refused version never does.
    #[test]
    fn a_failure_is_dispatched_only_after_the_whole_request_is_written() {
        let request = || {
            let action = super::Action::Workspace(super::WorkspaceAction::InspectRuntime);
            super::prepare_action(None, action).unwrap()
        };
        let deadline = || Some(Instant::now() + Duration::from_secs(5));
        // The supervisor is gone before the request is written.
        let (stream, peer) = UnixStream::pair().unwrap();
        drop(peer);
        let failure = super::send_prepared_action(stream, request(), deadline()).unwrap_err();
        assert!(!failure.dispatched, "{}", failure.detail);
        // It reads the whole request, then answers in another EONW version.
        for answer in [None, Some(super::VERSION + 1), Some(super::VERSION)] {
            let (stream, mut peer) = UnixStream::pair().unwrap();
            let written = request().1.len();
            let server = thread::spawn(move || {
                let mut bytes = vec![0; written];
                peer.read_exact(&mut bytes).unwrap();
                let Some(version) = answer else { return };
                let mut reply =
                    super::encode_response(&super::Response::Failure(super::failure("busy", "x")))
                        .unwrap();
                reply[4..6].copy_from_slice(&version.to_le_bytes());
                // A truncated reply of this version fails after dispatch.
                reply.truncate(if version == super::VERSION {
                    14
                } else {
                    reply.len()
                });
                peer.write_all(&reply).unwrap();
            });
            let failure = super::send_prepared_action(stream, request(), deadline()).unwrap_err();
            server.join().unwrap();
            let incompatible = failure.kind == super::EndpointFailureKind::Incompatible;
            assert_eq!(
                incompatible,
                answer == Some(super::VERSION + 1),
                "{}",
                failure.detail
            );
            assert_eq!(failure.dispatched, !incompatible, "{}", failure.detail);
        }
    }

    #[test]
    fn handle_client_dispatches_buffered_request_after_peer_closes() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let request = super::Request {
            id: "r1".into(),
            caller: None,
            action: super::Action::Workspace(super::WorkspaceAction::InspectRuntime),
        };
        peer.write_all(&super::encode_request(&request).unwrap())
            .unwrap();
        drop(peer);

        assert!(
            super::handle_client(stream, |received, _peer| {
                assert_eq!(received, super::Incoming::Request(request));
                (
                    super::ControlResponse::Workspace(super::Response::Failure(super::failure(
                        "stopped", "done",
                    ))),
                    true,
                )
            })
            .shutdown()
        );
    }

    /// A versioned reply to the Reserved Stop names its supervisor's EONW:
    /// below the first that answers the Reserved Stop it predates it, and
    /// from that one on it is no Stop result.
    #[test]
    fn a_versioned_stop_reply_predates_only_below_the_first_reserved_stop() {
        use eon_workspace_protocol::stop;
        const GENERATION: &str = "g1-0123456789abcdef0123456789abcdef";
        let answer = |version: u16| {
            let (stream, mut peer) = UnixStream::pair().unwrap();
            let server = std::thread::spawn(move || {
                let mut request = [0; stop::REQUEST_BYTES];
                peer.read_exact(&mut request).unwrap();
                let mut reply = super::encode_response(&super::Response::Failure(super::failure(
                    "unsupported-version",
                    "not this EONW",
                )))
                .unwrap();
                reply[4..6].copy_from_slice(&version.to_le_bytes());
                peer.write_all(&reply).unwrap();
            });
            let answer = super::send_stop_on(stream, GENERATION);
            server.join().unwrap();
            answer
        };
        let last = stop::FIRST_VERSION - 1;
        assert!(
            matches!(answer(last), Ok(super::StopAnswer::Predates(version)) if version == last)
        );
        for version in [stop::FIRST_VERSION, super::VERSION + 1] {
            let Err(failure) = answer(version) else {
                panic!("EONW {version} gave a Stop result");
            };
            assert_eq!(failure.kind, super::EndpointFailureKind::Corrupt);
            assert!(
                failure.detail.contains(&format!("EONW {version}")),
                "{}",
                failure.detail
            );
        }
    }

    /// The supervisor recognises the Reserved Stop before its version check
    /// and answers it, and anything that goes wrong with it, in its framing.
    #[test]
    fn handle_client_answers_the_reserved_stop_in_its_framing() {
        use eon_workspace_protocol::stop;
        const GENERATION: &str = "g1-0123456789abcdef0123456789abcdef";
        let exchange = |request: &[u8], reply: Option<stop::Reply>| {
            let (stream, mut peer) = UnixStream::pair().unwrap();
            peer.write_all(request).unwrap();
            peer.shutdown(std::net::Shutdown::Write).unwrap();
            let dispatched = std::cell::Cell::new(false);
            let accepted = super::handle_client(stream, |received, _peer| {
                dispatched.set(true);
                assert_eq!(received, super::Incoming::Stop(GENERATION.into()));
                super::Dispatched::Stop(reply.clone().unwrap(), true)
            });
            let mut received = Vec::new();
            peer.read_to_end(&mut received).unwrap();
            (
                dispatched.get(),
                accepted.shutdown(),
                stop::decode_reply(&received),
            )
        };
        let request = stop::encode_request(GENERATION).unwrap();
        let stopped = stop::Reply::Stopped {
            generation: GENERATION.into(),
            terminals: vec!["terminal-1".into()],
        };
        assert_eq!(
            exchange(&request, Some(stopped.clone())),
            (true, true, Ok(stopped))
        );
        // A forged length is refused from the header alone, which is all
        // the peer sends: unread bytes would reset the connection on close.
        let mut forged = request[..stop::HEADER_BYTES].to_vec();
        forged[8] = 36;
        let (dispatched, shutdown, reply) = exchange(&forged, None);
        assert!(!dispatched && !shutdown);
        assert!(
            matches!(&reply, Ok(stop::Reply::Failure { code, .. }) if code == "malformed-action"),
            "{reply:?}"
        );
        let (dispatched, _, reply) = exchange(&request[..30], None);
        assert!(!dispatched);
        assert!(
            matches!(&reply, Ok(stop::Reply::Failure { code, .. }) if code == "malformed-action"),
            "{reply:?}"
        );
    }

    #[test]
    fn handle_client_waits_for_request_on_a_nonblocking_stream() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let request = super::Request {
            id: "r1".into(),
            caller: None,
            action: super::Action::Workspace(super::WorkspaceAction::Inspect),
        };
        let encoded = super::encode_request(&request).unwrap();
        let response = super::Response::Failure(super::failure("test-response", "done"));
        let expected = super::encode_response(&response).unwrap();
        let peer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            peer.write_all(&encoded).unwrap();
            let mut received = vec![0; expected.len()];
            peer.read_exact(&mut received).unwrap();
            assert_eq!(received, expected);
        });

        let shutdown = super::handle_client(stream, |received, _peer| {
            assert_eq!(received, super::Incoming::Request(request));
            (super::ControlResponse::Workspace(response), true)
        })
        .shutdown();
        let peer_result = peer.join();
        assert!(shutdown, "delayed request was not dispatched");
        peer_result.unwrap();
    }

    /// C18: a client that dribbles its request, each byte well inside
    /// `CLIENT_IO_TIMEOUT` of the last, still has one whole-request budget
    /// from its accept. It gets the one failure reply and its request never
    /// dispatches, however it later completes; the Reserved Stop is bounded
    /// alike.
    #[test]
    fn a_dribbling_request_is_bounded_by_one_whole_request_deadline() {
        use eon_workspace_protocol::stop;
        let action = super::encode_request(&super::Request {
            id: "d".repeat(100),
            caller: None,
            action: super::Action::Workspace(super::WorkspaceAction::Inspect),
        })
        .unwrap();
        let reserved = stop::encode_request("g1-0123456789abcdef0123456789abcdef").unwrap();
        for request in [action, reserved] {
            let (stream, mut peer) = UnixStream::pair().unwrap();
            let pace = super::CLIENT_IO_TIMEOUT / 3;
            let dribble = pace * (request.len() - super::HEADER_BYTES) as u32;
            let client = thread::spawn(move || {
                peer.write_all(&request[..super::HEADER_BYTES]).unwrap();
                for byte in &request[super::HEADER_BYTES..] {
                    thread::sleep(pace);
                    if peer.write_all(&[*byte]).is_err() {
                        break;
                    }
                }
                // Bytes the window never read may reset the connection
                // before its reply is read; what arrives is that reply.
                let mut reply = Vec::new();
                let _ = peer.read_to_end(&mut reply);
                reply
            });
            let start = Instant::now();
            let accepted = super::handle_client(stream, |_, _peer| -> super::Dispatched {
                panic!("a request read past its deadline was dispatched")
            });
            let held = start.elapsed();
            assert!(!accepted.shutdown());
            assert!(held < dribble, "{held:?} of {dribble:?}");
            let reply = client.join().unwrap();
            if !reply.is_empty() {
                let code = match super::decode_response(&reply) {
                    Ok(super::Response::Failure(failure)) => failure.code,
                    _ => match stop::decode_reply(&reply) {
                        Ok(stop::Reply::Failure { code, .. }) => code,
                        other => panic!("{other:?}"),
                    },
                };
                assert_eq!(code, "malformed-action");
            }
        }
    }

    #[test]
    fn a_slow_reader_holds_the_supervisor_no_longer_than_the_reply_deadline() {
        let history = |text: String| {
            super::Response::History(eon_workspace_protocol::v51::TerminalHistory {
                target: eon_workspace_protocol::v51::TerminalTarget {
                    workspace: "s1".into(),
                    tab: "t1".into(),
                    id: eon_workspace_protocol::v51::TargetId::Pane("p1".into()),
                    terminal: "terminal-1".into(),
                },
                caller: None,
                revision: 1,
                screen: eon_workspace_protocol::v51::Screen::Primary,
                rows: 24,
                columns: 80,
                title: String::new(),
                directory: Vec::new(),
                omitted_lines: 0,
                cut_bytes: 0,
                text,
            })
        };
        let request = |id: &str| {
            super::encode_request(&super::Request {
                id: id.into(),
                caller: None,
                action: super::Action::ReadTerminalHistory {
                    target: "p1".into(),
                },
            })
            .unwrap()
        };
        // An 8 MiB reply, drained 32 KiB every 20 ms: whole, it would take
        // about 5 s. Reads that close together keep the reader well inside
        // CLIENT_IO_TIMEOUT on a loaded machine, so only the deadline stops
        // the reply.
        let full = history("a".repeat(8 * 1024 * 1024));
        let size = super::encode_response(&full).unwrap().len();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(&request("slow")).unwrap();
        let slow = thread::spawn(move || {
            let mut received = 0;
            let mut chunk = vec![0; 32 * 1024];
            loop {
                match peer.read(&mut chunk) {
                    Ok(0) | Err(_) => return received,
                    Ok(read) => received += read,
                }
                thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        super::handle_client(stream, |_, _peer| {
            (super::ControlResponse::Workspace(full), false)
        });
        let held = start.elapsed();
        assert!(
            held >= super::REPLY_DEADLINE && held < super::REPLY_DEADLINE + Duration::from_secs(1),
            "{held:?}"
        );
        assert!(slow.join().unwrap() < size);
        // The supervisor then serves the next client whole.
        let small = history("next".into());
        let expected = super::encode_response(&small).unwrap();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(&request("next")).unwrap();
        super::handle_client(stream, |_, _peer| {
            (super::ControlResponse::Workspace(small), false)
        });
        let mut received = Vec::new();
        peer.read_to_end(&mut received).unwrap();
        assert_eq!(received, expected);
    }

    #[test]
    fn a_reply_past_its_deadline_stops_even_while_the_client_takes_bytes() {
        // The socket has room for the whole reply, so every write would
        // succeed; the deadline alone stops it.
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        super::write_reply(&mut stream, b"reply", Instant::now());
        drop(stream);
        let mut received = Vec::new();
        peer.read_to_end(&mut received).unwrap();
        assert!(received.is_empty(), "{received:?}");
        // Within its deadline the same reply arrives whole.
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        super::write_reply(
            &mut stream,
            b"reply",
            Instant::now() + super::REPLY_DEADLINE,
        );
        drop(stream);
        let mut received = Vec::new();
        peer.read_to_end(&mut received).unwrap();
        assert_eq!(received, b"reply");
    }

    #[test]
    fn discovery_deadline_covers_a_dribbling_response() {
        let root = temporary_directory();
        let socket = root.join("eon.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let deadline = Instant::now() + super::CLIENT_IO_TIMEOUT;
            assert!(super::read_incoming(&mut stream, deadline).is_ok());
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
            Some("terminal-1.run".into()),
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
            ("unknown-session", 3),
            ("ambiguous-session", 1),
            ("session-limit", 1),
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
    fn a_bound_socket_is_kept_until_closed() {
        let root = temporary_directory();
        let path = root.join("eon.sock");
        let socket =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        socket
            .bind(&socket2::SockAddr::unix(&path).unwrap())
            .unwrap();

        // A stream connect is refused before listen(), even on Linux.
        let rejected = ControlListener::bind(&path);
        assert!(matches!(rejected, Err(error) if error.contains("already active")));

        drop(socket);
        drop(ControlListener::bind(&path).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_live_listener_that_cannot_accept_is_unreachable() {
        use super::EndpointFailureKind::{Dead, Unreachable};
        use socket2::{Domain, SockAddr, Socket, Type};
        let root = temporary_directory();
        // Every system refuses a stream connect before listen(2).
        let bound = root.join("bound.sock");
        let socket = Socket::new(Domain::UNIX, Type::STREAM, None).unwrap();
        socket.bind(&SockAddr::unix(&bound).unwrap()).unwrap();
        // A full accept queue: macOS refuses, Linux polls to POLLHUP.
        let busy = root.join("busy.sock");
        let (_listener, _queued) = crate::fixtures::busy_listener(&busy);
        let closed = root.join("closed.sock");
        drop(UnixListener::bind(&closed).unwrap());

        assert!(super::CONTROL_TIMEOUT > Duration::from_secs(10));
        for (path, expected) in [(bound, Unreachable), (busy, Unreachable), (closed, Dead)] {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            let (sender, receiver) = std::sync::mpsc::channel();
            let target = path.clone();
            // The full CONTROL_TIMEOUT, under a 10 s watchdog.
            thread::spawn(move || {
                let result = super::connect_control(&target);
                sender
                    .send(result.map(drop).map_err(|error| error.kind))
                    .unwrap();
            });
            let result = receiver
                .recv_timeout(Duration::from_secs(10))
                .expect("the control connect hung");
            assert_eq!(result, Err(expected), "{}", path.display());
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_socket_that_vanishes_after_inspection_leaves_the_path_available() {
        let root = temporary_directory();
        let path = root.join("eon.sock");
        let original = ControlListener::bind(&path).unwrap();
        let observed = socket_identity(&path).unwrap().unwrap();
        drop(original);

        super::probe_stale_socket(&path, "Eon control", observed).unwrap();
        drop(ControlListener::bind(&path).unwrap());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_datagram_socket_is_kept_and_reported() {
        let root = temporary_directory();
        let path = root.join("eon.sock");
        let datagram = std::os::unix::net::UnixDatagram::bind(&path).unwrap();
        let observed = socket_identity(&path).unwrap().unwrap();

        let rejected = ControlListener::bind(&path);
        assert!(matches!(rejected, Err(error) if error.contains("is a Unix datagram socket")));
        assert_eq!(socket_identity(&path).unwrap(), Some(observed));

        drop(datagram);
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

        assert!(remove_socket_if_identity(&socket, observed).is_err());

        assert!(UnixStream::connect(&socket).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
}
