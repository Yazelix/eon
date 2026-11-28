//! The host's one platform seam: every `libc` call, the descriptor flags,
//! process groups, signals, polling and socket options they take, and every
//! difference between Linux and macOS. Elsewhere `libc` supplies only errno
//! names for the errors the host builds and classifies.

use crate::errno;
use rustix::{fs::OFlags, io::FdFlags};
use std::{
    ffi::c_int,
    fs::File,
    io,
    os::{
        fd::{AsFd, AsRawFd, BorrowedFd, IntoRawFd, OwnedFd, RawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command},
    sync::{
        OnceLock,
        atomic::{AtomicI32, Ordering},
    },
};

pub(crate) use libc::pid_t;
pub(crate) use libc::{O_CLOEXEC, O_NOFOLLOW, O_NONBLOCK, POLLERR, POLLHUP, POLLIN, POLLOUT};
pub(crate) use rustix::process::Signal;

/// This process's effective user ID.
pub(crate) fn effective_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

/// A pipe whose ends close on exec, nonblocking if asked.
pub(crate) fn pipe(nonblocking: bool) -> io::Result<(File, File)> {
    let (reader, writer) = io::pipe()?;
    let reader = File::from(OwnedFd::from(reader));
    let writer = File::from(OwnedFd::from(writer));
    if nonblocking {
        set_fd_flags(&reader, OFlags::NONBLOCK)?;
        set_fd_flags(&writer, OFlags::NONBLOCK)?;
    }
    Ok((reader, writer))
}

/// Writes one byte to `fd`; async-signal-safe, for a signal handler's wake.
fn wake(fd: RawFd) {
    // SAFETY: errno is this thread's live storage. write is async-signal-safe,
    // reads static storage and retains no pointer; invalid/full pipes only fail.
    unsafe {
        let location = errno_location();
        let saved = *location;
        libc::write(fd, b"x".as_ptr().cast(), 1);
        *location = saved;
    }
}

/// Reads the nonblocking `fd` until it is empty.
pub(crate) fn drain(fd: impl AsFd) {
    let fd = fd.as_fd();
    let mut drained = [0u8; 64];
    while matches!(rustix::io::read(fd, &mut drained), Ok(1..)) {}
}

/// The termination signal the host received, or 0.
static TERMINATION_SIGNAL: AtomicI32 = AtomicI32::new(0);
static SIGNAL_WRITE: AtomicI32 = AtomicI32::new(-1);
/// Both ends live for the process; only this module can access the writer.
static SIGNAL_PIPE: OnceLock<(File, File)> = OnceLock::new();

extern "C" fn note_termination(number: c_int) {
    TERMINATION_SIGNAL.store(number, Ordering::Relaxed);
    wake(SIGNAL_WRITE.load(Ordering::Relaxed));
}

pub(crate) fn termination_signal() -> c_int {
    TERMINATION_SIGNAL.load(Ordering::Relaxed)
}

pub(crate) fn clear_termination() {
    TERMINATION_SIGNAL.store(0, Ordering::Relaxed);
}

/// The permanent wake reader, absent before the handler is installed.
pub(crate) fn termination_reader() -> Option<BorrowedFd<'static>> {
    SIGNAL_PIPE.get().map(|(read, _)| read.as_fd())
}

