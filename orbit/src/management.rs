//! Management of ORB-C13: a private
//! management socket beside the presentation socket, an exact 0600 live
//! record published at Ready through the launcher's ready claim, one
//! management connection with bounded requests, and a tombstone that
//! replaces the record when the run ends. Every path and identity is
//! checked here; Bend decides every answer (src/management.bend), including
//! whether ORB-C15's `ProgramStatus` gets the records, ORB-C17's
//! `ReadVisible` the visible text and ORB-C5's lease input the PTY. The
//! connection also holds the process its lease holder names for ORB-C3's
//! admission.

use crate::{
    Timer,
    boundary::{Fact, Foreground, ManagementEvent as Event},
    errno, platform,
    socket::Listener,
};
use orbit_protocol::management::{
    self as protocol, ClientMessage, EndpointIdentity, Failure, FailureCode, LeaseInput,
    LiveIdentity, ObjectIdentity, ProcessOutcome, ProgramStatus, Record, Refusal, ServerMessage,
    TerminationReason, Tombstone, VisibleText,
};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::ExitStatus,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

/// The bound on a management connection's negotiation and on
/// writing the Stopped answer.
pub(crate) const NEGOTIATION_MS: u32 = 1000;

static NEXT_ARTIFACT: AtomicU64 = AtomicU64::new(0);

/// The identities `--management-v4` names.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Launch {
    pub(crate) session_id: String,
    pub(crate) run_id: String,
    pub(crate) component_generation: String,
}

impl Launch {
    /// Checks the bounds: each identity has 1 to 128 UTF-8 bytes.
    pub(crate) fn new(values: [&OsStr; 3]) -> Result<Self, String> {
        let mut checked = Vec::with_capacity(3);
        for (field, value) in ["Session ID", "run ID", "component generation"]
            .into_iter()
            .zip(values)
        {
            let value = value
                .to_str()
                .filter(|value| !value.is_empty() && value.len() <= protocol::MAX_ID_BYTES)
                .ok_or_else(|| {
                    format!(
                        "{field} must contain 1 through {} UTF-8 bytes",
                        protocol::MAX_ID_BYTES
                    )
                })?;
            checked.push(value.to_owned());
        }
        let [session_id, run_id, component_generation] =
            <[String; 3]>::try_from(checked).map_err(|_| "three identities".to_owned())?;
        Ok(Self {
            session_id,
            run_id,
            component_generation,
        })
    }

    /// Opens the management socket before the child starts.
    pub(crate) fn prepare(self, socket: &Path) -> io::Result<Prepared> {
        let path = artifact_path(socket, ".management");
        let listener = Listener::open(&path)?;
        Ok(Prepared {
            launch: self,
            path,
            listener,
        })
    }
}

pub(crate) struct Prepared {
    launch: Launch,
    path: PathBuf,
    listener: Listener,
}

/// A published run: its management socket, its record, the identity the
/// record carries, a connection waiting for Bend's admission, and the one
/// admitted connection.
pub(crate) struct Owner {
    pub(crate) listener: Listener,
    record: RecordGuard,
    pub(crate) identity: LiveIdentity,
    pub(crate) incoming: Option<UnixStream>,
    pub(crate) connection: Option<Connection>,
}

