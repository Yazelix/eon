//! The host's one platform seam: every `libc` call, the descriptor flags,
//! process groups, signals, polling and socket options they take, and every
//! difference between Linux and macOS. Elsewhere `libc` supplies only errno
//! names for the errors the host builds and classifies.

use crate::errno;
use std::{
    ffi::c_int,
    fs::File,
    io,
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::net::UnixStream,
    },
    ptr,
};

pub(crate) use libc::{O_CLOEXEC, O_NOFOLLOW, O_NONBLOCK, POLLERR, POLLHUP, POLLIN, POLLOUT};
pub(crate) use libc::{SIGHUP, SIGKILL, pid_t};

const TIOCSCTTY_REQUEST: libc::c_ulong = libc::TIOCSCTTY as libc::c_ulong;

const TIOCSWINSZ_REQUEST: libc::c_ulong = libc::TIOCSWINSZ as libc::c_ulong;

/// This process's effective user ID.
pub(crate) fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

/// A pipe whose ends close on exec, nonblocking if asked.
pub(crate) fn pipe(nonblocking: bool) -> io::Result<(File, File)> {
    let mut fds = [0; 2];
    #[cfg(target_os = "linux")]
    {
        let flags = libc::O_CLOEXEC | if nonblocking { libc::O_NONBLOCK } else { 0 };
        // SAFETY: pipe2 initializes both descriptors on success.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), flags) } == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        // SAFETY: pipe initializes both descriptors on success. The host is
        // single-threaded, so no fork can see them before they close on exec.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        for fd in fds {
            // SAFETY: fcntl only changes the flags of the owned descriptors.
            let status = if nonblocking { libc::O_NONBLOCK } else { 0 };
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1
                || unsafe { libc::fcntl(fd, libc::F_SETFL, status) } == -1
            {
                return Err(io::Error::last_os_error());
            }
        }
    }
    // SAFETY: The pipe transferred ownership of both descriptors.
    Ok(unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) })
}

/// Writes one byte to `fd`; async-signal-safe, for a signal handler's wake.
pub(crate) fn wake(fd: RawFd) {
    // SAFETY: write is async-signal-safe; a full pipe already wakes the wait.
    unsafe { libc::write(fd, b"x".as_ptr().cast(), 1) };
}

/// Reads the nonblocking `fd` until it is empty.
pub(crate) fn drain(fd: RawFd) {
    let mut drained = [0u8; 64];
    // SAFETY: read fills the owned buffer from the nonblocking pipe.
    while unsafe { libc::read(fd, drained.as_mut_ptr().cast(), drained.len()) } > 0 {}
}

/// Runs `handler` on SIGINT, SIGTERM and SIGHUP.
///
/// # Safety
///
/// `handler` must be async-signal-safe.
pub(crate) unsafe fn on_termination(handler: extern "C" fn(c_int)) -> io::Result<()> {
    for number in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: The caller guarantees the handler is async-signal-safe.
        if unsafe { libc::signal(number, handler as *const () as libc::sighandler_t) }
            == libc::SIG_ERR
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Waits up to `timeout` milliseconds, or forever when it is -1, for one of
/// `fds`' events; returns each descriptor's ready events in order. poll skips
/// a negative descriptor.
pub(crate) fn poll(
    fds: impl IntoIterator<Item = (RawFd, i16)>,
    timeout: i32,
) -> io::Result<Vec<i16>> {
    let mut pollfds: Vec<_> = fds
        .into_iter()
        .map(|(fd, events)| libc::pollfd {
            fd,
            events,
            revents: 0,
        })
        .collect();
    // SAFETY: poll reads and writes the owned pollfds.
    if unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, timeout) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(pollfds.iter().map(|pollfd| pollfd.revents).collect())
}

