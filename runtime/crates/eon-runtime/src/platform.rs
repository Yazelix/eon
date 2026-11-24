//! Runtime's sole unsafe platform seam; policy remains with its callers.

use std::{
    io,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd},
        unix::{ffi::OsStrExt, net::UnixStream},
    },
    path::{Path, PathBuf},
    time::Duration,
};

/// A private child's PID must stay reserved until its owner waits for it.
pub(crate) fn require_waitable_children() -> io::Result<()> {
    let mut action = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: a null action queries the disposition and initializes the output on success.
    let action = unsafe {
        if libc::sigaction(libc::SIGCHLD, std::ptr::null(), action.as_mut_ptr()) != 0 {
            return Err(io::Error::last_os_error());
        }
        action.assume_init()
    };
    if action.sa_sigaction == libc::SIG_IGN || action.sa_flags & libc::SA_NOCLDWAIT != 0 {
        return Err(io::Error::other(
            "SIGCHLD disposition automatically reaps children",
        ));
    }
    Ok(())
}

/// System-wide monotonic time, including suspend, shared by replacement processes.
pub(crate) fn boot_time() -> io::Result<Duration> {
    #[cfg(target_os = "linux")]
    let time = rustix::time::clock_gettime_dynamic(rustix::time::DynamicClockId::Boottime)?;
    #[cfg(target_os = "macos")]
    let time = {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: rustix lacks this Darwin raw clock; retain its clock domain across
        // replacement processes. clock_gettime writes only this live timespec.
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &raw mut time) } != 0 {
            return Err(io::Error::last_os_error());
        }
        time
    };
    let seconds = u64::try_from(time.tv_sec)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "negative boot time"))?;
    let nanos = u32::try_from(time.tv_nsec)
        .ok()
        .filter(|nanos| *nanos < 1_000_000_000)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid boot nanoseconds"))?;
    Ok(Duration::new(seconds, nanos))
}

/// The longest Unix socket path this platform binds: `sun_path` less its NUL.
pub(crate) const SOCKET_PATH_BYTES: usize = std::mem::size_of::<libc::sockaddr_un>()
    - std::mem::offset_of!(libc::sockaddr_un, sun_path)
    - 1;

/// Whether a Unix socket can bind at `path`.
pub(crate) fn socket_path_fits(path: &Path) -> bool {
    path.as_os_str().len() <= SOCKET_PATH_BYTES
}

/// `product`'s runtime root without `XDG_RUNTIME_DIR`, which a Linux session
/// should set, so Eon warns and uses a private temporary directory.
#[cfg(target_os = "linux")]
pub(crate) fn fallback_runtime_directory(product: &str, uid: u32) -> PathBuf {
    eprintln!(
        "{product}: warning: XDG_RUNTIME_DIR is unset; using a private temporary runtime root"
    );
    std::env::temp_dir().join(format!("{product}-{uid}"))
}

/// `product`'s runtime root without `XDG_RUNTIME_DIR`, which macOS lacks. Its
/// per-user `$TMPDIR` is too long for a window's terminal sockets, so Eon uses
/// a private directory in `/tmp`, as tmux does.
#[cfg(target_os = "macos")]
pub(crate) fn fallback_runtime_directory(product: &str, uid: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/{product}-{uid}"))
}

/// Reads what `stream` holds now, without waiting and without changing the
/// blocking mode its other users share: `WouldBlock` when it holds nothing,
/// and 0 at its end.
pub(crate) fn read_available(stream: &UnixStream, buffer: &mut [u8]) -> io::Result<usize> {
    rustix::net::recv(stream, buffer, rustix::net::RecvFlags::DONTWAIT)
        .map(|(_, read)| read)
        .map_err(Into::into)
}

