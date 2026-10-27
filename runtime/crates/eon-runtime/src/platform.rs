//! Platform mechanisms behind Runtime's terminal identity checks. The policy
//! that uses them stays in `sessions`.

use std::{
    io,
    os::{fd::AsRawFd, unix::net::UnixStream},
};

/// What the kernel reports about a process that still has its pid.
pub(crate) struct Process {
    /// The start identity, in the unit Orbit publishes as `process_start`.
    pub(crate) start: u64,
    /// Whether the process has exited and awaits reaping.
    pub(crate) ended: bool,
}

/// The pid and uid of the stream's peer.
#[cfg(target_os = "linux")]
pub(crate) fn peer_identity(stream: &UnixStream) -> Option<(libc::pid_t, libc::uid_t)> {
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: credentials points to writable ucred storage of the declared length.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &raw mut length,
        )
    } == -1
        || length as usize != std::mem::size_of::<libc::ucred>()
    {
        return None;
    }
    Some((credentials.pid, credentials.uid))
}

/// The pid and uid of the stream's peer.
#[cfg(target_os = "macos")]
pub(crate) fn peer_identity(stream: &UnixStream) -> Option<(libc::pid_t, libc::uid_t)> {
    let mut pid: libc::pid_t = 0;
    let mut length = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: pid points to writable pid_t storage of the declared length.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            (&raw mut pid).cast(),
            &raw mut length,
        )
    } == -1
        || length as usize != std::mem::size_of::<libc::pid_t>()
    {
        return None;
    }
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: getpeereid fills both identities of the stream's peer.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &raw mut uid, &raw mut gid) } == -1 {
        return None;
    }
    Some((pid, uid))
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
    let start = fields
        .nth(18)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or("invalid terminal process identity")?;
    Ok(Some(Process {
        start,
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
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    // A nonzero argument makes libproc report zombies too, so their start
    // identity is still checked.
    // SAFETY: proc_pidinfo fills at most `size` bytes of the zeroed record.
    let read = unsafe {
        libc::proc_pidinfo(
            pid_value,
            libc::PROC_PIDTBSDINFO,
            1,
            (&raw mut info).cast(),
            size,
        )
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
        ended: info.pbi_status == libc::SZOMB,
    }))
}