/// The admitted management connection: one request and one answer at a time,
/// each within its kind's bound, a one-second bound until Bend grants the
/// lease, and the same bound on writing each answer. A refused connection
/// stays until its Failure is written.
pub(crate) struct Connection {
    pub(crate) stream: UnixStream,
    peer_uid: u32,
    input: Vec<u8>,
    output: Vec<u8>,
    written: usize,
    close_after_flush: bool,
    /// Whether a second answer, a refusal of a request sent while the first
    /// was being written, waits in `output` behind it; nothing more is read
    /// until both are written.
    queued: bool,
    /// The bound on writing the answer in `output`, armed as it is queued: a
    /// peer that does not read it in time loses the connection, and with it
    /// the lease (ORB-C13, ORB-C17).
    reply: Timer,
    /// The negotiation bound, which Bend's Lease ends. Bend's session holds
    /// the slot; Rust reads this only to time the bound and to answer the
    /// lease Stopped after a termination signal, which the host carries out
    /// without the worker.
    pub(crate) deadline: Option<Timer>,
    /// Why the last request was refused, for the Failure Bend sends.
    refusal: Option<String>,
    /// The decoded lease input or name Bend is deciding on.
    pending: Option<Pending>,
    /// A ReadVisible request read while a synchronized update was held; the
    /// host reports it once none is and the output is written, and declines
    /// a request read meanwhile.
    held_read: bool,
    /// The process, by process ID and start identity, that the lease holder
    /// named as the only peer ORB-C3 admits; it dies with the connection.
    name: Option<(u32, u64)>,
}

/// A request's content, which the host keeps while Bend decides it.
enum Pending {
    Input(LeaseInput),
    Allow(u32, u64),
}

impl Prepared {
    /// Crosses Ready: publishes the live record beside the presentation
    /// socket. A launcher's empty retained record is claimed with an
    /// exclusive lock first, so only one run publishes it.
    pub(crate) fn publish(self, presentation: &Path) -> io::Result<Owner> {
        let uid = platform::effective_uid();
        let identity = LiveIdentity {
            session_id: self.launch.session_id,
            run_id: self.launch.run_id,
            component_generation: self.launch.component_generation,
            record_generation: protocol::RECORD_GENERATION,
            management_generation: protocol::VERSION,
            process_id: std::process::id(),
            process_start: platform::process_start_identity(std::process::id())?,
            uid,
            presentation: endpoint_identity(presentation)?,
            management: endpoint_identity(&self.path)?,
        };
        let live = protocol::encode_record(&Record::Live(identity.clone()))
            .map_err(|_| errno(libc::EPROTO))?;
        let record = RecordGuard::publish(&artifact_path(presentation, ".record"), &live)?;
        Ok(Owner {
            listener: self.listener,
            record,
            identity,
            incoming: None,
            connection: None,
        })
    }
}

