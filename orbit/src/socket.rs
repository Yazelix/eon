//! The private listener, peer negotiation and the bounded output queues of
//! the interactive client and the metadata observer.

use crate::{
    Step,
    boundary::{FirstMessage, Flags},
    errno, platform,
};
use orbit_protocol::{
    MAX_FRAME_BYTES,
    session::{
        self, ClientMessage, Failure, FailureCode, Metadata, ScrollOutcome, SelectionAction,
        ServerMessage, VerticalDirection, WheelOutcome,
    },
};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, RawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
};

/// Answers a connection that arrived while another peer negotiates with
/// Busy, written without waiting, and closes it.
pub(crate) fn turn_away(mut stream: UnixStream) {
    if let Ok(bytes) = session::encode_server_message(&ServerMessage::Busy) {
        let _ = stream.write_all(&bytes);
    }
}

pub(crate) fn peer_disconnected(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EPIPE | libc::ECONNRESET | libc::ENOTCONN)
    )
}

struct SocketGuard {
    path: PathBuf,
    identity: Option<(u64, u64)>,
}

impl SocketGuard {
    /// Removes the pathname while it is still this socket; an observed
    /// replacement is left alone without an error.
    fn unlink_owned(&mut self) -> io::Result<()> {
        let Some((device, inode)) = self.identity else {
            return Ok(());
        };
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.identity = None;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        self.identity = None;
        if metadata.file_type().is_socket() && metadata.dev() == device && metadata.ino() == inode {
            fs::remove_file(&self.path)?;
        }
        Ok(())
    }
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        let _ = self.unlink_owned();
    }
}

/// The private listener; its guard removes the socket pathname it created.
pub(crate) struct Listener {
    pub(crate) listener: UnixListener,
    guard: SocketGuard,
}

impl Listener {
    pub(crate) fn open(path: &Path) -> io::Result<Self> {
        let parent = path.parent().ok_or_else(|| errno(libc::EINVAL))?;
        let metadata = fs::symlink_metadata(parent)?;
        let uid = platform::effective_uid();
        if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700 {
            return Err(errno(libc::EACCES));
        }
        // A lock on the private parent serializes
        // concurrent starts, so one claim of a stale pathname wins (ORB-C1).
        let claim = File::open(parent)?;
        claim.try_lock().map_err(|error| match error {
            fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::ResourceBusy,
                format!("socket claim already in progress at {}", path.display()),
            ),
            fs::TryLockError::Error(error) => error,
        })?;
        Self::remove_stale(path, uid)?;
        let listener = UnixListener::bind(path)?;
        let mut guard = SocketGuard {
            path: path.to_owned(),
            identity: None,
        };
        let metadata = fs::symlink_metadata(path)?;
        guard.identity = Some((metadata.dev(), metadata.ino()));
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, guard })
    }

    pub(crate) fn unlink_owned(&mut self) -> io::Result<()> {
        self.guard.unlink_owned()
    }

    /// Removes a socket pathname a server left behind: a private socket that
    /// nobody listens on. A live server's pathname, or anything that is not
    /// an exact user-owned 0600 socket, is refused and left alone.
    fn remove_stale(path: &Path, uid: u32) -> io::Result<()> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if !metadata.file_type().is_socket()
            || metadata.uid() != uid
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(errno(libc::EEXIST));
        }
        match UnixStream::connect(path) {
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("server already listening at {}", path.display()),
            )),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                ) =>
            {
                fs::remove_file(path)
            }
            Err(error) => Err(error),
        }
    }
}

#[derive(Default)]
struct PendingWrite {
    bytes: Vec<u8>,
    written: usize,
}

