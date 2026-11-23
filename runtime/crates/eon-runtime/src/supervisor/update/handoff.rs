//! The settled supervisor's private EONH v1 resource table (eon-t3fyd.8).
//! EONU owns the request and checkpoint frames. Orbit owns each Live record.

use super::{Target, import, preflight, validate_handoff_target};
use crate::{
    Inputs, LaunchFacts,
    checkpoint::{Header, update::Scope},
    control::{ControlListener, PrivateListener},
    diagnostics::DiagnosticLog,
    generation::{Identity, NamespaceLock, current_generation, generation_directory},
    platform,
    sessions::{AttachName, RunningSession, validate_carried_lease},
    supervisor::{
        GenerationCleanup, LaunchMode, LaunchOptions, LifecycleLock, SESSION_CLEANUP_TIMEOUT,
        SupervisorState, WindowSettings, base_runtime_directory, configuration_directory,
        effective_uid, request_id, runtime_directory, service, supervisor_lock_path,
        validate_private_directory,
    },
    update::{self, Reply, UpdateListener, refusal},
    windows::{WINDOW_RUNTIME, runtime_window},
};
use eon_workspace_protocol::update::{
    Carry, Code, HEADER_BYTES, Kind, Operation, Outcome, Preflight, Refused, TargetCheck, Updated,
    UpdatedPresentation, valid_id35,
};
use orbit_protocol::management::{self, LiveIdentity, ObjectIdentity, Record};
use rustix::{
    fs::{FileType, FlockOperation, OFlags, Stat, fcntl_getfl, flock, fstat},
    io::pread,
    net::{AddressFamily, SocketType, getsockname, sockopt},
};
use std::{
    collections::HashSet,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Seek, Write},
    net::Shutdown,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::Path,
    process::Command,
    thread,
    time::{Duration, Instant},
};

pub(crate) const RESUME_UPDATE: &str = "--update-resume";
const PREFIX: &[u8; 6] = b"EONH\x01\0";
const HOSTS: usize = 256;
const CAP: usize = 17_975_533;
const _: () = assert!(
    CAP == 18
        + (4 + 12 + 8192)
        + (4 + 12 + 16 * 1024 * 1024 + 128 * 1024)
        + 132
        + 53
        + 2
        + HOSTS * (20 + 16 + 4 + management::MAX_RECORD_BYTES)
);

#[derive(Clone, Copy, Debug)]
struct Descriptor {
    number: RawFd,
    object: ObjectIdentity,
}

impl Descriptor {
    fn write(bytes: &mut Vec<u8>, fd: BorrowedFd<'_>) -> io::Result<()> {
        if fd.as_raw_fd() < 3 {
            return Err(invalid("update resource occupies standard I/O"));
        }
        bytes.extend(fd.as_raw_fd().to_le_bytes());
        write_object(bytes, object(&fstat(fd)?));
        Ok(())
    }

    fn inspect<T>(
        &self,
        check: impl FnOnce(BorrowedFd<'_>, Stat) -> io::Result<T>,
    ) -> io::Result<T> {
        platform::inspect_inherited(self.number, |fd| {
            let stat = fstat(fd)?;
            if object(&stat) != self.object {
                return Err(invalid("inherited descriptor identity changed"));
            }
            check(fd, stat)
        })
    }

    fn take(self) -> io::Result<OwnedFd> {
        platform::take_inherited(self.number)
    }

    fn socket<T>(
        &self,
        listening: bool,
        check: impl FnOnce(BorrowedFd<'_>) -> io::Result<T>,
    ) -> io::Result<T> {
        self.inspect(|fd, _| {
            // Darwin pads unnamed sockaddr_un values, which rustix's pathname decoder rejects.
            if getsockname(fd)?.address_family() != AddressFamily::UNIX
                || sockopt::socket_type(fd)? != SocketType::STREAM
                || platform::socket_listening(fd)? != listening
            {
                return Err(invalid("inherited socket has the wrong type or role"));
            }
            check(fd)
        })
    }
}

#[allow(clippy::unnecessary_cast)] // st_dev differs between Linux and Darwin.
fn object(stat: &Stat) -> ObjectIdentity {
    ObjectIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino,
    }
}

fn write_object(out: &mut Vec<u8>, object: ObjectIdentity) {
    out.extend(object.device.to_le_bytes());
    out.extend(object.inode.to_le_bytes());
}

fn write_frame(bytes: &mut Vec<u8>, frame: &[u8]) {
    bytes.extend((frame.len() as u32).to_le_bytes());
    bytes.extend(frame);
}

fn invalid(detail: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail.into())
}