/// Inspect a descriptor inherited by the single-threaded resume entry.
pub(crate) fn inspect_inherited<T>(
    number: RawFd,
    inspect: impl FnOnce(BorrowedFd<'_>) -> io::Result<T>,
) -> io::Result<T> {
    if number < 3 {
        return Err(io::Error::other("invalid inherited descriptor number"));
    }
    // SAFETY: F_GETFD takes only the integer. A successful query proves the
    // fd is open; the resume entry starts no thread or other owner during inspection.
    let (flags, fd) = unsafe {
        let flags = libc::fcntl(number, libc::F_GETFD);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        (flags, BorrowedFd::borrow_raw(number))
    };
    if flags & libc::FD_CLOEXEC != 0 {
        return Err(io::Error::other("descriptor was not carried through exec"));
    }
    inspect(fd)
}

/// Take one fully validated and uniquely listed descriptor. Marking it
/// close-on-exec also prevents a second call from taking the same number.
pub(crate) fn take_inherited(number: RawFd) -> io::Result<OwnedFd> {
    use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};
    inspect_inherited(number, |fd| {
        fcntl_setfd(fd, fcntl_getfd(fd)? | FdFlags::CLOEXEC)?;
        Ok(())
    })?;
    // SAFETY: the fd was open with CLOEXEC clear at the single-threaded entry,
    // validation has finished, and the claim above excludes a second owner.
    Ok(unsafe { OwnedFd::from_raw_fd(number) })
}

// An incomplete inventory is a refusal. The bound also limits allocation when
// the descriptor table is unusually large.
const MAX_LISTED_FDS: usize = 1 << 20;

#[cfg(target_os = "linux")]
fn open_descriptors() -> io::Result<Vec<RawFd>> {
    let mut numbers = Vec::new();
    for entry in std::fs::read_dir("/proc/self/fd")? {
        if numbers.len() == MAX_LISTED_FDS {
            return Err(io::Error::other("descriptor inventory exceeds its bound"));
        }
        let name = entry?.file_name();
        let number = name
            .to_str()
            .and_then(|name| name.parse().ok())
            .filter(|&number| number >= 0)
            .ok_or_else(|| io::Error::other("invalid descriptor inventory entry"))?;
        numbers.push(number);
    }
    Ok(numbers)
}

#[cfg(target_os = "macos")]
fn open_descriptors() -> io::Result<Vec<RawFd>> {
    use libc::{PROC_PIDLISTFDS, proc_fdinfo, proc_pidinfo};
    let size = std::mem::size_of::<proc_fdinfo>();
    let mut capacity = 256;
    loop {
        let mut entries = Vec::<proc_fdinfo>::with_capacity(capacity);
        let bytes = capacity * size;
        // SAFETY: the buffer has space for bytes and the returned byte count
        // names the fully initialized proc_fdinfo records.
        let read = unsafe {
            proc_pidinfo(
                std::process::id() as libc::pid_t,
                PROC_PIDLISTFDS,
                0,
                entries.as_mut_ptr().cast(),
                bytes as libc::c_int,
            )
        };
        if read <= 0 {
            return Err(io::Error::last_os_error());
        }
        let read = read as usize;
        if read > bytes || !read.is_multiple_of(size) {
            return Err(io::Error::other("invalid descriptor inventory size"));
        }
        if read < bytes {
            // SAFETY: proc_pidinfo initialized read / size complete entries.
            unsafe { entries.set_len(read / size) };
            return Ok(entries.into_iter().map(|entry| entry.proc_fd).collect());
        }
        if capacity == MAX_LISTED_FDS {
            return Err(io::Error::other("descriptor inventory exceeds its bound"));
        }
        capacity *= 2;
    }
}

/// Call only once every producer of potentially inheritable fds is settled.
/// F_GETFD uses the number without borrowing an fd another thread may close.
fn inheritable_descriptors() -> io::Result<impl Iterator<Item = io::Result<RawFd>>> {
    Ok(open_descriptors()?.into_iter().filter_map(|number| {
        if number < 3 {
            return None;
        }
        // SAFETY: F_GETFD only takes the integer fd number.
        let flags = unsafe { libc::fcntl(number, libc::F_GETFD) };
        if flags < 0 {
            let error = io::Error::last_os_error();
            // Includes the already closed /proc/self/fd directory handle.
            return (error.raw_os_error() != Some(libc::EBADF)).then_some(Err(error));
        }
        (flags & libc::FD_CLOEXEC == 0).then_some(Ok(number))
    }))
}

#[cfg(test)]
pub(crate) fn inherited_numbers() -> io::Result<Vec<RawFd>> {
    inheritable_descriptors()?.collect()
}