impl PendingWrite {
    fn start(&mut self, bytes: &[u8]) -> io::Result<()> {
        if !self.bytes.is_empty() {
            return Err(errno(libc::EBUSY));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }

    /// Writes the rest of the message; a peer that takes no bytes is gone.
    fn flush(
        &mut self,
        fd: RawFd,
        mut write: impl FnMut(&[u8]) -> io::Result<usize>,
    ) -> io::Result<Step> {
        let result = loop {
            if self.written == self.bytes.len() {
                break Ok(Step::Done);
            }
            match write(&self.bytes[self.written..]) {
                Ok(0) => break Err(errno(libc::EPIPE)),
                Ok(written) => self.written += written,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(Step::Blocked(fd));
                }
                Err(error) => break Err(error),
            }
        };
        self.bytes.clear();
        self.written = 0;
        result
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageClass {
    Ordered,
    Preview,
    Frame,
    ScrollOutcome,
    Metadata,
}

impl MessageClass {
    fn of(message: &ServerMessage) -> Self {
        match message {
            ServerMessage::VerticalPreview(_) => Self::Preview,
            ServerMessage::Frame(_)
            | ServerMessage::WheelOutcome(WheelOutcome::Viewport { .. }) => Self::Frame,
            ServerMessage::ScrollOutcome(ScrollOutcome::Viewport { .. }) => Self::ScrollOutcome,
            _ => Self::Ordered,
        }
    }
}

/// A client's or observer's whole encoded messages, written without blocking.
/// A newer frame or scroll outcome replaces queued
/// frames and previews that have not started, a newer preview replaces one such
/// preview, nothing replaces a scroll outcome, and a newer metadata replaces
/// queued metadata that has not started. A wheel's viewport outcome counts as
/// a frame.
#[derive(Default)]
pub(crate) struct OutputQueue {
    messages: VecDeque<(Vec<u8>, MessageClass)>,
    offset: usize,
    bytes: usize,
}

impl OutputQueue {
    /// Queues one message unless it would exceed `bound` bytes.
    fn push(&mut self, bytes: Vec<u8>, class: MessageClass, bound: usize) -> bool {
        let (remove, removed_bytes) = self.replaceable_suffix(class);
        if self
            .bytes
            .saturating_sub(removed_bytes)
            .saturating_add(bytes.len())
            > bound
        {
            return false;
        }
        self.messages.truncate(self.messages.len() - remove);
        self.bytes = self.bytes - removed_bytes + bytes.len();
        self.messages.push_back((bytes, class));
        true
    }

    fn replaceable_suffix(&self, class: MessageClass) -> (usize, usize) {
        let replaceable = |back: MessageClass| match class {
            MessageClass::Ordered => false,
            MessageClass::Preview => back == MessageClass::Preview,
            MessageClass::Frame | MessageClass::ScrollOutcome => {
                matches!(back, MessageClass::Preview | MessageClass::Frame)
            }
            MessageClass::Metadata => back == MessageClass::Metadata,
        };
        let mut remove = 0;
        let mut removed_bytes = 0;
        for (index, (bytes, back)) in self.messages.iter().enumerate().rev() {
            if !replaceable(*back) || index == 0 && self.offset != 0 {
                break;
            }
            remove += 1;
            removed_bytes += bytes.len();
            if class == MessageClass::Preview {
                break;
            }
        }
        (remove, removed_bytes)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// Whether a message that replaces nothing fits under `bound`.
    pub(crate) fn can_push_ordered(
        &self,
        message: &ServerMessage,
        bound: usize,
    ) -> io::Result<bool> {
        let encoded = session::encode_server_message(message).map_err(|_| errno(libc::EPROTO))?;
        Ok(self.bytes.saturating_add(encoded.len()) <= bound)
    }

    /// Whether a scroll outcome of any size fits under `bound`, checked before
    /// the viewport moves so a commit is never cut by pressure.
    pub(crate) fn can_push_scroll_outcome(&self, bound: usize) -> bool {
        self.bytes
            .saturating_sub(self.replaceable_suffix(MessageClass::ScrollOutcome).1)
            .saturating_add(session::MAX_PAYLOAD_BYTES + session::HEADER_BYTES)
            <= bound
    }

    /// Whether a frame of any size fits under `bound`, checked before a return
    /// to live output moves the viewport.
    pub(crate) fn can_push_frame(&self, bound: usize) -> bool {
        self.bytes
            .saturating_sub(self.replaceable_suffix(MessageClass::Frame).1)
            .saturating_add(MAX_FRAME_BYTES + session::HEADER_BYTES)
            <= bound
    }

    /// Whether a host selection phase's answer, frame and, when it finishes,
    /// copied text fit under `bound`, counting the unstarted frames the phase
    /// supersedes, checked before a gesture moves.
    pub(crate) fn can_push_host_selection(&self, bound: usize, finishing: bool) -> bool {
        let required = if finishing {
            MAX_FRAME_BYTES + session::MAX_COPY_BYTES + 9 + 3 * session::HEADER_BYTES
        } else {
            MAX_FRAME_BYTES + 2 * session::HEADER_BYTES
        };
        self.bytes
            .saturating_sub(self.replaceable_suffix(MessageClass::Frame).1)
            .saturating_add(required)
            <= bound
    }

    /// Whether an answer and a frame of any size fit under `bound`, replacing
    /// nothing.
    pub(crate) fn can_push_result_frame(&self, bound: usize) -> bool {
        self.bytes
            .saturating_add(MAX_FRAME_BYTES + 2 * session::HEADER_BYTES)
            <= bound
    }

    /// Drops the unstarted frames and previews at the end of the queue, so a
    /// selection phase's answer and frame replace an obsolete drag backlog.
    pub(crate) fn supersede_frames(&mut self) {
        let (remove, removed_bytes) = self.replaceable_suffix(MessageClass::Frame);
        self.messages.truncate(self.messages.len() - remove);
        self.bytes -= removed_bytes;
    }

    /// Writes until the queue empties or the peer would block; false when the
    /// peer has gone.
    fn flush(&mut self, mut write: impl FnMut(&[u8]) -> io::Result<usize>) -> io::Result<bool> {
        while let Some((message, _)) = self.messages.front() {
            match write(&message[self.offset..]) {
                Ok(0) => return Ok(false),
                Ok(written) => {
                    self.offset += written;
                    self.bytes -= written;
                    if self.offset == message.len() {
                        self.messages.pop_front();
                        self.offset = 0;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(true),
                Err(error) if peer_disconnected(&error) => return Ok(false),
                Err(error) => return Err(error),
            }
        }
        Ok(true)
    }
}

pub(crate) struct Peer {
    pub(crate) stream: UnixStream,
    read_closed: bool,
    pending: PendingWrite,
    /// An attached interactive client's messages, bounded by Bend.
    pub(crate) output: OutputQueue,
    pub(crate) output_bound: usize,
    session: Session,
    incoming: Vec<u8>,
    pub(crate) request: Option<ClientMessage>,
    /// A request that failed to decode, held for Bend to answer.
    pub(crate) refused: Option<Refusal>,
    /// The wheel Bend is deciding on now.
    pub(crate) wheel: Option<Wheel>,
    /// The pointer phase Bend is deciding on now.
    pub(crate) selection: Option<Selection>,
    last_metadata: Option<Metadata>,
}

/// A pointer phase Bend is deciding on, with the facts that describe it and
/// the mouse input it sends the terminal if Bend routes it there (ORB-C9).
pub(crate) struct Selection {
    pub(crate) action: SelectionAction,
    pub(crate) flags: Flags,
    pub(crate) routed: Vec<u8>,
}

/// A physical vertical wheel Bend is deciding on, with the input it sends the
/// terminal if Bend routes it there (ORB-C8).
pub(crate) struct Wheel {
    pub(crate) direction: VerticalDirection,
    pub(crate) routed: Vec<u8>,
}

/// What reading the attached client found.
enum Received {
    /// A request, or one that failed to decode, waits for Bend.
    Message,
    /// The client closed its write side between messages.
    Closed,
    Blocked,
}

/// An attached client before the host waits on it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClientState {
    Open,
    /// The client has gone; its attachment ends.
    Closed,
    /// The client's output passed its bound.
    Pressured,
}

/// What a ready attached client brought.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClientEvent {
    /// A request, or one that failed to decode, waits for Bend.
    Request,
    /// The client closed, fully or only its write half.
    Closed { full: bool },
}

/// What the observer's socket brought; an observer may send nothing.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ObserverInput {
    Quiet,
    Closed,
    /// A message header, or a malformed one, which Bend refuses with
    /// Failure(Protocol).
    Message,
    /// Another ORBS version, dropped without an answer.
    Unsupported,
}

/// Whether the observer's queue took its metadata or had no room for it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ObserverSend {
    Sent,
    Pressured,
}

/// Why the attached client's request was refused.
pub(crate) enum Refusal {
    Protocol(String),
    Version,
}

#[derive(Default)]
enum Session {
    #[default]
    Raw,
    ReadingRole(Vec<u8>),
    Negotiated(Role),
    SendingRole(Role),
    Attached(Role),
    AbortedHello,
    Disconnected,
    /// A message did not fit the interactive client's output bound.
    Pressured,
    /// Exited is queued; the output drains before Bend closes the socket.
    Exiting,
    /// The first message was not a role request; the detail answers it.
    Refused(String),
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Interactive,
    Metadata,
}

impl Peer {
    pub(crate) fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            read_closed: false,
            pending: PendingWrite::default(),
            output: OutputQueue::default(),
            output_bound: 0,
            session: Session::default(),
            incoming: Vec::new(),
            request: None,
            refused: None,
            wheel: None,
            selection: None,
            last_metadata: None,
        }
    }
}

