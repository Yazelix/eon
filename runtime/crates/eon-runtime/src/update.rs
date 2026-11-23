//! A full-Eon window's private Update endpoint, `update.sock` (EON-C11
//! planned Update control): its listener, bounded reads and, with the EONU
//! codec in `eon-workspace-protocol`, its one-request exchange. Nothing here
//! decodes EONW. A failure here never fails the window's start or ends its
//! supervisor: the window then cannot be updated in place.
// The isolated adoption route serves EONU in tests. Production waits for
// quiescing, restore-failure handling and native presentation replacement.
#![cfg_attr(not(test), allow(dead_code))]

use crate::{
    control::{
        CLIENT_IO_TIMEOUT, PrivateListener, REPLY_DEADLINE, SocketIdentity, socket_identity,
        socket_identity_from, write_reply,
    },
    diagnostics::DiagnosticLog,
    platform,
    supervisor::{effective_uid, validate_private_directory},
};
use eon_workspace_protocol::update::{
    self as eonu, Code, Direction, HEADER_BYTES, Inspect, Inspected, Kind, PREFIX_BYTES, Preflight,
    Preflighted, Refused, RequestId,
};
use std::{
    fmt, fs,
    io::{self, Read, Write},
    net::Shutdown,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// The endpoint's name in a lifetime directory.
pub(crate) const SOCKET: &str = "update.sock";

/// The production bind remains gated while the adoption route is private.
pub(crate) fn serves_update() -> bool {
    cfg!(test)
}

/// Whether a test asked to fail at `point`: this thread's
/// `inject` (in-process tests) or the process's `EON_UPDATE_TEST_FAULT`
/// (a window under test). Never in production.
pub(crate) fn fault(point: &str) -> bool {
    #[cfg(test)]
    if INJECTED.with(|injected| injected.get().contains(&point)) {
        return true;
    }
    cfg!(test) && std::env::var_os("EON_UPDATE_TEST_FAULT").is_some_and(|value| value == point)
}

#[cfg(test)]
thread_local! {
    pub(crate) static INJECTED: std::cell::Cell<&'static [&'static str]> =
        const { std::cell::Cell::new(&[]) };
    /// One-shot signal fired when an active `await_hangup` wait starts,
    /// strictly after the client has passed `peer(&stream)`, or dropped when
    /// cleared by `ClientHooks` on return or unwind. See
    /// `release_before_hangup`.
    static RELEASE_BEFORE_HANGUP: std::cell::RefCell<Option<std::sync::mpsc::Sender<()>>> =
        const { std::cell::RefCell::new(None) };
}

/// Makes this thread fail at each of `points` until the next call.
#[cfg(test)]
pub(crate) fn inject(points: &'static [&'static str]) {
    INJECTED.with(|injected| injected.set(points));
}

/// Arms (or clears, with `None`) a one-shot signal fired when this thread
/// enters an active `await_hangup` wait, strictly after `peer(&stream)`.
///
/// A fixture that must close before the client's next step blocks on the
/// matching receiver with plain `recv()`, never on a timer: the server end is
/// closed only after the client has either passed `peer(&stream)` and reached
/// `await_hangup`, or hung up and dropped the `Sender` via `ClientHooks` drop.
/// That ensures the client's peer credential read always happens on a connected
/// socket in every interleaving (avoiding macOS `LOCAL_PEERPID` returning
/// `ENOTCONN`); a stalled test is bounded by the test runner's own timeout.
#[cfg(test)]
pub(crate) fn release_before_hangup(release: Option<std::sync::mpsc::Sender<()>>) {
    RELEASE_BEFORE_HANGUP.with(|slot| *slot.borrow_mut() = release);
}

/// `set`, a socket option's result, or under the fault `point` the EINVAL
/// Darwin returns once the peer has closed: Linux tests reach the Darwin
/// path through it.
fn option(set: io::Result<()>, point: &str) -> io::Result<()> {
    if fault(point) {
        return Err(io::Error::from_raw_os_error(libc::EINVAL));
    }
    set
}

/// The text of a test wait that never saw the window hang up.
pub(crate) const NO_HANGUP: &str = "the window's hang-up was not observed";