/// Replace this process using only the command's explicit environment.
/// A returned error keeps the current environment, signals and stdio intact.
pub(crate) fn exec_inheriting(
    command: &std::process::Command,
    descriptors: &[BorrowedFd<'_>],
) -> io::Result<()> {
    use rustix::io::{FdFlags, fcntl_getfd, fcntl_setfd};
    use std::ffi::{CString, OsStr};

    let program = command.get_program();
    if !Path::new(program).is_absolute() {
        return Err(io::Error::other("update executable is not absolute"));
    }
    let cstring = |value: &OsStr| CString::new(value.as_bytes()).map_err(io::Error::other);
    let path = cstring(program)?;
    let arguments = std::iter::once(program)
        .chain(command.get_args())
        .map(cstring)
        .collect::<io::Result<Vec<_>>>()?;
    let environment = command
        .get_envs()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .map(|(key, value)| {
            let mut entry = key.to_os_string();
            entry.push("=");
            entry.push(value);
            cstring(&entry)
        })
        .collect::<io::Result<Vec<_>>>()?;
    let pointers = |strings: &[CString]| {
        strings
            .iter()
            .map(|value| value.as_ptr())
            .chain([std::ptr::null()])
            .collect::<Vec<_>>()
    };
    let argv = pointers(&arguments);
    let envp = pointers(&environment);
    let flags = descriptors
        .iter()
        .map(fcntl_getfd)
        .collect::<rustix::io::Result<Vec<_>>>()?;
    if flags.iter().any(|flag| !flag.contains(FdFlags::CLOEXEC)) {
        return Err(io::Error::other("update resource lacks close-on-exec"));
    }
    // The allowlist still has CLOEXEC here. Only stdio may already lack it.
    if let Some(number) = inheritable_descriptors()?.next() {
        return Err(io::Error::other(format!(
            "unexpected inheritable descriptor {}",
            number?
        )));
    }
    let mut changed = 0;
    let result = (|| {
        for (fd, flags) in descriptors.iter().zip(&flags) {
            fcntl_setfd(fd, *flags - FdFlags::CLOEXEC)?;
            changed += 1;
        }
        // SAFETY: path, argv and envp point to live NUL-terminated strings
        // and both pointer arrays end with null.
        unsafe { libc::execve(path.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
        Err(io::Error::last_os_error())
    })();
    for (fd, flags) in descriptors.iter().zip(flags).take(changed) {
        // These are still borrowed from live owners. A restoration failure
        // must not let this process spawn with leaking descriptors.
        if fcntl_setfd(fd, flags).is_err() {
            std::process::exit(1);
        }
    }
    result
}

#[cfg(target_os = "linux")]
pub(crate) fn socket_listening(fd: impl AsFd) -> io::Result<bool> {
    rustix::net::sockopt::socket_acceptconn(fd).map_err(Into::into)
}

#[cfg(target_os = "macos")]
pub(crate) fn socket_listening(fd: impl AsFd) -> io::Result<bool> {
    // XNU sys/proc_info.h: socket_fdinfo starts with proc_fileinfo (24 bytes)
    // and vinfo_stat (136), then two u64 handles and three ints before soi_options.
    const OPTIONS: usize = 24 + 136 + 2 * 8 + 3 * 4;
    const PROC_PIDFDSOCKETINFO: libc::c_int = 3;
    let mut info = [0u8; 1024];
    // SAFETY: the descriptor belongs to this process, and the buffer is writable
    // for the full advertised length. libproc copies the fixed result into it.
    let length = unsafe {
        libc::proc_pidfdinfo(
            std::process::id() as libc::pid_t,
            fd.as_fd().as_raw_fd(),
            PROC_PIDFDSOCKETINFO,
            info.as_mut_ptr().cast(),
            info.len() as libc::c_int,
        )
    };
    if length <= 0 {
        return Err(io::Error::last_os_error());
    }
    if (length as usize) < OPTIONS + 2 {
        return Err(io::Error::other("incomplete Darwin socket information"));
    }
    let options = u16::from_ne_bytes([info[OPTIONS], info[OPTIONS + 1]]);
    Ok(options & libc::SO_ACCEPTCONN as u16 != 0)
}

/// Inspect queued bytes without consuming an inherited stream.
pub(crate) fn stream_closed(fd: impl AsFd) -> io::Result<bool> {
    use rustix::{
        event::{PollFd, PollFlags, Timespec, poll},
        io::Errno,
        net::{RecvFlags, recv},
    };
    // HUP is useful even when a final frame still precedes EOF.
    let mut readiness = [PollFd::new(&fd, PollFlags::IN)];
    loop {
        match poll(&mut readiness, Some(&Timespec::default())) {
            Err(Errno::INTR) => continue,
            result => result?,
        };
        break;
    }
    if readiness[0].revents().contains(PollFlags::HUP) {
        return Ok(true);
    }
    loop {
        match recv(
            &fd,
            &mut [0u8; 1][..],
            RecvFlags::PEEK | RecvFlags::DONTWAIT,
        ) {
            Ok((_, count)) => return Ok(count == 0),
            Err(Errno::INTR) => continue,
            Err(Errno::CONNRESET | Errno::NOTCONN) => return Ok(true),
            Err(Errno::AGAIN) => return Ok(false),
            Err(error) => return Err(error.into()),
        }
    }
}

/// What the kernel reports about a process that still has its pid.
pub(crate) struct Process {
    /// The start identity, in the unit Orbit publishes as `process_start`.
    pub(crate) start: u64,
    pub(crate) parent: u32,
    /// Whether the process has exited and awaits reaping.
    pub(crate) ended: bool,
}

/// Kernel evidence for advisory helper recognition, never a signalling target.
#[cfg(target_os = "linux")]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct HelperProcess {
    pub identity: orbit_protocol::management::ProcessIdentity,
    pub parent: u32,
    pub group: u32,
    pub executable: PathBuf,
    pub executable_object: (u64, u64),
    pub arguments: Vec<Vec<u8>>,
}

#[cfg(target_os = "linux")]
pub(crate) fn helper_process(pid: u32, read_arguments: bool) -> Option<HelperProcess> {
    use std::{fs, io::Read, os::unix::fs::MetadataExt};
    let directory = PathBuf::from(format!("/proc/{pid}"));
    let bounded = |name: &str| -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        fs::File::open(directory.join(name))
            .ok()?
            .take(4097)
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() <= 4096).then_some(bytes)
    };
    let stat = || -> Option<(u64, u32, u32)> {
        let bytes = bounded("stat")?;
        let end = bytes.iter().rposition(|&byte| byte == b')')?;
        let fields: Vec<_> = std::str::from_utf8(&bytes[end + 1..])
            .ok()?
            .split_whitespace()
            .collect();
        if matches!(*fields.first()?, "T" | "t" | "Z" | "X") {
            return None;
        }
        Some((
            fields.get(19)?.parse().ok()?,
            fields.get(1)?.parse().ok()?,
            fields.get(2)?.parse().ok()?,
        ))
    };
    let before = stat()?;
    let executable = fs::read_link(directory.join("exe")).ok()?;
    let object = |path: &Path| -> Option<(u64, u64)> {
        let metadata = fs::metadata(path).ok()?;
        metadata
            .is_file()
            .then_some((metadata.dev(), metadata.ino()))
    };
    let identity = object(&directory.join("exe"))?;
    // Deleted or replaced installations do not supply a positive role identity.
    if !executable.is_absolute() || object(&executable)? != identity {
        return None;
    }
    let arguments = if read_arguments {
        let arguments = bounded("cmdline")?;
        if arguments.last() != Some(&0) {
            return None;
        }
        arguments[..arguments.len() - 1]
            .split(|&byte| byte == 0)
            .map(<[u8]>::to_vec)
            .collect()
    } else {
        Vec::new()
    };
    if stat()? != before
        || fs::read_link(directory.join("exe")).ok()? != executable
        || object(&directory.join("exe"))? != identity
        || object(&executable)? != identity
    {
        return None;
    }
    Some(HelperProcess {
        identity: orbit_protocol::management::ProcessIdentity {
            pid,
            start: before.0,
        },
        parent: before.1,
        group: before.2,
        executable,
        executable_object: identity,
        arguments,
    })
}