struct Reader<'a> {
    bytes: &'a [u8],
    numbers: HashSet<RawFd>,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, len: usize) -> io::Result<&'a [u8]> {
        let (bytes, remaining) = self
            .bytes
            .split_at_checked(len)
            .ok_or_else(|| invalid("truncated update handoff"))?;
        self.bytes = remaining;
        Ok(bytes)
    }

    fn array<const N: usize>(&mut self) -> io::Result<[u8; N]> {
        Ok(self.bytes(N)?.try_into().expect("fixed length"))
    }

    fn object(&mut self) -> io::Result<ObjectIdentity> {
        Ok(ObjectIdentity {
            device: u64::from_le_bytes(self.array()?),
            inode: u64::from_le_bytes(self.array()?),
        })
    }

    fn descriptor(&mut self) -> io::Result<Descriptor> {
        let number = i32::from_le_bytes(self.array()?);
        if number < 3 || !self.numbers.insert(number) {
            return Err(invalid("invalid or duplicate inherited descriptor"));
        }
        Ok(Descriptor {
            number,
            object: self.object()?,
        })
    }

    fn frame(&mut self, bound: usize) -> io::Result<&'a [u8]> {
        let len = u32::from_le_bytes(self.array()?) as usize;
        if len == 0 || len > bound {
            return Err(invalid("invalid update handoff frame length"));
        }
        self.bytes(len)
    }
}

struct Listener {
    fd: Descriptor,
    path: ObjectIdentity,
}

impl Listener {
    fn write(bytes: &mut Vec<u8>, listener: &PrivateListener) -> io::Result<()> {
        Descriptor::write(bytes, listener.listener().as_fd())?;
        let (device, inode) = listener.identity();
        write_object(bytes, ObjectIdentity { device, inode });
        Ok(())
    }

    fn read(reader: &mut Reader<'_>) -> io::Result<Self> {
        Ok(Self {
            fd: reader.descriptor()?,
            path: reader.object()?,
        })
    }

    fn validate(&self, path: &Path) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != effective_uid()
            || metadata.mode() & 0o7777 != 0o600
            || (metadata.dev(), metadata.ino()) != (self.path.device, self.path.inode)
        {
            return Err(invalid("inherited listener path has changed"));
        }
        self.fd.socket(true, |fd| {
            let address = UnixListener::from(fd.try_clone_to_owned()?).local_addr()?;
            if address.as_pathname().map(Path::as_os_str) != Some(path.as_os_str())
                || !fcntl_getfl(fd)?.contains(OFlags::NONBLOCK)
            {
                return Err(invalid(
                    "inherited listener has a different address or flags",
                ));
            }
            Ok(())
        })
    }

    fn take(self, path: &Path) -> io::Result<PrivateListener> {
        Ok(PrivateListener::inherited(
            self.fd.take()?.into(),
            path.into(),
            (self.path.device, self.path.inode),
        ))
    }
}

struct Venus {
    name: AttachName,
    control: Descriptor,
    stderr: Descriptor,
}

