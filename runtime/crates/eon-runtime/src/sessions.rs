use super::{
    cli::Inputs,
    diagnostics::{self, DiagnosticLog},
    managed_environment, platform,
    supervisor::{
        LaunchMode, SESSION_START_TIMEOUT, effective_uid, request_id, status_code, stop,
        validate_private_directory,
    },
    workspace::{is_directory_picker_endpoint, is_directory_picker_session},
};
use eon_workspace_protocol::v23::MAX_SESSIONS;
use orbit_protocol::management::{
    self as management, ClientMessage as ManagementClientMessage, EndpointIdentity, LiveIdentity,
    ObjectIdentity, ProcessOutcome, ProgramStatus, Record as ManagementRecord,
    ServerMessage as ManagementServerMessage, TerminationReason, Tombstone,
};
use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fs,
    io::{Read, Write},
    os::fd::OwnedFd,
    os::unix::{
        ffi::OsStrExt,
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn session_number(value: &str) -> Option<usize> {
    let number = value.strip_prefix("session-")?;
    if number.starts_with('0') || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number.parse().ok()
}

fn managed_session_number(value: &str) -> Result<Option<usize>, String> {
    if value == "directory-picker" || is_directory_picker_session(value) {
        Ok(None)
    } else {
        session_number(value)
            .map(Some)
            .ok_or_else(|| format!("invalid terminal identity {value:?}"))
    }
}

struct RecordSnapshot {
    object: ObjectIdentity,
    record: ManagementRecord,
}

struct ReadyClaim {
    file: fs::File,
    path: PathBuf,
    object: ObjectIdentity,
}

impl ReadyClaim {
    fn create(path: &Path) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or_else(|| format!("terminal record {} has no parent", path.display()))?;
        validate_private_directory(parent)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .map_err(|error| {
                format!(
                    "cannot create terminal Ready claim {}: {error}",
                    path.display()
                )
            })?;
        let metadata = file.metadata().map_err(|error| {
            format!(
                "cannot inspect terminal Ready claim {}: {error}",
                path.display()
            )
        })?;
        let claim = Self {
            object: object_identity(&metadata),
            file,
            path: path.to_path_buf(),
        };
        claim
            .file
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|error| {
                format!(
                    "cannot protect terminal Ready claim {}: {error}",
                    path.display()
                )
            })?;
        if !claim.exact_path_is_empty()? {
            return Err(format!(
                "terminal Ready claim {} changed while it was created",
                path.display()
            ));
        }
        Ok(claim)
    }

    fn retains_path(&self) -> Result<bool, String> {
        let current = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "cannot revalidate terminal Ready claim {}: {error}",
                    self.path.display()
                ));
            }
        };
        Ok(current.file_type().is_file()
            && current.uid() == effective_uid()
            && current.mode() & 0o7777 == 0o600
            && object_identity(&current) == self.object)
    }

    fn exact_path_is_empty(&self) -> Result<bool, String> {
        let metadata = self.file.metadata().map_err(|error| {
            format!(
                "cannot inspect terminal Ready claim {}: {error}",
                self.path.display()
            )
        })?;
        if !metadata.file_type().is_file()
            || metadata.uid() != effective_uid()
            || metadata.mode() & 0o7777 != 0o600
            || object_identity(&metadata) != self.object
        {
            return Err(format!(
                "terminal Ready claim {} changed while retained",
                self.path.display()
            ));
        }
        Ok(metadata.len() == 0 && self.retains_path()?)
    }

    fn lock_for_rollback(&self, deadline: Instant) -> Result<bool, String> {
        loop {
            match self.file.try_lock() {
                Ok(()) => return self.exact_path_is_empty(),
                Err(fs::TryLockError::WouldBlock) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(25));
                }
                Err(fs::TryLockError::WouldBlock) => {
                    return Err(format!(
                        "terminal Ready claim {} remained busy",
                        self.path.display()
                    ));
                }
                Err(fs::TryLockError::Error(error)) => {
                    return Err(format!(
                        "cannot lock terminal Ready claim {}: {error}",
                        self.path.display()
                    ));
                }
            }
        }
    }

    fn remove_empty(&self) -> Result<(), String> {
        if !self.exact_path_is_empty()? {
            return Err(format!(
                "terminal Ready claim {} changed before cleanup",
                self.path.display()
            ));
        }
        fs::remove_file(&self.path).map_err(|error| {
            format!(
                "cannot remove terminal Ready claim {}: {error}",
                self.path.display()
            )
        })
    }
}

impl Drop for ReadyClaim {
    fn drop(&mut self) {
        if self
            .file
            .metadata()
            .is_ok_and(|metadata| metadata.len() == 0)
            && fs::symlink_metadata(&self.path)
                .is_ok_and(|metadata| object_identity(&metadata) == self.object)
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct ManagedCandidate {
    number: Option<usize>,
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
                "cannot open terminal record {}: {error}",
                path.display()
            ));
        }
    };
    let metadata = file
        .metadata()
        .map_err(|error| format!("cannot inspect terminal record {}: {error}", path.display()))?;
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || metadata.len() > management::MAX_RECORD_BYTES as u64
    {
        return Err(format!(
            "terminal record {} must be an owned mode-0600 regular file no larger than {} bytes",
            path.display(),
            management::MAX_RECORD_BYTES
        ));
    }
    let object = object_identity(&metadata);
    let mut bytes = Vec::with_capacity(management::MAX_RECORD_BYTES.saturating_add(1));
    Read::by_ref(&mut file)
        .take(management::MAX_RECORD_BYTES.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read terminal record {}: {error}", path.display()))?;
    if bytes.len() > management::MAX_RECORD_BYTES {
        return Err(format!(
            "terminal record {} exceeds {} bytes",
            path.display(),
            management::MAX_RECORD_BYTES
        ));
    }
    let current = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot revalidate terminal record {}: {error}",
            path.display()
        )
    })?;
    if !current.file_type().is_file()
        || current.uid() != effective_uid()
        || current.mode() & 0o7777 != 0o600
        || object_identity(&current) != object
    {
        return Err(format!(
            "terminal record {} changed while it was being read",
            path.display()
        ));
    }
    let record = management::decode_record(&bytes)
        .map_err(|error| format!("invalid terminal record {}: {error}", path.display()))?;
    Ok(Some(RecordSnapshot { object, record }))
}