/// The pid and uid of the stream's peer.
#[cfg(target_os = "linux")]
pub(crate) fn peer_identity(stream: impl AsFd) -> io::Result<(libc::pid_t, libc::uid_t)> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials and length are valid writable storage. libc preserves legal PID 0,
    // which rustix 1.1.4 cannot represent (eon-ojwmi).
    if unsafe {
        libc::getsockopt(
            stream.as_fd().as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &raw mut length,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid peer credentials length",
        ));
    }
    Ok((credentials.pid, credentials.uid))
}

/// The pid and uid of the stream's peer.
#[cfg(target_os = "macos")]
pub(crate) fn peer_identity(stream: impl AsFd) -> io::Result<(libc::pid_t, libc::uid_t)> {
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: no supported safe API reads Darwin LOCAL_PEERPID; pid points to
    // writable pid_t storage of the declared length.
    if unsafe {
        libc::getsockopt(
            stream.as_fd().as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&raw mut pid).cast(),
            &raw mut length,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of::<libc::pid_t>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid peer process ID length",
        ));
    }
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: std peer_cred is unstable and rustix has no getpeereid wrapper;
    // the live stream and writable UID/GID storage remain valid for this call.
    if unsafe { libc::getpeereid(stream.as_fd().as_raw_fd(), &raw mut uid, &raw mut gid) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok((pid, uid))
}

