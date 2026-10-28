//! Platform mechanisms behind Runtime's terminal identity checks and runtime
//! paths. The policy that uses them stays in `sessions` and `supervisor`.

use std::{
    io,
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::{Path, PathBuf},
};

/// The longest Unix socket path this platform binds: `sun_path` less its NUL.
const SOCKET_PATH_BYTES: usize = std::mem::size_of::<libc::sockaddr_un>()
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

#[cfg(test)]
mod tests {
    use super::{SOCKET_PATH_BYTES, socket_path_fits};
    use crate::supervisor::temporary_directory;
    use std::{fs, os::unix::net::UnixListener};

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