fn endpoint_matches(path: &Path, expected: &EndpointIdentity) -> Result<(), String> {
    if expected.path != path.as_os_str().as_bytes() {
        return Err(format!(
            "terminal identity names endpoint {}, expected {}",
            Path::new(OsStr::from_bytes(&expected.path)).display(),
            path.display()
        ));
    }
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect terminal endpoint {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_socket()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || object_identity(&metadata) != expected.object
    {
        return Err(format!(
            "terminal endpoint {} does not match its owned mode-0600 socket identity",
            path.display()
        ));
    }
    Ok(())
}

fn validate_management_identity(
    identity: &LiveIdentity,
    component_generation: &str,
    expected_run: Option<&str>,
    require_live_endpoints: bool,
) -> Result<(Option<usize>, PathBuf), String> {
    let number = managed_session_number(&identity.session_id)?;
    if expected_run.is_some_and(|expected| identity.run_id != expected) {
        return Err("terminal record reports a different run identity".into());
    }
    if identity.component_generation != component_generation {
        return Err(format!(
            "terminal record reports component generation {}, expected {component_generation}",
            identity.component_generation
        ));
    }
    if identity.record_generation != management::RECORD_GENERATION
        || identity.management_generation != management::VERSION
    {
        return Err("terminal record reports an unsupported management generation".into());
    }
    if identity.uid != effective_uid() {
        return Err(format!(
            "terminal record reports UID {}, expected {}",
            identity.uid,
            effective_uid()
        ));
    }
    let presentation = PathBuf::from(OsStr::from_bytes(&identity.presentation.path));
    let runtime = presentation
        .parent()
        .ok_or_else(|| "terminal presentation endpoint has no parent".to_string())?;
    let expected_presentation = match number {
        None => presentation
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(is_directory_picker_endpoint),
        Some(1) => presentation == runtime.join("orbit.sock"),
        Some(number) => presentation == runtime.join(format!("session-{number}.sock")),
    };
    if !expected_presentation {
        return Err(format!(
            "terminal identity {} uses unexpected presentation endpoint {}",
            identity.session_id,
            presentation.display()
        ));
    }
    let management_path = artifact_path(&presentation, ".management");
    if identity.management.path != management_path.as_os_str().as_bytes() {
        return Err(format!(
            "terminal identity {} uses an unexpected management endpoint",
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

pub(super) fn unix_connect_with_timeout(
    path: &Path,
    timeout: Duration,
) -> std::io::Result<UnixStream> {
    let address = socket2::SockAddr::unix(path)?;
    let socket = socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None)?;
    socket.connect_timeout(&address, timeout)?;
    Ok(UnixStream::from(OwnedFd::from(socket)))
}

fn read_management_response(
    stream: &mut UnixStream,
    deadline: Instant,
    operation: &str,
) -> Result<ManagementServerMessage, String> {
    let mut read_exact = |mut bytes: &mut [u8], context: &str| -> Result<(), String> {
        while !bytes.is_empty() {
            let timeout = operation_timeout(deadline, operation)?;
            stream
                .set_read_timeout(Some(timeout))
                .map_err(|error| format!("cannot bound {operation}: {error}"))?;
            match stream.read(bytes) {
                Ok(0) => return Err(format!("{context}: unexpected end of file")),
                Ok(read) => bytes = &mut bytes[read..],
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => return Err(format!("{context}: {error}")),
            }
        }
        Ok(())
    };
    let mut bytes = vec![0; management::HEADER_BYTES];
    read_exact(&mut bytes, "cannot read terminal management result")?;
    let length = management::server_message_len(&bytes)
        .map_err(|error| format!("invalid terminal management result: {error}"))?
        .ok_or("incomplete terminal management header")?;
    bytes.resize(length, 0);
    read_exact(
        &mut bytes[management::HEADER_BYTES..],
        "cannot read complete terminal management result",
    )?;
    management::decode_server_message(&bytes)
        .map_err(|error| format!("invalid terminal management result: {error}"))
}

fn validate_management_peer(stream: &UnixStream, identity: &LiveIdentity) -> Result<(), String> {
    let (process_id, uid) = platform::peer_identity(stream)
        .ok_or("cannot validate terminal management peer credentials")?;
    if u32::try_from(process_id).ok() != Some(identity.process_id) || uid != identity.uid {
        return Err("terminal management peer differs from its Ready process identity".into());
    }
    let process = platform::process(identity.process_id)?
        .ok_or("cannot validate terminal process identity: no such process")?;
    if process.start != identity.process_start {
        return Err("terminal process start differs from its Ready identity".into());
    }
    Ok(())
}

/// Leases `candidate`, whose Orbit log is found in `orbit_logs` when given.
fn acquire_management(
    candidate: ManagedCandidate,
    orbit_logs: Option<&Path>,
    deadline: Instant,
) -> Result<RunningSession, String> {
    let current = read_management_record(&candidate.record_path)?.ok_or_else(|| {
        format!(
            "terminal record {} disappeared",
            candidate.record_path.display()
        )
    })?;
    if current.object != candidate.record
        || current.record != ManagementRecord::Live(candidate.identity.clone())
    {
        return Err(format!(
            "terminal record {} changed before lease acquisition",
            candidate.record_path.display()
        ));
    }
    let management_path = Path::new(OsStr::from_bytes(&candidate.identity.management.path));
    endpoint_matches(management_path, &candidate.identity.management)?;
    let timeout = operation_timeout(deadline, "terminal lease acquisition")?;
    let mut stream = unix_connect_with_timeout(management_path, timeout).map_err(|error| {
        format!(
            "cannot connect to terminal management endpoint {}: {error}",
            management_path.display()
        )
    })?;
    validate_management_peer(&stream, &candidate.identity)?;
    let timeout = operation_timeout(deadline, "terminal lease acquisition")?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| format!("cannot bound terminal lease acquisition: {error}"))?;
    let request = management::encode_client_message(&ManagementClientMessage::Acquire {
        expected: candidate.identity.clone(),
        record: candidate.record,
    })
    .map_err(|error| format!("cannot encode terminal lease request: {error}"))?;
    stream
        .write_all(&request)
        .map_err(|error| format!("cannot send terminal lease request: {error}"))?;
    match read_management_response(&mut stream, deadline, "terminal lease acquisition")? {
        ManagementServerMessage::Lease(identity) if identity == candidate.identity => {}
        ManagementServerMessage::Busy => {
            return Err(format!(
                "terminal run {} is already managed",
                candidate.identity.run_id
            ));
        }
        ManagementServerMessage::Failure(failure) => {
            return Err(format!(
                "terminal rejected lease acquisition: {}",
                failure.detail
            ));
        }
        _ => return Err("terminal returned the wrong lease result".into()),
    }
    stream
        .set_read_timeout(None)
        .and_then(|()| stream.set_write_timeout(None))
        .and_then(|()| stream.set_nonblocking(true))
        .map_err(|error| format!("cannot configure terminal management lease: {error}"))?;
    let orbit_log = orbit_logs.map(|directory| {
        diagnostics::orbit_log_path(
            directory,
            &candidate.identity.component_generation,
            &candidate.identity.session_id,
            &candidate.identity.run_id,
        )
    });
    Ok(RunningSession {
        number: candidate.number,
        id: candidate.identity.session_id.clone(),
        endpoint: candidate.endpoint,
        record: candidate.record_path,
        record_object: candidate.record,
        identity: candidate.identity,
        orbit_log,
        lease: stream,
        program_status: None,
        stop_sent: false,
        reap_failure_reported: false,
        child: None,
        finished: None,
    })
}

fn cleanup_record(path: &Path, expected: ObjectIdentity) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "cannot inspect ended terminal record {}: {error}",
            path.display()
        )
    })?;
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || object_identity(&metadata) != expected
    {
        return Err(format!(
            "ended terminal record {} changed before cleanup",
            path.display()
        ));
    }
    fs::remove_file(path).map_err(|error| {
        format!(
            "cannot remove ended terminal record {}: {error}",
            path.display()
        )
    })
}

fn recorded_process_is_dead(identity: &LiveIdentity) -> Result<bool, String> {
    let Some(process) = platform::process(identity.process_id)? else {
        return Ok(true);
    };
    if process.start != identity.process_start {
        return Err("terminal process start differs from its Ready identity".into());
    }
    Ok(process.ended)
}

fn remove_dead_endpoint(path: &Path, expected: &EndpointIdentity) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "cannot inspect dead terminal endpoint {}: {error}",
                path.display()
            ));
        }
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != effective_uid()
        || metadata.mode() & 0o7777 != 0o600
        || object_identity(&metadata) != expected.object
    {
        return Err(format!(
            "dead terminal endpoint {} changed before cleanup",
            path.display()
        ));
    }
    fs::remove_file(path).map_err(|error| {
        format!(
            "cannot remove dead terminal endpoint {}: {error}",
            path.display()
        )
    })
}

fn retire_dead_run(
    record: &Path,
    object: ObjectIdentity,
    identity: &LiveIdentity,
    deadline: Instant,
) -> Result<(), String> {
    remove_dead_endpoints(identity)?;
    reap_untracked_child(identity.process_id, deadline)?;
    cleanup_record(record, object)
}

fn remove_dead_endpoints(identity: &LiveIdentity) -> Result<(), String> {
    remove_dead_endpoint(
        Path::new(OsStr::from_bytes(&identity.presentation.path)),
        &identity.presentation,
    )?;
    remove_dead_endpoint(
        Path::new(OsStr::from_bytes(&identity.management.path)),
        &identity.management,
    )
}

pub(super) fn recover_sessions(
    runtime: &Path,
    mode: LaunchMode,
    component_generation: &str,
    orbit_logs: Option<&Path>,
    deadline: Instant,
) -> Result<(Vec<RunningSession>, bool), String> {
    let mut sessions = acquire_generation_sessions(
        runtime,
        mode,
        component_generation,
        &[],
        orbit_logs,
        deadline,
    )?;
    let mut recovered_picker = false;
    for picker in sessions
        .iter_mut()
        .filter(|session| session.number.is_none())
    {
        recovered_picker = true;
        stop_managed_sessions(
            std::slice::from_mut(picker),
            operation_timeout(deadline, "stale directory-picker cleanup")?,
        )?;
    }
    sessions.retain(|session| session.number.is_some());
    Ok((sessions, recovered_picker))
}

pub(super) fn acquire_generation_sessions(
    runtime: &Path,
    mode: LaunchMode,
    component_generation: &str,
    owned: &[RunningSession],
    orbit_logs: Option<&Path>,
    deadline: Instant,
) -> Result<Vec<RunningSession>, String> {
    let mut paths = fs::read_dir(runtime)
        .map_err(|error| {
            format!(
                "cannot list terminal runtime {}: {error}",
                runtime.display()
            )
        })?
        .filter_map(|entry| match entry {
            Ok(entry) if entry.file_name().as_bytes().ends_with(b".record") => {
                Some(Ok(entry.path()))
            }
            Ok(_) => None,
            Err(error) => Some(Err(format!(
                "cannot inspect terminal entry in {}: {error}",
                runtime.display()
            ))),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if paths.len() > MAX_SESSIONS + 1 {
        return Err(format!(
            "terminal runtime {} exceeds the {}-record recovery limit",
            runtime.display(),
            MAX_SESSIONS + 1
        ));
    }
    paths.sort_by(|left, right| {
        left.as_os_str()
            .as_bytes()
            .cmp(right.as_os_str().as_bytes())
    });

    let mut candidates = Vec::new();
    let mut pickers = Vec::new();
    let mut numbers = HashSet::new();
    for record_path in paths {
        if owned.iter().any(|session| session.record == record_path) {
            continue;
        }
        operation_timeout(deadline, "terminal recovery")?;
        let snapshot = read_management_record(&record_path)?
            .ok_or_else(|| format!("terminal record {} disappeared", record_path.display()))?;
        let identity = match snapshot.record {
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
                        "ended terminal record {} has the wrong identity path",
                        record_path.display()
                    ));
                }
                let management_path =
                    Path::new(OsStr::from_bytes(&tombstone.identity.management.path));
                loop {
                    if endpoint_removed(&endpoint, tombstone.identity.presentation.object)?
                        && endpoint_removed(management_path, tombstone.identity.management.object)?
                    {
                        break;
                    }
                    operation_timeout(deadline, "ended terminal cleanup")?;
                    thread::sleep(Duration::from_millis(25));
                }
                recorded_process_is_dead(&tombstone.identity)?;
                reap_untracked_child(tombstone.identity.process_id, deadline)?;
                cleanup_record(&record_path, snapshot.object)?;
                continue;
            }
        };
        let (number, endpoint) =
            validate_management_identity(&identity, component_generation, None, false)?;
        if artifact_path(&endpoint, ".record") != record_path {
            return Err(format!(
                "terminal record {} has the wrong presentation identity",
                record_path.display()
            ));
        }
        if recorded_process_is_dead(&identity)? {
            retire_dead_run(&record_path, snapshot.object, &identity, deadline)?;
            continue;
        }
        validate_management_identity(&identity, component_generation, None, true)?;
        let candidate = ManagedCandidate {
            number,
            endpoint,
            record_path,
            record: snapshot.object,
            identity,
        };
        if let Some(number) = number {
            if !numbers.insert(number) {
                return Err(format!("duplicate live terminal identity session-{number}"));
            }
            candidates.push(candidate);
        } else {
            pickers.push(candidate);
        }
    }
    candidates.sort_by_key(|candidate| candidate.number);
    if mode == LaunchMode::Terminal && candidates.len() > 1 {
        return Err("eonbase cannot recover more than one live terminal".into());
    }

    candidates
        .into_iter()
        .chain(pickers)
        .map(|candidate| acquire_management(candidate, orbit_logs, deadline))
        .collect()
}