impl Peer {
    fn flush(&mut self) -> io::Result<Step> {
        let Self {
            stream, pending, ..
        } = self;
        pending.flush(stream.as_raw_fd(), |bytes| stream.write(bytes))
    }

    /// Writes the observer's queued messages without waiting; the rest stays
    /// queued until its socket has room. A gone observer is an error.
    pub(crate) fn flush_observer(&mut self) -> io::Result<()> {
        let Self { stream, output, .. } = self;
        let flushed = output.flush(|bytes| stream.write(bytes));
        if !matches!(flushed, Ok(true)) {
            self.session = Session::Failed;
        }
        match flushed {
            Ok(true) => Ok(()),
            Ok(false) => Err(errno(libc::EPIPE)),
            Err(error) => Err(error),
        }
    }

    /// Reads a peer's first message, or None while it is incomplete; Bend
    /// decides whether its role is admitted.
    pub(crate) fn recv_role(&mut self) -> io::Result<Option<FirstMessage>> {
        if matches!(self.session, Session::Raw) {
            if !self.pending.bytes.is_empty() {
                return Err(errno(libc::EBUSY));
            }
            self.session = Session::ReadingRole(Vec::with_capacity(session::HEADER_BYTES));
        }
        loop {
            let Session::ReadingRole(request) = &mut self.session else {
                return Err(errno(libc::EALREADY));
            };
            if request.len() == session::HEADER_BYTES {
                break;
            }
            let mut bytes = [0; session::HEADER_BYTES];
            match self
                .stream
                .read(&mut bytes[..session::HEADER_BYTES - request.len()])
            {
                Ok(0) => return Err(errno(libc::EPIPE)),
                Ok(len) => request.extend_from_slice(&bytes[..len]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) => return Err(error),
            }
        }

        let Session::ReadingRole(request) = &self.session else {
            unreachable!("role request was collected")
        };
        // Role requests have no payload, so a valid header with one is refused
        // without reading it.
        let decoded = match session::client_message_len(request) {
            Ok(Some(length)) if length > session::HEADER_BYTES => None,
            Ok(_) => Some(session::decode_client_message(request)),
            Err(error) => Some(Err(error)),
        };
        let message = match decoded {
            Some(Ok(ClientMessage::Hello)) => {
                self.session = Session::Negotiated(Role::Interactive);
                FirstMessage::Hello
            }
            Some(Ok(ClientMessage::ObserveMetadata)) => {
                self.session = Session::Negotiated(Role::Metadata);
                FirstMessage::ObserveMetadata
            }
            Some(Err(session::Error::UnsupportedVersion { .. })) => {
                self.session = Session::Disconnected;
                FirstMessage::Unsupported
            }
            Some(Err(error)) => {
                self.session = Session::Refused(error.to_string());
                FirstMessage::Refused
            }
            None | Some(Ok(_)) => {
                self.session = Session::Refused(
                    "first client message must be Hello or ObserveMetadata".into(),
                );
                FirstMessage::Refused
            }
        };
        Ok(Some(message))
    }

    /// A detached peer that closed before its role is answered as a Hello;
    /// its attachment then ends at the next wait.
    pub(crate) fn abort_hello(&mut self) -> FirstMessage {
        self.session = Session::AbortedHello;
        FirstMessage::Hello
    }

    pub(crate) fn send_role(&mut self, role: Role) -> io::Result<Step> {
        if role == Role::Interactive && matches!(self.session, Session::AbortedHello) {
            self.session = Session::Disconnected;
            return Ok(Step::Done);
        }
        if matches!(self.session, Session::Negotiated(current) if current == role) {
            let response = session::encode_server_message(&match role {
                Role::Interactive => ServerMessage::Attached,
                Role::Metadata => ServerMessage::ObservingMetadata,
            })
            .map_err(|_| errno(libc::EPROTO))?;
            self.pending.start(&response)?;
            self.session = Session::SendingRole(role);
        }
        if !matches!(self.session, Session::SendingRole(current) if current == role) {
            return Err(errno(libc::EPROTO));
        }
        let step = match self.flush() {
            Ok(step) => step,
            Err(error) if role == Role::Interactive && peer_disconnected(&error) => {
                self.session = Session::Disconnected;
                return Ok(Step::Done);
            }
            Err(error) => {
                self.session = Session::Failed;
                return Err(error);
            }
        };
        if step == Step::Done {
            self.session = Session::Attached(role);
        }
        Ok(step)
    }

    /// Queues one message for the interactive client and writes what the
    /// socket takes now; it never waits for the client to read.
    pub(crate) fn send_server(
        &mut self,
        message: impl FnOnce() -> io::Result<ServerMessage>,
    ) -> io::Result<()> {
        // Let the current Bend effect finish; wake reports the lost or pressured peer.
        if matches!(self.session, Session::Disconnected | Session::Pressured) {
            return Ok(());
        }
        if !matches!(self.session, Session::Attached(Role::Interactive)) || self.request.is_some() {
            return Err(errno(libc::EPROTO));
        }
        let message = message()?;
        let class = MessageClass::of(&message);
        let encoded = session::encode_server_message(&message).map_err(|_| errno(libc::EPROTO))?;
        if !self.output.push(encoded, class, self.output_bound) {
            self.session = Session::Pressured;
            return Ok(());
        }
        self.flush_output()
    }

    /// Writes queued output without blocking; a lost peer becomes Disconnected.
    fn flush_output(&mut self) -> io::Result<()> {
        let Self { stream, output, .. } = self;
        match output.flush(|bytes| stream.write(bytes)) {
            Ok(true) => Ok(()),
            Ok(false) => {
                self.session = Session::Disconnected;
                Ok(())
            }
            Err(code) => {
                self.session = Session::Failed;
                Err(code)
            }
        }
    }

    /// Queues Exited once, then waits until the client has taken all output,
    /// because Bend closes the socket next.
    pub(crate) fn send_exited(&mut self, code: i32) -> io::Result<Step> {
        if matches!(self.session, Session::Attached(Role::Interactive)) {
            self.send_server(|| Ok(ServerMessage::Exited { code }))?;
            if matches!(self.session, Session::Attached(Role::Interactive)) {
                self.session = Session::Exiting;
            }
        }
        match self.session {
            Session::Exiting => {
                self.flush_output()?;
                Ok(match self.session {
                    Session::Exiting if !self.output.is_empty() => {
                        Step::Blocked(self.stream.as_raw_fd())
                    }
                    _ => Step::Done,
                })
            }
            Session::Disconnected | Session::Pressured => Ok(Step::Done),
            _ => Err(errno(libc::EPROTO)),
        }
    }