/// Installs the audited handler with its own permanent nonblocking wake pipe.
pub(crate) fn on_termination() -> io::Result<()> {
    if SIGNAL_PIPE.get().is_none() {
        let _ = SIGNAL_PIPE.set(pipe(true)?);
    }
    let (reader, writer) = SIGNAL_PIPE.get().unwrap();
    drain(reader);
    SIGNAL_WRITE.store(writer.as_raw_fd(), Ordering::Relaxed);
    for number in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: note_termination only stores lock-free atomics and writes a
        // byte to the nonblocking pipe, preserving errno, without allocation.
        if unsafe { libc::signal(number, note_termination as *const () as libc::sighandler_t) }
            == libc::SIG_ERR
        {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn errno_location() -> *mut c_int {
    // SAFETY: both accessors return this thread's live errno storage without
    // allocation or locking, including when called in a signal handler.
    unsafe {
        #[cfg(target_os = "linux")]
        {
            libc::__errno_location()
        }
        #[cfg(target_os = "macos")]
        {
            libc::__error()
        }
    }
}

/// Waits up to `timeout` milliseconds, or forever when it is -1, for one of
/// `fds`' events; returns each slot's ready events in order. An absent
/// descriptor is skipped, as poll skips a negative descriptor.
pub(crate) fn poll<'fd>(
    fds: impl IntoIterator<Item = (Option<BorrowedFd<'fd>>, i16)>,
    timeout: i32,
) -> io::Result<Vec<i16>> {
    use rustix::event::{PollFd, PollFlags, Timespec};
    let fds: Vec<_> = fds.into_iter().collect();
    let mut pollfds: Vec<_> = fds
        .iter()
        .filter_map(|&(fd, events)| {
            Some(PollFd::from_borrowed_fd(
                fd?,
                PollFlags::from_bits_retain(events as _),
            ))
        })
        .collect();
    let timeout = (timeout >= 0).then(|| Timespec {
        tv_sec: (timeout / 1000).into(),
        tv_nsec: ((timeout % 1000) * 1_000_000).into(),
    });
    rustix::event::poll(&mut pollfds, timeout.as_ref())?;
    let mut ready = pollfds.iter();
    Ok(fds
        .iter()
        .map(|(fd, _)| {
            if fd.is_some() {
                ready.next().unwrap().revents().bits() as _
            } else {
                0
            }
        })
        .collect())
}

/// Spawns while the worker's borrowed pipe ends remain owned by the caller.
pub(crate) fn spawn_worker(
    mut command: Command,
    reader: &File,
    writer: &File,
) -> io::Result<Child> {
    // SAFETY: the borrowed Files keep both descriptors live through spawn;
    // the owned Command cannot retain this hook afterwards. configure_worker
    // only runs async-signal-safe calls in the child, where 3/4 are reserved.
    unsafe {
        let reader = BorrowedFd::borrow_raw(reader.as_raw_fd());
        let writer = BorrowedFd::borrow_raw(writer.as_raw_fd());
        command.pre_exec(move || configure_worker(reader, writer));
    }
    command.spawn()
}

/// Spawns the PTY child with its already-owned stdin as controlling terminal.
pub(crate) fn spawn_child(mut command: Command, parent: pid_t) -> io::Result<Child> {
    // SAFETY: the production Command owns PTY stdin through spawn and exec;
    // the failure test returns on parent mismatch before accessing stdin.
    // configure_child runs only async-signal-safe calls, and this owned
    // Command is dropped after spawn, so its hook cannot be reused.
    unsafe {
        command.pre_exec(move || {
            // Command has installed its owned PTY at fd 0; the constant borrow
            // cannot trigger borrow_raw's invalid-minus-one assertion.
            configure_child(parent, BorrowedFd::borrow_raw(libc::STDIN_FILENO))
        });
    }
    command.spawn()
}

/// Moves the fact pipe's reader to descriptor 3 and the decision pipe's
/// writer to 4 in the Bend worker. Only async-signal-safe calls run.
fn configure_worker(reader: BorrowedFd<'_>, writer: BorrowedFd<'_>) -> io::Result<()> {
    // On Linux the worker dies with the host; elsewhere it exits when the
    // host's end of its fact pipe closes.
    #[cfg(target_os = "linux")]
    rustix::process::set_parent_process_death_signal(Some(Signal::KILL))?;
    // Move both ends above 4 first; keep both attempts and the old error order.
    // Copies close on exec/child exit, including after a failed setup.
    let reader = rustix::io::fcntl_dupfd_cloexec(reader, 10).map(IntoRawFd::into_raw_fd);
    let writer = rustix::io::fcntl_dupfd_cloexec(writer, 10).map(IntoRawFd::into_raw_fd);
    let writer = writer?;
    let reader = reader?;
    // SAFETY: spawn_worker confines this to the forked worker; 3/4 are its
    // reserved protocol descriptors, and neither temporary can be overwritten.
    // dup2 is async-signal-safe and clears CLOEXEC on both destinations.
    if unsafe { libc::dup2(reader, 3) == -1 || libc::dup2(writer, 4) == -1 } {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// A PTY of this size: its nonblocking master and its blocking slave, both
/// closed on exec.
pub(crate) fn open_pty(cols: u16, rows: u16) -> io::Result<(File, File)> {
    use rustix::{
        fs::Mode,
        pty::{self, OpenptFlags},
    };
    // openpt rewrites Linux ENOSPC to EAGAIN; glibc's getpt does not.
    #[cfg(target_os = "linux")]
    let master = rustix::fs::open(c"/dev/ptmx", OFlags::RDWR, Mode::empty())?;
    #[cfg(target_os = "macos")]
    let master = pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    pty::grantpt(&master)?;
    pty::unlockpt(&master)?;
    let open_slave = || {
        // On Darwin, pinned rustix reports a ptsname_r failure as OS error -1
        // rather than errno. Keep safe ownership and fail startup in that case.
        let name = pty::ptsname(&master, Vec::new())?;
        rustix::fs::open(&name, OFlags::RDWR | OFlags::NOCTTY, Mode::empty())
    };
    #[cfg(target_os = "linux")]
    let slave = pty::ioctl_tiocgptpeer(&master, OpenptFlags::RDWR | OpenptFlags::NOCTTY)
        .or_else(|_| open_slave())?;
    #[cfg(target_os = "macos")]
    let slave = open_slave()?;
    // Both libc openpty implementations ignore the initial resize's error.
    let _ = rustix::termios::tcsetwinsize(
        &slave,
        rustix::termios::Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    );
    set_fd_flags(&master, OFlags::NONBLOCK)?;
    set_fd_flags(&slave, OFlags::empty())?;
    Ok((File::from(master), File::from(slave)))
}

fn set_fd_flags(fd: impl AsFd, status: OFlags) -> io::Result<()> {
    let fd = fd.as_fd();
    let old_status = rustix::fs::fcntl_getfl(fd)?;
    rustix::fs::fcntl_setfl(fd, old_status | status)?;
    let old_descriptor = rustix::io::fcntl_getfd(fd)?;
    rustix::io::fcntl_setfd(fd, old_descriptor | FdFlags::CLOEXEC)?;
    Ok(())
}

/// Tells the PTY master's terminal its size in cells and pixels.
pub(crate) fn set_window_size(
    master: impl AsFd,
    cols: u16,
    rows: u16,
    width: u16,
    height: u16,
) -> io::Result<()> {
    let winsize = rustix::termios::Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: width,
        ws_ypixel: height,
    };
    rustix::termios::tcsetwinsize(master, winsize).map_err(Into::into)
}

/// The PTY master's foreground process group, if it has one.
#[cfg(target_os = "linux")]
pub(crate) fn foreground_group(master: impl AsFd) -> Option<pid_t> {
    rustix::termios::tcgetpgrp(master)
        .ok()
        .map(|group| group.as_raw_pid())
}

/// The PTY master's foreground process group, if it has one.
#[cfg(target_os = "macos")]
pub(crate) fn foreground_group(master: impl AsFd) -> Option<pid_t> {
    // SAFETY: the borrowed descriptor stays live, and libc preserves a zero PID.
    // Keep this until eon-ojwmi: rustix 1.1.4 makes zero an invalid NonZero PID.
    let group = unsafe { libc::tcgetpgrp(master.as_fd().as_raw_fd()) };
    (group > 0).then_some(group)
}

/// The most processes the background count walks below the child.
const BACKGROUND_WALK: usize = 512;

/// The terminal's background processes (ORB-C15): processes below `child`
/// that descend, by parent process, from a member of the `foreground` group,
/// are outside it, are neither stopped nor zombies, and started at least 1 s
/// ago. A walk that reaches 512 processes or counts 255 reports 255.
pub(crate) fn background_processes(
    child: pid_t,
    foreground: Option<pid_t>,
) -> (u8, orbit_protocol::management::ProgramProcesses) {
    use orbit_protocol::management::{
        MAX_BACKGROUND_IDENTITIES, ProcessIdentity, ProgramProcesses,
    };
    let mut census = ProgramProcesses::default();
    let (Some(foreground), Some(clock)) = (foreground, Clock::now()) else {
        return (0, census);
    };
    let Some(root) = process(child, &clock) else {
        return (0, census);
    };
    census.complete = true;
    census.foreground = process(foreground, &clock).and_then(|leader| {
        (leader.group == foreground && leader.start != 0).then_some(ProcessIdentity {
            pid: foreground.try_into().ok()?,
            start: leader.start,
        })
    });
    // Each pending process and whether its children descend from a member.
    let mut pending = vec![(child, root.group == foreground)];
    let (mut walked, mut count) = (0, 0u8);
    while let Some((parent, below)) = pending.pop() {
        for pid in children(parent, BACKGROUND_WALK - walked) {
            walked += 1;
            if walked == BACKGROUND_WALK {
                census.complete = false;
                return (u8::MAX, census);
            }
            // A process that exited, or whose pid was reused, is skipped.
            let Some(process) = process(pid, &clock).filter(|process| process.parent == parent)
            else {
                continue;
            };
            let member = process.group == foreground;
            if below && !member && process.running && process.settled {
                count += 1;
                if census.background.len() < MAX_BACKGROUND_IDENTITIES
                    && process.start != 0
                    && !census
                        .background
                        .iter()
                        .any(|identity| identity.pid == pid as u32)
                {
                    census.background.push(ProcessIdentity {
                        pid: pid as u32,
                        start: process.start,
                    });
                } else {
                    census.complete = false;
                }
                if count == u8::MAX {
                    census.complete = false;
                    return (count, census);
                }
            }
            pending.push((pid, below || member));
        }
    }
    (count, census)
}

/// One process as the background count reads it.
struct Process {
    start: u64,
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
        let second = rustix::param::clock_ticks_per_second();
        let time = rustix::time::clock_gettime(rustix::time::ClockId::Boottime);
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
        start: started,
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
    let mut info = libc::proc_bsdinfo {
        pbi_flags: 0,
        pbi_status: 0,
        pbi_xstatus: 0,
        pbi_pid: 0,
        pbi_ppid: 0,
        pbi_uid: 0,
        pbi_gid: 0,
        pbi_ruid: 0,
        pbi_rgid: 0,
        pbi_svuid: 0,
        pbi_svgid: 0,
        rfu_1: 0,
        pbi_comm: [0; libc::MAXCOMLEN],
        pbi_name: [0; 32],
        pbi_nfiles: 0,
        pbi_pgid: 0,
        pbi_pjobc: 0,
        e_tdev: 0,
        e_tpgid: 0,
        pbi_nice: 0,
        pbi_start_tvsec: 0,
        pbi_start_tvusec: 0,
    };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    // SAFETY: info is writable for exactly size bytes and remains live for the call;
    // the result is used only after the complete record and matching PID are checked.
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
        start: started(&info)?,
        parent: info.pbi_ppid.try_into().ok()?,
        group: info.pbi_pgid.try_into().ok()?,
        running: !matches!(info.pbi_status, libc::SSTOP | libc::SZOMB),
        settled: started(&info)?.saturating_add(1_000_000) <= clock.now,
    })
}

/// Fails unless `parent` still parents this process, then makes it a session
/// leader with stdin as controlling terminal. Only async-signal-safe calls run.
fn configure_child(parent: pid_t, stdin: BorrowedFd<'_>) -> io::Result<()> {
    // On Linux the child hears SIGHUP if the host dies; elsewhere the closed
    // PTY master hangs it up.
    #[cfg(target_os = "linux")]
    rustix::process::set_parent_process_death_signal(Some(Signal::HUP))?;
    if rustix::process::getppid().map_or(0, |pid| pid.as_raw_pid()) != parent {
        return Err(errno(libc::ESRCH));
    }
    rustix::process::setsid()?;
    rustix::process::ioctl_tiocsctty(stdin)?;
    Ok(())
}

/// Signals the child's process group, or the child itself before its setsid
/// completes; a child already gone is no error.
pub(crate) fn signal(pid: pid_t, signal: Signal) -> io::Result<()> {
    use rustix::{io::Errno, process};
    let pid = positive_pid(pid).ok_or_else(|| errno(libc::EINVAL))?;
    match process::kill_process_group(pid, signal) {
        Ok(()) => Ok(()),
        Err(Errno::SRCH) => match process::kill_process(pid, signal) {
            Ok(()) | Err(Errno::SRCH) => Ok(()),
            Err(error) => Err(error.into()),
        },
        Err(error) => Err(error.into()),
    }
}

/// Signals a process group; an absent group is ignored.
pub(crate) fn signal_group(group: pid_t, signal: Signal) {
    if let Some(group) = positive_pid(group) {
        let _ = rustix::process::kill_process_group(group, signal);
    }
}

fn positive_pid(pid: pid_t) -> Option<rustix::process::Pid> {
    if pid <= 0 {
        return None;
    }
    rustix::process::Pid::from_raw(pid)
}

/// A zero-length send, which fails only once the peer can no longer read.
/// It never raises SIGPIPE on Linux.
pub(crate) fn send_nothing(stream: &UnixStream) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let flags = rustix::net::SendFlags::NOSIGNAL;
    #[cfg(not(target_os = "linux"))]
    let flags = rustix::net::SendFlags::empty();
    rustix::net::send(stream, &[], flags)?;
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn send_bench_trace(
    mut send: impl FnMut() -> io::Result<usize>,
    timeout: std::time::Duration,
    drop_mode: &std::sync::atomic::AtomicBool,
) -> io::Result<usize> {
    use std::{
        sync::atomic::Ordering,
        time::{Duration, Instant},
    };
    let start = Instant::now();
    loop {
        let error = match send() {
            Ok(sent) => return Ok(sent),
            Err(error) => error,
        };
        let interrupted = error.kind() == io::ErrorKind::Interrupted;
        if !interrupted
            && error.kind() != io::ErrorKind::WouldBlock
            && error.raw_os_error() != Some(libc::ENOBUFS)
        {
            return Err(error);
        }
        if drop_mode.load(Ordering::Relaxed) {
            return Err(error);
        }
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            drop_mode.store(true, Ordering::Relaxed);
            return Err(io::ErrorKind::TimedOut.into());
        }
        if !interrupted {
            // Darwin's writable sender can still have a full datagram receiver.
            std::thread::sleep(Duration::from_millis(1).min(remaining));
        }
    }
}