pub(super) struct RunningSession {
    pub(super) number: Option<usize>,
    pub(super) id: String,
    pub(super) endpoint: PathBuf,
    record: PathBuf,
    record_object: ObjectIdentity,
    identity: LiveIdentity,
    pub(super) orbit_log: Option<PathBuf>,
    /// This Runtime is the lease's one reader and keeps at most one request
    /// in flight on it (EON-C31 Polling).
    lease: UnixStream,
    program_status: Option<ProgramStatusRequest>,
    stop_sent: bool,
    /// The supervisor reported that this terminal cannot be reconciled yet;
    /// later reaps retry without waiting.
    pub(super) reap_failure_reported: bool,
    child: Option<Child>,
    finished: Option<SessionEnd>,
}

#[derive(Clone, Copy)]
enum SessionEnd {
    Managed(TerminationReason, ProcessOutcome),
    OwnerLost,
}

/// A `ProgramStatus` request in flight on a lease: the request bytes not yet
/// written, the reply bytes read so far, and the poller's deadline.
struct ProgramStatusRequest {
    unsent: Vec<u8>,
    reply: Vec<u8>,
    deadline: Instant,
}

enum ReplyProgress {
    Complete,
    WouldBlock,
    Closed,
}

/// Writes what remains of the request, then reads the reply without reading
/// past its end. A header that does not decode ends the reply, which then
/// fails to decode. A lease that ends or fails reports `Closed`, which the EOF
/// probe owns.
fn advance_program_status(
    lease: &mut UnixStream,
    request: &mut ProgramStatusRequest,
) -> ReplyProgress {
    while !request.unsent.is_empty() {
        match lease.write(&request.unsent) {
            Ok(0) => return ReplyProgress::Closed,
            Ok(written) => drop(request.unsent.drain(..written)),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return ReplyProgress::WouldBlock;
            }
            Err(_) => return ReplyProgress::Closed,
        }
    }
    loop {
        let Ok(length) = management::server_message_len(&request.reply) else {
            return ReplyProgress::Complete;
        };
        let length = length.unwrap_or(management::HEADER_BYTES);
        let start = request.reply.len();
        if start == length {
            return ReplyProgress::Complete;
        }
        request.reply.resize(length, 0);
        let read = lease.read(&mut request.reply[start..]);
        request.reply.truncate(start + *read.as_ref().unwrap_or(&0));
        match read {
            Ok(0) => return ReplyProgress::Closed,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return ReplyProgress::WouldBlock;
            }
            Err(_) => return ReplyProgress::Closed,
        }
    }
}

/// What one read of a `ProgramStatus` request found. Busy ends the request; a
/// passed deadline or an ended lease leaves it in flight, so later reads keep
/// draining it and return its reply if it comes, while `send_program_status`
/// sends no other until then. The EOF probe owns an ended lease.
pub(super) enum ProgramStatusRead {
    /// No reply yet, before the deadline.
    Pending,
    /// No reply by the deadline, a Busy refusal (EON-C31), an ended lease, or
    /// no request in flight.
    Missed,
    Reply(ProgramStatus),
}

/// Sends a `ProgramStatus` request without blocking, unless one is already in
/// flight, a Stop was sent, or the run ended. Returns whether it sent one.
pub(super) fn send_program_status(
    session: &mut RunningSession,
    deadline: Instant,
) -> Result<bool, String> {
    if session.is_finished() || session.stop_sent || session.program_status.is_some() {
        return Ok(false);
    }
    let unsent = management::encode_client_message(&ManagementClientMessage::ProgramStatus)
        .map_err(|error| format!("cannot encode terminal program status request: {error}"))?;
    let request = session.program_status.insert(ProgramStatusRequest {
        unsent,
        reply: Vec::new(),
        deadline,
    });
    advance_program_status(&mut session.lease, request);
    Ok(true)
}

/// Reads the in-flight `ProgramStatus` reply without blocking.
pub(super) fn read_program_status(
    session: &mut RunningSession,
) -> Result<ProgramStatusRead, String> {
    let Some(request) = &mut session.program_status else {
        return Ok(ProgramStatusRead::Missed);
    };
    match advance_program_status(&mut session.lease, request) {
        ReplyProgress::Complete => {}
        ReplyProgress::WouldBlock if Instant::now() < request.deadline => {
            return Ok(ProgramStatusRead::Pending);
        }
        ReplyProgress::WouldBlock | ReplyProgress::Closed => {
            return Ok(ProgramStatusRead::Missed);
        }
    }
    let reply = std::mem::take(&mut request.reply);
    session.program_status = None;
    match management::decode_server_message(&reply) {
        Ok(ManagementServerMessage::ProgramStatus(status)) => Ok(ProgramStatusRead::Reply(status)),
        Ok(ManagementServerMessage::Busy) => Ok(ProgramStatusRead::Missed),
        Ok(_) => Err("terminal returned the wrong program status result".into()),
        Err(error) => Err(format!("invalid terminal program status result: {error}")),
    }
}

/// Advances the in-flight `ProgramStatus` reply without blocking and discards
/// it once whole, so a Stop written next is never pipelined behind the request
/// and its answer is never read as the reply. Returns whether nothing remains
/// in flight.
fn drain_program_status(session: &mut RunningSession) -> Result<bool, String> {
    let Some(request) = &mut session.program_status else {
        return Ok(true);
    };
    match advance_program_status(&mut session.lease, request) {
        ReplyProgress::Complete => {
            session.program_status = None;
            Ok(true)
        }
        ReplyProgress::WouldBlock => Ok(false),
        ReplyProgress::Closed => Err("terminal ended before answering program status".into()),
    }
}

impl RunningSession {
    /// Whether Orbit exited without publishing this terminal's result.
    pub(super) fn owner_lost(&self) -> bool {
        matches!(self.finished, Some(SessionEnd::OwnerLost))
    }

    pub(super) fn is_finished(&self) -> bool {
        self.finished.is_some()
    }

    /// Whether this run ended by an explicit Stop.
    pub(super) fn stopped(&self) -> bool {
        matches!(
            self.finished,
            Some(SessionEnd::Managed(TerminationReason::ExplicitStop, _))
        )
    }

    /// A tracked terminal that was sent Stop is one whose Stop failed.
    pub(super) fn stop_failed(&self) -> bool {
        self.stop_sent && !self.is_finished()
    }

    pub(super) fn run_id(&self) -> &str {
        &self.identity.run_id
    }

    /// The suffix a failure line carries to name this run's Orbit log.
    pub(super) fn orbit_log_suffix(&self) -> String {
        diagnostics::orbit_log_suffix(self.orbit_log.as_deref())
    }

    /// Whether this live terminal is the one an `EON_TERMINAL` value names.
    pub(super) fn is_terminal(&self, caller: &str) -> bool {
        !self.is_finished() && caller == terminal_identity(&self.id, &self.identity.run_id)
    }
}

/// Names the calling terminal for `eon` commands. Only Orbit terminals carry
/// it; the supervisor's other children never inherit the launching terminal's.
pub(super) const TERMINAL_VARIABLE: &str = "EON_TERMINAL";
/// A popup terminal's owning tab identity when it started.
pub(super) const TAB_VARIABLE: &str = "EON_TAB";
/// Runtime-private marker on the default shell the supervisor starts, after it read
/// the configuration through its log; the managed shell launcher removes it.
pub(super) const CONFIG_WARNED_VARIABLE: &str = "EON_CONFIG_WARNED";

/// The `EON_TERMINAL` value of one exact Orbit run, unique across generations
/// and windows; callers treat it as opaque.
fn terminal_identity(session_id: &str, run_id: &str) -> String {
    format!("{session_id}.{run_id}")
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_orbit(
    inputs: &Inputs,
    config: &Path,
    socket: &Path,
    session_id: &str,
    component_generation: &str,
    directory: &Path,
    child: &[OsString],
    tab: Option<&str>,
    log: &DiagnosticLog,
    deadline: Instant,
) -> Result<RunningSession, String> {
    let run_id = request_id();
    let record_path = artifact_path(socket, ".record");
    let mut orbit_command = orbit_command(
        inputs,
        config,
        socket,
        session_id,
        &run_id,
        component_generation,
        directory,
        child,
        tab,
        Some(log),
    )?;
    let (orbit_stderr, orbit_log) = log
        .orbit_directory()
        .map_err(str::to_string)
        .and_then(|directory| {
            diagnostics::open_orbit_log(directory, component_generation, session_id, &run_id)
        })
        .map(|(file, path)| (Stdio::from(file), Some(path)))
        .unwrap_or_else(|error| {
            log.note(&format!(
                "terminal {session_id}: {error}; continuing without an Orbit log"
            ));
            (Stdio::null(), None)
        });
    orbit_command.stderr(orbit_stderr);
    let claim = ReadyClaim::create(&record_path)?;
    let spawned = orbit_command.spawn();
    // Only Orbit should retain the log's open file description and liveness lock.
    drop(orbit_command);
    let mut orbit = match spawned {
        Ok(orbit) => orbit,
        Err(error) => {
            return match claim.remove_empty() {
                Ok(()) => Err(format!("cannot launch terminal: {error}")),
                Err(cleanup_error) => Err(format!(
                    "cannot launch terminal: {error}; cannot roll back terminal: {cleanup_error}"
                )),
            };
        }
    };
    let result = (|| {
        loop {
            let record = if claim.retains_path()? {
                None
            } else {
                read_management_record(&record_path)?
            };
            match record {
                Some(RecordSnapshot {
                    object,
                    record: ManagementRecord::Live(identity),
                }) => {
                    if let Some(status) = orbit
                        .try_wait()
                        .map_err(|error| format!("cannot observe terminal startup: {error}"))?
                    {
                        return Err(format!(
                            "terminal exited before its management lease was acquired (status {})",
                            status_code(status)
                        ));
                    }
                    let (number, endpoint) = validate_management_identity(
                        &identity,
                        component_generation,
                        Some(&run_id),
                        true,
                    )?;
                    if identity.session_id != session_id || endpoint != socket {
                        return Err("terminal Ready identity does not match its launch".into());
                    }
                    return acquire_management(
                        ManagedCandidate {
                            number,
                            endpoint,
                            record_path,
                            record: object,
                            identity,
                        },
                        None,
                        deadline,
                    );
                }
                Some(RecordSnapshot {
                    record: ManagementRecord::Tombstone(tombstone),
                    ..
                }) => {
                    return Err(match tombstone.outcome {
                        ProcessOutcome::ExitCode(code) => {
                            format!("terminal command exited with status {code} during startup")
                        }
                        ProcessOutcome::Signal(signal) => {
                            format!("terminal command was ended by signal {signal} during startup")
                        }
                    });
                }
                None => {
                    if let Some(status) = orbit
                        .try_wait()
                        .map_err(|error| format!("cannot observe terminal startup: {error}"))?
                    {
                        return if claim.exact_path_is_empty()? {
                            Err(format!(
                                "terminal exited before publishing Ready (status {})",
                                status_code(status)
                            ))
                        } else {
                            Err(format!(
                                "terminal exited before its management lease was acquired (status {})",
                                status_code(status)
                            ))
                        };
                    }
                    if Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(25));
                    } else {
                        return Err("terminal did not publish Ready within five seconds".into());
                    }
                }
            }
        }
    })();
    match result {
        Ok(mut session) => {
            session.child = Some(orbit);
            session.orbit_log = orbit_log;
            Ok(session)
        }
        Err(error) => {
            let error = match rollback_unleased_orbit(
                orbit,
                claim,
                (&run_id, component_generation, socket),
                deadline,
            ) {
                Ok(()) => error,
                Err(stop_error) => format!("{error}; cannot roll back terminal: {stop_error}"),
            };
            Err(format!(
                "{error}{}",
                diagnostics::orbit_log_suffix(orbit_log.as_deref())
            ))
        }
    }
}

