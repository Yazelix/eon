//! Management of ORB-C13: a private
//! management socket beside the presentation socket, an exact 0600 live
//! record published at Ready through the launcher's ready claim, one
//! management connection with bounded requests, and a tombstone that
//! replaces the record when the run ends. Every path and identity is
//! checked here; Bend decides every answer (src/management.bend), including
//! whether ORB-C15's `ProgramStatus` gets the records.

use crate::{
    Timer,
    boundary::{Fact, Foreground, ManagementEvent as Event},
    errno, platform,
    socket::Listener,
};
use orbit_protocol::management::{
    self as protocol, ClientMessage, EndpointIdentity, Failure, FailureCode, LiveIdentity,
    ObjectIdentity, ProcessOutcome, ProgramStatus, Record, ServerMessage, TerminationReason,
    Tombstone,
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
    time::Duration,
};

/// The bound on a management connection's negotiation and on
/// writing the Stopped answer.
pub(crate) const NEGOTIATION_MS: u32 = 1000;

static NEXT_ARTIFACT: AtomicU64 = AtomicU64::new(0);

/// The identities `--management-v3` names.
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
/// each at most 4 KiB, and a one-second bound until Bend grants the lease. A
/// refused connection stays until its Failure is written.
pub(crate) struct Connection {
    pub(crate) stream: UnixStream,
    peer_uid: u32,
    input: Vec<u8>,
    output: Vec<u8>,
    written: usize,
    close_after_flush: bool,
    /// The negotiation bound, which Bend's Lease ends. Bend's session holds
    /// the slot; Rust reads this only to time the bound and to answer the
    /// lease Stopped after a termination signal, which the host carries out
    /// without the worker.
    pub(crate) deadline: Option<Timer>,
    /// Why the last request was refused, for the Failure Bend sends.
    refusal: Option<String>,
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
            process_start: platform::process_start_identity()?,
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
            deadline: Some(deadline),
            refusal: None,
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

    /// Closes a connection whose negotiation bound passed.
    pub(crate) fn expire(&mut self) -> Option<Event> {
        let deadline = self.connection.as_mut()?.deadline.as_mut()?;
        if !deadline.expired() {
            return None;
        }
        self.connection = None;
        Some(Event::Gone)
    }

    /// Reads the connection's next request and reports it for Bend: one
    /// whole request at most 4 KiB, never two at once, an Acquire whose
    /// identity, record, peer and sockets all match this run, and a
    /// ProgramStatus with the PTY's foreground group sampled as it is read.
    pub(crate) fn read(
        &mut self,
        foreground: impl FnOnce() -> Foreground,
    ) -> io::Result<Option<Fact>> {
        Ok(self.request()?.map(|request| match request {
            Ok(event) => Fact::Management(event),
            Err(ProgramStatusRequest) => Fact::ProgramStatus(foreground()),
        }))
    }

    fn request(&mut self) -> io::Result<Option<Result<Event, ProgramStatusRequest>>> {
        let Some(connection) = self.connection.as_mut() else {
            return Ok(None);
        };
        if !connection.output.is_empty() {
            return Ok(None);
        }
        let remaining = protocol::MAX_MESSAGE_BYTES.saturating_sub(connection.input.len());
        if remaining == 0 {
            return Ok(Some(Ok(
                connection.refuse("management request exceeded its bound")
            )));
        }
        let mut bytes = [0; protocol::MAX_MESSAGE_BYTES];
        match connection.stream.read(&mut bytes[..remaining]) {
            Ok(0) => {
                self.connection = None;
                return Ok(Some(Ok(Event::Gone)));
            }
            Ok(read) => connection.input.extend_from_slice(&bytes[..read]),
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
        if connection.input.len() != length {
            return Ok(Some(Ok(
                connection.refuse("management requests cannot be pipelined")
            )));
        }
        let message = match protocol::decode_client_message(&connection.input) {
            Ok(message) => message,
            Err(error) => return Ok(Some(Ok(connection.refuse(&error.to_string())))),
        };
        connection.input.clear();
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
        })))
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
            if !connection.close_after_flush {
                return Ok(None);
            }
        }
        self.connection = None;
        Ok(Some(Event::Gone))
    }

    /// Answers the lease with the run's tombstone, waiting at most the
    /// negotiation bound for its socket to take it.
    pub(crate) fn reply_stopped(&mut self, tombstone: Tombstone) {
        let Some(connection) = self.connection.as_mut().filter(|c| c.deadline.is_none()) else {
            return;
        };
        let Ok(stopped) = encode(&ServerMessage::Stopped(tombstone)) else {
            return;
        };
        let stream = &mut connection.stream;
        if stream.set_nonblocking(false).is_ok()
            && stream
                .set_write_timeout(Some(Duration::from_millis(NEGOTIATION_MS.into())))
                .is_ok()
        {
            let _ = stream.write_all(&stopped);
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
        self.refusal = Some(detail.to_owned());
        Event::Bad
    }

    fn respond(&mut self, message: &ServerMessage, close: bool) -> io::Result<()> {
        if !self.output.is_empty() {
            return Err(errno(libc::EPROTO));
        }
        self.output = encode(message)?;
        self.written = 0;
        self.close_after_flush = close;
        Ok(())
    }

    /// Whether the connection waits to write its answer.
    pub(crate) fn writing(&self) -> bool {
        !self.output.is_empty()
    }

    pub(crate) fn deadline(&self) -> Option<std::time::Instant> {
        self.deadline.as_ref().and_then(Timer::deadline)
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