/// Shrinks the socket's send buffer, so a test can fill it.
#[cfg(test)]
pub(crate) fn set_send_buffer(stream: &UnixStream, size: usize) -> io::Result<()> {
    rustix::net::sockopt::set_socket_send_buffer_size(stream, size).map_err(Into::into)
}

// A raw PID remains representable until peer_pid validates it; rustix's
// UCred contains a NonZero PID and cannot preserve that validation order.
#[cfg(target_os = "linux")]
fn peer_credentials(stream: &UnixStream) -> io::Result<libc::ucred> {
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of_val(&credentials) as libc::socklen_t;
    // SAFETY: getsockopt writes at most length bytes to the live integer-only
    // credentials record; the borrowed stream keeps the descriptor open.
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
    Ok(credentials)
}

/// The effective user ID of the socket's peer.
#[cfg(target_os = "linux")]
pub(crate) fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    Ok(peer_credentials(stream)?.uid)
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

/// The process ID of the socket's peer, as the kernel recorded it when the
/// peer connected (ORB-C3).
#[cfg(target_os = "linux")]
pub(crate) fn peer_pid(stream: &UnixStream) -> io::Result<u32> {
    peer_pid_value(peer_credentials(stream)?.pid)
}

/// The process ID of the socket's peer, as the kernel recorded it when the
/// peer connected (ORB-C3).
#[cfg(target_os = "macos")]
pub(crate) fn peer_pid(stream: &UnixStream) -> io::Result<u32> {
    let mut pid: pid_t = 0;
    let mut length = std::mem::size_of::<pid_t>() as libc::socklen_t;
    // SAFETY: getsockopt fills the peer's pid of the owned socket.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&raw mut pid).cast(),
            &raw mut length,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    peer_pid_value(pid)
}