fn rollback_unleased_orbit(
    mut orbit: Child,
    claim: ReadyClaim,
    launch: (&str, &str, &Path),
    deadline: Instant,
) -> Result<(), String> {
    let claim_error = match claim.lock_for_rollback(deadline) {
        Ok(true) => {
            while Instant::now() < deadline {
                match orbit.try_wait() {
                    Ok(Some(_)) => return claim.remove_empty(),
                    Ok(None) => thread::sleep(Duration::from_millis(25)),
                    Err(_) => break,
                }
            }
            stop(&mut orbit);
            return claim.remove_empty();
        }
        Ok(false) => None,
        Err(error) => Some(error),
    };
    // Orbit never rewrites its record after exiting, so observe exit before reading it.
    let exited = orbit
        .try_wait()
        .map_err(|error| format!("cannot observe terminal rollback: {error}"))?
        .is_some();
    let Ok(Some(RecordSnapshot { object, record })) = read_management_record(&claim.path) else {
        return claim_error.map_or(Ok(()), Err);
    };
    let (identity, ended) = match record {
        ManagementRecord::Live(identity) => (identity, exited),
        ManagementRecord::Tombstone(tombstone) => (tombstone.identity, true),
    };
    if identity.process_id != orbit.id() {
        return claim_error.map_or(Ok(()), Err);
    }
    if ended {
        // Orbit published its final record: a tombstone after its child ended,
        // or Live when Orbit itself died. Cleanup may no longer signal it, so
        // wait for its own exit, then remove this launch's exact residue so the
        // terminal number stays creatable.
        let (run_id, component_generation, socket) = launch;
        let (_, endpoint) =
            validate_management_identity(&identity, component_generation, Some(run_id), false)?;
        if endpoint != socket {
            return Err("terminal final record does not match its launch".into());
        }
        while orbit
            .try_wait()
            .map_err(|error| format!("cannot observe terminal rollback: {error}"))?
            .is_none()
        {
            operation_timeout(deadline, "terminal exit after its final record")?;
            thread::sleep(Duration::from_millis(25));
        }
        remove_dead_endpoints(&identity)?;
        return cleanup_record(&claim.path, object);
    }
    let mut session = acquire_management(
        ManagedCandidate {
            number: managed_session_number(&identity.session_id)?,
            endpoint: PathBuf::from(OsStr::from_bytes(&identity.presentation.path)),
            record_path: claim.path.clone(),
            record: object,
            identity,
        },
        None,
        deadline,
    )?;
    session.child = Some(orbit);
    let timeout = operation_timeout(deadline, "terminal rollback")?;
    stop_managed_sessions(std::slice::from_mut(&mut session), timeout).map(|_| ())
}

#[allow(clippy::too_many_arguments)]
fn orbit_command(
    inputs: &Inputs,
    config: &Path,
    socket: &Path,
    session_id: &str,
    run_id: &str,
    component_generation: &str,
    directory: &Path,
    child: &[OsString],
    tab: Option<&str>,
    log: Option<&DiagnosticLog>,
) -> Result<Command, String> {
    let mut orbit_command = Command::new(&inputs.orbit);
    orbit_command
        .arg("serve")
        .arg(socket)
        .arg("--management-v3")
        .arg(session_id)
        .arg(run_id)
        .arg(component_generation)
        .arg("--ansi-palette-v1")
        .arg(inputs.defaults.ansi_palette)
        .arg("--")
        .env("EON_CONFIG_HOME", config)
        .env(TERMINAL_VARIABLE, terminal_identity(session_id, run_id))
        .env_remove(TAB_VARIABLE)
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null());
    if let Some(tab) = tab {
        orbit_command.env(TAB_VARIABLE, tab);
    }
    if let Some(session_bin) = &inputs.managed.shell_bin {
        orbit_command.env("PATH", managed_environment::session_path(session_bin)?);
    }
    let shell = if child.is_empty() {
        managed_environment::shell_command(config, &inputs.defaults, log)?
    } else {
        Vec::new()
    };
    // Mark only Eon's own Nu and Bash launchers, which remove the marker before exec.
    // A bare name is one only when the session PATH puts Eon's commands first; an
    // absolute path such as /usr/bin/bash is the host's shell and would pass it on.
    let marked = shell.first().is_some_and(|program| {
        inputs.managed.shell_bin.is_some()
            && !program.contains('/')
            && matches!(
                managed_environment::tool(OsStr::new(program)),
                Some(managed_environment::Tool::Nu | managed_environment::Tool::Bash)
            )
    });
    if marked {
        orbit_command.env(CONFIG_WARNED_VARIABLE, "1");
    } else {
        orbit_command.env_remove(CONFIG_WARNED_VARIABLE);
    }
    if child.is_empty() {
        orbit_command.args(shell);
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
            "terminal endpoint {} was replaced during cleanup",
            path.display()
        )),
        Err(error) => Err(format!(
            "cannot observe terminal endpoint cleanup {}: {error}",
            path.display()
        )),
    }
}

fn reap_untracked_child(process_id: u32, deadline: Instant) -> Result<(), String> {
    let pid = i32::try_from(process_id)
        .ok()
        .filter(|pid| *pid > 0)
        .ok_or("invalid ended terminal process identity")?;
    loop {
        // A failed Ready rollback can drop our Child handle. Reap only an
        // exact child after managed cleanup; recovered runs return ECHILD.
        let mut status = 0;
        // SAFETY: status is writable; WNOHANG sends no signal and cannot wait on a non-child.
        let result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if result > 0 {
            return Ok(());
        }
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ECHILD) {
                return Ok(());
            }
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(format!("cannot reap ended terminal: {error}"));
        }
        operation_timeout(deadline, "terminal process reaping")?;
        thread::sleep(Duration::from_millis(25));
    }
}

/// Ends a run whose Orbit exited without publishing a result. A Stop it
/// never completed fails, but the run is still reconciled as ended.
fn owner_lost(
    session: &mut RunningSession,
    reason: Option<TerminationReason>,
) -> Result<SessionEnd, String> {
    session.finished = Some(SessionEnd::OwnerLost);
    eprintln!(
        "terminal {}: Orbit exited without publishing a terminal result",
        session.id
    );
    if reason != Some(TerminationReason::ExplicitStop) {
        Ok(SessionEnd::OwnerLost)
    } else {
        Err("terminal owner exited without completing Stop".into())
    }
}

/// Waits for the run's tombstone, for `reason` or, given none, for either.
fn wait_and_finalize_tombstone(
    session: &mut RunningSession,
    reason: Option<TerminationReason>,
    response: Option<&Tombstone>,
    deadline: Instant,
) -> Result<SessionEnd, String> {
    let (reason, outcome, record_object) = loop {
        let Some(snapshot) = read_management_record(&session.record)? else {
            // A failed Orbit removes its own Live record as it exits, and
            // publishes no result.
            if recorded_process_is_dead(&session.identity)? {
                remove_dead_endpoints(&session.identity)?;
                reap_untracked_child(session.identity.process_id, deadline)?;
                return owner_lost(session, reason);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "ended terminal record {} disappeared while its Orbit still runs",
                    session.record.display()
                ));
            }
            thread::sleep(Duration::from_millis(25));
            continue;
        };
        match snapshot.record {
            ManagementRecord::Live(identity) if identity == session.identity => {
                if recorded_process_is_dead(&identity)? {
                    // Death can race Orbit's final atomic record publication.
                    let current = read_management_record(&session.record)?;
                    let Some(current) = current.filter(|current| {
                        current.record == ManagementRecord::Live(identity.clone())
                    }) else {
                        operation_timeout(deadline, "terminal tombstone reconciliation")?;
                        continue;
                    };
                    if current.object != session.record_object {
                        return Err("terminal Live record changed before cleanup".into());
                    }
                    retire_dead_run(&session.record, current.object, &identity, deadline)?;
                    return owner_lost(session, reason);
                }
                operation_timeout(deadline, "terminal tombstone reconciliation")?;
                thread::sleep(Duration::from_millis(25));
            }
            ManagementRecord::Tombstone(tombstone)
                if tombstone.identity == session.identity
                    && reason.is_none_or(|reason| tombstone.reason == reason) =>
            {
                if response.is_some_and(|response| response != &tombstone) {
                    return Err("terminal stop result differs from its final record".into());
                }
                break (tombstone.reason, tombstone.outcome, snapshot.object);
            }
            _ => return Err("terminal final record does not match the acquired run".into()),
        }
    };
    let management_path = Path::new(OsStr::from_bytes(&session.identity.management.path));
    loop {
        if endpoint_removed(&session.endpoint, session.identity.presentation.object)?
            && endpoint_removed(management_path, session.identity.management.object)?
        {
            break;
        }
        operation_timeout(deadline, "terminal endpoint cleanup")?;
        thread::sleep(Duration::from_millis(25));
    }
    if let Some(child) = &mut session.child {
        loop {
            if child
                .try_wait()
                .map_err(|error| format!("cannot reap ended terminal: {error}"))?
                .is_some()
            {
                break;
            }
            operation_timeout(deadline, "terminal process reaping")?;
            thread::sleep(Duration::from_millis(25));
        }
    } else {
        reap_untracked_child(session.identity.process_id, deadline)?;
    }
    cleanup_record(&session.record, record_object)?;
    let end = SessionEnd::Managed(reason, outcome);
    session.finished = Some(end);
    Ok(end)
}