/// The process with `pid`, or `None` when no process has it.
#[cfg(target_os = "linux")]
pub(crate) fn process(pid: u32) -> Result<Option<Process>, String> {
    let stat = match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot validate terminal process identity: {error}"
            ));
        }
    };
    let mut fields = stat
        .rsplit_once(')')
        .map(|(_, fields)| fields.split_whitespace())
        .ok_or("invalid terminal process identity")?;
    let state = fields.next().ok_or("invalid terminal process identity")?;
    let parent = fields
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or("invalid terminal process parent")?;
    let start = fields
        .nth(17)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or("invalid terminal process identity")?;
    Ok(Some(Process {
        start,
        parent,
        ended: matches!(state, "Z" | "X"),
    }))
}

/// The process with `pid`, or `None` when no process has it. Its start is in
/// microseconds, as Orbit reads it from libproc.
#[cfg(target_os = "macos")]
pub(crate) fn process(pid: u32) -> Result<Option<Process>, String> {
    let Ok(pid_value) = libc::pid_t::try_from(pid) else {
        return Ok(None);
    };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    // A nonzero argument makes libproc report zombies too, so their start
    // identity is still checked.
    // SAFETY: no std/rustix libproc API exists; zero is valid for this integer
    // record, and proc_pidinfo writes at most size bytes into its live storage.
    let (read, info) = unsafe {
        let mut info = std::mem::zeroed::<libc::proc_bsdinfo>();
        let read = libc::proc_pidinfo(
            pid_value,
            libc::PROC_PIDTBSDINFO,
            1,
            (&raw mut info).cast(),
            size,
        );
        (read, info)
    };
    if read <= 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(None);
        }
        return Err(format!(
            "cannot validate terminal process identity: {error}"
        ));
    }
    if read != size || info.pbi_pid != pid || info.pbi_start_tvusec >= 1_000_000 {
        return Err("invalid terminal process identity".into());
    }
    let start = info
        .pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .ok_or("invalid terminal process identity")?;
    Ok(Some(Process {
        start,
        parent: info.pbi_ppid,
        ended: info.pbi_status == libc::SZOMB,
    }))
}