impl Owner {
    /// Accepts one waiting management connection for Bend to admit.
    pub(crate) fn accept(&mut self) -> io::Result<Option<Event>> {
        match self.listener.listener.accept() {
            Ok((stream, _)) => {
                self.incoming = Some(stream);
                Ok(Some(Event::Connect))
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Admits the waiting connection under the negotiation bound.
    pub(crate) fn admit(&mut self) -> io::Result<()> {
        let stream = self.incoming.take().ok_or_else(|| errno(libc::EPROTO))?;
        stream.set_nonblocking(true)?;
        let mut deadline = Timer::default();
        deadline.set(Some(NEGOTIATION_MS));
        self.connection = Some(Connection {
            peer_uid: platform::peer_uid(&stream)?,
            stream,
            input: Vec::new(),
            output: Vec::new(),
            written: 0,
            close_after_flush: false,
            queued: false,
            reply: Timer::default(),
            deadline: Some(deadline),
            refusal: None,
            pending: None,
            held_read: false,
            name: None,
        });
        Ok(())
    }

    /// Answers the waiting connection Busy, if it takes it now, and closes it.
    pub(crate) fn busy(&mut self) -> io::Result<()> {
        if let Some(mut stream) = self.incoming.take() {
            stream.set_nonblocking(true)?;
            let busy = encode(&ServerMessage::Busy)?;
            let _ = stream.write(&busy);
        }
        Ok(())
    }

    /// Closes a connection whose negotiation bound passed, or whose answer
    /// was not written within its bound.
    pub(crate) fn expire(&mut self) -> Option<Event> {
        let connection = self.connection.as_mut()?;
        let negotiation = connection.deadline.as_mut().is_some_and(Timer::expired);
        if !(connection.reply.expired() || negotiation) {
            return None;
        }
        self.connection = None;
        Some(Event::Gone)
    }

    /// Reads the connection's next request and reports it for Bend: one
    /// whole request within its kind's bound, an Acquire whose identity,
    /// record, peer and sockets all match this run, and a ProgramStatus with
    /// the PTY's foreground group sampled as it is read. A request that breaks
    /// framing is Bad; a whole one that does not decode, or a read that brings
    /// more than one whole request, is Invalid, and the whole requests are
    /// dropped. A ReadVisible read while a synchronized update is `held` waits
    /// until none is (ORB-C17).
    pub(crate) fn read(
        &mut self,
        foreground: impl FnOnce() -> Foreground,
        held: bool,
    ) -> io::Result<Option<Fact>> {
        Ok(self.request(held)?.map(|request| match request {
            Ok(event) => Fact::Management(event),
            Err(ProgramStatusRequest) => Fact::ProgramStatus(foreground()),
        }))
    }

    fn request(&mut self, held: bool) -> io::Result<Option<Result<Event, ProgramStatusRequest>>> {
        let Some(connection) = self.connection.as_mut() else {
            return Ok(None);
        };
        if !connection.reads() {
            return Ok(None);
        }
        let start = connection.input.len();
        let remaining = protocol::MAX_INPUT_MESSAGE_BYTES.saturating_sub(start);
        if remaining == 0 {
            return Ok(Some(Ok(
                connection.refuse("management request exceeded its bound")
            )));
        }
        connection.input.resize(start + remaining, 0);
        let read = connection.stream.read(&mut connection.input[start..]);
        connection
            .input
            .truncate(start + read.as_ref().map_or(0, |read| *read));
        match read {
            Ok(0) => {
                self.connection = None;
                return Ok(Some(Ok(Event::Gone)));
            }
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(None);
            }
            Err(error) if crate::socket::peer_disconnected(&error) => {
                self.connection = None;
                return Ok(Some(Ok(Event::Gone)));
            }
            Err(error) => return Err(error),
        }
        let length = match protocol::client_message_len(&connection.input) {
            Ok(Some(length)) => length,
            Ok(None) => return Ok(None),
            Err(error) => return Ok(Some(Ok(connection.refuse(&error.to_string())))),
        };
        if connection.input.len() < length {
            return Ok(None);
        }
        if connection.input.len() > length {
            // Pipelined: every whole request goes, and a partial one stays.
            loop {
                match protocol::client_message_len(&connection.input) {
                    Ok(Some(length)) if length <= connection.input.len() => {
                        connection.input.drain(..length);
                    }
                    Ok(_) => break,
                    Err(error) => return Ok(Some(Ok(connection.refuse(&error.to_string())))),
                }
            }
            return Ok(Some(Ok(
                connection.invalid("management requests cannot be pipelined")
            )));
        }
        let message = protocol::decode_client_message(&connection.input);
        connection.input.clear();
        // A request that arrives while a ReadVisible request waits, or before
        // the last answer is written, is in flight beside it: it is declined,
        // and the held read or the answer stays.
        if connection.held_read || connection.writing() {
            return Ok(Some(Ok(connection.invalid(
                "a management request arrived while another was in flight",
            ))));
        }
        let message = match message {
            Ok(message) => message,
            Err(error) => return Ok(Some(Ok(connection.invalid(&error.to_string())))),
        };
        Ok(Some(Ok(match message {
            ClientMessage::Acquire { expected, record } => {
                let valid = connection.peer_uid == self.identity.uid
                    && expected == self.identity
                    && self
                        .record
                        .identity()
                        .is_ok_and(|current| current == record)
                    && endpoint_matches(&self.identity.presentation)
                    && endpoint_matches(&self.identity.management);
                if valid {
                    Event::AcquireValid
                } else {
                    Event::AcquireInvalid
                }
            }
            ClientMessage::Status => Event::Status,
            ClientMessage::Stop => Event::Stop,
            ClientMessage::ProgramStatus => return Ok(Some(Err(ProgramStatusRequest))),
            ClientMessage::ReadVisible if held => {
                connection.held_read = true;
                return Ok(None);
            }
            ClientMessage::ReadVisible => Event::Read,
            ClientMessage::Input(input) => {
                connection.pending = Some(Pending::Input(input));
                Event::Input
            }
            ClientMessage::AttachAllow {
                process_id,
                process_start,
            } => {
                connection.pending = Some(Pending::Allow(process_id, process_start));
                Event::Allow
            }
        })))
    }

    /// Whether a ReadVisible request waits for a synchronized update to end
    /// and no answer is being written, so it can be reported once no update
    /// is held.
    pub(crate) fn read_waits(&self) -> bool {
        self.connection
            .as_ref()
            .is_some_and(|c| c.held_read && c.output.is_empty())
    }

    /// Reports the held ReadVisible request, once no update is held and the
    /// connection's output is written, so its answer never meets a pending
    /// refusal.
    pub(crate) fn release_read(&mut self) -> Option<Event> {
        let connection = self
            .connection
            .as_mut()
            .filter(|c| c.held_read && c.output.is_empty())?;
        connection.held_read = false;
        Some(Event::Read)
    }

    /// Forgets a held ReadVisible request, as the run ends.
    pub(crate) fn drop_held_read(&mut self) {
        if let Some(connection) = self.connection.as_mut() {
            connection.held_read = false;
        }
    }

    /// Whether `peer` is the process the lease holder named, matched by the
    /// socket's peer process ID and that process's start identity, read now
    /// (ORB-C3). Without a lease or a name, no peer is.
    pub(crate) fn admits(&self, peer: &UnixStream) -> bool {
        let Some((process_id, process_start)) = self
            .connection
            .as_ref()
            .filter(|connection| connection.deadline.is_none() && !connection.close_after_flush)
            .and_then(|connection| connection.name)
        else {
            return false;
        };
        platform::peer_pid(peer).is_ok_and(|pid| pid == process_id)
            && platform::process_start_identity(process_id)
                .is_ok_and(|start| start == process_start)
    }

    /// Grants the lease: the negotiation bound ends and Lease names the run.
    pub(crate) fn lease(&mut self) -> io::Result<()> {
        let identity = self.identity.clone();
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| errno(libc::EPROTO))?;
        connection.deadline = None;
        connection.respond(&ServerMessage::Lease(identity), false)
    }

    pub(crate) fn status(&mut self) -> io::Result<()> {
        let identity = self.identity.clone();
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| errno(libc::EPROTO))?;
        connection.respond(&ServerMessage::Status(identity), false)
    }