pub(super) fn session_finished(session: &mut RunningSession) -> Result<Option<i32>, String> {
    let end = if let Some(end) = session.finished {
        end
    } else {
        // An in-flight reply's bytes belong to its request; bytes after it are
        // unsolicited, such as the Stopped an Orbit sends when a signal stops
        // it (ORB-C12), and are discarded. Only the record says how the run
        // ended.
        let progress = match &mut session.program_status {
            Some(request) => advance_program_status(&mut session.lease, request),
            None => ReplyProgress::Complete,
        };
        match progress {
            ReplyProgress::Complete => match session
                .lease
                .read(&mut [0; management::MAX_MESSAGE_BYTES])
            {
                Ok(0) => {}
                Ok(_) => return Ok(None),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => return Ok(None),
                Err(_) => {}
            },
            ReplyProgress::WouldBlock => return Ok(None),
            ReplyProgress::Closed => {}
        }
        // A run that already failed to reconcile gets one more look per reap,
        // so it cannot stall the supervisor's loop and requests.
        let wait = if session.reap_failure_reported {
            Duration::ZERO
        } else {
            SESSION_START_TIMEOUT
        };
        wait_and_finalize_tombstone(session, None, None, Instant::now() + wait)?
    };
    Ok(Some(match end {
        SessionEnd::Managed(_, ProcessOutcome::ExitCode(code)) => code,
        SessionEnd::Managed(_, ProcessOutcome::Signal(_)) | SessionEnd::OwnerLost => 1,
    }))
}