/// Under the fault `point`, waits until the peer has hung up, so a test
/// orders the window's close before the client's next step. Fails, so that
/// step never runs, when no hang-up comes within 1 s.
fn await_hangup(stream: &UnixStream, point: &str) -> io::Result<()> {
    if !fault(point) {
        return Ok(());
    }
    #[cfg(test)]
    if let Some(tx) = RELEASE_BEFORE_HANGUP.with(|slot| slot.borrow_mut().take()) {
        let _ = tx.send(());
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        let mut readiness = [rustix::event::PollFd::new(
            stream,
            rustix::event::PollFlags::IN,
        )];
        let timeout = rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 10_000_000,
        };
        if rustix::event::poll(&mut readiness, Some(&timeout)).is_ok()
            && readiness[0]
                .revents()
                .contains(rustix::event::PollFlags::HUP)
        {
            return Ok(());
        }
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, NO_HANGUP))
}

/// A bound `update.sock`.
pub(crate) struct UpdateListener(pub(crate) PrivateListener);

impl UpdateListener {
    /// The window's endpoint in `runtime`, or none, with the reason in the
    /// window's log. Never an error: the window runs without it.
    pub(crate) fn bind(runtime: &Path, log: &DiagnosticLog) -> Option<Self> {
        #[cfg(test)]
        crate::fixtures::assert_not_update_validator("the Update listener");
        if !serves_update() {
            return None;
        }
        let path = runtime.join(SOCKET);
        let bound = if fault("bind") {
            Err("an injected bind failure".to_owned())
        } else if platform::socket_path_fits(&path) {
            PrivateListener::bind(&path, "Eon Update")
        } else {
            Err(format!(
                "Eon Update socket path {} exceeds this platform's socket path limit",
                path.display()
            ))
        };
        match bound {
            Ok(listener) => Some(Self(listener)),
            Err(error) => {
                log.note(&format!("this window cannot be updated in place: {error}"));
                None
            }
        }
    }

    pub(crate) fn path(&self) -> &Path {
        self.0.path()
    }

    /// Reads one waiting request. Never an error of the loop: an accept error is
    /// logged once per kind, a connection error drops that connection.
    pub(crate) fn serve(&self, log: &DiagnosticLog, handle: impl FnOnce(Kind, &[u8], Reply)) {
        let accepted = if fault("accept") {
            Err(io::Error::from_raw_os_error(libc::EMFILE))
        } else {
            self.0.listener().accept()
        };
        match accepted {
            Ok((stream, _)) => {
                if let Ok((kind, bytes, reply)) =
                    receive_connection(stream, Instant::now() + CLIENT_IO_TIMEOUT)
                {
                    handle(kind, &bytes, reply);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => log.note_once(&format!(
                "cannot accept an Eon Update client: {:?}",
                error.kind()
            )),
        }
    }
}

/// How a connection ended, for tests to tell the branches apart.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Served {
    /// Another user, or a peer whose credentials cannot be read: closed with
    /// nothing written.
    Peer,
    /// The request was not complete before the deadline or EOF.
    Partial,
    /// A reply of this kind was written (or tried).
    Replied(Kind, Option<Code>),
}

fn peer(stream: &UnixStream) -> io::Result<(libc::pid_t, libc::uid_t)> {
    if fault("peer-unreadable") {
        return Err(io::ErrorKind::NotConnected.into());
    }
    let (pid, uid) = platform::peer_identity(stream)?;
    Ok((
        pid,
        if fault("peer-uid") {
            uid.wrapping_add(1)
        } else {
            uid
        },
    ))
}

pub(crate) struct Reply {
    stream: UnixStream,
    read_deadline: Instant,
}

impl Reply {
    pub(crate) fn stream(&self) -> &UnixStream {
        &self.stream
    }

    pub(crate) fn inherited(stream: UnixStream) -> Self {
        Self {
            stream,
            read_deadline: Instant::now(),
        }
    }

    pub(crate) fn commit(self, result: Result<eonu::Updated, Refused>) {
        self.send(
            result.and_then(|reply| {
                reply
                    .encode()
                    .map(|bytes| (Kind::Updated, bytes))
                    .map_err(|error| {
                        refusal(
                            reply.request,
                            Code::Internal,
                            format!("cannot encode commit: {error}"),
                        )
                    })
            }),
            true,
        );
    }