    /// Answers ProgramStatus with the terminal's records.
    pub(crate) fn program_status(&mut self, status: ProgramStatus) -> io::Result<()> {
        self.respond(&ServerMessage::ProgramStatus(status))
    }

    /// Answers the request Busy; the lease stays.
    pub(crate) fn refuse(&mut self) -> io::Result<()> {
        self.respond(&ServerMessage::Busy)
    }

    /// Answers ReadVisible with the visible text, or refuses it when the
    /// extraction failed (ORB-C17).
    pub(crate) fn visible(&mut self, visible: Option<VisibleText>) -> io::Result<()> {
        self.respond(&match visible {
            Some(visible) => ServerMessage::Visible(visible),
            None => ServerMessage::Refused(Refusal::ReadFailed),
        })
    }

    /// The lease input Bend decided to write.
    pub(crate) fn take_input(&mut self) -> Option<LeaseInput> {
        match self.connection.as_mut()?.pending.take()? {
            Pending::Input(input) => Some(input),
            Pending::Allow(..) => None,
        }
    }

    /// Answers lease input: whether a written paste was bracketed, or why
    /// nothing was written (ORB-C5).
    pub(crate) fn input(&mut self, outcome: Result<bool, Refusal>) -> io::Result<()> {
        self.respond(&match outcome {
            Ok(bracketed) => ServerMessage::Input { bracketed },
            Err(refusal) => ServerMessage::Refused(refusal),
        })
    }