    fn recv_request(&mut self) -> io::Result<Received> {
        if !matches!(self.session, Session::Attached(Role::Interactive)) {
            return Err(errno(libc::EPROTO));
        }
        if self.request.is_some() || self.refused.is_some() {
            return Err(errno(libc::EBUSY));
        }
        loop {
            let target = if self.incoming.len() < session::HEADER_BYTES {
                session::HEADER_BYTES
            } else {
                match session::client_message_len(&self.incoming) {
                    Ok(length) => length.ok_or_else(|| errno(libc::EPROTO))?,
                    Err(error) => return Ok(self.refuse(error)),
                }
            };
            if self.incoming.len() == target {
                break;
            }
            let mut bytes = [0; 8192];
            let remaining = target - self.incoming.len();
            let read_len = remaining.min(bytes.len());
            match self.stream.read(&mut bytes[..read_len]) {
                Ok(0) if self.incoming.is_empty() => return Ok(Received::Closed),
                Ok(0) => return Err(errno(libc::EPIPE)),
                Ok(len) => self.incoming.extend_from_slice(&bytes[..len]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(Received::Blocked);
                }
                Err(error) => return Err(error),
            }
        }
        let request = match session::decode_client_message(&self.incoming) {
            Ok(request) => request,
            Err(error) => return Ok(self.refuse(error)),
        };
        self.incoming.clear();
        self.request = Some(request);
        Ok(Received::Message)
    }

    /// Holds a request that failed to decode for Bend to answer, and discards
    /// the rest of the input, since the attachment ends after it.
    fn refuse(&mut self, error: session::Error) -> Received {
        self.incoming.clear();
        self.refused = Some(match error {
            session::Error::UnsupportedVersion { .. } => Refusal::Version,
            error => Refusal::Protocol(error.to_string()),
        });
        Received::Message
    }

    /// Writes queued output and reports an attachment that has already ended
    /// or passed its bound, before the host waits on the client.
    pub(crate) fn prepare_wait(&mut self) -> io::Result<ClientState> {
        match self.session {
            Session::Disconnected => return Ok(ClientState::Closed),
            Session::Pressured => return Ok(ClientState::Pressured),
            Session::Attached(Role::Interactive) => {}
            _ => return Err(errno(libc::EPROTO)),
        }
        if self.request.is_some() {
            return Err(errno(libc::EBUSY));
        }
        self.flush_output()?;
        Ok(if matches!(self.session, Session::Disconnected) {
            ClientState::Closed
        } else {
            ClientState::Open
        })
    }

    /// The events the host waits for: requests until the client closes its
    /// write side, and room to write while output is queued. A full close is
    /// reported as a hangup either way.
    pub(crate) fn wait_events(&self) -> i16 {
        (if self.read_closed {
            0
        } else {
            platform::POLLIN
        }) | if self.output.is_empty() {
            0
        } else {
            platform::POLLOUT
        }
    }

    /// Serves a ready client: a whole request, or its close. Room to write
    /// alone brings nothing; the next wait flushes the output first.
    pub(crate) fn on_ready(&mut self, revents: i16) -> io::Result<Option<ClientEvent>> {
        if revents & (platform::POLLIN | platform::POLLHUP | platform::POLLERR) == 0 {
            return Ok(None);
        }
        if self.read_closed {
            return Ok(self
                .closed_fully()
                .then_some(ClientEvent::Closed { full: true }));
        }
        let full = match self.recv_request() {
            Ok(Received::Message) => return Ok(Some(ClientEvent::Request)),
            Ok(Received::Blocked) => return Ok(None),
            Ok(Received::Closed) => self.closed_fully(),
            Err(error) if peer_disconnected(&error) => true,
            Err(error) => return Err(error),
        };
        self.read_closed = true;
        Ok(Some(ClientEvent::Closed { full }))
    }

    /// Writes what a refused client's queue still holds; true once it is all
    /// written or the client is gone.
    pub(crate) fn drain(&mut self) -> bool {
        let _ = self.flush_output();
        self.output.is_empty() || matches!(self.session, Session::Disconnected | Session::Failed)
    }

    /// Whether a client whose write side closed has closed its read side too.
    /// macOS reports POLLHUP for either, so a zero-length send asks the
    /// socket: it fails only once the peer can no longer read.
    pub(crate) fn closed_fully(&self) -> bool {
        platform::send_nothing(&self.stream).is_err_and(|error| peer_disconnected(&error))
    }

    /// Whether the client closed its write side; it may still read.
    pub(crate) fn half_closed(&self) -> bool {
        self.read_closed
    }

    /// Reads what the observer sent. Once a whole header has arrived it is a
    /// message the observer may not send, held for Bend to refuse
    /// without reading the rest.
    pub(crate) fn observer_input(&mut self) -> io::Result<ObserverInput> {
        if !matches!(self.session, Session::Attached(Role::Metadata)) {
            return Err(errno(libc::EPROTO));
        }
        if self.refused.is_some() {
            return Ok(ObserverInput::Message);
        }
        while self.incoming.len() < session::HEADER_BYTES {
            let mut bytes = [0; session::HEADER_BYTES];
            let remaining = session::HEADER_BYTES - self.incoming.len();
            match self.stream.read(&mut bytes[..remaining]) {
                Ok(0) => return Ok(ObserverInput::Closed),
                Ok(len) => self.incoming.extend_from_slice(&bytes[..len]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(ObserverInput::Quiet);
                }
                Err(error) => return Err(error),
            }
        }
        let detail = match session::client_message_len(&self.incoming) {
            Err(session::Error::UnsupportedVersion { .. }) => {
                return Ok(ObserverInput::Unsupported);
            }
            Err(error) => error.to_string(),
            Ok(_) => "metadata observers cannot send messages".to_owned(),
        };
        self.refused = Some(Refusal::Protocol(detail));
        Ok(ObserverInput::Message)
    }

    /// Queues Failure(Protocol) behind the observer's metadata for the message
    /// Bend refused, and writes what the socket takes now; Bend closes the
    /// observer next.
    pub(crate) fn refuse_observer(&mut self) -> io::Result<()> {
        let Some(Refusal::Protocol(detail)) = self.refused.take() else {
            return Err(errno(libc::EPROTO));
        };
        self.discard_input();
        let failure = ServerMessage::Failure(Failure {
            code: FailureCode::Protocol,
            detail,
        });
        let encoded = session::encode_server_message(&failure).map_err(|_| errno(libc::EPROTO))?;
        if self
            .output
            .push(encoded, MessageClass::Ordered, self.output_bound)
        {
            let _ = self.flush_output();
        }
        Ok(())
    }

    /// Discards input that has already arrived, so
    /// closing with unread input does not reset the peer before it reads its
    /// answer.
    fn discard_input(&mut self) {
        self.incoming.clear();
        let mut discard = [0; 4096];
        for _ in 0..16 {
            if !matches!(self.stream.read(&mut discard), Ok(read) if read > 0) {
                break;
            }
        }
    }