    fn send(mut self, result: Result<(Kind, Vec<u8>), Refused>, deferred: bool) -> Served {
        let (kind, code, bytes) = match result {
            Ok((kind, bytes)) => (kind, None, bytes),
            Err(refused) => {
                let code = refused.code;
                let bytes = refused.encode().expect("a refusal's detail fits its bound");
                (Kind::Refused, Some(code), bytes)
            }
        };
        write_reply(&mut self.stream, &bytes, Instant::now() + REPLY_DEADLINE);
        if deferred {
            // A complete request was read before the child started. Drain only
            // already queued input so a silent client cannot stall the window.
            let _ = self.stream.shutdown(Shutdown::Write);
            let mut sink = [0; 4096];
            for _ in 0..16 {
                if !matches!(platform::read_available(&self.stream, &mut sink), Ok(1..)) {
                    break;
                }
            }
        } else {
            finish(self.stream, self.read_deadline);
        }
        Served::Replied(kind, code)
    }

    pub(crate) fn inspect(
        self,
        kind: Kind,
        bytes: &[u8],
        inspect: impl FnOnce(RequestId) -> Result<Inspected, String>,
    ) -> Served {
        self.send(
            answer(kind, bytes, inspect).map(|bytes| (Kind::Inspected, bytes)),
            false,
        )
    }