/// Moves the fact pipe's reader to descriptor 3 and the decision pipe's
/// writer to 4 in the Bend worker. Only async-signal-safe calls run.
///
/// # Safety
///
/// Call it only between fork and exec of the worker, where nothing else owns
/// descriptors 3 and 4.
pub(crate) unsafe fn configure_worker(reader: RawFd, writer: RawFd) -> io::Result<()> {
    // SAFETY: Only async-signal-safe prctl, fcntl and dup2 run, in the
    // forked worker the caller guarantees.
    unsafe {
        // On Linux the worker dies with the host; elsewhere it exits
        // when the host's end of its fact pipe closes.
        #[cfg(target_os = "linux")]
        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
            return Err(io::Error::last_os_error());
        }
        // Move both ends above 4 first, so neither dup2 overwrites the other.
        let reader = libc::fcntl(reader, libc::F_DUPFD_CLOEXEC, 10);
        let writer = libc::fcntl(writer, libc::F_DUPFD_CLOEXEC, 10);
        if reader == -1
            || writer == -1
            || libc::dup2(reader, 3) == -1
            || libc::dup2(writer, 4) == -1
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// A PTY of this size: its nonblocking master and its blocking slave, both
/// closed on exec.
pub(crate) fn open_pty(cols: u16, rows: u16) -> io::Result<(File, File)> {
    let mut master = -1;
    let mut slave = -1;
    let mut size = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: openpty initializes both descriptors on success.
    if unsafe {
        libc::openpty(
            &raw mut master,
            &raw mut slave,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut size,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Successful openpty transferred ownership of both descriptors.
    let master = unsafe { File::from_raw_fd(master) };
    // SAFETY: Successful openpty transferred ownership of both descriptors.
    let slave = unsafe { File::from_raw_fd(slave) };
    set_fd_flags(master.as_raw_fd(), libc::O_NONBLOCK)?;
    set_fd_flags(slave.as_raw_fd(), 0)?;
    Ok((master, slave))
}

fn set_fd_flags(fd: i32, status: i32) -> io::Result<()> {
    // SAFETY: fcntl operates on an owned live descriptor.
    let old_status = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if old_status == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, old_status | status) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: fcntl operates on an owned live descriptor.
    let old_descriptor = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if old_descriptor == -1
        || unsafe { libc::fcntl(fd, libc::F_SETFD, old_descriptor | libc::FD_CLOEXEC) } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Tells the PTY master's terminal its size in cells and pixels.
pub(crate) fn set_window_size(
    master: RawFd,
    cols: u16,
    rows: u16,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let winsize = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: width,
        ws_ypixel: height,
    };
    // SAFETY: ioctl writes the size to the owned PTY master.
    if unsafe { libc::ioctl(master, TIOCSWINSZ_REQUEST, &winsize) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The PTY master's foreground process group, if it has one.
pub(crate) fn foreground_group(master: RawFd) -> Option<pid_t> {
    // SAFETY: tcgetpgrp reads the owned PTY master.
    let group = unsafe { libc::tcgetpgrp(master) };
    (group > 0).then_some(group)
}

/// The most processes the background count walks below the child.
const BACKGROUND_WALK: usize = 512;

/// The terminal's background processes (ORB-C15): processes below `child`
/// that descend, by parent process, from a member of the `foreground` group,
/// are outside it, are neither stopped nor zombies, and started at least 1 s
/// ago. A walk that reaches 512 processes or counts 255 reports 255.
pub(crate) fn background_processes(child: pid_t, foreground: Option<pid_t>) -> u8 {
    let (Some(foreground), Some(clock)) = (foreground, Clock::now()) else {
        return 0;
    };
    let Some(root) = process(child, &clock) else {
        return 0;
    };
    // Each pending process and whether its children descend from a member.
    let mut pending = vec![(child, root.group == foreground)];
    let (mut walked, mut count) = (0, 0u8);
    while let Some((parent, below)) = pending.pop() {
        for pid in children(parent, BACKGROUND_WALK - walked) {
            walked += 1;
            if walked == BACKGROUND_WALK {
                return u8::MAX;
            }
            // A process that exited, or whose pid was reused, is skipped.
            let Some(process) = process(pid, &clock).filter(|process| process.parent == parent)
            else {
                continue;
            };
            let member = process.group == foreground;
            if below && !member && process.running && process.settled {
                count += 1;
                if count == u8::MAX {
                    return count;
                }
            }
            pending.push((pid, below || member));
        }
    }
    count
}

/// One process as the background count reads it.
struct Process {
    parent: pid_t,
    group: pid_t,
    /// Neither stopped nor a zombie.
    running: bool,
    /// Started at least 1 s before the count.
    settled: bool,
}

/// The boot clock in ticks, which `/proc` start times count.
#[cfg(target_os = "linux")]
struct Clock {
    now: u64,
    second: u64,
}

#[cfg(target_os = "linux")]
impl Clock {
    fn now() -> Option<Self> {
        // SAFETY: sysconf has no preconditions.
        let second = u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).ok()?;
        // SAFETY: clock_gettime fills the zeroed time.
        let mut time: libc::timespec = unsafe { std::mem::zeroed() };
        if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &raw mut time) } == -1 {
            return None;
        }
        let now = u64::try_from(time.tv_sec).ok()? * second
            + u64::try_from(time.tv_nsec).ok()? * second / 1_000_000_000;
        Some(Self { now, second })
    }
}

/// At most `limit` children of `pid`, from every thread's list.
#[cfg(target_os = "linux")]
fn children(pid: pid_t, limit: usize) -> Vec<pid_t> {
    let mut found = Vec::new();
    let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) else {
        return found;
    };
    for task in tasks.flatten() {
        let Ok(children) = std::fs::read_to_string(task.path().join("children")) else {
            continue;
        };
        for child in children.split_whitespace() {
            if found.len() == limit {
                return found;
            }
            found.extend(child.parse::<pid_t>().ok());
        }
    }
    found
}