    /// Whether `title` and `working_directory` differ from the metadata last
    /// queued for the observer, or none was queued.
    pub(crate) fn metadata_changed(&self, title: &str, working_directory: &str) -> bool {
        self.last_metadata
            .as_ref()
            .is_none_or(|last| last.title != title || last.working_directory != working_directory)
    }

    /// Whether no metadata was queued for the observer yet.
    pub(crate) fn metadata_fresh(&self) -> bool {
        self.last_metadata.is_none()
    }

    /// Queues metadata for the observer and writes what the socket takes now.
    /// `ordered` metadata stays queued, and other metadata replaces metadata
    /// that has not started, as Bend decides; only metadata the bounded queue
    /// cannot take is pressure.
    pub(crate) fn send_metadata(
        &mut self,
        metadata: Metadata,
        ordered: bool,
    ) -> io::Result<ObserverSend> {
        if !matches!(self.session, Session::Attached(Role::Metadata)) {
            return Err(errno(libc::EPROTO));
        }
        let class = if ordered {
            MessageClass::Ordered
        } else {
            MessageClass::Metadata
        };
        let encoded = session::encode_server_message(&ServerMessage::Metadata(metadata.clone()))
            .map_err(|_| errno(libc::EPROTO))?;
        if !self.output.push(encoded, class, self.output_bound) {
            return Ok(ObserverSend::Pressured);
        }
        self.last_metadata = Some(metadata);
        self.flush_observer()?;
        Ok(ObserverSend::Sent)
    }

    /// Queues Exited behind the observer's output and writes what the socket
    /// takes now; Bend then closes the observer.
    pub(crate) fn send_observer_exited(&mut self, code: i32) -> io::Result<()> {
        if !matches!(self.session, Session::Attached(Role::Metadata)) {
            return Err(errno(libc::EPROTO));
        }
        let encoded = session::encode_server_message(&ServerMessage::Exited { code })
            .map_err(|_| errno(libc::EPROTO))?;
        if self
            .output
            .push(encoded, MessageClass::Ordered, self.output_bound)
        {
            self.flush_observer()?;
        }
        Ok(())
    }
}

impl Peer {
    /// Answers a negotiated peer whose role is taken with Busy, then closes it.
    pub(crate) fn reject_busy(mut self) -> io::Result<()> {
        if !matches!(self.session, Session::Negotiated(_)) {
            return Err(errno(libc::EPROTO));
        }
        let bytes = session::encode_server_message(&ServerMessage::Busy)
            .map_err(|_| errno(libc::EPROTO))?;
        let _ = self.stream.write_all(&bytes);
        Ok(())
    }

    /// Answers a refused first message with Failure(Protocol), then closes the
    /// peer.
    pub(crate) fn refuse_peer(mut self) -> io::Result<()> {
        let Session::Refused(detail) = std::mem::take(&mut self.session) else {
            return Err(errno(libc::EPROTO));
        };
        self.discard_input();
        let failure = ServerMessage::Failure(Failure {
            code: FailureCode::Protocol,
            detail,
        });
        if let Ok(bytes) = session::encode_server_message(&failure) {
            let _ = self.stream.write_all(&bytes);
        }
        Ok(())
    }

