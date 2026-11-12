//! Platform mechanisms behind Runtime's terminal identity checks and runtime
//! paths. The policy that uses them stays in `sessions` and `supervisor`.

use std::{
    io,
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::{Path, PathBuf},
    time::Duration,
};

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
        // SAFETY: writable timespec; rustix 1.1.4 lacks Darwin's suspend-inclusive raw clock.
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

/// Reads what `stream` holds now, without waiting and without changing the
/// blocking mode its other users share: `WouldBlock` when it holds nothing,
/// and 0 at its end.
pub(crate) fn read_available(stream: &UnixStream, buffer: &mut [u8]) -> io::Result<usize> {
    rustix::net::recv(stream, buffer, rustix::net::RecvFlags::DONTWAIT)
        .map(|(_, read)| read)
        .map_err(Into::into)
}

/// What the kernel reports about a process that still has its pid.
pub(crate) struct Process {
    /// The start identity, in the unit Orbit publishes as `process_start`.
    pub(crate) start: u64,
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
pub(crate) fn peer_identity(stream: &UnixStream) -> io::Result<(libc::pid_t, libc::uid_t)> {
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
pub(crate) fn peer_identity(stream: &UnixStream) -> io::Result<(libc::pid_t, libc::uid_t)> {
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
    // SAFETY: getpeereid fills both identities of the stream's peer.
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &raw mut uid, &raw mut gid) } == -1 {
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
    use super::{SOCKET_PATH_BYTES, read_available, socket_path_fits};
    use crate::supervisor::temporary_directory;
    use std::{
        fs,
        io::{self, Write},
        net::Shutdown,
        os::{
            fd::AsRawFd,
            unix::net::{UnixListener, UnixStream},
        },
    };

    #[test]
    fn read_available_does_not_change_socket_mode() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        // SAFETY: F_GETFL only reads the open file's status flags.
        let flags = || unsafe { libc::fcntl(stream.as_raw_fd(), libc::F_GETFL) };
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