/// The fields of `/proc/<pid>/stat` after the process name, which may hold
/// any bytes, numbered from the state as 0.
#[cfg(target_os = "linux")]
fn stat(pid: &str) -> io::Result<Vec<String>> {
    let stat = std::fs::read(format!("/proc/{pid}/stat"))?;
    let name_end = stat.iter().rposition(|&byte| byte == b')');
    let fields = name_end
        .and_then(|end| std::str::from_utf8(&stat[end + 1..]).ok())
        .ok_or_else(|| errno(libc::EPROTO))?;
    Ok(fields.split_whitespace().map(str::to_owned).collect())
}

#[cfg(target_os = "linux")]
fn process(pid: pid_t, clock: &Clock) -> Option<Process> {
    let fields = stat(&pid.to_string()).ok()?;
    let started: u64 = fields.get(19)?.parse().ok()?;
    Some(Process {
        parent: fields.get(1)?.parse().ok()?,
        group: fields.get(2)?.parse().ok()?,
        running: !matches!(fields.first()?.as_str(), "T" | "t" | "Z" | "X"),
        settled: started.saturating_add(clock.second) <= clock.now,
    })
}

/// The wall clock in microseconds, which libproc start times count.
#[cfg(target_os = "macos")]
struct Clock {
    now: u64,
}

#[cfg(target_os = "macos")]
impl Clock {
    fn now() -> Option<Self> {
        let since = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some(Self {
            now: u64::try_from(since.as_micros()).ok()?,
        })
    }
}

/// At most `limit` children of `pid`.
#[cfg(target_os = "macos")]
fn children(pid: pid_t, limit: usize) -> Vec<pid_t> {
    let mut pids = vec![0 as pid_t; limit];
    let size = std::mem::size_of_val(pids.as_slice()) as libc::c_int;
    // SAFETY: proc_listchildpids fills at most `size` bytes of the owned
    // buffer and returns how many pids it wrote.
    let found = unsafe { libc::proc_listchildpids(pid, pids.as_mut_ptr().cast(), size) };
    pids.truncate(usize::try_from(found).unwrap_or(0));
    pids
}

/// The BSD record of `pid`, if libproc returns all of it for that process.
#[cfg(target_os = "macos")]
fn bsd_info(pid: pid_t) -> Option<libc::proc_bsdinfo> {
    // SAFETY: proc_pidinfo fills the zeroed record for `pid`.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    let read =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (read == size && u32::try_from(pid) == Ok(info.pbi_pid)).then_some(info)
}

/// A BSD record's start time in microseconds.
#[cfg(target_os = "macos")]
fn started(info: &libc::proc_bsdinfo) -> Option<u64> {
    if info.pbi_start_tvusec >= 1_000_000 {
        return None;
    }
    info.pbi_start_tvsec
        .checked_mul(1_000_000)?
        .checked_add(info.pbi_start_tvusec)
}