    /// Answers the attached client's refused request with Failure(Protocol)
    /// and the decoder's detail.
    pub(crate) fn answer_refused(&mut self) -> io::Result<()> {
        let Some(Refusal::Protocol(detail)) = self.refused.take() else {
            return Err(errno(libc::EPROTO));
        };
        self.send_server(|| {
            Ok(ServerMessage::Failure(Failure {
                code: FailureCode::Protocol,
                detail,
            }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use std::net::Shutdown;

    fn read_server_message(stream: &mut UnixStream) -> ServerMessage {
        // macOS refuses a timeout on a socket whose peer already closed; the
        // answer is buffered by then.
        let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(1)));
        let mut header = [0; session::HEADER_BYTES];
        stream.read_exact(&mut header).unwrap();
        let length = session::server_message_len(&header).unwrap().unwrap();
        let mut bytes = Vec::from(header);
        bytes.resize(length, 0);
        stream
            .read_exact(&mut bytes[session::HEADER_BYTES..])
            .unwrap();
        session::decode_server_message(&bytes).unwrap()
    }

    fn open_metadata_observer() -> (Peer, UnixStream) {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut socket = Peer::new(stream);
        socket.output_bound = 1 << 20;
        peer.write_all(&session::encode_client_message(&ClientMessage::ObserveMetadata).unwrap())
            .unwrap();
        assert_eq!(
            socket.recv_role().unwrap(),
            Some(FirstMessage::ObserveMetadata)
        );
        assert_eq!(socket.send_role(Role::Metadata).unwrap(), Step::Done);
        assert!(matches!(
            read_server_message(&mut peer),
            ServerMessage::ObservingMetadata
        ));
        (socket, peer)
    }

    #[test]
    fn pending_write_resumes_without_duplicate_bytes() {
        let mut pending = PendingWrite::default();
        pending.start(b"abc").unwrap();
        let mut writes = [Ok(1), Err(io::Error::from(io::ErrorKind::WouldBlock))].into_iter();
        let blocked = pending.flush(17, |_| writes.next().unwrap()).unwrap();
        assert_eq!(blocked, Step::Blocked(17));
        assert_eq!(code(pending.start(b"x")), Some(libc::EBUSY));
        let ready = pending
            .flush(17, |bytes| {
                assert_eq!(bytes, b"bc");
                Ok(bytes.len())
            })
            .unwrap();
        assert_eq!((ready, pending.bytes.len()), (Step::Done, 0));
    }

    #[test]
    fn attached_socket_keeps_canonical_requests_typed() {
        // A child forked meanwhile would hold the peer open past its close.
        let _guard = FORK_AND_SOCKET_TEST.lock().unwrap();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut socket = Peer::new(stream);
        socket.session = Session::Attached(Role::Interactive);
        let size = session::SurfaceSize {
            cols: 100,
            rows: 30,
            screen_width: 1000,
            screen_height: 600,
            cell_width: 10,
            cell_height: 20,
            padding_top: 0,
            padding_bottom: 0,
            padding_left: 0,
            padding_right: 0,
        };
        peer.write_all(&session::encode_client_message(&ClientMessage::Resize(size)).unwrap())
            .unwrap();
        assert!(matches!(socket.recv_request().unwrap(), Received::Message));
        assert_eq!(socket.request, Some(ClientMessage::Resize(size)));

        // A ready client brings a whole request; room to write alone brings nothing.
        socket.request = None;
        assert_eq!(socket.prepare_wait().unwrap(), ClientState::Open);
        assert_eq!(socket.wait_events(), platform::POLLIN);
        assert_eq!(socket.on_ready(platform::POLLOUT).unwrap(), None);
        let paste = session::encode_client_message(&ClientMessage::Paste(b"x".to_vec())).unwrap();
        peer.write_all(&paste[..5]).unwrap();
        assert_eq!(socket.on_ready(platform::POLLIN).unwrap(), None);
        peer.write_all(&paste[5..]).unwrap();
        assert_eq!(
            socket.on_ready(platform::POLLIN).unwrap(),
            Some(ClientEvent::Request)
        );
        assert_eq!(socket.request, Some(ClientMessage::Paste(b"x".to_vec())));
        assert_eq!(code(socket.prepare_wait()), Some(libc::EBUSY));

        // A write-side close keeps the client for output, even when the
        // platform reports POLLHUP for it, as macOS does; a full close ends it.
        socket.request = None;
        peer.shutdown(Shutdown::Write).unwrap();
        assert_eq!(
            socket.on_ready(platform::POLLIN).unwrap(),
            Some(ClientEvent::Closed { full: false })
        );
        assert_eq!(socket.wait_events(), 0);
        assert!(socket.half_closed() && !socket.closed_fully());
        assert_eq!(socket.on_ready(platform::POLLHUP).unwrap(), None);
        drop(peer);
        assert!(socket.closed_fully());
        assert_eq!(
            socket.on_ready(platform::POLLHUP).unwrap(),
            Some(ClientEvent::Closed { full: true })
        );
        socket.session = Session::Raw;
        assert_eq!(code(socket.prepare_wait()), Some(libc::EPROTO));
    }

    #[test]
    fn failed_interactive_send_reports_full_close_before_the_wait() {
        let (stream, peer) = UnixStream::pair().unwrap();
        drop(peer);
        let mut socket = Peer::new(stream);
        socket.session = Session::Attached(Role::Interactive);
        socket.output_bound = 1 << 20;
        socket.send_server(|| Ok(ServerMessage::Accepted)).unwrap();
        assert!(matches!(socket.session, Session::Disconnected));
        socket.send_server(|| Err(errno(libc::EIO))).unwrap();
        assert_eq!(socket.prepare_wait().unwrap(), ClientState::Closed);
    }

    #[test]
    fn output_queue_replaces_unstarted_presentations_within_its_bound() {
        let mut queue = OutputQueue::default();
        assert!(queue.push(vec![0; 4], MessageClass::Ordered, 16));
        assert!(queue.push(vec![1; 4], MessageClass::Preview, 16));
        assert!(queue.push(vec![2; 4], MessageClass::Preview, 16));
        assert!(queue.push(vec![3; 6], MessageClass::Frame, 16));
        assert!(queue.push(vec![4; 6], MessageClass::Frame, 16));
        let classes: Vec<_> = queue.messages.iter().map(|(bytes, _)| bytes[0]).collect();
        assert_eq!((classes, queue.bytes), (vec![0, 4], 10));
        assert!(!queue.push(vec![5; 7], MessageClass::Ordered, 16));
        assert_eq!(queue.bytes, 10);

        let mut taken = Vec::new();
        let mut room = 5;
        assert!(
            queue
                .flush(|bytes| {
                    let length = bytes.len().min(room);
                    if length == 0 {
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                    room -= length;
                    taken.extend_from_slice(&bytes[..length]);
                    Ok(length)
                })
                .unwrap()
        );
        assert_eq!((taken.len(), queue.offset), (5, 1));
        // A started frame stays; the newer one queues behind it.
        assert!(queue.push(vec![6; 6], MessageClass::Frame, 16));
        assert_eq!(queue.messages.len(), 2);
    }

    #[test]
    fn output_queue_keeps_scroll_outcomes_and_admits_them_before_moving() {
        let mut queue = OutputQueue::default();
        assert!(queue.push(vec![1; 4], MessageClass::Preview, 64));
        assert!(queue.push(vec![2; 4], MessageClass::Frame, 64));
        // A scroll outcome replaces unstarted presentations, and nothing replaces it.
        assert!(queue.push(vec![3; 8], MessageClass::ScrollOutcome, 64));
        assert!(queue.push(vec![4; 4], MessageClass::Frame, 64));
        assert!(queue.push(vec![5; 4], MessageClass::ScrollOutcome, 64));
        let kept: Vec<_> = queue.messages.iter().map(|(bytes, _)| bytes[0]).collect();
        assert_eq!(kept, vec![3, 5]);

        let largest = session::MAX_PAYLOAD_BYTES + session::HEADER_BYTES;
        let mut queue = OutputQueue::default();
        assert!(queue.can_push_scroll_outcome(largest));
        assert!(queue.push(vec![0; 4], MessageClass::Frame, usize::MAX));
        // Replaceable output does not count against a scroll outcome.
        assert!(queue.can_push_scroll_outcome(largest));
        // Only an unstarted suffix is replaceable: an ordered message keeps the
        // frame before it.
        assert!(queue.push(vec![0; 4], MessageClass::Ordered, usize::MAX));
        assert!(!queue.can_push_scroll_outcome(largest + 7));
        assert!(queue.can_push_scroll_outcome(largest + 8));

        // A frame needs room for the largest frame, not the largest payload.
        let frame = MAX_FRAME_BYTES + session::HEADER_BYTES;
        assert!(!queue.can_push_frame(frame + 7));
        assert!(queue.can_push_frame(frame + 8));
        assert!(!queue.can_push_scroll_outcome(frame + 8));
    }

    /// Under the bound the worker sends, a maximum frame whose first byte is
    /// already written still leaves room for one maximum scroll outcome
    /// (ORB-C7).
    #[test]
    fn worker_bound_admits_a_maximum_scroll_outcome_behind_a_started_maximum_frame() {
        let bound = crate::host::built_worker_output_bound();
        let mut queue = OutputQueue::default();
        let frame = vec![0; MAX_FRAME_BYTES + session::HEADER_BYTES];
        assert!(queue.push(frame, MessageClass::Frame, bound));
        let mut writes = [Ok(1), Err(io::Error::from(io::ErrorKind::WouldBlock))].into_iter();
        assert!(queue.flush(|_| writes.next().unwrap()).unwrap());
        assert_eq!(queue.offset, 1);
        assert!(queue.can_push_scroll_outcome(bound));
        let outcome = vec![0; session::MAX_PAYLOAD_BYTES + session::HEADER_BYTES];
        assert!(queue.push(outcome, MessageClass::ScrollOutcome, bound));
        assert_eq!(queue.messages.len(), 2);
    }

    #[test]
    fn pressured_interactive_client_is_reported_without_waiting() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut socket = Peer::new(stream);
        socket.session = Session::Attached(Role::Interactive);
        socket.output_bound = 64 * 1024;

        let mut sent = 0;
        while matches!(socket.session, Session::Attached(_)) {
            socket.send_server(|| Ok(ServerMessage::Accepted)).unwrap();
            sent += 1;
            assert!(sent < 1_000_000, "the bound never applied");
        }
        assert!(matches!(socket.session, Session::Pressured));
        assert!(socket.output.bytes <= socket.output_bound);
        assert_eq!(socket.prepare_wait().unwrap(), ClientState::Pressured);

        // Closing leaves the client whole messages, then at most a prefix of one.
        drop(socket);
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).unwrap();
        let accepted = session::encode_server_message(&ServerMessage::Accepted).unwrap();
        let whole = bytes.len() / accepted.len();
        assert!(whole > 0 && whole < sent);
        for message in bytes.chunks(accepted.len()) {
            assert_eq!(message, &accepted[..message.len()]);
        }
    }

    #[test]
    fn waits_write_queued_output_as_the_client_reads() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        peer.set_nonblocking(true).unwrap();
        let mut socket = Peer::new(stream);
        socket.session = Session::Attached(Role::Interactive);
        socket.output_bound = 64 * 1024 * 1024;
        while socket.output.is_empty() {
            socket.send_server(|| Ok(ServerMessage::Accepted)).unwrap();
        }
        let queued = socket.output.bytes;
        assert_eq!(socket.wait_events(), platform::POLLIN | platform::POLLOUT);

        let mut buffer = vec![0; 1 << 20];
        let mut read = 0;
        while let Ok(length @ 1..) = peer.read(&mut buffer) {
            read += length;
        }
        assert!(read > 0);
        assert_eq!(socket.prepare_wait().unwrap(), ClientState::Open);
        assert!(socket.output.bytes < queued);
    }

    #[test]
    fn socket_protocol_mechanics_preserve_canonical_bytes_and_bend_policy() {
        let _guard = FORK_AND_SOCKET_TEST.lock().unwrap();
        const HELLO: [u8; 12] = [b'O', b'R', b'B', b'S', 14, 0, 1, 0, 0, 0, 0, 0];
        const ATTACHED: [u8; 12] = [b'O', b'R', b'B', b'S', 14, 0, 129, 0, 0, 0, 0, 0];

        assert_eq!(
            session::encode_client_message(&ClientMessage::Hello).unwrap(),
            HELLO
        );
        assert_eq!(
            session::encode_server_message(&ServerMessage::Attached).unwrap(),
            ATTACHED
        );

        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut socket = Peer::new(stream);
        assert_eq!(socket.recv_role().unwrap(), None);
        peer.write_all(&HELLO[..5]).unwrap();
        assert_eq!(socket.recv_role().unwrap(), None);
        peer.write_all(&HELLO[5..]).unwrap();
        assert_eq!(socket.recv_role().unwrap(), Some(FirstMessage::Hello));
        assert_eq!(code(socket.recv_role()), Some(libc::EALREADY));
        socket.output_bound = 1 << 20;
        assert_eq!(socket.send_role(Role::Interactive).unwrap(), Step::Done);
        let mut response = [0; ATTACHED.len()];
        peer.read_exact(&mut response).unwrap();
        assert_eq!(response, ATTACHED);
        drop(socket);

        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut socket = Peer::new(stream);
        peer.write_all(&session::encode_client_message(&ClientMessage::ObserveMetadata).unwrap())
            .unwrap();
        assert_eq!(
            socket.recv_role().unwrap().unwrap(),
            FirstMessage::ObserveMetadata
        );
        assert_eq!(socket.send_role(Role::Metadata).unwrap(), Step::Done);
        let mut observing = [0; ATTACHED.len()];
        peer.read_exact(&mut observing).unwrap();
        assert_eq!(
            observing.as_slice(),
            session::encode_server_message(&ServerMessage::ObservingMetadata).unwrap()
        );
        drop(socket);

        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        peer.set_nonblocking(true).unwrap();
        let mut socket = Peer::new(stream);
        let mut incompatible = HELLO;
        incompatible[4..6].copy_from_slice(&(session::VERSION + 1).to_le_bytes());
        peer.write_all(&incompatible).unwrap();
        assert_eq!(
            socket.recv_role().unwrap().unwrap(),
            FirstMessage::Unsupported
        );
        drop(socket);
        assert_eq!(peer.read(&mut response).unwrap(), 0);

        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut socket = Peer::new(stream);
        let mut malformed = HELLO;
        malformed[6] = 255;
        peer.write_all(&malformed).unwrap();
        assert_eq!(socket.recv_role().unwrap().unwrap(), FirstMessage::Refused);
        socket.refuse_peer().unwrap();
        assert!(matches!(
            read_server_message(&mut peer),
            ServerMessage::Failure(Failure {
                code: FailureCode::Protocol,
                ..
            })
        ));

        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut socket = Peer::new(stream);
        let focus = ClientMessage::Focus(session::FocusEvent::Gained);
        peer.write_all(&session::encode_client_message(&focus).unwrap())
            .unwrap();
        assert_eq!(socket.recv_role().unwrap().unwrap(), FirstMessage::Refused);
        socket.refuse_peer().unwrap();
        assert_eq!(
            read_server_message(&mut peer),
            ServerMessage::Failure(Failure {
                code: FailureCode::Protocol,
                detail: "first client message must be Hello or ObserveMetadata".into(),
            })
        );

        let (stream, peer) = UnixStream::pair().unwrap();
        peer.shutdown(Shutdown::Both).unwrap();
        drop(peer);
        let mut socket = Peer::new(stream);
        socket.session = Session::Negotiated(Role::Interactive);
        assert_eq!(socket.send_role(Role::Interactive).unwrap(), Step::Done);
        assert!(matches!(socket.session, Session::Disconnected));
        assert_eq!(
            code(socket.send_role(Role::Interactive)),
            Some(libc::EPROTO)
        );
    }

    #[test]
    fn metadata_observer_is_canonical_read_only_and_reports_changes() {
        let _guard = FORK_AND_SOCKET_TEST.lock().unwrap();
        let (mut observer, mut peer) = open_metadata_observer();

        let directory = "file:///tmp/orbit";
        let metadata = |revision, title: &str| Metadata {
            revision,
            title: title.into(),
            working_directory: directory.into(),
        };
        assert!(observer.metadata_fresh());
        assert!(observer.metadata_changed("initial", directory));
        assert_eq!(
            observer
                .send_metadata(metadata(1, "initial"), true)
                .unwrap(),
            ObserverSend::Sent
        );
        assert!(matches!(
            read_server_message(&mut peer),
            ServerMessage::Metadata(Metadata { revision: 1, title, .. })
                if title == "initial"
        ));
        // Bend asks whether metadata repeats the last queued.
        assert!(!observer.metadata_fresh());
        assert!(!observer.metadata_changed("initial", directory));
        assert!(observer.metadata_changed("initial", "file:///tmp"));
        assert!(observer.metadata_changed("changed", directory));

        assert_eq!(
            observer
                .send_metadata(metadata(3, "changed"), false)
                .unwrap(),
            ObserverSend::Sent
        );
        assert!(matches!(
            read_server_message(&mut peer),
            ServerMessage::Metadata(Metadata { revision: 3, title, .. })
                if title == "changed"
        ));
        assert_eq!(observer.observer_input().unwrap(), ObserverInput::Quiet);
        observer.send_observer_exited(7).unwrap();
        assert!(matches!(
            read_server_message(&mut peer),
            ServerMessage::Exited { code: 7 }
        ));
        drop(observer);
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);

        let (mut observer, mut peer) = open_metadata_observer();
        peer.write_all(
            &session::encode_client_message(&ClientMessage::Focus(session::FocusEvent::Gained))
                .unwrap(),
        )
        .unwrap();
        // An observer may not send requests: the message is refused with
        // Failure(Protocol).
        assert_eq!(observer.observer_input().unwrap(), ObserverInput::Message);
        observer.refuse_observer().unwrap();
        match read_server_message(&mut peer) {
            ServerMessage::Failure(failure) => {
                assert_eq!(failure.code, FailureCode::Protocol);
                assert_eq!(failure.detail, "metadata observers cannot send messages");
            }
            other => panic!("expected Failure, received {other:?}"),
        }

        // Another ORBS version is dropped without an answer.
        let (mut observer, mut peer) = open_metadata_observer();
        let mut other = session::encode_client_message(&ClientMessage::Hello).unwrap();
        other[4..6].copy_from_slice(&(session::VERSION + 1).to_le_bytes());
        peer.write_all(&other).unwrap();
        assert_eq!(
            observer.observer_input().unwrap(),
            ObserverInput::Unsupported
        );
    }