    /// Records the named process on the leased connection, replacing any
    /// earlier name, and answers Allowed (ORB-C13).
    pub(crate) fn allow(&mut self) -> io::Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| errno(libc::EPROTO))?;
        let Some(Pending::Allow(process_id, process_start)) = connection.pending.take() else {
            return Err(errno(libc::EPROTO));
        };
        connection.name = Some((process_id, process_start));
        connection.respond(&ServerMessage::Allowed, false)
    }

    /// Answers a request that did not decode, or one in flight beside
    /// another, with a typed refusal, after any answer being written; the
    /// lease stays.
    pub(crate) fn decline(&mut self) -> io::Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| errno(libc::EPROTO))?;
        connection.refusal = None;
        connection.respond(&ServerMessage::Refused(Refusal::InvalidRequest), false)
    }

    fn respond(&mut self, message: &ServerMessage) -> io::Result<()> {
        self.connection
            .as_mut()
            .ok_or_else(|| errno(libc::EPROTO))?
            .respond(message, false)
    }

    /// Refuses the connection's request with this failure and closes it once
    /// the answer is written.
    pub(crate) fn reject(&mut self, code: FailureCode) -> io::Result<()> {
        let connection = self
            .connection
            .as_mut()
            .ok_or_else(|| errno(libc::EPROTO))?;
        let detail = match code {
            FailureCode::InvalidIdentity => "live Orbit identity did not match".to_owned(),
            FailureCode::InvalidRequest => connection
                .refusal
                .take()
                .unwrap_or_else(|| "acquire the management lease before this request".to_owned()),
        };
        // A refused connection names no process (ORB-C3).
        connection.name = None;
        connection.respond(&ServerMessage::Failure(Failure { code, detail }), true)
    }

    /// Writes the connection's answer as its socket takes it; a connection
    /// that closed, or was answered with a refusal, goes.
    pub(crate) fn flush(&mut self) -> io::Result<Option<Event>> {
        let Some(connection) = self.connection.as_mut() else {
            return Ok(None);
        };
        while connection.written < connection.output.len() {
            match connection
                .stream
                .write(&connection.output[connection.written..])
            {
                Ok(0) => break,
                Ok(written) => connection.written += written,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if crate::socket::peer_disconnected(&error) => break,
                Err(error) => return Err(error),
            }
        }
        if connection.written == connection.output.len() {
            connection.output.clear();
            connection.written = 0;
            connection.queued = false;
            connection.reply.set(None);
            if !connection.close_after_flush {
                return Ok(None);
            }
        }
        self.connection = None;
        Ok(Some(Event::Gone))
    }

    /// Answers the lease with the run's tombstone after the rest of any
    /// answer being written, waiting at most the negotiation bound in all for
    /// its socket to take them; once the bound passes, nothing more is
    /// written. A connection closing after a refusal gets only that refusal.
    pub(crate) fn reply_stopped(&mut self, tombstone: Tombstone) {
        let Some(connection) = self.connection.as_mut().filter(|c| c.deadline.is_none()) else {
            return;
        };
        let mut bytes = connection.output.split_off(connection.written);
        if !connection.close_after_flush {
            let Ok(stopped) = encode(&ServerMessage::Stopped(tombstone)) else {
                return;
            };
            bytes.extend(stopped);
        }
        let end = Instant::now() + Duration::from_millis(NEGOTIATION_MS.into());
        let stream = &mut connection.stream;
        if stream.set_nonblocking(false).is_err() {
            return;
        }
        let mut unwritten = &bytes[..];
        while !unwritten.is_empty() {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() || stream.set_write_timeout(Some(left)).is_err() {
                return;
            }
            match stream.write(unwritten) {
                Ok(0) => return,
                Ok(written) => unwritten = &unwritten[written..],
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return,
            }
        }
    }

    /// Replaces the live record with one tombstone carrying why the run
    /// ended and the child's exact exit code or signal.
    pub(crate) fn finish(
        &mut self,
        reason: TerminationReason,
        status: ExitStatus,
    ) -> io::Result<Tombstone> {
        let outcome = match (status.code(), status.signal()) {
            (Some(code), _) => ProcessOutcome::ExitCode(code),
            (None, Some(signal)) => ProcessOutcome::Signal(signal),
            (None, None) => return Err(errno(libc::EPROTO)),
        };
        let tombstone = Tombstone {
            identity: self.identity.clone(),
            reason,
            outcome,
        };
        let bytes = protocol::encode_record(&Record::Tombstone(tombstone.clone()))
            .map_err(|_| errno(libc::EPROTO))?;
        self.record.replace_and_preserve(&bytes)?;
        Ok(tombstone)
    }
}