    pub(crate) fn preflight(self, result: Result<Preflighted, Refused>, deferred: bool) {
        self.send(
            result.and_then(|reply| {
                reply
                    .encode()
                    .map(|bytes| (Kind::Preflighted, bytes))
                    .map_err(|error| {
                        refusal(
                            reply.request,
                            Code::Internal,
                            format!("cannot encode preflight: {error}"),
                        )
                    })
            }),
            deferred,
        );
    }
}

pub(crate) fn refusal(request: RequestId, code: Code, mut detail: String) -> Refused {
    detail.truncate(detail.floor_char_boundary(eonu::MAX_DETAIL_BYTES));
    Refused {
        request,
        code,
        detail,
    }
}

pub(crate) fn decode_preflight(bytes: &[u8]) -> Result<Preflight, Refused> {
    Preflight::decode(bytes)
        .map_err(|error| refusal(frame_request(bytes), Code::Malformed, error.to_string()))
}

pub(crate) fn frame_request(bytes: &[u8]) -> RequestId {
    bytes
        .get(HEADER_BYTES..HEADER_BYTES + 16)
        .and_then(|id| id.try_into().ok())
        .unwrap_or([0; 16])
}

/// The request has one deadline in total. The prefix decides its version
/// before the reader accepts any more bytes.
fn receive_connection(
    stream: UnixStream,
    deadline: Instant,
) -> Result<(Kind, Vec<u8>, Reply), Served> {
    match peer(&stream) {
        Ok((_, uid)) if uid == effective_uid() => {}
        _ => return Err(Served::Peer),
    }
    // Accepted sockets inherit the listener's nonblocking mode on macOS.
    if stream.set_nonblocking(false).is_err() {
        return Err(Served::Partial);
    }
    let mut reply = Reply {
        stream,
        read_deadline: deadline,
    };
    let mut head = [0; HEADER_BYTES];
    let read = |stream: &mut UnixStream, bytes: &mut [u8]| {
        read_exact_before(stream, bytes, deadline, Instant::now).map_err(|_| Served::Partial)
    };
    read(&mut reply.stream, &mut head[..PREFIX_BYTES])?;
    let invalid = match eonu::version(head[..PREFIX_BYTES].try_into().expect("6 bytes")) {
        Err(error) => Some((Code::Malformed, error.to_string())),
        Ok(version) if version != eonu::VERSION => {
            Some((Code::Version, version_refusal(eonu::VERSION, version)))
        }
        Ok(_) => None,
    };
    if let Some((code, detail)) = invalid {
        return Err(reply.send(Err(refusal([0; 16], code, detail)), false));
    }
    read(&mut reply.stream, &mut head[PREFIX_BYTES..])?;
    let header = match eonu::header(&head, Direction::Request) {
        Ok(header) => header,
        Err(error) => {
            return Err(reply.send(
                Err(refusal([0; 16], Code::Malformed, error.to_string())),
                false,
            ));
        }
    };
    let mut frame = vec![0; HEADER_BYTES + header.length];
    frame[..HEADER_BYTES].copy_from_slice(&head);
    read(&mut reply.stream, &mut frame[HEADER_BYTES..])?;
    Ok((header.kind, frame, reply))
}

#[cfg(test)]
pub(crate) fn serve_connection(
    stream: UnixStream,
    deadline: Instant,
    inspect: impl FnOnce(RequestId) -> Result<Inspected, String>,
) -> Served {
    match receive_connection(stream, deadline) {
        Ok((kind, bytes, reply)) => reply.inspect(kind, &bytes, inspect),
        Err(served) => served,
    }
}

/// The reply to a whole, header-checked request frame.
fn answer(
    kind: Kind,
    frame: &[u8],
    inspect: impl FnOnce(RequestId) -> Result<Inspected, String>,
) -> Result<Vec<u8>, Refused> {
    let malformed = |request, error: eonu::Error| Refused {
        request,
        code: Code::Malformed,
        detail: error.to_string(),
    };
    match kind {
        Kind::Inspect => {
            let request = Inspect::decode(frame).map_err(|error| malformed([0; 16], error))?;
            let internal = |error: String| {
                refusal(
                    request.request,
                    Code::Internal,
                    format!("this window cannot describe itself: {error}"),
                )
            };
            let inspected = inspect(request.request).map_err(internal)?;
            let encoded = if fault("encode") {
                Err(eonu::Error::Malformed("an injected encode failure"))
            } else {
                inspected.encode()
            };
            encoded.map_err(|error| internal(error.to_string()))
        }
        // Every request payload starts with its request ID.
        _ => Err(Refused {
            request: frame_request(frame),
            code: Code::Unsupported,
            detail: "this window does not serve that Update request".into(),
        }),
    }
}

/// Stops writing and discards what the client may still send, within the
/// request deadline and 64 KiB, so a refusal sent before the end of a
/// request reaches it: closing with unread data would reset the connection.
fn finish(mut stream: UnixStream, deadline: Instant) {
    const DRAIN: usize = 64 * 1024;
    let _ = stream.set_nonblocking(false);
    let _ = stream.shutdown(Shutdown::Write);
    let mut drained = 0;
    let mut sink = [0; 4096];
    while drained < DRAIN {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        match stream.read_within(&mut sink, left) {
            Ok(0) | Err(_) => return,
            Ok(read) => drained += read,
        }
    }
}

/// The approved REFUSED code 2 detail, also the client's text: the window
/// speaks `window`, the requesting build `build`.
pub(crate) fn version_refusal(window: u16, build: u16) -> String {
    format!(
        "this window speaks Update v{window}; this build speaks v{build}; move the window by hand once"
    )
}

/// The one manual move a window that cannot be updated in place needs.
pub(crate) const MANUAL_MOVE: &str = "move it to a new window by hand once";

/// Why a client cannot update a window over `update.sock`. Every refusal
/// happens before the window changes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// No endpoint: the window predates in-place Update or could not bind it.
    Absent,
    /// The endpoint's path exceeds this platform's socket path limit.
    PathTooLong,
    /// The directory, the socket object or the peer is not this user's own.
    Unsafe(String),
    /// The window speaks another EONU version.
    Version { window: u16, ours: u16 },
    /// The window refused the request.
    Refused { code: Code, detail: String },
    /// The window's reply is not a valid EONU v1 reply to this request.
    Malformed(String),
    /// The answering supervisor is not this lifetime's.
    NotThisWindow(&'static str),
    /// The socket was replaced while the window answered.
    EndpointChanged,
    /// No answer: a connect, write or read failed or timed out. The window
    /// may still be fine.
    Unavailable(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absent => write!(f, "this window cannot be updated in place; {MANUAL_MOVE}"),
            Self::PathTooLong => write!(
                f,
                "this window's Update socket path exceeds this platform's socket path limit; {MANUAL_MOVE}"
            ),
            Self::Unsafe(reason) => write!(f, "this window's Update endpoint is unsafe: {reason}"),
            Self::Version { window, ours } => f.write_str(&version_refusal(*window, *ours)),
            Self::Refused { code, detail } => {
                write!(f, "the window refused ({}): {detail}", code.name())
            }
            Self::Malformed(reason) => {
                write!(f, "the window's Update reply is malformed: {reason}")
            }
            Self::NotThisWindow(reason) => write!(f, "not this window: {reason}"),
            Self::EndpointChanged => {
                f.write_str("the window's Update endpoint changed while it answered")
            }
            Self::Unavailable(reason) => write!(f, "the window did not answer: {reason}"),
        }
    }
}