fn peer_pid_value(pid: pid_t) -> io::Result<u32> {
    u32::try_from(pid)
        .ok()
        .filter(|pid| *pid != 0)
        .ok_or_else(|| errno(libc::EPROTO))
}

/// The kernel's start time of process `pid`, in clock ticks since boot.
#[cfg(target_os = "linux")]
pub(crate) fn process_start_identity(pid: u32) -> io::Result<u64> {
    stat(&pid.to_string())?
        .get(19)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| errno(libc::EPROTO))
}

/// The kernel's start time of process `pid` in microseconds, as macOS
/// reports it.
#[cfg(target_os = "macos")]
pub(crate) fn process_start_identity(pid: u32) -> io::Result<u64> {
    pid_t::try_from(pid)
        .ok()
        .and_then(bsd_info)
        .and_then(|info| started(&info))
        .filter(|identity| *identity != 0)
        .ok_or_else(|| errno(libc::EPROTO))
}

#[cfg(test)]
mod descriptor_tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn socket_credentials_preserve_identity_and_reject_invalid_pids() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        assert_eq!(peer_uid(&stream).unwrap(), effective_uid());
        assert_eq!(peer_pid(&stream).unwrap(), std::process::id());
        for pid in [0, -1, i32::MIN] {
            assert_eq!(
                peer_pid_value(pid).unwrap_err().raw_os_error(),
                Some(libc::EPROTO)
            );
        }
        for pid in [1, i32::MAX] {
            assert_eq!(peer_pid_value(pid).unwrap(), pid as u32);
        }
    }

    fn observe_errno(operation: impl FnOnce()) -> c_int {
        // SAFETY: the pointer is this thread's errno. The supplied operation
        // only invokes our non-panicking handler; restore errno before returning.
        unsafe {
            let location = errno_location();
            let saved = *location;
            *location = libc::EDOM;
            operation();
            let observed = *location;
            *location = saved;
            observed
        }
    }

    #[test]
    fn termination_installer_owns_and_reuses_a_nonblocking_pipe() {
        let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
        // eon-test runs this signal-disposition test in its own nextest process.
        on_termination().unwrap();
        let (reader, writer) = SIGNAL_PIPE.get().unwrap();
        let descriptors = (reader.as_raw_fd(), writer.as_raw_fd());
        for fd in [reader, writer] {
            assert!(
                rustix::fs::fcntl_getfl(fd)
                    .unwrap()
                    .contains(OFlags::NONBLOCK)
            );
            assert!(
                rustix::io::fcntl_getfd(fd)
                    .unwrap()
                    .contains(FdFlags::CLOEXEC)
            );
        }
        let saved_signal = TERMINATION_SIGNAL.swap(0, Ordering::Relaxed);
        note_termination(libc::SIGTERM);
        let signal = termination_signal();
        let ready = poll([(termination_reader(), POLLIN)], 0).unwrap();
        on_termination().unwrap();
        let drained = poll([(termination_reader(), POLLIN)], 0).unwrap();
        let installed = SIGNAL_WRITE.load(Ordering::Relaxed);
        note_termination(libc::SIGINT);
        let mut byte = [0];
        let read = rustix::io::read(termination_reader().unwrap(), &mut byte);
        TERMINATION_SIGNAL.store(saved_signal, Ordering::Relaxed);
        assert_eq!(signal, libc::SIGTERM);
        assert_eq!(ready, [POLLIN]);
        assert_eq!(drained, [0]);
        assert_eq!(installed, descriptors.1);
        let (reader, writer) = SIGNAL_PIPE.get().unwrap();
        assert_eq!((reader.as_raw_fd(), writer.as_raw_fd()), descriptors);
        assert_eq!(read.unwrap(), 1);
        assert_eq!(&byte, b"x");
    }

    #[test]
    fn termination_wake_preserves_errno_even_when_the_pipe_is_full() {
        let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
        // eon-test also isolates each nextest test in its own process.
        let (mut reader, writer) = pipe(true).unwrap();
        let bytes = [0];
        loop {
            match rustix::io::write(&writer, &bytes) {
                Ok(_) => {}
                Err(rustix::io::Errno::AGAIN) => break,
                result => panic!("fill wake pipe: {result:?}"),
            }
        }
        let saved_write = SIGNAL_WRITE.swap(writer.as_raw_fd(), Ordering::Relaxed);
        let saved_signal = TERMINATION_SIGNAL.swap(0, Ordering::Relaxed);
        let full_errno = observe_errno(|| note_termination(libc::SIGTERM));
        let full_signal = termination_signal();
        drain(&reader);
        let ready_errno = observe_errno(|| note_termination(libc::SIGINT));
        let ready_signal = termination_signal();
        SIGNAL_WRITE.store(saved_write, Ordering::Relaxed);
        TERMINATION_SIGNAL.store(saved_signal, Ordering::Relaxed);
        assert_eq!(full_errno, libc::EDOM);
        assert_eq!(full_signal, libc::SIGTERM);
        assert_eq!(ready_errno, libc::EDOM);
        assert_eq!(ready_signal, libc::SIGINT);
        let mut byte = [0];
        reader.read_exact(&mut byte).unwrap();
        assert_eq!(&byte, b"x");
    }

    #[test]
    fn a_fresh_pty_has_no_foreground_group() {
        let (master, slave) = open_pty(80, 24).unwrap();
        assert_eq!(foreground_group(&master), None);
        for (fd, nonblocking) in [(&master, true), (&slave, false)] {
            assert!(
                rustix::io::fcntl_getfd(fd)
                    .unwrap()
                    .contains(FdFlags::CLOEXEC)
            );
            assert_eq!(
                rustix::fs::fcntl_getfl(fd)
                    .unwrap()
                    .contains(OFlags::NONBLOCK),
                nonblocking
            );
        }
        let size = rustix::termios::tcgetwinsize(&slave).unwrap();
        assert_eq!(
            (size.ws_col, size.ws_row, size.ws_xpixel, size.ws_ypixel),
            (80, 24, 0, 0)
        );
        let settings = rustix::termios::tcgetattr(&slave).unwrap();
        assert!(
            settings
                .local_modes
                .contains(rustix::termios::LocalModes::ICANON | rustix::termios::LocalModes::ECHO)
        );
    }

    #[test]
    fn signals_validate_ids_and_fall_back_to_the_direct_child() {
        use std::{os::unix::process::ExitStatusExt, process::Command};

        let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
        for pid in [0, -1, i32::MIN] {
            assert!(positive_pid(pid).is_none());
            assert_eq!(
                signal(pid, Signal::CONT).unwrap_err().raw_os_error(),
                Some(libc::EINVAL)
            );
        }
        let mut child = Command::new("sleep").arg("30").spawn().unwrap();
        let result = signal(child.id().try_into().unwrap(), Signal::KILL);
        if result.is_err() {
            child.kill().unwrap();
        }
        let status = child.wait().unwrap();
        result.unwrap();
        assert_eq!(status.signal(), Some(libc::SIGKILL));
    }

    #[test]
    fn polling_skips_slots_and_drain_reads_until_empty() {
        let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
        let (mut reader, mut writer) = pipe(true).unwrap();
        let slots = [
            (None, POLLIN),
            (Some(reader.as_fd()), POLLIN),
            (None, POLLOUT),
        ];
        assert_eq!(poll(slots, 0).unwrap(), [0, 0, 0]);
        writer.write_all(&[b'x'; 200]).unwrap();
        assert_eq!(poll(slots, 0).unwrap(), [0, POLLIN, 0]);
        drain(&reader);
        assert_eq!(poll(slots, 0).unwrap(), [0, 0, 0]);
        assert_eq!(
            reader.read(&mut [0]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        drop(writer);
        drain(&reader);
        assert_eq!(reader.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn pipes_keep_descriptor_flags_and_deliver_eof() {
        let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
        for nonblocking in [false, true] {
            let (mut reader, mut writer) = pipe(nonblocking).unwrap();
            for end in [&reader, &writer] {
                assert!(
                    rustix::io::fcntl_getfd(end)
                        .unwrap()
                        .contains(FdFlags::CLOEXEC)
                );
                assert_eq!(
                    rustix::fs::fcntl_getfl(end)
                        .unwrap()
                        .contains(OFlags::NONBLOCK),
                    nonblocking
                );
            }
            set_fd_flags(&reader, OFlags::empty()).unwrap();
            if nonblocking {
                assert_eq!(
                    reader.read(&mut [0]).unwrap_err().kind(),
                    io::ErrorKind::WouldBlock
                );
            }
            writer.write_all(b"pipe").unwrap();
            let mut bytes = [0; 4];
            reader.read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes, b"pipe");
            set_fd_flags(&reader, OFlags::NONBLOCK).unwrap();
            drop(writer);
            assert_eq!(reader.read(&mut bytes).unwrap(), 0);
        }
    }
}

#[cfg(test)]
mod bench_trace_tests {
    use super::send_bench_trace;
    use std::{
        io,
        os::unix::net::UnixDatagram,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };

    fn full_queue() -> (UnixDatagram, UnixDatagram, u64) {
        let (sender, receiver) = UnixDatagram::pair().unwrap();
        sender.set_nonblocking(true).unwrap();
        receiver
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        for sequence in 0u64..100_000 {
            match sender.send(&sequence.to_le_bytes()) {
                Ok(8) => (),
                Err(e)
                    if e.kind() == io::ErrorKind::WouldBlock
                        || e.raw_os_error() == Some(libc::ENOBUFS) =>
                {
                    return (sender, receiver, sequence);
                }
                result => panic!("cannot fill trace queue: {result:?}"),
            }
        }
        panic!("trace queue did not fill");
    }

    #[test]
    fn bench_trace_retries_a_full_queue_until_it_drains() {
        let (sender, receiver, queued) = full_queue();
        let drop_mode = AtomicBool::new(false);
        let (release, wait) = mpsc::channel();
        let mut retries = 0;
        std::thread::scope(|scope| {
            let reader = scope.spawn(move || {
                wait.recv_timeout(Duration::from_secs(10)).unwrap();
                let mut packet = [0; 8];
                for sequence in 0..=queued {
                    assert_eq!(receiver.recv(&mut packet).unwrap(), 8);
                    assert_eq!(u64::from_le_bytes(packet), sequence);
                }
                receiver.set_nonblocking(true).unwrap();
                assert_eq!(
                    receiver.recv(&mut packet).unwrap_err().kind(),
                    io::ErrorKind::WouldBlock
                );
            });
            assert_eq!(
                send_bench_trace(
                    || {
                        let result = sender.send(&queued.to_le_bytes());
                        if result.is_err() {
                            retries += 1;
                            if retries == 1 {
                                release.send(()).unwrap();
                            }
                        }
                        result
                    },
                    Duration::from_secs(10),
                    &drop_mode
                )
                .unwrap(),
                8
            );
            reader.join().unwrap();
        });
        assert!(retries > 0, "the receiver drained before any retry");
        assert!(!drop_mode.load(Ordering::Relaxed));
    }

    #[test]
    fn bench_trace_timeout_switches_later_records_to_drop_mode() {
        let (sender, _receiver, _) = full_queue();
        let drop_mode = AtomicBool::new(false);
        let error = send_bench_trace(
            || sender.send(b"lost"),
            Duration::from_millis(5),
            &drop_mode,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(drop_mode.load(Ordering::Relaxed));
        let mut attempts = 0;
        let error = send_bench_trace(
            || {
                attempts += 1;
                sender.send(b"also lost")
            },
            Duration::from_secs(2),
            &drop_mode,
        )
        .unwrap_err();
        assert!(
            error.kind() == io::ErrorKind::WouldBlock
                || error.raw_os_error() == Some(libc::ENOBUFS)
        );
        assert_eq!(attempts, 1, "drop mode must not wait again");
    }

    #[test]
    fn bench_trace_drop_mode_does_not_retry_interrupted_sends() {
        let drop_mode = AtomicBool::new(true);
        let mut attempts = 0;
        let error = send_bench_trace(
            || {
                attempts += 1;
                assert_eq!(attempts, 1, "drop mode retried EINTR");
                Err(io::Error::from_raw_os_error(libc::EINTR))
            },
            Duration::from_secs(10),
            &drop_mode,
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(attempts, 1);
    }

    #[test]
    fn bench_trace_permanent_errors_do_not_retry() {
        let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
        for closed in [false, true] {
            let (sender, receiver) = UnixDatagram::pair().unwrap();
            sender.set_nonblocking(true).unwrap();
            let packet = vec![0; if closed { 8 } else { 1024 * 1024 }];
            let _receiver = (!closed).then_some(receiver);
            let drop_mode = AtomicBool::new(false);
            let mut attempts = 0;
            let error = send_bench_trace(
                || {
                    attempts += 1;
                    sender.send(&packet)
                },
                Duration::from_secs(2),
                &drop_mode,
            )
            .unwrap_err();
            if !closed {
                assert_eq!(error.raw_os_error(), Some(libc::EMSGSIZE));
            }
            assert_eq!(attempts, 1);
            assert!(!drop_mode.load(Ordering::Relaxed));
        }
    }
}