    #[test]
    fn nonreading_observer_converges_on_the_latest_metadata() {
        let _guard = FORK_AND_SOCKET_TEST.lock().unwrap();
        let (mut observer, mut peer) = open_metadata_observer();
        platform::set_send_buffer(&observer.stream, 4096).unwrap();

        let title = "x".repeat(1000);
        let metadata = |revision| Metadata {
            revision,
            title: format!("{revision:04}-{title}"),
            working_directory: String::new(),
        };
        // Far more than the socket holds: every change is queued, and each
        // newer one replaces the unsent one before it.
        for revision in 1..=32 {
            assert_eq!(
                observer
                    .send_metadata(metadata(revision), revision == 1)
                    .unwrap(),
                ObserverSend::Sent
            );
        }
        assert!(!observer.output.is_empty());
        assert!(observer.output.bytes < 3 * 1024);

        peer.set_nonblocking(true).unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        while !observer.output.is_empty() {
            match peer.read(&mut buffer) {
                Ok(read) => bytes.extend_from_slice(&buffer[..read]),
                Err(error) => assert_eq!(error.kind(), io::ErrorKind::WouldBlock),
            }
            observer.flush_observer().unwrap();
        }
        while let Ok(read @ 1..) = peer.read(&mut buffer) {
            bytes.extend_from_slice(&buffer[..read]);
        }
        let mut revisions = Vec::new();
        let mut rest = bytes.as_slice();
        while !rest.is_empty() {
            let length = session::server_message_len(rest).unwrap().unwrap();
            let ServerMessage::Metadata(received) =
                session::decode_server_message(&rest[..length]).unwrap()
            else {
                panic!("the observer received something other than metadata");
            };
            assert_eq!(received, metadata(received.revision));
            revisions.push(received.revision);
            rest = &rest[length..];
        }
        assert_eq!(revisions.first(), Some(&1));
        assert_eq!(revisions.last(), Some(&32));
        assert!(revisions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(revisions.len() < 32, "{revisions:?}");
    }

    #[test]
    fn observer_pressure_is_metadata_its_queue_cannot_take() {
        let _guard = FORK_AND_SOCKET_TEST.lock().unwrap();
        let (mut observer, _peer) = open_metadata_observer();
        let metadata = |revision, title: String| Metadata {
            revision,
            title,
            working_directory: String::new(),
        };
        // A socket that takes nothing more keeps the first metadata queued,
        // and nothing replaces it.
        while observer.stream.write(&[0; 65536]).is_ok() {}
        assert_eq!(
            observer
                .send_metadata(metadata(1, "x".into()), true)
                .unwrap(),
            ObserverSend::Sent
        );
        assert!(!observer.output.is_empty());
        observer.output_bound = observer.output.bytes + 64;
        assert_eq!(
            observer
                .send_metadata(metadata(2, "y".repeat(100)), false)
                .unwrap(),
            ObserverSend::Pressured
        );
        assert_eq!(
            observer
                .send_metadata(metadata(3, "z".into()), false)
                .unwrap(),
            ObserverSend::Sent
        );
    }

    #[test]
    fn listener_enforces_private_exact_socket_cleanup() {
        // A child forked meanwhile would hold the parent's lock past its close.
        let _guard = FORK_AND_SOCKET_TEST.lock().unwrap();
        let directory = TestDir::new();
        let path = directory.0.join("orbit.sock");
        let listener = Listener::open(&path).unwrap();
        assert_eq!(fs::symlink_metadata(&path).unwrap().mode() & 0o777, 0o600);
        // A loser names the cause and the path.
        let message = |result: io::Result<Listener>| result.err().unwrap().to_string();
        assert_eq!(
            message(Listener::open(&path)),
            format!("server already listening at {}", path.display())
        );
        let claim = File::open(&directory.0).unwrap();
        claim.lock().unwrap();
        assert_eq!(
            message(Listener::open(&path)),
            format!("socket claim already in progress at {}", path.display())
        );
        drop(claim);
        drop(listener);
        assert!(!path.exists());

        let mut listener = Listener::open(&path).unwrap();
        fs::remove_file(&path).unwrap();
        let replacement = UnixListener::bind(&path).unwrap();
        listener.unlink_owned().unwrap();
        drop(listener);
        assert!(path.exists());
        drop(replacement);
        fs::remove_file(&path).unwrap();

        let replacement = UnixListener::bind(&path).unwrap();
        drop(SocketGuard {
            path: path.clone(),
            identity: None,
        });
        assert!(path.exists());
        drop(replacement);
        fs::remove_file(&path).unwrap();

        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(code(Listener::open(&path)), Some(libc::EACCES));
    }
}
