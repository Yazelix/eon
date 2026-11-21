//! Private test process setup and kernel reports, in Linux's `/proc` formats
//! on every platform so assertions read the same bytes.

#![allow(unsafe_code)]

#[cfg(target_os = "linux")]
use std::path::Path;
use std::{fs, io, os::unix::process::CommandExt, process::Command};

pub enum Terminal {
    Stdin,
    Other(fs::File),
    None,
}

/// Starts a private session, optionally acquiring the test's controlling PTY.
pub fn session(command: &mut Command, terminal: Terminal, reset_hangup: bool) {
    let stdin = io::stdin();
    // SAFETY: only async-signal-safe signal/setsid/ioctl calls run in the child;
    // the stdin handle is initialized before fork and the other PTY is owned by
    // the hook through spawn. The valid SIGHUP disposition is reset only there.
    unsafe {
        command.pre_exec(move || {
            if reset_hangup && libc::signal(libc::SIGHUP, libc::SIG_DFL) == libc::SIG_ERR {
                return Err(io::Error::last_os_error());
            }
            rustix::process::setsid()?;
            match &terminal {
                Terminal::Stdin => rustix::process::ioctl_tiocsctty(&stdin)?,
                Terminal::Other(device) => rustix::process::ioctl_tiocsctty(device)?,
                Terminal::None => {}
            }
            Ok(())
        });
    }
}

/// Whether `pid` still names a process, an unreaped zombie included.
#[cfg(target_os = "linux")]
pub fn exists(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

/// The start identity of `pid`, in the unit Eon Runtime compares with
/// Orbit's `process_start`.
#[cfg(target_os = "linux")]
pub fn start_identity(pid: u32) -> u64 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    stat.rsplit_once(')')
        .unwrap()
        .1
        .split_whitespace()
        .nth(19)
        .unwrap()
        .parse()
        .unwrap()
}

/// The environment `pid` started with: each entry ends in a NUL.
#[cfg(target_os = "linux")]
pub fn environ(pid: u32) -> Vec<u8> {
    fs::read(format!("/proc/{pid}/environ")).unwrap()
}

/// The arguments `pid` started with: each ends in a NUL.
#[cfg(target_os = "linux")]
pub fn cmdline(pid: u32) -> Vec<u8> {
    fs::read(format!("/proc/{pid}/cmdline")).unwrap()
}

#[cfg(target_os = "macos")]
fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
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
    // A nonzero argument makes libproc report zombies too.
    // SAFETY: proc_pidinfo fills at most `size` bytes of the zeroed record.
    let read = unsafe {
        libc::proc_pidinfo(
            libc::pid_t::try_from(pid).unwrap(),
            libc::PROC_PIDTBSDINFO,
            1,
            (&raw mut info).cast(),
            size,
        )
    };
    if read <= 0 {
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::ESRCH)
        );
        return None;
    }
    assert_eq!(read, size);
    Some(info)
}

/// Whether `pid` still names a process, an unreaped zombie included.
#[cfg(target_os = "macos")]
pub fn exists(pid: u32) -> bool {
    bsd_info(pid).is_some()
}

/// The start identity of `pid`, in the unit Eon Runtime compares with
/// Orbit's `process_start`: microseconds, as libproc reports it.
#[cfg(target_os = "macos")]
pub fn start_identity(pid: u32) -> u64 {
    let info = bsd_info(pid).unwrap();
    info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec
}

/// The arguments and then the environment `pid` started with, each entry
/// ending in a NUL, from `KERN_PROCARGS2`: the argument count, the
/// executable path, NUL padding, the arguments, then the environment up to an
/// empty string.
#[cfg(target_os = "macos")]
fn procargs(pid: u32) -> (Vec<u8>, Vec<u8>) {
    let mut size = libc::c_int::default();
    let mut length = std::mem::size_of_val(&size);
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    // SAFETY: sysctl writes one c_int into `size`.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&raw mut size).cast(),
            &raw mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    let mut buffer = vec![0_u8; usize::try_from(size).unwrap()];
    let mut length = buffer.len();
    let mut mib = [
        libc::CTL_KERN,
        libc::KERN_PROCARGS2,
        libc::c_int::try_from(pid).unwrap(),
    ];
    // SAFETY: sysctl writes at most `length` bytes into `buffer`.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &raw mut length,
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    let (count, strings) = buffer[..length].split_at(std::mem::size_of::<libc::c_int>());
    let count = usize::try_from(libc::c_int::from_ne_bytes(count.try_into().unwrap())).unwrap();
    let executable = strings.iter().position(|byte| *byte == 0).unwrap();
    let mut strings = strings[executable..]
        .split(|byte| *byte == 0)
        .skip_while(|entry| entry.is_empty());
    let arguments = terminated(strings.by_ref().take(count));
    let environment = terminated(strings.take_while(|entry| !entry.is_empty()));
    (arguments, environment)
}