/// `lifetime`'s endpoint and its identity, checked before connecting: a
/// private directory of ours and our own mode-0600 socket at a path the
/// platform can reach. The client connects through this exact path.
pub(crate) fn endpoint(lifetime: &Path) -> Result<(PathBuf, SocketIdentity), Refusal> {
    validate_private_directory(lifetime).map_err(Refusal::Unsafe)?;
    let path = lifetime.join(SOCKET);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(Refusal::Absent),
        Err(error) => {
            return Err(Refusal::Unsafe(format!(
                "cannot inspect {}: {error}",
                path.display()
            )));
        }
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(Refusal::Unsafe(format!(
            "{} must be an owned mode-0600 Unix socket",
            path.display()
        )));
    }
    if !platform::socket_path_fits(&path) {
        return Err(Refusal::PathTooLong);
    }
    let identity = socket_identity_from(&metadata);
    Ok((path, identity))
}

/// Asks `lifetime`'s window who it is, over `update.sock`, before
/// `deadline`. Reads no EONW and never opens `eon.sock`.
pub(crate) fn inspect(lifetime: &Path, deadline: Instant) -> Result<Inspected, Refusal> {
    let request = request_id();
    let bytes = Inspect { request }
        .encode()
        .expect("INSPECT fits its bound");
    let (frame, pid) = exchange(lifetime, &bytes, deadline)?;
    let reply = decode_reply(
        &frame,
        Kind::Inspected,
        request,
        Inspected::decode,
        |reply| reply.request,
    )?;
    let namespace = lifetime.parent().and_then(Path::parent);
    let reported = Path::new(std::ffi::OsStr::from_bytes(&reply.namespace));
    if lifetime.file_name().and_then(|name| name.to_str()) != Some(reply.lifetime.as_str()) {
        return Err(Refusal::NotThisWindow(
            "the window reports another lifetime",
        ));
    }
    if !namespace.is_some_and(|namespace| same_directory(namespace, reported)) {
        return Err(Refusal::NotThisWindow(
            "the window reports another namespace",
        ));
    }
    if pid > 0 && u32::try_from(pid) != Ok(reply.pid) {
        return Err(Refusal::NotThisWindow("the window reports another process"));
    }
    Ok(reply)
}

/// Preflights a target against an already selected window. The caller owns
/// the request ID so a lost reply can be resolved through inspection.
pub(crate) fn preflight(
    lifetime: &Path,
    request: &Preflight,
    deadline: Instant,
) -> Result<Preflighted, Refusal> {
    if lifetime.file_name().and_then(|name| name.to_str()) != Some(request.lifetime.as_str()) {
        return Err(Refusal::NotThisWindow("the request names another lifetime"));
    }
    let bytes = request.encode().map_err(|error| malformed(&error))?;
    let (frame, _) = exchange(lifetime, &bytes, deadline)?;
    let reply = decode_reply(
        &frame,
        Kind::Preflighted,
        request.request,
        Preflighted::decode,
        |reply| reply.request,
    )?;
    if reply.target_build != request.target_build
        || reply.caller_origin.is_some() != request.caller.is_some()
    {
        return Err(Refusal::Malformed(
            "the reply disagrees with the preflight request".into(),
        ));
    }
    Ok(reply)
}

fn exchange(
    lifetime: &Path,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(Vec<u8>, libc::pid_t), Refusal> {
    let (path, identity) = endpoint(lifetime)?;
    let unavailable = |error: io::Error| Refusal::Unavailable(error.to_string());
    let left = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or(Refusal::Unavailable(
                "the window did not answer in time".into(),
            ))
    };
    let mut stream =
        crate::sessions::unix_connect_with_timeout(&path, left()?).map_err(unavailable)?;
    let (pid, uid) = peer(&stream).map_err(unavailable)?;
    if uid != effective_uid() {
        return Err(Refusal::Unsafe("the window's peer is another user".into()));
    }
    await_hangup(&stream, "await-close-before-write").map_err(unavailable)?;
    let write_timeout = option(
        stream.set_write_timeout(Some(left()?)),
        "einval-on-write-timeout",
    );
    crate::sessions::bound_io(&stream, write_timeout).map_err(unavailable)?;
    // A window that refused early may have closed already: its reply is
    // still there to read.
    let _ = stream.write_all(bytes);
    await_hangup(&stream, "await-close-before-read").map_err(unavailable)?;
    let frame = read_reply(&mut stream, deadline)?;
    // Before any result, refusals included, so a socket replaced while it
    // was answering is seen.
    if socket_identity(&path).ok().flatten() != Some(identity) {
        return Err(Refusal::EndpointChanged);
    }
    Ok((frame, pid))
}