impl Venus {
    fn read(reader: &mut Reader<'_>) -> io::Result<Option<Self>> {
        match reader.array::<1>()?[0] {
            0 => Ok(None),
            1 => Ok(Some(Self {
                name: AttachName {
                    process_id: u32::from_le_bytes(reader.array()?),
                    process_start: u64::from_le_bytes(reader.array()?),
                },
                control: reader.descriptor()?,
                stderr: reader.descriptor()?,
            })),
            _ => Err(invalid("invalid inherited Venus tag")),
        }
    }

    fn validate(&self) -> io::Result<()> {
        if !exact_child(self.name)? {
            return Err(invalid("inherited Venus is not the recorded exact child"));
        }
        self.control.socket(false, |fd| {
            if !UnixStream::from(fd.try_clone_to_owned()?)
                .local_addr()?
                .is_unnamed()
            {
                return Err(invalid("inherited Venus control is not a socketpair"));
            }
            Ok(())
        })?;
        self.stderr.inspect(|fd, stat| {
            if FileType::from_raw_mode(stat.st_mode) != FileType::Fifo
                || (fcntl_getfl(fd)? & OFlags::ACCMODE) != OFlags::RDONLY
            {
                return Err(invalid("inherited Venus stderr is not a read pipe"));
            }
            Ok(())
        })
    }
}

fn exact_child(name: AttachName) -> io::Result<bool> {
    Ok(platform::process(name.process_id)
        .map_err(invalid)?
        .is_some_and(|process| {
            process.start == name.process_start && process.parent == std::process::id()
        }))
}

/// Held separately from native admission until S5a replaces the presentation.
pub(in crate::supervisor) struct InheritedPresentation {
    name: AttachName,
    control: UnixStream,
    _relay: thread::JoinHandle<()>,
}

impl InheritedPresentation {
    pub(in crate::supervisor) fn name(&self) -> AttachName {
        self.name
    }