#[cfg(target_os = "macos")]
fn terminated<'a>(entries: impl Iterator<Item = &'a [u8]>) -> Vec<u8> {
    entries
        .flat_map(|entry| entry.iter().copied().chain([0]))
        .collect()
}

/// The environment `pid` started with: each entry ends in a NUL.
#[cfg(target_os = "macos")]
pub fn environ(pid: u32) -> Vec<u8> {
    procargs(pid).1
}

/// The arguments `pid` started with: each ends in a NUL.
#[cfg(target_os = "macos")]
pub fn cmdline(pid: u32) -> Vec<u8> {
    procargs(pid).0
}

/// Whether the process `pid` waits, as in a poll: Some(true) when its main
/// thread sleeps on Linux, or no thread runs on macOS; None once it has ended.
/// Linux shows a page-in as `D`, so a sleep there is a real wait. macOS also
/// counts loader and page-in waits as not running, so there it is best effort.
#[cfg(target_os = "linux")]
pub fn waits(pid: u32) -> std::io::Result<Option<bool>> {
    let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let state = stat
        .rsplit_once(')')
        .and_then(|(_, rest)| rest.trim_start().chars().next())
        .ok_or_else(|| std::io::Error::other(format!("unreadable /proc/{pid}/stat: {stat}")))?;
    Ok(match state {
        'Z' | 'X' => None,
        state => Some(state == 'S'),
    })
}

#[cfg(target_os = "macos")]
pub fn waits(pid: u32) -> std::io::Result<Option<bool>> {
    let mut info = libc::proc_taskinfo {
        pti_virtual_size: 0,
        pti_resident_size: 0,
        pti_total_user: 0,
        pti_total_system: 0,
        pti_threads_user: 0,
        pti_threads_system: 0,
        pti_policy: 0,
        pti_faults: 0,
        pti_pageins: 0,
        pti_cow_faults: 0,
        pti_messages_sent: 0,
        pti_messages_received: 0,
        pti_syscalls_mach: 0,
        pti_syscalls_unix: 0,
        pti_csw: 0,
        pti_threadnum: 0,
        pti_numrunning: 0,
        pti_priority: 0,
    };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    // SAFETY: proc_pidinfo writes at most `size` bytes into the local info.
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if read == size {
        return Ok(Some(info.pti_numrunning == 0));
    }
    let error = std::io::Error::last_os_error();
    // An ended process, a zombie included, has no task to describe.
    if read <= 0 && error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(None);
    }
    Err(std::io::Error::other(format!(
        "proc_pidinfo({pid}) gave {read} of {size} bytes: {error}"
    )))
}

/// The process ID the kernel reports for the peer that connected `stream`.
#[cfg(target_os = "linux")]
pub fn peer_pid(stream: &std::os::unix::net::UnixStream) -> u32 {
    use std::os::fd::AsRawFd;
    let mut credentials = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the descriptor is a live Unix socket and both out-pointers name
    // initialized storage of the size passed.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &raw mut length,
        )
    };
    assert_eq!(result, 0, "{}", io::Error::last_os_error());
    u32::try_from(credentials.pid).unwrap()
}

/// The process ID the kernel reports for the peer that connected `stream`.
#[cfg(target_os = "macos")]
pub fn peer_pid(stream: &std::os::unix::net::UnixStream) -> u32 {
    use std::os::fd::AsRawFd;
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: the descriptor is a live Unix socket and both out-pointers name
    // initialized storage of the size passed.
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&raw mut pid).cast(),
            &raw mut length,
        )
    };
    assert_eq!(result, 0, "{}", io::Error::last_os_error());
    u32::try_from(pid).unwrap()
}