/// A ProgramStatus request, which Bend hears as its own fact.
struct ProgramStatusRequest;

impl Connection {
    fn refuse(&mut self, detail: &str) -> Event {
        self.input.clear();
        // A refused connection's held read gets no answer.
        self.held_read = false;
        self.refusal = Some(detail.to_owned());
        Event::Bad
    }

    /// A whole request that did not decode, or a pipelined one: the lease
    /// declines it, and a negotiating connection is refused with `detail`.
    fn invalid(&mut self, detail: &str) -> Event {
        self.refusal = Some(detail.to_owned());
        Event::Invalid
    }

    /// Queues an answer, which must be written within the bound; one answer
    /// may wait behind another being written, and the bound stays the first's.
    fn respond(&mut self, message: &ServerMessage, close: bool) -> io::Result<()> {
        if self.queued || self.close_after_flush {
            return Err(errno(libc::EPROTO));
        }
        self.pending = None;
        let bytes = encode(message)?;
        if self.output.is_empty() {
            self.output = bytes;
            self.written = 0;
            self.reply.set(Some(NEGOTIATION_MS));
        } else {
            self.output.extend(bytes);
            self.queued = true;
        }
        self.close_after_flush = close;
        Ok(())
    }

    /// Whether the connection waits to write its answer.
    pub(crate) fn writing(&self) -> bool {
        !self.output.is_empty()
    }

    /// Whether the next request is read: not while an answer waits behind
    /// another, or a refused connection closes.
    pub(crate) fn reads(&self) -> bool {
        !self.queued && !self.close_after_flush
    }

    /// The earliest of the negotiation bound and the answer's bound.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        let negotiation = self.deadline.as_ref().and_then(Timer::deadline);
        match (negotiation, self.reply.deadline()) {
            (Some(negotiation), Some(reply)) => Some(negotiation.min(reply)),
            (negotiation, reply) => negotiation.or(reply),
        }
    }
}

fn encode(message: &ServerMessage) -> io::Result<Vec<u8>> {
    protocol::encode_server_message(message).map_err(|_| errno(libc::EPROTO))
}

/// Whether an endpoint is still the exact socket a record names.
fn endpoint_matches(expected: &EndpointIdentity) -> bool {
    endpoint_identity(Path::new(OsStr::from_bytes(&expected.path)))
        .is_ok_and(|current| current == *expected)
}

fn artifact_path(socket: &Path, suffix: &str) -> PathBuf {
    let mut path = socket.as_os_str().to_os_string();
    path.push(suffix);
    PathBuf::from(path)
}

fn endpoint_identity(path: &Path) -> io::Result<EndpointIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    validate(&metadata, Kind::Socket)?;
    Ok(EndpointIdentity {
        path: path.as_os_str().as_bytes().to_vec(),
        object: object_identity(&metadata),
    })
}

#[derive(Clone, Copy)]
enum Kind {
    Socket,
    Regular,
}

/// Records and sockets must be exact user-owned 0600 objects of their type.
fn validate(metadata: &fs::Metadata, kind: Kind) -> io::Result<()> {
    let typed = match kind {
        Kind::Socket => metadata.file_type().is_socket(),
        Kind::Regular => metadata.file_type().is_file(),
    };
    let uid = platform::effective_uid();
    if !typed || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o600 {
        return Err(errno(libc::EACCES));
    }
    Ok(())
}