pub(super) fn stop_managed_sessions(
    sessions: &mut [RunningSession],
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let deadline = Instant::now() + timeout;
    let request = management::encode_client_message(&ManagementClientMessage::Stop)
        .map_err(|error| format!("cannot encode terminal stop: {error}"))?;
    let write_stop = |session: &mut RunningSession| {
        let remaining = operation_timeout(deadline, "terminal stop")?;
        session
            .lease
            .set_nonblocking(false)
            .and_then(|()| session.lease.set_write_timeout(Some(remaining)))
            .map_err(|error| format!("cannot bound terminal stop: {error}"))?;
        session.stop_sent = true;
        session
            .lease
            .write_all(&request)
            .map_err(|error| format!("cannot send terminal stop: {error}"))
    };
    // Each lease gets Stop once its in-flight reply is drained, so a silent
    // terminal delays only its own Stop.
    let mut writes = sessions
        .iter()
        .filter(|session| !session.is_finished())
        .map(|_| None)
        .collect::<Vec<Option<Result<(), String>>>>();
    loop {
        for (session, write) in sessions
            .iter_mut()
            .filter(|session| !session.is_finished())
            .zip(&mut writes)
            .filter(|(_, write)| write.is_none())
        {
            *write = match drain_program_status(session) {
                Ok(true) => Some(write_stop(session)),
                Ok(false) => continue,
                Err(error) => Some(Err(error)),
            };
        }
        if writes.iter().all(Option::is_some) {
            break;
        }
        if Instant::now() >= deadline {
            for write in writes.iter_mut().filter(|write| write.is_none()) {
                *write = Some(Err(
                    "terminal did not answer program status before stop".into()
                ));
            }
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }
    let writes = writes.into_iter().flatten();

    let mut errors = Vec::new();
    for (session, write) in sessions
        .iter_mut()
        .filter(|session| !session.is_finished())
        .zip(writes)
    {
        let response = if write.is_ok() {
            match read_management_response(&mut session.lease, deadline, "terminal stop response") {
                Ok(ManagementServerMessage::Stopped(tombstone))
                    if tombstone.identity == session.identity
                        && tombstone.reason == TerminationReason::ExplicitStop =>
                {
                    Some(Ok(tombstone))
                }
                Ok(ManagementServerMessage::Failure(failure)) => Some(Err(format!(
                    "terminal rejected stop for {}: {}",
                    session.id, failure.detail
                ))),
                Ok(_) => Some(Err(format!(
                    "terminal returned the wrong stop result for {}",
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
                Some(TerminationReason::ExplicitStop),
                Some(&tombstone),
                deadline,
            ),
            Some(Err(error)) => Err(error),
            None => wait_and_finalize_tombstone(
                session,
                Some(TerminationReason::ExplicitStop),
                None,
                deadline,
            ),
        };
        if let Err(error) = result {
            if matches!(read_management_record(&session.record), Ok(Some(RecordSnapshot {
                record: ManagementRecord::Tombstone(ref tombstone), ..
            })) if tombstone.identity == session.identity && tombstone.reason == TerminationReason::NaturalExit)
                && let Err(cleanup) = wait_and_finalize_tombstone(
                    session,
                    Some(TerminationReason::NaturalExit),
                    None,
                    deadline,
                )
            {
                errors.push(format!(
                    "{}: cannot reconcile natural exit: {cleanup}",
                    session.id
                ));
            }
            errors.push(format!("{}: {error}", session.id));
        }
    }
    if errors.is_empty() {
        Ok(sessions
            .iter()
            .filter(|session| session.stopped())
            .map(|session| session.id.clone())
            .collect())
    } else {
        for session in sessions {
            if let Err(error) = session
                .lease
                .set_read_timeout(None)
                .and_then(|()| session.lease.set_write_timeout(None))
                .and_then(|()| session.lease.set_nonblocking(true))
            {
                errors.push(format!(
                    "{}: cannot restore terminal management lease after failed stop: {error}",
                    session.id
                ));
            }
        }
        Err(errors.join("; "))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::{
        EndpointIdentity, LiveIdentity, ManagementRecord, ObjectIdentity, ProcessOutcome,
        ProgramStatusRead, RunningSession, TerminationReason, Tombstone,
        is_directory_picker_endpoint, managed_session_number, management, orbit_command,
        read_management_response, session_number, stop_managed_sessions, unix_connect_with_timeout,
    };
    use crate::{diagnostics::DiagnosticLog, supervisor::temporary_directory};
    use orbit_protocol::management::ServerMessage as ManagementServerMessage;
    use std::{
        ffi::{OsStr, OsString},
        fs,
        io::{Read, Write},
        os::{
            fd::AsRawFd,
            unix::{
                ffi::OsStrExt,
                fs::{OpenOptionsExt, PermissionsExt},
                net::{UnixListener, UnixStream},
            },
        },
        path::{Path, PathBuf},
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    #[test]
    fn lost_owner_cleanup_requires_death_and_original_owned_objects() {
        let root = temporary_directory();
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let process = PathBuf::from(format!("/proc/{}", child.id()));
        let start = fs::read_to_string(process.join("stat"))
            .unwrap()
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap();
        let endpoint = root.join("orbit.sock");
        let management_path = root.join("orbit.sock.management");
        let _presentation = UnixListener::bind(&endpoint).unwrap();
        let _management = UnixListener::bind(&management_path).unwrap();
        for path in [&endpoint, &management_path] {
            fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        }
        let endpoint_identity = |path: &Path| EndpointIdentity {
            path: path.as_os_str().as_bytes().to_vec(),
            object: super::object_identity(&fs::symlink_metadata(path).unwrap()),
        };
        let identity = LiveIdentity {
            session_id: "session-1".into(),
            run_id: "lost-owner".into(),
            component_generation: "component-1".into(),
            record_generation: management::RECORD_GENERATION,
            management_generation: management::VERSION,
            process_id: child.id(),
            process_start: start,
            uid: super::effective_uid(),
            presentation: endpoint_identity(&endpoint),
            management: endpoint_identity(&management_path),
        };
        let record = root.join("orbit.sock.record");
        let bytes = management::encode_record(&ManagementRecord::Live(identity.clone())).unwrap();
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&record)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        let (lease, peer) = UnixStream::pair().unwrap();
        lease.set_nonblocking(true).unwrap();
        drop(peer);
        let mut session = RunningSession {
            number: Some(1),
            id: identity.session_id.clone(),
            endpoint: endpoint.clone(),
            record: record.clone(),
            record_object: super::object_identity(&fs::symlink_metadata(&record).unwrap()),
            identity,
            orbit_log: None,
            lease,
            program_status: None,
            stop_sent: false,
            reap_failure_reported: false,
            child: None,
            finished: None,
        };
        assert!(
            super::wait_and_finalize_tombstone(
                &mut session,
                Some(TerminationReason::NaturalExit),
                None,
                Instant::now() + Duration::from_millis(10),
            )
            .is_err()
        );
        assert!(record.exists() && endpoint.exists() && management_path.exists());
        assert!(!session.is_finished());
        child.kill().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !super::recorded_process_is_dead(&session.identity).unwrap() {
            assert!(Instant::now() < deadline, "test owner did not exit");
            thread::sleep(Duration::from_millis(1));
        }

        let held_record = root.join("held-record");
        fs::rename(&record, &held_record).unwrap();
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&record)
            .unwrap()
            .write_all(&bytes)
            .unwrap();
        assert!(super::session_finished(&mut session).is_err());
        assert_eq!(fs::read(&record).unwrap(), bytes);
        assert!(endpoint.exists() && management_path.exists());
        fs::remove_file(&record).unwrap();
        fs::rename(held_record, &record).unwrap();

        let held_endpoint = root.join("held-endpoint");
        fs::rename(&endpoint, &held_endpoint).unwrap();
        let replacement = UnixListener::bind(&endpoint).unwrap();
        fs::set_permissions(
            &endpoint,
            std::os::unix::fs::PermissionsExt::from_mode(0o600),
        )
        .unwrap();
        assert!(super::session_finished(&mut session).is_err());
        assert!(record.exists() && endpoint.exists());
        fs::remove_file(&endpoint).unwrap();
        drop(replacement);
        fs::rename(held_endpoint, &endpoint).unwrap();

        assert!(
            super::wait_and_finalize_tombstone(
                &mut session,
                Some(TerminationReason::ExplicitStop),
                None,
                Instant::now() + Duration::from_secs(1),
            )
            .is_err()
        );
        assert_eq!(super::session_finished(&mut session).unwrap(), Some(1));
        assert!(session.is_finished());
        assert!(!process.exists(), "lost direct child was not reaped");
        assert!(!record.exists() && !endpoint.exists() && !management_path.exists());
        assert!(
            stop_managed_sessions(&mut [session], Duration::from_secs(1))
                .unwrap()
                .is_empty(),
            "owner loss was reported as successful Stop"
        );
        let _ = child.wait();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn recorded_process_is_dead_checks_its_identity_until_reaped() {
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let start = crate::platform::process(child.id())
            .unwrap()
            .expect("live child has no process")
            .start;
        let endpoint = EndpointIdentity {
            path: Vec::new(),
            object: ObjectIdentity {
                device: 1,
                inode: 1,
            },
        };
        let identity = LiveIdentity {
            session_id: "session-1".into(),
            run_id: "liveness".into(),
            component_generation: "component-1".into(),
            record_generation: management::RECORD_GENERATION,
            management_generation: management::VERSION,
            process_id: child.id(),
            process_start: start,
            uid: super::effective_uid(),
            presentation: endpoint.clone(),
            management: endpoint,
        };
        let mismatched = LiveIdentity {
            process_start: start + 1,
            ..identity.clone()
        };
        let assert_start_checked = || {
            let error = super::recorded_process_is_dead(&mismatched).unwrap_err();
            assert!(error.contains("process start differs"), "{error}");
        };
        assert!(!super::recorded_process_is_dead(&identity).unwrap());
        assert_start_checked();
        child.kill().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !super::recorded_process_is_dead(&identity).unwrap() {
            assert!(Instant::now() < deadline, "test child did not exit");
            thread::sleep(Duration::from_millis(1));
        }
        assert_start_checked();
        child.wait().unwrap();
        assert!(super::recorded_process_is_dead(&identity).unwrap());
    }

    #[test]
    fn discovery_reaps_untracked_ended_children_before_removing_records() {
        let root = temporary_directory();
        for reason in [
            Some(TerminationReason::NaturalExit),
            Some(TerminationReason::ExplicitStop),
            None,
        ] {
            let mut child = Command::new("true").spawn().unwrap();
            let pid = child.id();
            let process_path = PathBuf::from(format!("/proc/{pid}"));
            let start = fs::read_to_string(process_path.join("stat"))
                .unwrap()
                .rsplit_once(')')
                .unwrap()
                .1
                .split_whitespace()
                .nth(19)
                .unwrap()
                .parse()
                .unwrap();
            let endpoint = root.join("k1.sock");
            let record = root.join("k1.sock.record");
            let identity = LiveIdentity {
                session_id: "directory-picker-1".into(),
                run_id: format!("run-{pid}"),
                component_generation: "component-1".into(),
                record_generation: management::RECORD_GENERATION,
                management_generation: management::VERSION,
                process_id: pid,
                process_start: start,
                uid: super::effective_uid(),
                presentation: EndpointIdentity {
                    path: endpoint.as_os_str().as_bytes().to_vec(),
                    object: ObjectIdentity {
                        device: 1,
                        inode: 1,
                    },
                },
                management: EndpointIdentity {
                    path: root
                        .join("k1.sock.management")
                        .as_os_str()
                        .as_bytes()
                        .to_vec(),
                    object: ObjectIdentity {
                        device: 1,
                        inode: 2,
                    },
                },
            };
            let deadline = Instant::now() + Duration::from_secs(1);
            while !super::recorded_process_is_dead(&identity).unwrap() {
                assert!(Instant::now() < deadline, "test child did not exit");
                thread::sleep(Duration::from_millis(5));
            }
            let value = reason.map_or_else(
                || ManagementRecord::Live(identity.clone()),
                |reason| {
                    ManagementRecord::Tombstone(Tombstone {
                        identity: identity.clone(),
                        reason,
                        outcome: ProcessOutcome::ExitCode(0),
                    })
                },
            );
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&record)
                .unwrap()
                .write_all(&management::encode_record(&value).unwrap())
                .unwrap();
            if let ManagementRecord::Tombstone(mut wrong) = value.clone() {
                wrong.identity.process_start += 1;
                fs::write(
                    &record,
                    management::encode_record(&ManagementRecord::Tombstone(wrong)).unwrap(),
                )
                .unwrap();
                let error = super::acquire_generation_sessions(
                    &root,
                    super::LaunchMode::Workspace,
                    "component-1",
                    &[],
                    None,
                    deadline,
                )
                .err()
                .expect("wrong process start was accepted");
                assert!(error.contains("process start differs"), "{error}");
                assert!(
                    process_path.exists(),
                    "wrong identity authorized child reaping"
                );
                assert!(record.exists(), "wrong identity retired its record");
                fs::write(&record, management::encode_record(&value).unwrap()).unwrap();
            }
            let sessions = super::acquire_generation_sessions(
                &root,
                super::LaunchMode::Workspace,
                "component-1",
                &[],
                None,
                deadline,
            )
            .unwrap();
            let reaped = !process_path.exists();
            let _ = child.wait();
            assert!(sessions.is_empty());
            assert!(!record.exists());
            assert!(
                reaped,
                "discovery removed {reason:?} record without reaping its child"
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rollback_after_child_exit_keeps_the_terminal_number_creatable() {
        let root = temporary_directory();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let record = root.join("session-5.sock.record");
        for tombstone in [true, false] {
            let claim = super::ReadyClaim::create(&record).unwrap();
            let mut orbit = Command::new("true").spawn().unwrap();
            let pid = orbit.id();
            let process_path = PathBuf::from(format!("/proc/{pid}"));
            // Orbit's endpoints, left behind as if it ended before removing them.
            let endpoint = |name: &str| {
                let path = root.join(name);
                drop(UnixListener::bind(&path).unwrap());
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
                let object = super::object_identity(&fs::symlink_metadata(&path).unwrap());
                (path, object)
            };
            let (presentation, presentation_object) = endpoint("session-5.sock");
            let (management_path, management_object) = endpoint("session-5.sock.management");
            let identity = LiveIdentity {
                session_id: "session-5".into(),
                run_id: format!("run-{pid}"),
                component_generation: "component-1".into(),
                record_generation: management::RECORD_GENERATION,
                management_generation: management::VERSION,
                process_id: pid,
                process_start: 1,
                uid: super::effective_uid(),
                presentation: EndpointIdentity {
                    path: presentation.as_os_str().as_bytes().to_vec(),
                    object: presentation_object,
                },
                management: EndpointIdentity {
                    path: management_path.as_os_str().as_bytes().to_vec(),
                    object: management_object,
                },
            };
            if !tombstone {
                // Orbit itself ended after Ready; Eon has already observed it.
                orbit.wait().unwrap();
            }
            let value = if tombstone {
                ManagementRecord::Tombstone(Tombstone {
                    identity,
                    reason: TerminationReason::NaturalExit,
                    outcome: ProcessOutcome::ExitCode(7),
                })
            } else {
                ManagementRecord::Live(identity)
            };
            // Orbit atomically replaces the empty claim with its record.
            let published = root.join("published");
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&published)
                .unwrap()
                .write_all(&management::encode_record(&value).unwrap())
                .unwrap();
            fs::rename(&published, &record).unwrap();

            let deadline = Instant::now() + Duration::from_secs(2);
            let launch = (format!("run-{pid}"), presentation.clone());
            super::rollback_unleased_orbit(
                orbit,
                claim,
                (&launch.0, "component-1", &launch.1),
                deadline,
            )
            .unwrap();
            assert!(
                !record.exists() && !presentation.exists() && !management_path.exists(),
                "rollback left residue (tombstone: {tombstone})"
            );
            assert!(!process_path.exists(), "rollback did not reap the terminal");
            drop(super::ReadyClaim::create(&record).unwrap());
        }
        fs::remove_dir_all(root).unwrap();
    }

    /// A leased run of `session-<number>` whose tombstone for `reason` is
    /// already recorded, with the lease's other end as Orbit's.
    fn tombstoned_session(
        root: &Path,
        number: usize,
        reason: TerminationReason,
    ) -> (RunningSession, UnixStream, Tombstone) {
        let endpoint = root.join(format!("session-{number}.sock"));
        let identity = LiveIdentity {
            session_id: format!("session-{number}"),
            run_id: format!("run-{number}"),
            component_generation: "component-1".into(),
            record_generation: management::RECORD_GENERATION,
            management_generation: management::VERSION,
            process_id: std::process::id(),
            process_start: 1,
            uid: super::effective_uid(),
            presentation: EndpointIdentity {
                path: endpoint.as_os_str().as_bytes().to_vec(),
                object: ObjectIdentity {
                    device: 1,
                    inode: 1,
                },
            },
            management: EndpointIdentity {
                path: root
                    .join(format!("management-{number}"))
                    .as_os_str()
                    .as_bytes()
                    .to_vec(),
                object: ObjectIdentity {
                    device: 1,
                    inode: 2,
                },
            },
        };
        let tombstone = Tombstone {
            identity: identity.clone(),
            reason,
            outcome: ProcessOutcome::ExitCode(0),
        };
        let record = root.join(format!("session-{number}.record"));
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&record)
            .unwrap()
            .write_all(
                &management::encode_record(&ManagementRecord::Tombstone(tombstone.clone()))
                    .unwrap(),
            )
            .unwrap();
        let (lease, peer) = UnixStream::pair().unwrap();
        lease.set_nonblocking(true).unwrap();
        let session = RunningSession {
            number: Some(number),
            id: identity.session_id.clone(),
            endpoint,
            record_object: super::object_identity(&fs::symlink_metadata(&record).unwrap()),
            record,
            identity,
            orbit_log: None,
            lease,
            program_status: None,
            stop_sent: false,
            reap_failure_reported: false,
            child: None,
            finished: None,
        };
        (session, peer, tombstone)
    }

    #[test]
    fn failed_natural_cleanup_still_finalizes_other_runs_and_restores_leases() {
        let root = temporary_directory();
        let mut peers = Vec::new();
        let mut sessions = Vec::new();
        for (index, reason) in [
            TerminationReason::NaturalExit,
            TerminationReason::ExplicitStop,
        ]
        .into_iter()
        .enumerate()
        {
            let (session, mut peer, tombstone) = tombstoned_session(&root, index + 1, reason);
            peer.write_all(
                &management::encode_server_message(&ManagementServerMessage::Stopped(tombstone))
                    .unwrap(),
            )
            .unwrap();
            peers.push(peer);
            sessions.push(session);
        }
        fs::write(&sessions[0].endpoint, "replacement").unwrap();
        let error = stop_managed_sessions(&mut sessions, Duration::from_secs(1)).unwrap_err();
        assert!(error.contains("replaced during cleanup"), "{error}");
        assert!(
            sessions[1].is_finished(),
            "one failure skipped a completed run"
        );
        assert!(!sessions[1].record.exists());
        assert_eq!(
            fs::read_to_string(&sessions[0].endpoint).unwrap(),
            "replacement"
        );
        for session in &sessions {
            assert_eq!(session.lease.read_timeout().unwrap(), None);
            assert_eq!(session.lease.write_timeout().unwrap(), None);
            // SAFETY: F_GETFL only reads flags from this test-owned descriptor.
            let flags = unsafe { libc::fcntl(session.lease.as_raw_fd(), libc::F_GETFL) };
            assert_ne!(flags & libc::O_NONBLOCK, 0);
        }
        assert!(
            !super::send_program_status(&mut sessions[0], Instant::now()).unwrap(),
            "polled a lease with a Stop pending"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// Writes an executable shell script to a fresh path. A child shell writes
    /// it: had this process opened it for writing, a child forked meanwhile by
    /// a parallel test would inherit that descriptor, and exec would fail with
    /// ETXTBSY.
    pub(crate) fn script(path: &Path, body: &str) {
        let status = Command::new("/bin/sh")
            .args(["-c", "printf '%s' \"$1\" > \"$0\""])
            .arg(path)
            .arg(body)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "{status}");
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    /// A real Orbit run of `script`, leased by this Runtime. Orbit is the one
    /// at `ORBIT_BIN`, which eon-test and the eon package's check set.
    pub(crate) fn leased_orbit(root: &Path, script: &str) -> RunningSession {
        let orbit =
            PathBuf::from(std::env::var_os("ORBIT_BIN").expect("ORBIT_BIN names the Orbit to run"));
        leased_orbit_with_program(root, script, &orbit)
    }

    pub(crate) fn leased_orbit_with_program(
        root: &Path,
        script: &str,
        orbit: &Path,
    ) -> RunningSession {
        fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
        let mut inputs = crate::fixtures::inputs();
        inputs.orbit = orbit.into();
        let log = DiagnosticLog::open_in(Ok(root.join("launch-logs")), "g1-test", "workspace");
        super::start_orbit(
            &inputs,
            root,
            &root.join("orbit.sock"),
            "session-1",
            "component-1",
            root,
            &["sh".into(), "-c".into(), script.into()],
            None,
            &log,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap()
    }

    #[test]
    fn startup_failure_names_the_orbit_log() {
        let root = temporary_directory();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let launcher = root.join("orbit-startup-failure");
        script(
            &launcher,
            "#!/bin/sh\nprintf '%s\\n' 'orbit startup diagnostic' >&2\nexit 9\n",
        );
        let mut inputs = crate::fixtures::inputs();
        inputs.orbit = launcher;
        let log = DiagnosticLog::open_in(Ok(root.join("launch-logs")), "g1-test", "workspace");
        let error = super::start_orbit(
            &inputs,
            &root,
            &root.join("orbit.sock"),
            "session-1",
            "component-1",
            &root,
            &["sh".into(), "-c".into(), "exec sleep 30".into()],
            None,
            &log,
            Instant::now() + Duration::from_secs(5),
        )
        .err()
        .expect("an Orbit that exits before Ready started a terminal");
        assert!(
            error.contains("terminal exited before publishing Ready (status 9)"),
            "{error}"
        );
        let (_, orbit_log) = error
            .rsplit_once("; Orbit log: ")
            .expect("the startup failure names the Orbit log");
        assert!(
            Path::new(orbit_log).starts_with(root.join("launch-logs/orbit")),
            "{error}"
        );
        assert_eq!(
            fs::read_to_string(orbit_log).unwrap(),
            "orbit startup diagnostic\n"
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// The run record of a leased Orbit this test started.
    pub(crate) fn record(session: &RunningSession) -> &Path {
        &session.record
    }

    /// Shuts the lease's read side, so the Runtime reads it as closed.
    pub(crate) fn close_lease(session: &RunningSession) {
        session.lease.shutdown(std::net::Shutdown::Read).unwrap();
    }

    /// The process id of a leased Orbit this test started.
    pub(crate) fn orbit_pid(session: &RunningSession) -> i32 {
        session.child.as_ref().unwrap().id() as i32
    }

    /// Requests program status and reads the reply as the poller does.
    fn poll(session: &mut RunningSession) -> management::ProgramStatus {
        let deadline = Instant::now() + Duration::from_secs(2);
        assert!(super::send_program_status(session, deadline).unwrap());
        loop {
            match super::read_program_status(session).unwrap() {
                ProgramStatusRead::Pending => thread::sleep(Duration::from_millis(1)),
                ProgramStatusRead::Reply(status) => return status,
                ProgramStatusRead::Missed => panic!("program status missed its deadline"),
            }
        }
    }

    #[test]
    fn eof_probe_and_stop_never_misread_an_in_flight_program_status_reply() {
        let root = temporary_directory();
        let mut session = leased_orbit(
            &root,
            "printf '\\033]7501;state=working:id=w\\007'; exec sleep 30",
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while poll(&mut session).records.is_empty() {
            assert!(Instant::now() < deadline, "Orbit never reported the record");
            thread::sleep(Duration::from_millis(10));
        }

        // The whole reply is waiting on the lease, unread, when the EOF probe
        // runs.
        session
            .lease
            .write_all(
                &management::encode_client_message(&management::ClientMessage::ProgramStatus)
                    .unwrap(),
            )
            .unwrap();
        session.program_status = Some(super::ProgramStatusRequest {
            unsent: Vec::new(),
            reply: Vec::new(),
            deadline: Instant::now() + Duration::from_secs(2),
        });
        let mut readable = libc::pollfd {
            fd: session.lease.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: readable is one valid pollfd for this test-owned lease.
        assert_eq!(unsafe { libc::poll(&mut readable, 1, 2000) }, 1);
        assert_eq!(super::session_finished(&mut session).unwrap(), None);
        let ProgramStatusRead::Reply(status) = super::read_program_status(&mut session).unwrap()
        else {
            panic!("the EOF probe lost the in-flight reply");
        };
        assert_eq!(status.records[0].id, "w");

        // A Stop right behind a request waits for its reply first.
        assert!(super::send_program_status(&mut session, Instant::now()).unwrap());
        assert_eq!(
            stop_managed_sessions(std::slice::from_mut(&mut session), Duration::from_secs(5))
                .unwrap(),
            ["session-1"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_silent_terminal_delays_only_its_own_stop() {
        let roots = [temporary_directory(), temporary_directory()];
        let mut sessions = roots
            .iter()
            .map(|root| leased_orbit(root, "exec sleep 30"))
            .collect::<Vec<_>>();
        let silent = orbit_pid(&sessions[0]);
        // SAFETY: signals the test-owned Orbit process.
        assert_eq!(unsafe { libc::kill(silent, libc::SIGSTOP) }, 0);
        assert!(super::send_program_status(&mut sessions[0], Instant::now()).unwrap());
        assert!(stop_managed_sessions(&mut sessions, Duration::from_secs(1)).is_err());
        assert!(
            sessions[1].is_finished(),
            "the silent terminal blocked another's Stop"
        );
        assert!(!sessions[0].is_finished());

        // Its lease survived the missed reply and still stops it.
        // SAFETY: signals the test-owned Orbit process.
        assert_eq!(unsafe { libc::kill(silent, libc::SIGCONT) }, 0);
        assert_eq!(
            stop_managed_sessions(&mut sessions[..1], Duration::from_secs(5)).unwrap(),
            ["session-1"]
        );
        for root in roots {
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn an_orbit_stopped_by_a_signal_ends_only_its_terminal() {
        let roots = [temporary_directory(), temporary_directory()];
        let mut sessions = roots
            .iter()
            .map(|root| leased_orbit(root, "exec sleep 30"))
            .collect::<Vec<_>>();
        // Orbit answers its lease Stopped, unasked, and records the stop
        // (ORB-C12).
        // SAFETY: signals the test-owned Orbit process.
        assert_eq!(
            unsafe { libc::kill(orbit_pid(&sessions[0]), libc::SIGTERM) },
            0
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while super::session_finished(&mut sessions[0]).unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "the stopped terminal never ended"
            );
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!sessions[0].record.exists());

        assert_eq!(super::session_finished(&mut sessions[1]).unwrap(), None);
        poll(&mut sessions[1]);
        assert_eq!(
            stop_managed_sessions(&mut sessions[1..], Duration::from_secs(5)).unwrap(),
            ["session-1"]
        );
        for root in roots {
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn an_undecodable_or_unsolicited_result_ends_only_its_request() {
        let root = temporary_directory();
        let request =
            management::encode_client_message(&management::ClientMessage::ProgramStatus).unwrap();
        let mut sent = vec![0; request.len()];
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut session, mut peer, tombstone) =
            tombstoned_session(&root, 1, TerminationReason::ExplicitStop);
        assert!(super::send_program_status(&mut session, deadline).unwrap());
        peer.read_exact(&mut sent).unwrap();
        assert_eq!(sent, request);
        peer.write_all(&[0xff; management::HEADER_BYTES]).unwrap();
        assert_eq!(super::session_finished(&mut session).unwrap(), None);
        assert!(!matches!(
            super::read_program_status(&mut session),
            Ok(ProgramStatusRead::Reply(_))
        ));

        // The next poll sends a fresh request and reads its reply.
        assert!(super::send_program_status(&mut session, deadline).unwrap());
        peer.read_exact(&mut sent).unwrap();
        assert_eq!(sent, request);
        peer.write_all(
            &management::encode_server_message(&ManagementServerMessage::ProgramStatus(
                management::ProgramStatus {
                    count: 0,
                    background: 0,
                    update: 1,
                    terminal_title: String::new(),
                    omitted: Vec::new(),
                    records: Vec::new(),
                },
            ))
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            super::read_program_status(&mut session).unwrap(),
            ProgramStatusRead::Reply(_)
        ));

        // Stop on that lease still succeeds.
        peer.write_all(
            &management::encode_server_message(&ManagementServerMessage::Stopped(tombstone))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            stop_managed_sessions(std::slice::from_mut(&mut session), Duration::from_secs(1))
                .unwrap(),
            ["session-1"]
        );

        // An unsolicited byte before EOF only delays the end.
        let (mut session, mut peer, _) =
            tombstoned_session(&root, 2, TerminationReason::NaturalExit);
        peer.write_all(&[0x42]).unwrap();
        drop(peer);
        let deadline = Instant::now() + Duration::from_secs(1);
        while super::session_finished(&mut session).unwrap().is_none() {
            assert!(Instant::now() < deadline, "the terminal never ended");
        }
        assert!(session.is_finished());
        fs::remove_dir_all(root).unwrap();
    }

    // A debug Orbit takes about 100 s per turn of a flood, so this measures the
    // packaged Orbit at ORBIT_BIN, as the eon package's check does.
    #[test]
    #[ignore = "measures a release Orbit at ORBIT_BIN"]
    fn stop_waits_only_orbit_answer_time_for_an_in_flight_reply_under_pty_load() {
        let root = temporary_directory();
        // The flood starts once the lease is held.
        let mut session = leased_orbit(
            &root,
            "printf '\\033]7501;state=working:id=load\\007'\n\
             while [ ! -e flood ]; do sleep 0.01; done\n\
             exec yes",
        );
        fs::write(root.join("flood"), "").unwrap();
        thread::sleep(Duration::from_millis(100));
        let mut delays = Vec::new();
        for _ in 0..100 {
            assert!(super::send_program_status(&mut session, Instant::now()).unwrap());
            let started = Instant::now();
            while !super::drain_program_status(&mut session).unwrap() {
                assert!(started.elapsed() < Duration::from_secs(5), "no reply");
            }
            delays.push(started.elapsed());
            thread::sleep(Duration::from_millis(10));
        }
        delays.sort();
        let (median, p99, max) = (delays[50], delays[98], delays[99]);
        // Straight to stderr, which the test harness does not capture, so the
        // package check's log keeps the distribution.
        writeln!(
            std::io::stderr(),
            "Stop's wait for an in-flight reply under PTY load: median {median:?}, p99 {p99:?}, max {max:?}"
        )
        .unwrap();
        // Orbit targets an answer within 250 ms, and the 2 s worker-stall
        // bound caps each worker exchange behind it (ORB-C15).
        assert!(median < Duration::from_millis(250), "median {median:?}");
        assert!(max < Duration::from_secs(2), "max {max:?}");
        assert!(super::send_program_status(&mut session, Instant::now()).unwrap());
        assert_eq!(
            stop_managed_sessions(std::slice::from_mut(&mut session), Duration::from_secs(5))
                .unwrap(),
            ["session-1"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn session_identity_is_positive_canonical_decimal() {
        assert_eq!(session_number("session-1"), Some(1));
        assert_eq!(session_number("session-256"), Some(256));
        assert_eq!(managed_session_number("directory-picker"), Ok(None));
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
        for valid in ["pick.sock", "k1.sock", "k256.sock"] {
            assert!(is_directory_picker_endpoint(valid));
        }
        for invalid in [
            "k0.sock",
            "k01.sock",
            "k.sock",
            "k-1.sock",
            "k1.sock.record",
            "../k1.sock",
            "k184467440737095516160.sock",
        ] {
            assert!(!is_directory_picker_endpoint(invalid), "accepted {invalid}");
        }
    }

    #[test]
    fn unix_connection_attempt_obeys_timeout() {
        let root = temporary_directory();
        let path = root.join("backlog.sock");
        let socket =
            socket2::Socket::new(socket2::Domain::UNIX, socket2::Type::STREAM, None).unwrap();
        socket
            .bind(&socket2::SockAddr::unix(&path).unwrap())
            .unwrap();
        socket.listen(0).unwrap();
        let listener = UnixListener::from(std::os::fd::OwnedFd::from(socket));
        let queued = UnixStream::connect(&path).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let attempt = thread::spawn(move || {
            sender
                .send(unix_connect_with_timeout(&path, Duration::from_millis(50)))
                .unwrap();
        });

        let result = receiver.recv_timeout(Duration::from_secs(1));
        drop(queued);
        drop(listener);
        attempt.join().unwrap();

        assert!(
            result
                .expect("Unix connection exceeded its timeout")
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn management_response_obeys_one_deadline_across_partial_reads() {
        let (mut reader, mut writer) = UnixStream::pair().unwrap();
        let response = management::encode_server_message(&ManagementServerMessage::Busy).unwrap();
        let sender = thread::spawn(move || {
            for byte in response {
                if writer.write_all(&[byte]).is_err() {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        });
        let started = Instant::now();
        let result = read_management_response(
            &mut reader,
            started + Duration::from_millis(80),
            "terminal management response",
        );
        let elapsed = started.elapsed();
        drop(reader);
        sender.join().unwrap();

        assert!(result.is_err(), "received {result:?}");
        assert!(elapsed >= Duration::from_millis(40));
        assert!(elapsed < Duration::from_secs(1));
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
        let mut inputs = crate::fixtures::inputs();
        inputs.orbit = "/managed/orbit".into();
        inputs.venus = "/managed/venus".into();
        inputs.managed.shell_bin = Some("/managed/bin".into());
        inputs.defaults.ansi_palette = "101010,101010,101010,101010,101010,101010,101010,101010,101010,101010,101010,101010,101010,101010,101010,101010";
        let config = root.as_path();
        let socket = Path::new("/runtime/orbit.sock");
        fs::write(
            root.join("config.toml"),
            "[shell]\ncommand = [\"eon-bash\", \"--norc\"]\n",
        )
        .unwrap();

        let default = orbit_command(
            &inputs,
            config,
            socket,
            "session-1",
            "run-1",
            "component-1",
            root.as_path(),
            &[],
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            default.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "serve",
                "/runtime/orbit.sock",
                "--management-v3",
                "session-1",
                "run-1",
                "component-1",
                "--ansi-palette-v1",
                inputs.defaults.ansi_palette,
                "--",
                "eon-bash",
                "--norc",
            ]
            .map(OsString::from)
        );
        assert_eq!(
            command_environment(&default, "EON_CONFIG_HOME"),
            Some(Some(config.as_os_str()))
        );
        assert_eq!(
            command_environment(&default, "EON_TERMINAL"),
            Some(Some(OsStr::new("session-1.run-1")))
        );
        assert_eq!(command_environment(&default, "XDG_CONFIG_HOME"), None);
        assert_eq!(command_environment(&default, "EON_TAB"), Some(None));
        // Fails when the default shell loses the marker that keeps the supervisor's
        // already-logged unknown-key warning out of its scrollback.
        assert_eq!(
            command_environment(&default, "EON_CONFIG_WARNED"),
            Some(Some(OsStr::new("1")))
        );
        // Fails when a default shell that is not Eon's launcher is marked: it would
        // pass the marker on and silence later launches inside that terminal.
        for shell in ["zsh", "/usr/bin/bash"] {
            fs::write(
                root.join("config.toml"),
                format!("[shell]\ncommand = [\"{shell}\"]\n"),
            )
            .unwrap();
            let other = orbit_command(
                &inputs,
                config,
                socket,
                "session-3",
                "run-3",
                "component-3",
                root.as_path(),
                &[],
                None,
                None,
            )
            .unwrap();
            assert_eq!(
                command_environment(&other, "EON_CONFIG_WARNED"),
                Some(None),
                "{shell}"
            );
        }
        assert_eq!(default.get_current_dir(), Some(root.as_path()));
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
            &inputs,
            config,
            socket,
            "session-2",
            "run-2",
            "component-2",
            root.as_path(),
            &["codex".into(), "--model".into(), "test".into()],
            Some("t3"),
            None,
        )
        .unwrap();
        assert_eq!(
            command_environment(&explicit, "EON_TAB"),
            Some(Some(OsStr::new("t3")))
        );
        // Fails when an explicit command, whose configuration the supervisor did not
        // read, is marked and so drops the warning.
        assert_eq!(
            command_environment(&explicit, "EON_CONFIG_WARNED"),
            Some(None)
        );
        assert_eq!(
            explicit.get_args().map(OsString::from).collect::<Vec<_>>(),
            [
                "serve",
                "/runtime/orbit.sock",
                "--management-v3",
                "session-2",
                "run-2",
                "component-2",
                "--ansi-palette-v1",
                inputs.defaults.ansi_palette,
                "--",
                "codex",
                "--model",
                "test",
            ]
            .map(OsString::from)
        );
        fs::remove_dir_all(root).unwrap();
    }
}