fn decode_reply<T>(
    frame: &[u8],
    expected: Kind,
    request: RequestId,
    decode: impl FnOnce(&[u8]) -> eonu::Result<T>,
    reply_id: impl FnOnce(&T) -> RequestId,
) -> Result<T, Refusal> {
    let another = || Refusal::Malformed("the reply answers another request".into());
    let reply = match eonu::kind_of(frame, Direction::Reply) {
        Ok(kind) if kind == expected => decode(frame),
        Ok(Kind::Refused) => {
            let refused =
                Refused::decode(frame, Direction::Reply).map_err(|error| malformed(&error))?;
            if refused.request != request && refused.request != [0; 16] {
                return Err(another());
            }
            return Err(Refusal::Refused {
                code: refused.code,
                detail: refused.detail,
            });
        }
        Ok(_) => Err(eonu::Error::Malformed("reply kind")),
        Err(error) => Err(error),
    }
    .map_err(|error| malformed(&error))?;
    if reply_id(&reply) != request {
        return Err(another());
    }
    Ok(reply)
}

fn malformed(error: &eonu::Error) -> Refusal {
    Refusal::Malformed(error.to_string())
}

/// Exactly one reply frame, its prefix read first.
fn read_reply(stream: &mut UnixStream, deadline: Instant) -> Result<Vec<u8>, Refusal> {
    let read = |stream: &mut UnixStream, buffer: &mut [u8]| {
        read_exact_before(stream, buffer, deadline, Instant::now)
            .map_err(|error| Refusal::Unavailable(format!("the window's reply: {error}")))
    };
    let mut head = [0; HEADER_BYTES];
    read(stream, &mut head[..PREFIX_BYTES])?;
    match eonu::version(head[..PREFIX_BYTES].try_into().expect("6 bytes")) {
        Err(error) => return Err(malformed(&error)),
        Ok(window) if window != eonu::VERSION => {
            return Err(Refusal::Version {
                window,
                ours: eonu::VERSION,
            });
        }
        Ok(_) => {}
    }
    read(stream, &mut head[PREFIX_BYTES..])?;
    let header = eonu::header(&head, Direction::Reply).map_err(|error| malformed(&error))?;
    let mut frame = vec![0; HEADER_BYTES + header.length];
    frame[..HEADER_BYTES].copy_from_slice(&head);
    read(stream, &mut frame[HEADER_BYTES..])?;
    Ok(frame)
}

/// A request ID unique on this machine: pid, wall-clock nanoseconds and a
/// process counter. Not a credential.
fn request_id() -> RequestId {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    let mut id = [0; 16];
    id[..4].copy_from_slice(&std::process::id().to_le_bytes());
    id[4..12].copy_from_slice(&nanos.to_le_bytes());
    id[12..].copy_from_slice(&COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    id
}

/// Whether `a` and `b` name the same directory, whatever their spelling
/// (`/tmp` and `/private/tmp` on macOS).
pub(crate) fn same_directory(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.is_dir() && b.is_dir() && (a.dev(), a.ino()) == (b.dev(), b.ino()),
        _ => false,
    }
}

/// A stream whose next read waits at most a given time.
pub(crate) trait Source {
    fn read_within(&mut self, buffer: &mut [u8], timeout: Duration) -> io::Result<usize>;
}

impl Source for UnixStream {
    fn read_within(&mut self, buffer: &mut [u8], timeout: Duration) -> io::Result<usize> {
        // A peer that wrote and closed may refuse the option on Darwin; its
        // bytes are still there to read.
        let read_timeout = option(
            self.set_read_timeout(Some(timeout)),
            "einval-on-read-timeout",
        );
        crate::sessions::bound_io(self, read_timeout)?;
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
) -> io::Result<()> {
    let mut filled = 0;
    while filled < buffer.len() {
        let left = deadline.saturating_duration_since(now());
        if left.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        match source.read_within(&mut buffer[filled..], left) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                return Err(io::ErrorKind::TimedOut.into());
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