/// Runs the prepared update command while retaining terminal interrupts in its runner.
pub(crate) fn update_status(
    command: &mut std::process::Command,
) -> Result<std::process::ExitStatus, String> {
    // A caught no-op resets on exec, letting the product handle terminal
    // interrupts while the runner survives to retain its result.
    extern "C" fn interrupt(_: libc::c_int) {}
    let handler = interrupt as *const () as libc::sighandler_t;
    // SAFETY: no supported safe signal API; this fixed C-ABI no-op does nothing.
    let previous_int = unsafe { libc::signal(libc::SIGINT, handler) };
    if previous_int == libc::SIG_ERR {
        return Err(format!(
            "cannot retain update results: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: no supported safe signal API; the same fixed no-op is valid for SIGQUIT.
    let previous_quit = unsafe { libc::signal(libc::SIGQUIT, handler) };
    let status = if previous_quit == libc::SIG_ERR {
        Err(std::io::Error::last_os_error())
    } else {
        // Use the popup's PATH and inherit its terminal for product-owned consent.
        let status = command.status();
        // SAFETY: no supported safe restoration API; signal returned this valid handler.
        unsafe { libc::signal(libc::SIGQUIT, previous_quit) };
        status
    };
    // SAFETY: no supported safe restoration API; signal returned this valid handler.
    unsafe { libc::signal(libc::SIGINT, previous_int) };
    status.map_err(|error| format!("cannot run eon update: {error}"))
}

pub(crate) struct EventsInterrupt(libc::sighandler_t);

pub(crate) fn events_interrupt() -> io::Result<EventsInterrupt> {
    extern "C" fn interrupt(_: libc::c_int) {
        // SAFETY: no supported safe immediate-exit API; _exit is async-signal-safe,
        // exits this process without destructors, and never returns.
        unsafe { libc::_exit(130) }
    }
    // SAFETY: no supported safe signal API; the fixed C-ABI handler only calls _exit.
    let previous =
        unsafe { libc::signal(libc::SIGINT, interrupt as *const () as libc::sighandler_t) };
    if previous == libc::SIG_ERR {
        return Err(io::Error::last_os_error());
    }
    Ok(EventsInterrupt(previous))
}

impl EventsInterrupt {
    pub(crate) fn restore(self) {
        // SAFETY: no supported safe restoration API; the opaque token contains
        // only a valid previous handler returned by successful signal installation.
        unsafe { libc::signal(libc::SIGINT, self.0) };
    }
}

pub(crate) fn detach_session(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: stable std has no safe setsid registration API; this fixed hook
    // only invokes async-signal-safe setsid and converts its errno without allocation.
    unsafe {
        command.pre_exec(|| rustix::process::setsid().map(|_| ()).map_err(Into::into));
    }
}

/// One local calendar conversion shared by Runtime's two fixed numeric layouts.
pub(crate) struct LocalTime(libc::tm);

impl LocalTime {
    pub(crate) fn from_seconds(seconds: libc::time_t) -> Option<Self> {
        // SAFETY: std/rustix lack this local calendar conversion and time's range
        // and leap-second representation differ. localtime_r writes a live tm;
        // only successful initialization is read, and tm_zone is never dereferenced.
        let time = unsafe {
            let mut time = std::mem::MaybeUninit::uninit();
            if libc::localtime_r(&seconds, time.as_mut_ptr()).is_null() {
                return None;
            }
            time.assume_init()
        };
        Some(Self(time))
    }

    pub(crate) fn minute(&self) -> String {
        format!(
            "{}-{:02}-{:02} {:02}:{:02}",
            i64::from(self.0.tm_year) + 1900,
            self.0.tm_mon + 1,
            self.0.tm_mday,
            self.0.tm_hour,
            self.0.tm_min
        )
    }

    pub(crate) fn timestamp(&self, milliseconds: u32) -> String {
        let offset = if self.0.tm_isdst < 0 {
            String::new()
        } else {
            let seconds = self.0.tm_gmtoff.unsigned_abs();
            format!(
                "{}{:02}{:02}",
                if self.0.tm_gmtoff < 0 { '-' } else { '+' },
                seconds / 3600,
                seconds / 60 % 60
            )
        };
        format!(
            "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{milliseconds:03}{offset}",
            i64::from(self.0.tm_year) + 1900,
            self.0.tm_mon + 1,
            self.0.tm_mday,
            self.0.tm_hour,
            self.0.tm_min,
            self.0.tm_sec
        )
    }
}

/// The bytes `stream` holds that its reader has not read yet, from Linux's
/// `TIOCOUTQ` (`SIOCOUTQ`).
#[cfg(target_os = "linux")]
pub(crate) fn socket_unread(stream: &UnixStream) -> Option<usize> {
    let mut unread: libc::c_int = 0;
    // SAFETY: no typed safe std/rustix/socket2 API queries TIOCOUTQ; the live
    // stream owns this descriptor, and ioctl writes only into this c_int.
    let asked = unsafe { libc::ioctl(stream.as_raw_fd(), libc::TIOCOUTQ, &mut unread) };
    (asked == 0).then_some(unread as usize)
}

/// Elsewhere the connection turning writable is the measure of progress.
/// By XNU's source, a macOS Unix stream holds no unsent bytes of its own
/// (`SO_NWRITE` stays 0); its reader's reads give back send space, and it
/// polls writable at the 2 KiB low-water mark, so `reads` sees a reader
/// that drains it. eon-ciwn.8's Mac run proves this.
#[cfg(not(target_os = "linux"))]
pub(crate) fn socket_unread(_stream: &UnixStream) -> Option<usize> {
    None
}

#[cfg(target_os = "linux")]
pub(crate) fn account_home_bytes() -> io::Result<Vec<u8>> {
    use std::ffi::CStr;
    let uid = rustix::process::geteuid().as_raw();
    let mut bytes = vec![0u8; 1024];
    loop {
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        // SAFETY: no std/rustix NSS lookup API; the output pointers and buffer are valid. Strings
        // are copied before the buffer drops; a null result is never dereferenced.
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
                &mut result,
            )
        };
        if status == libc::ERANGE {
            // Bound malformed NSS records as well as normal buffer growth.
            if bytes.len() >= 1024 * 1024 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "OS account record exceeds 1 MiB",
                ));
            }
            bytes.resize(bytes.len() * 2, 0);
            continue;
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status));
        }
        if result.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("OS account for effective UID {uid} not found"),
            ));
        }
        // SAFETY: the NSS API has no safe equivalent; successful getpwuid_r initialized
        // entry and its buffer-backed strings, which remain live here.
        let entry = unsafe { entry.assume_init() };
        if entry.pw_dir.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "OS account has no home",
            ));
        }
        // SAFETY: no safe NSS API supplies these bytes; pw_dir is non-null and
        // NUL-terminated in the live buffer, and the bytes are copied before it drops.
        return Ok(unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes().to_vec());
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn account_home_bytes() -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "EON-C43 canonical account resolution is supported only on Linux",
    ))
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn test_children(pid: i32) -> Vec<i32> {
    let mut children = [0; 64];
    // SAFETY: no std/rustix API lists Darwin child PIDs; libproc writes only into
    // this live PID buffer, bounded by its byte size.
    let count = unsafe {
        libc::proc_listchildpids(
            pid,
            children.as_mut_ptr().cast(),
            std::mem::size_of_val(&children) as libc::c_int,
        )
    };
    children[..usize::try_from(count).unwrap()].to_vec()
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn test_program(pid: i32) -> Vec<u8> {
    let mut path = vec![0; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    // SAFETY: no std/rustix API reads another Darwin process's executable path;
    // libproc writes at most the length of this live buffer.
    let length = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
    path.truncate(usize::try_from(length).unwrap_or(0));
    path
}

#[cfg(test)]
mod tests {
    #[test]
    fn timestamp_preserves_leap_seconds_and_unknown_offset() {
        let mut time = super::LocalTime::from_seconds(0).unwrap();
        time.0.tm_sec = 60;
        time.0.tm_isdst = -1;
        assert!(time.timestamp(7).ends_with(":60.007"));
    }

    fn child(mode: &str) -> std::process::Command {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "platform::tests::platform_child", "--nocapture"])
            .env("EON_TEST_PLATFORM_MODE", mode);
        command
    }

    #[cfg(target_os = "linux")]
    fn signal_masks(pid: u32) -> (u64, u64) {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
        let mask = (1 << (libc::SIGINT - 1)) | (1 << (libc::SIGQUIT - 1));
        let read = |key: &str| {
            u64::from_str_radix(
                status
                    .lines()
                    .find_map(|line| line.strip_prefix(key))
                    .unwrap()
                    .trim(),
                16,
            )
            .unwrap()
                & mask
        };
        (read("SigCgt:"), read("SigIgn:"))
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn update_signals_are_restored_and_reset_on_exec() {
        let output = child("signals").output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[test]
    fn detached_process_is_its_session_and_group_leader() {
        let mut command = child("session");
        super::detach_session(&mut command);
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[test]
    fn platform_child() {
        let Ok(mode) = std::env::var("EON_TEST_PLATFORM_MODE") else {
            return;
        };
        match mode.as_str() {
            "session" => {
                let pid = rustix::process::getpid();
                assert_eq!(rustix::process::getsid(None).unwrap(), pid);
                assert_eq!(rustix::process::getpgrp(), pid);
            }
            #[cfg(target_os = "linux")]
            "signals" => {
                let before = signal_masks(std::process::id());
                assert!(super::update_status(&mut child("exec")).unwrap().success());
                assert_eq!(signal_masks(std::process::id()), before);
                let root = crate::supervisor::temporary_directory();
                let error =
                    super::update_status(&mut std::process::Command::new(root.join("missing")))
                        .unwrap_err();
                assert!(error.starts_with("cannot run eon update:"), "{error}");
                assert_eq!(signal_masks(std::process::id()), before);
                std::fs::remove_dir_all(root).unwrap();
            }
            #[cfg(target_os = "linux")]
            "exec" => {
                let mask = (1 << (libc::SIGINT - 1)) | (1 << (libc::SIGQUIT - 1));
                let parent = rustix::process::getppid().unwrap().as_raw_pid() as u32;
                assert_eq!(signal_masks(parent), (mask, 0));
                assert_eq!(signal_masks(std::process::id()), (0, 0));
            }
            _ => panic!("unknown platform child mode: {mode}"),
        }
    }

    use super::{SOCKET_PATH_BYTES, read_available, socket_path_fits};
    use crate::supervisor::temporary_directory;
    use std::{
        fs,
        io::{self, Write},
        net::Shutdown,
        os::unix::net::{UnixListener, UnixStream},
    };

    #[test]
    fn read_available_does_not_change_socket_mode() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let flags = || rustix::fs::fcntl_getfl(&stream).unwrap();
        let blocking = flags();
        let mut buffer = [0; 8];
        assert_eq!(
            read_available(&stream, &mut buffer).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        peer.write_all(b"abc").unwrap();
        assert_eq!(read_available(&stream, &mut buffer).unwrap(), 3);
        assert_eq!(&buffer[..3], b"abc");
        peer.shutdown(Shutdown::Write).unwrap();
        assert_eq!(read_available(&stream, &mut buffer).unwrap(), 0);
        assert_eq!(flags(), blocking);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn helper_observation_changes_when_the_same_pid_execs_a_replacement() {
        use std::{
            os::unix::ffi::OsStrExt,
            process::{Command, Stdio},
            thread,
            time::{Duration, Instant},
        };

        let root = temporary_directory();
        let output = Command::new("bash")
            .args(["-c", "printf %s \"$BASH\""])
            .output()
            .unwrap();
        let bash = std::path::PathBuf::from(String::from_utf8(output.stdout).unwrap());
        let executable = root.join("helper");
        let replacement = root.join("replacement");
        fs::copy(&bash, &executable).unwrap();
        fs::copy(&bash, &replacement).unwrap();
        let mut child = Command::new(&executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        // Spawn may still expose the parent image or an empty cmdline before exec.
        let observe = || {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if let Some(process) = super::helper_process(pid, true).filter(|process| {
                    process.executable == executable
                        && process.arguments.len() == 1
                        && process.arguments[0] == executable.as_os_str().as_bytes()
                }) {
                    break Some(process);
                }
                if Instant::now() >= deadline {
                    break None;
                }
                thread::sleep(Duration::from_millis(1));
            }
        };
        let observations = (|| -> io::Result<_> {
            let before = observe().ok_or_else(|| io::Error::other("initial helper evidence"))?;
            fs::rename(&replacement, &executable)?;
            writeln!(
                child
                    .stdin
                    .as_mut()
                    .ok_or_else(|| io::Error::other("missing helper input"))?,
                "exec \"{}\"",
                executable.display()
            )?;
            let after = observe().ok_or_else(|| io::Error::other("replacement helper evidence"))?;
            Ok((before, after))
        })();
        let killed = child.kill();
        let reaped = child.wait();
        let removed = fs::remove_dir_all(root);
        killed.unwrap();
        reaped.unwrap();
        removed.unwrap();
        let (before, after) = observations.unwrap();
        assert_eq!(before.identity, after.identity);
        assert_eq!(before.executable, after.executable);
        assert!(before.arguments == after.arguments);
        assert!(before != after, "replacement kept the old observation");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn peer_identity_preserves_a_zero_pid() {
        // An unconnected socket exposes the same zero PID as a namespace-hidden peer.
        let stream = UnixStream::from(
            rustix::net::socket_with(
                rustix::net::AddressFamily::UNIX,
                rustix::net::SocketType::STREAM,
                rustix::net::SocketFlags::CLOEXEC,
                None,
            )
            .unwrap(),
        );
        assert_eq!(super::peer_identity(&stream).unwrap(), (0, u32::MAX));
    }

    #[test]
    fn socket_path_bound_is_the_longest_path_that_binds() {
        let root = temporary_directory();
        let path = |length: usize| root.join("s".repeat(length - root.as_os_str().len() - 1));
        let (longest, too_long) = (path(SOCKET_PATH_BYTES), path(SOCKET_PATH_BYTES + 1));
        assert!(socket_path_fits(&longest) && !socket_path_fits(&too_long));
        drop(UnixListener::bind(&longest).unwrap());
        assert!(UnixListener::bind(&too_long).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