#[cfg(target_os = "macos")]
fn process(pid: pid_t, clock: &Clock) -> Option<Process> {
    let info = bsd_info(pid)?;
    Some(Process {
        parent: info.pbi_ppid.try_into().ok()?,
        group: info.pbi_pgid.try_into().ok()?,
        running: !matches!(info.pbi_status, libc::SSTOP | libc::SZOMB),
        settled: started(&info)?.saturating_add(1_000_000) <= clock.now,
    })
}

/// Fails unless `parent` still parents this process, then makes it a session
/// leader with its standard input, the PTY slave, as its controlling
/// terminal. Only async-signal-safe calls run.
///
/// # Safety
///
/// Call it only between fork and exec of the PTY child.
pub(crate) unsafe fn configure_child(parent: pid_t) -> io::Result<()> {
    // SAFETY: These process-control calls have no Rust alias.
    unsafe {
        // On Linux the child hears SIGHUP if the host dies; elsewhere the
        // closed PTY master hangs it up.
        #[cfg(target_os = "linux")]
        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGHUP) == -1 {
            return Err(io::Error::last_os_error());
        }
        if libc::getppid() != parent {
            return Err(io::Error::from_raw_os_error(libc::ESRCH));
        }
        if libc::setsid() == -1 || libc::ioctl(libc::STDIN_FILENO, TIOCSCTTY_REQUEST, 0) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Signals the child's process group, or the child itself before its setsid
/// completes; a child already gone is no error.
pub(crate) fn signal(pid: pid_t, signal: i32) -> io::Result<()> {
    // SAFETY: The child PID remains owned until it is reaped.
    if unsafe { libc::kill(-pid, signal) } == 0 {
        return Ok(());
    }
    let group_error = io::Error::last_os_error();
    if group_error.raw_os_error() != Some(libc::ESRCH) {
        return Err(group_error);
    }
    // SAFETY: Before setsid completes, the direct child is the fallback target.
    if unsafe { libc::kill(pid, signal) } == 0 {
        return Ok(());
    }
    let child_error = io::Error::last_os_error();
    if child_error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(child_error)
    }
}

/// Signals a process group; an absent group is ignored.
pub(crate) fn signal_group(group: pid_t, signal: i32) {
    // SAFETY: kill only sends a signal; an absent group is ESRCH.
    unsafe { libc::kill(-group, signal) };
}

/// A zero-length send, which fails only once the peer can no longer read.
/// It never raises SIGPIPE on Linux.
pub(crate) fn send_nothing(stream: &UnixStream) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let flags = libc::MSG_NOSIGNAL;
    #[cfg(not(target_os = "linux"))]
    let flags = 0;
    // SAFETY: A zero-length send reads nothing from the empty buffer.
    if unsafe { libc::send(stream.as_raw_fd(), [0u8; 1].as_ptr().cast(), 0, flags) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Shrinks the socket's send buffer, so a test can fill it.
#[cfg(test)]
pub(crate) fn set_send_buffer(stream: &UnixStream, size: c_int) -> io::Result<()> {
    // SAFETY: setsockopt reads one integer and changes only the owned socket.
    if unsafe {
        libc::setsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            (&raw const size).cast(),
            std::mem::size_of_val(&size) as libc::socklen_t,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The effective user ID of the socket's peer.
#[cfg(target_os = "linux")]
pub(crate) fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    // SAFETY: getsockopt fills the zeroed credentials of the owned socket.
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &raw mut length,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok(credentials.uid)
}

/// The effective user ID of the socket's peer.
#[cfg(target_os = "macos")]
pub(crate) fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: getpeereid fills both identities of the owned socket's peer.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &raw mut uid, &raw mut gid) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

/// The kernel's start time of this process, in clock ticks since boot.
#[cfg(target_os = "linux")]
pub(crate) fn process_start_identity() -> io::Result<u64> {
    stat("self")?
        .get(19)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| errno(libc::EPROTO))
}

/// The kernel's start time of this process in microseconds, as macOS
/// reports it.
#[cfg(target_os = "macos")]
pub(crate) fn process_start_identity() -> io::Result<u64> {
    bsd_info(std::process::id() as pid_t)
        .and_then(|info| started(&info))
        .filter(|identity| *identity != 0)
        .ok_or_else(|| errno(libc::EPROTO))
}