fn object_identity(metadata: &fs::Metadata) -> ObjectIdentity {
    ObjectIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

fn record_identity(path: &Path) -> io::Result<ObjectIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    validate(&metadata, Kind::Regular)?;
    Ok(object_identity(&metadata))
}

fn private_parent(path: &Path) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| errno(libc::EINVAL))?;
    let metadata = fs::symlink_metadata(parent)?;
    let uid = platform::effective_uid();
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700 {
        return Err(errno(libc::EACCES));
    }
    Ok(())
}

/// The live record this run published. It is removed when the run fails
/// while the record is still its own, and kept once a tombstone replaces it.
struct RecordGuard {
    path: PathBuf,
    identity: Option<ObjectIdentity>,
    contents: Vec<u8>,
}

impl RecordGuard {
    fn publish(path: &Path, bytes: &[u8]) -> io::Result<Self> {
        if let Some((mut claim, expected)) = ready_claim(path)? {
            claim.try_lock().map_err(|error| match error {
                fs::TryLockError::WouldBlock => errno(libc::EBUSY),
                fs::TryLockError::Error(error) => error,
            })?;
            if claim.metadata()?.len() != 0 || record_identity(path)? != expected {
                return Err(errno(libc::EBUSY));
            }
            claim.write_all(b"1")?;
        }
        atomic_replace(path, bytes)?;
        Ok(Self {
            path: path.to_owned(),
            identity: Some(record_identity(path)?),
            contents: bytes.to_vec(),
        })
    }

    fn identity(&self) -> io::Result<ObjectIdentity> {
        let expected = self.identity.ok_or_else(|| errno(libc::ESTALE))?;
        if record_identity(&self.path)? != expected {
            return Err(errno(libc::ESTALE));
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(platform::O_CLOEXEC | platform::O_NOFOLLOW | platform::O_NONBLOCK)
            .open(&self.path)?;
        let metadata = file.metadata()?;
        validate(&metadata, Kind::Regular)?;
        let mut contents = Vec::with_capacity(self.contents.len() + 1);
        Read::by_ref(&mut file)
            .take(self.contents.len() as u64 + 1)
            .read_to_end(&mut contents)?;
        if object_identity(&metadata) != expected
            || contents != self.contents
            || record_identity(&self.path)? != expected
        {
            return Err(errno(libc::ESTALE));
        }
        Ok(expected)
    }

    fn replace_and_preserve(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.identity()?;
        atomic_replace(&self.path, bytes)?;
        self.identity = None;
        Ok(())
    }
}

impl Drop for RecordGuard {
    fn drop(&mut self) {
        if self.identity().is_ok() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// A launcher's retained record: an exact empty 0600 file at the record's
/// pathname, or None when there is none to claim.
fn ready_claim(path: &Path) -> io::Result<Option<(File, ObjectIdentity)>> {
    private_parent(path)?;
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(platform::O_CLOEXEC | platform::O_NOFOLLOW | platform::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    validate(&metadata, Kind::Regular)?;
    if metadata.len() != 0 {
        return Ok(None);
    }
    let identity = object_identity(&metadata);
    if record_identity(path)? != identity {
        return Err(errno(libc::EBUSY));
    }
    Ok(Some((file, identity)))
}

/// Writes a record through a private temporary file and one rename.
fn atomic_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    private_parent(path)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate(&metadata, Kind::Regular)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(format!(
        ".tmp.{}.{}",
        std::process::id(),
        NEXT_ARTIFACT.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = PathBuf::from(temporary);
    let written = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(platform::O_CLOEXEC | platform::O_NOFOLLOW)
            .open(&temporary)?;
        file.write_all(bytes)?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        validate(&fs::symlink_metadata(&temporary)?, Kind::Regular)?;
        fs::rename(&temporary, path)?;
        validate(&fs::symlink_metadata(path)?, Kind::Regular)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}