    pub(in crate::supervisor) fn exited(&self) -> io::Result<bool> {
        use rustix::process::{Pid, WaitOptions, waitpid};
        if !exact_child(self.name)? {
            return Ok(true);
        }
        let pid = i32::try_from(self.name.process_id)
            .ok()
            .and_then(Pid::from_raw)
            .ok_or_else(|| invalid("invalid Venus PID"))?;
        match waitpid(Some(pid), WaitOptions::NOHANG) {
            Ok(status) => Ok(status.is_some()),
            Err(rustix::io::Errno::CHILD) => Ok(true),
            Err(rustix::io::Errno::INTR) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    pub(in crate::supervisor) fn close(self) {
        use rustix::process::{Pid, Signal, WaitOptions, kill_process, waitpid};
        let _ = self.control.shutdown(Shutdown::Both);
        let deadline = Instant::now() + SESSION_CLEANUP_TIMEOUT;
        while Instant::now() < deadline {
            if !matches!(self.exited(), Ok(false)) {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        if matches!(exact_child(self.name), Ok(true))
            && let Some(pid) = i32::try_from(self.name.process_id)
                .ok()
                .and_then(Pid::from_raw)
            && kill_process(pid, Signal::KILL).is_ok()
        {
            while matches!(
                waitpid(Some(pid), WaitOptions::empty()),
                Err(rustix::io::Errno::INTR)
            ) {}
        }
    }
}

struct Lease {
    fd: Descriptor,
    record: ObjectIdentity,
    identity: LiveIdentity,
}

struct Envelope {
    carry: Carry,
    check: TargetCheck,
    lifecycle: Descriptor,
    namespace: Descriptor,
    control: Listener,
    update: Listener,
    reply: Descriptor,
    venus: Option<Venus>,
    leases: Vec<Lease>,
}

impl Envelope {
    fn read(bytes: &[u8], handoff: RawFd) -> io::Result<Self> {
        let mut reader = Reader {
            bytes,
            numbers: HashSet::from([handoff]),
        };
        if reader.bytes(PREFIX.len())? != PREFIX {
            return Err(invalid("unsupported update handoff format"));
        }
        let pid = u32::from_le_bytes(reader.array()?);
        let start = u64::from_le_bytes(reader.array()?);
        if pid != std::process::id() || process_start()? != start {
            return Err(invalid("update handoff names a different supervisor"));
        }
        let carry = Carry::decode(reader.frame(HEADER_BYTES + Kind::Carry.bound())?)
            .map_err(io::Error::other)?;
        let check = TargetCheck::decode(reader.frame(HEADER_BYTES + Kind::TargetCheck.bound())?)
            .map_err(io::Error::other)?;
        let lifecycle = reader.descriptor()?;
        let namespace = reader.descriptor()?;
        let control = Listener::read(&mut reader)?;
        let update = Listener::read(&mut reader)?;
        let reply = reader.descriptor()?;
        let venus = Venus::read(&mut reader)?;
        let count = usize::from(u16::from_le_bytes(reader.array()?));
        if count == 0 || count > HOSTS || count != check.hosts.len() {
            return Err(invalid(
                "inherited lease count differs from the target inventory",
            ));
        }
        let leases = (0..count)
            .map(|_| {
                let fd = reader.descriptor()?;
                let record = reader.object()?;
                let encoded = reader.frame(management::MAX_RECORD_BYTES)?;
                let Record::Live(identity) =
                    management::decode_record(encoded).map_err(io::Error::other)?
                else {
                    return Err(invalid("inherited lease must carry a Live identity"));
                };
                Ok(Lease {
                    fd,
                    record,
                    identity,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        if !reader.bytes.is_empty() {
            return Err(invalid("trailing bytes in update handoff"));
        }
        Ok(Self {
            carry,
            check,
            lifecycle,
            namespace,
            control,
            update,
            reply,
            venus,
            leases,
        })
    }
}

fn process_start() -> io::Result<u64> {
    platform::process(std::process::id())
        .map_err(invalid)?
        .map(|process| process.start)
        .ok_or_else(|| invalid("supervisor process start is unavailable"))
}

fn read_file(fd: RawFd, runtime: &Path) -> io::Result<Vec<u8>> {
    platform::inspect_inherited(fd, |fd| {
        let stat = fstat(fd)?;
        let length = usize::try_from(stat.st_size).unwrap_or(0);
        if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
            || stat.st_uid != effective_uid()
            || stat.st_mode & 0o7777 != 0o600
            || stat.st_nlink != 0
            || object(&stat).device != fs::symlink_metadata(runtime)?.dev()
            || !(PREFIX.len()..=CAP).contains(&length)
        {
            return Err(invalid(
                "update handoff file is not an owned bounded unlinked file",
            ));
        }
        let mut bytes = vec![0; length];
        let mut offset = 0;
        while offset < length {
            match pread(fd, &mut bytes[offset..], offset as u64) {
                Ok(0) => return Err(invalid("truncated update handoff file")),
                Ok(count) => offset += count,
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
        }
        if pread(fd, &mut [0u8; 1][..], length as u64)? != 0 {
            return Err(invalid("trailing bytes in update handoff file"));
        }
        Ok(bytes)
    })
}

fn verify_lock(fd: &Descriptor, path: &Path) -> io::Result<()> {
    fd.inspect(|borrowed, stat| {
        let probe = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)?;
        let meta = probe.metadata()?;
        if !meta.is_file()
            || meta.uid() != effective_uid()
            || meta.mode() & 0o7777 != 0o600
            || object(&stat)
                != (ObjectIdentity {
                    device: meta.dev(),
                    inode: meta.ino(),
                })
            || !matches!(probe.try_lock(), Err(fs::TryLockError::WouldBlock))
        {
            return Err(invalid(
                "inherited lifecycle or namespace lock differs from its path",
            ));
        }
        // A separate open contends; only the inherited description can retake its lock.
        flock(borrowed, FlockOperation::NonBlockingLockExclusive)?;
        Ok(())
    })
}

pub(in crate::supervisor) struct Owner<'a> {
    pub(in crate::supervisor) lifecycle: &'a LifecycleLock,
    pub(in crate::supervisor) control: &'a ControlListener,
    pub(in crate::supervisor) inherited_attached: bool,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn begin(
    owner: &Owner<'_>,
    listener: &UpdateListener,
    state: &SupervisorState,
    inputs: &Inputs,
    root: &Path,
    facts: &LaunchFacts,
    request: Preflight,
    origin: Option<String>,
    check: TargetCheck,
    reply: &Reply,
) -> Refused {
    let id = request.request;
    let namespace = match NamespaceLock::try_acquire(root) {
        Ok(Some(lock)) => lock,
        Ok(None) => return refusal(id, Code::Busy, "the namespace launch lock is busy".into()),
        Err(error) => return refusal(id, Code::Internal, error),
    };
    let err = (|| -> io::Result<()> {
        let runtime = generation_directory(root, &request.lifetime);
        if owner.inherited_attached {
            return Err(invalid(
                "the retained presentation must be retired before another update",
            ));
        }
        let identity = Identity::read(&runtime)
            .map_err(invalid)?
            .ok_or_else(|| invalid("the window has no committed identity"))?;
        if !identity.matches(&check.lifetime, &check.created_build, &check.active_build) {
            return Err(invalid("active identity changed before update"));
        }
        if check.hosts.len() != state.sessions.len()
            || check.hosts.iter().zip(&state.sessions).any(|(host, row)| {
                let (_, _, identity) = row.carry_lease();
                host.terminal != identity.session_id || host.run != identity.run_id
            })
        {
            return Err(invalid(
                "the checked host inventory differs from the carried leases",
            ));
        }
        namespace
            .require_available(&request.target_build, &request.lifetime)
            .map_err(invalid)?;
        platform::require_waitable_children()?;
        let launch = inputs
            .launch
            .as_ref()
            .map_err(|error| invalid(error.clone()))?;
        let carry = Carry {
            request: id,
            lifetime: request.lifetime,
            from_build: request.expected_active_build,
            target_build: request.target_build,
            target_wrapper: request.target_wrapper,
            caller: request.caller,
            caller_origin: origin,
            reply_sent: false,
        };
        let mut bytes = PREFIX.to_vec();
        bytes.extend(std::process::id().to_le_bytes());
        bytes.extend(process_start()?.to_le_bytes());
        write_frame(&mut bytes, &carry.encode().map_err(io::Error::other)?);
        write_frame(&mut bytes, &check.encode().map_err(io::Error::other)?);

        let lifecycle = owner.lifecycle.file.as_fd();
        let namespace = namespace.lock().file.as_fd();
        let respond = reply.stream().as_fd();
        Descriptor::write(&mut bytes, lifecycle)?;
        Descriptor::write(&mut bytes, namespace)?;
        Listener::write(&mut bytes, &owner.control.0)?;
        Listener::write(&mut bytes, &listener.0)?;
        Descriptor::write(&mut bytes, respond)?;
        let mut descriptors = vec![
            lifecycle,
            namespace,
            owner.control.0.listener().as_fd(),
            listener.0.listener().as_fd(),
            respond,
        ];
        if let Some(venus) = &state.venus {
            let name = venus
                .name
                .ok_or_else(|| invalid("old Venus has no exact start identity"))?;
            if !exact_child(name)? {
                return Err(invalid(
                    "old Venus no longer has the recorded child identity",
                ));
            }
            let stderr = venus
                .retained_stderr
                .as_ref()
                .ok_or_else(|| invalid("old Venus stderr cannot be retained"))?;
            bytes.push(1);
            bytes.extend(name.process_id.to_le_bytes());
            bytes.extend(name.process_start.to_le_bytes());
            Descriptor::write(&mut bytes, venus.control.as_fd())?;
            Descriptor::write(&mut bytes, stderr.as_fd())?;
            descriptors.extend([venus.control.as_fd(), stderr.as_fd()]);
        } else {
            bytes.push(0);
        }
        bytes.extend((check.hosts.len() as u16).to_le_bytes());
        for row in &state.sessions {
            let (lease, record, identity) = row.carry_lease();
            Descriptor::write(&mut bytes, lease.as_fd())?;
            descriptors.push(lease.as_fd());
            write_object(&mut bytes, record);
            write_frame(
                &mut bytes,
                &management::encode_record(&Record::Live(identity.clone()))
                    .map_err(io::Error::other)?,
            );
        }
        if bytes.len() > CAP {
            return Err(invalid("update handoff exceeds its exact bound"));
        }
        let path = runtime.join(format!(".update-{}", request_id()));
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        fs::remove_file(path)?;
        file.write_all(&bytes)?;
        file.rewind()?;
        descriptors.push(file.as_fd());
        let mut command = launch.command(facts);
        if update::fault("exec-return") {
            command = Command::new("/does-not-exist/eon-update-resume");
        }
        command
            .arg(RESUME_UPDATE)
            .arg(carry.lifetime)
            .arg(file.as_raw_fd().to_string());
        if runtime_window(root).is_some() || root != base_runtime_directory("eon") {
            command.env(WINDOW_RUNTIME, root);
        }
        let result = platform::exec_inheriting(&command, &descriptors);
        #[cfg(test)]
        if update::fault("exec-return") {
            tests::verify_rollback_inheritance(root, &descriptors)?;
        }
        result
    })()
    .expect_err("successful exec never returns");
    refusal(
        id,
        Code::Internal,
        format!("cannot replace the supervisor: {err}"),
    )
}

pub(crate) fn resume_update(
    inputs: &Inputs,
    arguments: &[OsString],
) -> Result<i32, crate::control::Error> {
    resume(inputs, arguments).map_err(|error| error.to_string().into())
}

fn resume(inputs: &Inputs, arguments: &[OsString]) -> io::Result<i32> {
    let [generation, descriptor] = arguments else {
        return Err(invalid("invalid private resume arguments"));
    };
    let generation = generation
        .to_str()
        .filter(|value| valid_id35(value))
        .ok_or_else(|| invalid("invalid resume lifetime"))?;
    let handoff = descriptor
        .to_str()
        .filter(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|value| value.parse::<RawFd>().ok())
        .filter(|&fd| fd >= 3)
        .ok_or_else(|| invalid("invalid private resume descriptor"))?;
    let root = runtime_directory("eon");
    if !root.is_absolute() {
        return Err(invalid("resume namespace must be absolute"));
    }
    let runtime = generation_directory(&root, generation);
    for directory in [&root, &root.join("generations"), &runtime] {
        validate_private_directory(directory).map_err(invalid)?;
    }
    let envelope = Envelope::read(&read_file(handoff, &runtime)?, handoff)?;
    let Envelope {
        carry,
        check,
        lifecycle,
        namespace,
        control,
        update,
        reply,
        venus,
        leases,
    } = envelope;
    let launch = inputs
        .launch
        .as_ref()
        .map_err(|error| invalid(error.clone()))?;
    let caller_matches = match (&carry.caller, &carry.caller_origin) {
        (None, None) => true,
        (Some(caller), Some(origin)) => check.hosts.iter().any(|host| {
            host.origin.as_ref() == Some(origin)
                && crate::sessions::terminal_identity(&host.terminal, &host.run) == *caller
        }),
        _ => false,
    };
    let previous = Identity::read(&runtime)
        .map_err(invalid)?
        .ok_or_else(|| invalid("resume lifetime has no identity record"))?;
    let build = current_generation(inputs).map_err(invalid)?;
    if generation != carry.lifetime
        || generation != check.lifetime
        || root.as_os_str().as_bytes() != check.namespace
        || carry.from_build != check.active_build
        || carry.target_build != check.target_build
        || check.target_build != build
        || carry.target_wrapper != launch.facts().wrapper().as_os_str().as_bytes()
        || !previous.matches(generation, &check.created_build, &check.active_build)
        || !caller_matches
    {
        return Err(invalid(
            "resume identity, caller or target differs from admitted handoff",
        ));
    }
    preflight::check_host_versions(&check.hosts).map_err(|error| invalid(error.detail))?;
    let config = configuration_directory().map_err(invalid)?;
    let settings = WindowSettings::read(
        config,
        LaunchMode::Workspace,
        inputs,
        &LaunchOptions::default(),
        None,
    )
    .map_err(invalid)?;
    let base = base_runtime_directory("eon");
    let registry =
        crate::session_registry::Registry::new().map_err(|error| invalid(error.to_string()))?;
    let now = Instant::now();
    let boot = platform::boot_time()?;
    let scope = Scope {
        header: Header {
            namespace: root
                .to_str()
                .ok_or_else(|| invalid("invalid checkpoint namespace"))?
                .into(),
            generation: generation.into(),
            created_build: previous.created_build.clone(),
            active_build: previous.active_build.clone(),
        },
        range: check.identity_range,
    };
    let mut target = Target {
        inputs,
        settings: &settings,
        root: &root,
        generation,
        base: &base,
        registry: Some(registry),
        log: None,
    };
    validate_handoff_target(&target, &check.checkpoint, &scope, &check.hosts, now, boot)
        .map_err(io::Error::other)?;
    let control_path = runtime.join("eon.sock");
    let update_path = runtime.join(update::SOCKET);
    control.validate(&control_path)?;
    update.validate(&update_path)?;
    let reply_live = reply.socket(false, |fd| {
        let address = UnixStream::from(fd.try_clone_to_owned()?).local_addr()?;
        if address.as_pathname().map(Path::as_os_str) != Some(update_path.as_os_str()) {
            return Err(invalid("inherited reply does not belong to update.sock"));
        }
        match platform::peer_identity(fd) {
            Ok((_, uid)) if uid == effective_uid() => Ok(!carry.reply_sent),
            Err(error)
                if error.kind() == io::ErrorKind::NotConnected && platform::stream_closed(fd)? =>
            {
                Ok(false)
            }
            _ => Err(invalid(
                "inherited Update reply peer cannot be authenticated",
            )),
        }
    })?;
    if let Some(venus) = &venus {
        venus.validate()?;
    }
    let candidates = leases
        .into_iter()
        .zip(&check.hosts)
        .map(|(lease, host)| {
            let candidate = lease.fd.socket(false, |fd| {
                if !fcntl_getfl(fd)?.contains(OFlags::NONBLOCK) {
                    return Err(invalid("inherited lease is not nonblocking"));
                }
                validate_carried_lease(fd, &runtime, generation, host, lease.record, lease.identity)
                    .map_err(invalid)
            })?;
            Ok((lease.fd, candidate))
        })
        .collect::<io::Result<Vec<_>>>()?;
    platform::require_waitable_children()?;
    let lifecycle_path = supervisor_lock_path(&root, generation);
    let namespace_path = root.join("launch.lock");
    for (fd, path) in [(&lifecycle, &lifecycle_path), (&namespace, &namespace_path)] {
        verify_lock(fd, path)?;
    }

    let lifecycle = LifecycleLock::inherited(lifecycle.take()?.into());
    let namespace =
        NamespaceLock::inherited(&root, LifecycleLock::inherited(namespace.take()?.into()));
    let mut control = ControlListener(control.take(&control_path)?);
    let mut update = UpdateListener(update.take(&update_path)?);
    let reply = Reply::inherited(reply.take()?.into());
    let reply = reply_live.then_some(reply);
    let venus = venus
        .map(|venus| -> io::Result<_> {
            Ok((
                venus.name,
                UnixStream::from(venus.control.take()?),
                File::from(venus.stderr.take()?),
            ))
        })
        .transpose()?;
    let candidates = candidates
        .into_iter()
        .map(|(fd, candidate)| Ok((UnixStream::from(fd.take()?), candidate)))
        .collect::<io::Result<Vec<_>>>()?;
    drop(platform::take_inherited(handoff)?);

    let log = DiagnosticLog::open(generation, "eon");
    target.log = Some(&log);
    let rows = candidates
        .into_iter()
        .zip(check.hosts)
        .map(|((stream, candidate), host)| {
            RunningSession::resume_lease(candidate, stream, host, log.orbit_directory().ok())
        })
        .collect();
    let restore = (|| {
        if update::fault("restore") {
            return Err(invalid("injected restore failure"));
        }
        let state = import(
            &target,
            &check.checkpoint,
            &scope,
            rows,
            &control,
            now,
            boot,
        )
        .map_err(|(error, _)| io::Error::other(error))?;
        let venus = venus
            .map(|(name, control, stderr)| {
                Ok::<_, io::Error>(InheritedPresentation {
                    name,
                    control,
                    _relay: log.relay(stderr)?,
                })
            })
            .transpose()?;
        #[cfg(test)]
        tests::verify_child_inheritance(&root, venus.as_ref())?;
        let next = Identity {
            active_build: build.clone(),
            active_launcher: crate::generation::installed_launcher(inputs),
            ..previous.clone()
        };
        if let Err(error) = namespace.publish_active(&previous, &next, &lifecycle) {
            if Identity::read(&runtime).ok().flatten().as_ref() != Some(&next) {
                return Err(invalid(error));
            }
            log.note(&error);
        }
        if let Err(error) = next.project_launcher(&runtime) {
            log.note(&error);
        }
        Ok((state, venus))
    })();
    drop(namespace);
    let (state, venus) = match restore {
        Ok(restored) => restored,
        Err(error) => {
            if let Some(reply) = reply {
                reply.commit(Err(refusal(
                    carry.request,
                    Code::Internal,
                    format!("cannot restore the supervisor: {error}"),
                )));
            }
            return Err(error);
        }
    };
    let _cleanup = GenerationCleanup {
        runtime: &runtime,
        log: &log,
    };
    control.0.enable_cleanup();
    update.0.enable_cleanup();
    let last = if venus.is_some() {
        // S5a (eon-t3fyd.11) must send UPDATED(ready) after target admission, before enabling Commit.
        drop(reply);
        Outcome {
            request: carry.request,
            operation: Operation::Commit,
            refusal: None,
            detail: "supervisor adopted the target; target presentation admission is pending"
                .into(),
        }
    } else {
        let result = Ok(Updated {
            request: carry.request,
            lifetime: carry.lifetime,
            changed: carry.from_build != build,
            from_build: carry.from_build,
            to_build: build,
            presentation: UpdatedPresentation::Detached,
        });
        let last = commit_outcome(carry.request, &result);
        if let Some(reply) = reply {
            reply.commit(result);
        }
        last
    };
    service(
        inputs,
        &settings.config,
        &lifecycle,
        &runtime,
        generation,
        LaunchMode::Workspace,
        &log,
        state,
        control,
        Some(update),
        Some(last),
        venus,
    )
    .map_err(invalid)
}

pub(super) fn commit_outcome(
    request: eon_workspace_protocol::update::RequestId,
    result: &Result<Updated, Refused>,
) -> Outcome {
    let (code, detail) = match result {
        Ok(_) => (None, "supervisor adopted the target".into()),
        Err(refused) => (Some(refused.code), refused.detail.clone()),
    };
    Outcome {
        request,
        operation: Operation::Commit,
        refusal: code,
        detail,
    }
}

#[cfg(test)]
mod tests;
