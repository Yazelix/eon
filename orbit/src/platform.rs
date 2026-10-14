use crate::Result;
use orbit_protocol::{
    management::{EndpointIdentity, ObjectIdentity},
    session::SurfaceSize,
};
use std::{
    env,
    fs::{self, File, OpenOptions, TryLockError},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, RawFd},
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

static TERMINATE: AtomicBool = AtomicBool::new(false);
static NEXT_ARTIFACT: AtomicU64 = AtomicU64::new(0);
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);
const SHUTDOWN_LIMIT: Duration = Duration::from_secs(2);
const SHUTDOWN_POLL: Duration = Duration::from_millis(10);
const PTY_EIO_RETRY_DELAY: Duration = Duration::from_millis(10);
const TIOCSCTTY_REQUEST: libc::c_ulong = libc::TIOCSCTTY as libc::c_ulong;
const TIOCSWINSZ_REQUEST: libc::c_ulong = libc::TIOCSWINSZ as libc::c_ulong;

pub(crate) enum PtyIo {
    Ready(usize),
    Blocked,
    Closed,
}

pub(crate) struct Pty {
    master: File,
    child: Option<Child>,
}

impl Pty {
    #[cfg(test)]
    pub(crate) fn without_child_for_test() -> Result<Self> {
        Ok(Self {
            master: OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/null")?,
            child: None,
        })
    }

    pub(crate) fn spawn(command: &[String], size: SurfaceSize) -> Result<Self> {
        if command.is_empty() {
            return Err("PTY command cannot be empty".into());
        }
        let (master, slave) = open_pty(size)?;

        let stdin = slave.try_clone()?;
        let stdout = slave.try_clone()?;
        let mut child_command = Command::new(&command[0]);
        child_command
            .args(&command[1..])
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(slave))
            .env("TERM", "eon")
            .env("COLORTERM", "truecolor");
        unsafe {
            child_command.pre_exec(configure_child_terminal);
        }
        let child = child_command.spawn()?;
        Ok(Self {
            master,
            child: Some(child),
        })
    }

    pub(crate) fn resize(&self, size: SurfaceSize) -> Result {
        #[cfg(test)]
        if self.child.is_none() {
            return Ok(());
        }
        let winsize = winsize(size);
        let result = unsafe { libc::ioctl(self.master.as_raw_fd(), TIOCSWINSZ_REQUEST, &winsize) };
        if result == -1 {
            return Err(io::Error::last_os_error().into());
        }
        Ok(())
    }

    pub(crate) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child
            .as_mut()
            .expect("PTY child already reaped")
            .try_wait()
    }

    pub(crate) fn read(&mut self, bytes: &mut [u8]) -> Result<PtyIo> {
        nonblocking(|| self.master.read(bytes), self.child.is_some())
    }

    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<PtyIo> {
        nonblocking(|| self.master.write(bytes), false)
    }

    pub(crate) fn stop_and_reap(&mut self) -> Result<Option<ExitStatus>> {
        let Some(child) = self.child.as_mut() else {
            return Ok(None);
        };
        if let Some(status) = child.try_wait()? {
            // A waited PID may be reused, so it is no longer a safe process-group identity.
            self.child = None;
            return Ok(Some(status));
        }
        let child_group = child.id() as libc::pid_t;
        let foreground_group = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        signal_process_group(child_group, libc::SIGHUP)?;
        if foreground_group > 0 && foreground_group != child_group {
            signal_process_group(foreground_group, libc::SIGHUP)?;
        }

        let grace_deadline = Instant::now() + SHUTDOWN_GRACE;
        let mut status = None;
        while Instant::now() < grace_deadline {
            if let Some(exit) = child.try_wait()? {
                status = Some(exit);
                break;
            }
            thread::sleep(SHUTDOWN_POLL);
        }

        if status.is_none() {
            signal_process_group(child_group, libc::SIGKILL)?;
            let foreground_group = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
            if foreground_group > 0 && foreground_group != child_group {
                signal_process_group(foreground_group, libc::SIGKILL)?;
            }
            let deadline = Instant::now() + SHUTDOWN_LIMIT;
            loop {
                if let Some(exit) = child.try_wait()? {
                    status = Some(exit);
                    break;
                }
                if Instant::now() >= deadline {
                    return Err("PTY child was not reaped before the shutdown deadline".into());
                }
                thread::sleep(SHUTDOWN_POLL);
            }
        }
        self.child = None;
        Ok(status)
    }
}

fn open_pty(size: SurfaceSize) -> Result<(File, File)> {
    let mut master_fd = -1;
    let mut slave_fd = -1;
    let mut winsize = winsize(size);
    if unsafe {
        libc::openpty(
            &raw mut master_fd,
            &raw mut slave_fd,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut winsize,
        )
    } == -1
    {
        return Err(io::Error::last_os_error().into());
    }
    let master = unsafe { File::from_raw_fd(master_fd) };
    let slave = unsafe { File::from_raw_fd(slave_fd) };
    set_fd_flags(master.as_raw_fd(), libc::O_NONBLOCK)?;
    set_fd_flags(slave.as_raw_fd(), 0)?;
    Ok((master, slave))
}

fn configure_child_terminal() -> io::Result<()> {
    unsafe {
        if libc::setsid() == -1 {
            return Err(io::Error::last_os_error());
        }
        if libc::ioctl(libc::STDIN_FILENO, TIOCSCTTY_REQUEST, 0) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.stop_and_reap();
    }
}

fn signal_process_group(group: libc::pid_t, signal: libc::c_int) -> Result {
    if unsafe { libc::kill(-group, signal) } == 0 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(error.into())
    }
}

pub(crate) struct Readiness {
    pub(crate) listener: bool,
    pub(crate) management_listener: bool,
    pub(crate) pty_read: bool,
    pub(crate) pty_write: bool,
    pub(crate) client: bool,
    pub(crate) management_client: bool,
}

pub(crate) fn poll(
    listener: &UnixListener,
    management_listener: Option<&UnixListener>,
    pty: Option<(&Pty, bool)>,
    client: Option<&UnixStream>,
    management_client: Option<(&UnixStream, bool)>,
) -> Result<Readiness> {
    let mut fds = [pollfd(0, 0); 5];
    let mut len = 0;
    let mut push = |fd, events| {
        let index = len;
        fds[index] = pollfd(fd, events);
        len += 1;
        index
    };
    let listener_index = push(listener.as_raw_fd(), libc::POLLIN);
    let management_listener_index =
        management_listener.map(|listener| push(listener.as_raw_fd(), libc::POLLIN));
    let pty_index = pty.map(|(pty, writable)| {
        push(
            pty.master.as_raw_fd(),
            libc::POLLIN | if writable { libc::POLLOUT } else { 0 },
        )
    });
    let client_index = client.map(|client| push(client.as_raw_fd(), libc::POLLIN));
    let management_client_index = management_client.map(|(client, writable)| {
        push(
            client.as_raw_fd(),
            libc::POLLIN | if writable { libc::POLLOUT } else { 0 },
        )
    });

    loop {
        let result = unsafe { libc::poll(fds.as_mut_ptr(), len as _, 100) };
        if result >= 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error.into());
        }
    }

    let events = |index: Option<usize>| index.map_or(0, |index| fds[index].revents);
    let pty = events(pty_index);
    let management = events(management_client_index);
    Ok(Readiness {
        listener: events(Some(listener_index)) & libc::POLLIN != 0,
        management_listener: events(management_listener_index) & libc::POLLIN != 0,
        pty_read: pty & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0,
        pty_write: pty & libc::POLLOUT != 0,
        client: events(client_index) & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0,
        management_client: management & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0,
    })
}

pub(crate) struct SocketGuard(PathBuf, Option<(u64, u64)>);

impl Drop for SocketGuard {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.0).is_ok_and(|metadata| {
            metadata.file_type().is_socket()
                && self.1.is_none_or(|(device, inode)| {
                    metadata.dev() == device && metadata.ino() == inode
                })
        }) {
            let _ = fs::remove_file(&self.0);
        }
    }
}

pub(crate) fn create_listener(path: &Path) -> Result<(UnixListener, SocketGuard)> {
    let parent = validate_private_parent(path)?;
    let claim = File::open(parent)?;
    // ponytail: one accepted Session makes the parent the smallest artifact-free claim;
    // use a per-socket lock only if concurrent Sessions in one directory are accepted.
    match claim.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(format!("socket claim already in progress at {}", path.display()).into());
        }
        Err(TryLockError::Error(error)) => return Err(error.into()),
    }
    if let Some(metadata) = existing_metadata(path)? {
        validate_object(path, &metadata, ObjectKind::Socket)?;
        match UnixStream::connect(path) {
            Ok(_) => return Err(format!("server already listening at {}", path.display()).into()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                ) =>
            {
                fs::remove_file(path)?;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let listener = UnixListener::bind(path)?;
    let mut guard = SocketGuard(path.to_owned(), None);
    let metadata = fs::symlink_metadata(path)?;
    guard.1 = Some((metadata.dev(), metadata.ino()));
    listener.set_nonblocking(true)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok((listener, guard))
}

pub(crate) fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(target_os = "linux")]
pub(crate) fn peer_uid(stream: &UnixStream) -> Result<u32> {
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
        return Err(io::Error::last_os_error().into());
    }
    if length as usize != std::mem::size_of::<libc::ucred>() {
        return Err("unexpected Unix peer credential size".into());
    }
    Ok(credentials.uid)
}

#[cfg(target_os = "macos")]
pub(crate) fn peer_uid(stream: &UnixStream) -> Result<u32> {
    let mut uid = 0;
    let mut gid = 0;
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &raw mut uid, &raw mut gid) } == -1 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(uid)
}

#[cfg(target_os = "linux")]
pub(crate) fn process_start_identity() -> Result<u64> {
    let stat = fs::read_to_string("/proc/self/stat")?;
    let fields = stat.rsplit_once(')').ok_or("malformed /proc/self/stat")?.1;
    fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| "missing process start identity".into())
        .and_then(|value| value.parse::<u64>().map_err(Into::into))
}

#[cfg(target_os = "macos")]
pub(crate) fn process_start_identity() -> Result<u64> {
    let pid = std::process::id();
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    if read == 0 {
        return Err(io::Error::last_os_error().into());
    }
    if read != size || info.pbi_pid != pid || info.pbi_start_tvusec >= 1_000_000 {
        return Err("invalid macOS process start identity".into());
    }
    info.pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .filter(|identity| *identity != 0)
        .ok_or_else(|| "invalid macOS process start identity".into())
}

pub(crate) fn endpoint_identity(path: &Path) -> Result<EndpointIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    validate_object(path, &metadata, ObjectKind::Socket)?;
    Ok(EndpointIdentity {
        path: path.as_os_str().as_bytes().to_vec(),
        object: object_identity(&metadata),
    })
}

pub(crate) fn endpoint_matches(expected: &EndpointIdentity) -> bool {
    endpoint_identity(Path::new(std::ffi::OsStr::from_bytes(&expected.path)))
        .is_ok_and(|current| current == *expected)
}

pub(crate) struct RecordGuard {
    path: PathBuf,
    identity: Option<ObjectIdentity>,
    contents: Vec<u8>,
}

impl RecordGuard {
    pub(crate) fn publish(path: &Path, bytes: &[u8]) -> Result<Self> {
        let mut ready = ready_claim(path)?;
        if let Some((claim, expected)) = &mut ready {
            claim.try_lock()?;
            if claim.metadata()?.len() != 0 || record_identity(path)? != *expected {
                return Err("management Ready claim changed before publication".into());
            }
            claim.write_all(b"1")?;
        }
        atomic_record_replace(path, bytes)?;
        Ok(Self {
            path: path.to_owned(),
            identity: Some(record_identity(path)?),
            contents: bytes.to_vec(),
        })
    }

    pub(crate) fn identity(&self) -> Result<ObjectIdentity> {
        let expected = self.identity.ok_or("management record is no longer live")?;
        if record_identity(&self.path)? != expected {
            return Err("management record identity changed".into());
        }
        let mut file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&self.path)?;
        let metadata = file.metadata()?;
        validate_object(&self.path, &metadata, ObjectKind::Regular)?;
        let identity = object_identity(&metadata);
        let mut contents = Vec::with_capacity(self.contents.len().saturating_add(1));
        Read::by_ref(&mut file)
            .take(self.contents.len().saturating_add(1) as u64)
            .read_to_end(&mut contents)?;
        if identity != expected
            || contents != self.contents
            || record_identity(&self.path)? != identity
        {
            return Err("management record identity changed".into());
        }
        Ok(identity)
    }

    pub(crate) fn replace_and_preserve(&mut self, bytes: &[u8]) -> Result {
        self.identity()?;
        atomic_record_replace(&self.path, bytes)?;
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

fn record_identity(path: &Path) -> Result<ObjectIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    validate_object(path, &metadata, ObjectKind::Regular)?;
    Ok(object_identity(&metadata))
}

fn ready_claim(path: &Path) -> Result<Option<(File, ObjectIdentity)>> {
    validate_private_parent(path)?;
    let file = match OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    validate_object(path, &metadata, ObjectKind::Regular)?;
    if metadata.len() != 0 {
        return Ok(None);
    }
    let identity = object_identity(&metadata);
    if record_identity(path)? != identity {
        return Err("management Ready claim changed while it was opened".into());
    }
    Ok(Some((file, identity)))
}

fn existing_metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn atomic_record_replace(path: &Path, bytes: &[u8]) -> Result {
    validate_private_parent(path)?;
    if let Some(metadata) = existing_metadata(path)? {
        validate_object(path, &metadata, ObjectKind::Regular)?;
    }
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(format!(
        ".tmp.{}.{}",
        std::process::id(),
        NEXT_ARTIFACT.fetch_add(1, Ordering::Relaxed)
    ));
    let temporary = PathBuf::from(temporary);
    let result = (|| -> Result {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&temporary)?;
        file.write_all(bytes)?;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
        validate_object(
            &temporary,
            &fs::symlink_metadata(&temporary)?,
            ObjectKind::Regular,
        )?;
        fs::rename(&temporary, path)?;
        validate_object(path, &fs::symlink_metadata(path)?, ObjectKind::Regular)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[derive(Clone, Copy)]
enum ObjectKind {
    Socket,
    Regular,
}

fn validate_object(path: &Path, metadata: &fs::Metadata, kind: ObjectKind) -> Result {
    let type_matches = match kind {
        ObjectKind::Socket => metadata.file_type().is_socket(),
        ObjectKind::Regular => metadata.file_type().is_file(),
    };
    if !type_matches || metadata.uid() != current_uid() || metadata.mode() & 0o7777 != 0o600 {
        return Err(format!(
            "{} must be an exact user-owned 0600 {}",
            path.display(),
            match kind {
                ObjectKind::Socket => "socket",
                ObjectKind::Regular => "regular file",
            }
        )
        .into());
    }
    Ok(())
}

fn object_identity(metadata: &fs::Metadata) -> ObjectIdentity {
    ObjectIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

pub(crate) fn default_socket_path() -> Result<PathBuf> {
    let euid = current_uid();
    let directory = env::var_os("XDG_RUNTIME_DIR").map_or_else(
        || PathBuf::from(format!("/tmp/yazelix-orbit-{euid}")),
        |root| PathBuf::from(root).join("yazelix-orbit"),
    );
    if !directory.exists() {
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
    }
    let metadata = fs::symlink_metadata(&directory)?;
    if !is_private_directory(&metadata, euid) {
        return Err(format!("runtime directory {} is not private", directory.display()).into());
    }
    Ok(directory.join("orbit.sock"))
}

pub(crate) fn install_shutdown_signals() -> Result {
    set_signal_handler(
        &[libc::SIGINT, libc::SIGTERM, libc::SIGHUP],
        terminate as *const () as usize,
    )
}

pub(crate) fn ignore_broken_pipe() -> Result {
    set_signal_handler(&[libc::SIGPIPE], libc::SIG_IGN)
}

pub(crate) fn termination_requested() -> bool {
    TERMINATE.load(Ordering::Relaxed)
}

fn winsize(size: SurfaceSize) -> libc::winsize {
    libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: u16::try_from(size.screen_width).unwrap_or(u16::MAX),
        ws_ypixel: u16::try_from(size.screen_height).unwrap_or(u16::MAX),
    }
}

fn nonblocking(mut operation: impl FnMut() -> io::Result<usize>, retry_eio: bool) -> Result<PtyIo> {
    loop {
        match operation() {
            Ok(0) => return Ok(PtyIo::Closed),
            Ok(count) => return Ok(PtyIo::Ready(count)),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(PtyIo::Blocked);
            }
            Err(error) if retry_eio && error.raw_os_error() == Some(libc::EIO) => {
                thread::sleep(PTY_EIO_RETRY_DELAY);
                return Ok(PtyIo::Blocked);
            }
            Err(error) if error.raw_os_error() == Some(libc::EIO) => {
                return Ok(PtyIo::Closed);
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn pollfd(fd: RawFd, events: libc::c_short) -> libc::pollfd {
    libc::pollfd {
        fd,
        events,
        revents: 0,
    }
}

fn validate_private_parent(path: &Path) -> Result<&Path> {
    let parent = path.parent().ok_or("artifact path has no parent")?;
    let metadata = fs::symlink_metadata(parent)?;
    let euid = current_uid();
    if !is_private_directory(&metadata, euid) {
        return Err(format!(
            "artifact parent {} must be a user-owned 0700 directory",
            parent.display()
        )
        .into());
    }
    Ok(parent)
}

fn is_private_directory(metadata: &fs::Metadata, uid: u32) -> bool {
    metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o7777 == 0o700
}

fn set_fd_flags(fd: RawFd, status: libc::c_int) -> Result {
    let old_status = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if old_status == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, old_status | status) } == -1 {
        return Err(io::Error::last_os_error().into());
    }
    let old_descriptor = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if old_descriptor == -1
        || unsafe { libc::fcntl(fd, libc::F_SETFD, old_descriptor | libc::FD_CLOEXEC) } == -1
    {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

extern "C" fn terminate(_: libc::c_int) {
    TERMINATE.store(true, Ordering::Relaxed);
}

fn set_signal_handler(signals: &[libc::c_int], handler: usize) -> Result {
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = handler;
    unsafe {
        libc::sigemptyset(&mut action.sa_mask);
    }
    for &signal in signals {
        if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } == -1 {
            return Err(io::Error::last_os_error().into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(name: &str) -> Result<Self> {
            let path = env::temp_dir().join(format!(
                "orbit-platform-{name}-{}-{}",
                std::process::id(),
                NEXT_ARTIFACT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
            Ok(Self(path))
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn winsize_preserves_surface_pixels_beyond_the_cell_grid() {
        let size = SurfaceSize {
            cols: 100,
            rows: 40,
            cell_width: 9,
            cell_height: 18,
            screen_width: 920,
            screen_height: 740,
            padding_top: 10,
            padding_bottom: 10,
            padding_left: 10,
            padding_right: 10,
        };

        let size = winsize(size);

        assert_eq!((size.ws_col, size.ws_row), (100, 40));
        assert_eq!((size.ws_xpixel, size.ws_ypixel), (920, 740));
    }

    #[test]
    fn management_ready_claim_has_one_winner_and_survives_record_removal() -> Result {
        let directory = TestDir::new("ready-claim")?;
        let record_path = directory.0.join("orbit.record");
        let claim = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(&record_path)?;
        claim.lock()?;
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(0);
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let contested_path = record_path.clone();
        let publisher = thread::spawn(move || {
            started_tx.send(()).expect("test receiver remains live");
            let result = match RecordGuard::publish(&contested_path, b"live") {
                Ok(_) => Err("held Ready claim did not win".to_owned()),
                Err(error)
                    if matches!(
                        error.downcast_ref::<TryLockError>(),
                        Some(TryLockError::WouldBlock)
                    ) =>
                {
                    Ok(())
                }
                Err(error) => Err(format!("unexpected Ready claim error: {error}")),
            };
            let _ = finished_tx.send(result);
        });
        started_rx.recv()?;
        let contested = finished_rx.recv_timeout(Duration::from_secs(5));
        claim.unlock()?;
        publisher
            .join()
            .map_err(|_| "Ready claim publisher panicked")?;
        let contested = contested
            .map_err(|error| format!("Ready claim publication did not fail promptly: {error}"))?;
        contested?;
        assert!(fs::read(&record_path)?.is_empty());
        let record = RecordGuard::publish(&record_path, b"live")?;
        claim.try_lock()?;
        let live_path = record.path.clone();
        assert_eq!(fs::read(&live_path)?, b"live");
        drop(record);
        assert!(!live_path.exists());
        assert_ne!(claim.metadata()?.len(), 0);
        Ok(())
    }

    #[test]
    fn management_artifacts_and_kernel_identity_fail_closed() -> Result {
        let directory = TestDir::new("management")?;
        let record_path = directory.0.join("orbit.record");
        let record = RecordGuard::publish(&record_path, b"live")?;
        let original = record.identity()?;
        let metadata = fs::symlink_metadata(&record_path)?;
        assert!(metadata.file_type().is_file());
        assert_eq!(metadata.mode() & 0o7777, 0o600);
        assert_eq!(metadata.uid(), current_uid());

        fs::write(&record_path, b"truncated")?;
        assert!(record.identity().is_err());
        fs::write(&record_path, b"live")?;
        assert_eq!(record.identity()?, original);

        let fifo_path = directory.0.join("fifo-record");
        let fifo = RecordGuard::publish(&fifo_path, b"live")?;
        fs::remove_file(&fifo_path)?;
        let fifo_name = std::ffi::CString::new(fifo_path.as_os_str().as_bytes())?;
        assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);
        assert!(fifo.identity().is_err());
        drop(fifo);
        assert!(fs::symlink_metadata(&fifo_path)?.file_type().is_fifo());
        fs::remove_file(&fifo_path)?;

        let replacement = directory.0.join("replacement");
        fs::write(&replacement, b"replacement")?;
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o600))?;
        fs::rename(&replacement, &record_path)?;
        assert_ne!(record_identity(&record_path)?, original);
        assert!(record.identity().is_err());
        drop(record);
        assert_eq!(fs::read(&record_path)?, b"replacement");

        fs::set_permissions(&record_path, fs::Permissions::from_mode(0o4600))?;
        assert_eq!(fs::symlink_metadata(&record_path)?.mode() & 0o7777, 0o4600);
        assert!(RecordGuard::publish(&record_path, b"wrong mode").is_err());
        fs::remove_file(&record_path)?;
        std::os::unix::fs::symlink("target", &record_path)?;
        assert!(RecordGuard::publish(&record_path, b"symlink").is_err());
        fs::remove_file(&record_path)?;
        fs::create_dir(&record_path)?;
        assert!(RecordGuard::publish(&record_path, b"directory").is_err());
        fs::remove_dir(&record_path)?;

        let wrong_mode = directory.0.join("wrong-mode");
        fs::create_dir(&wrong_mode)?;
        fs::set_permissions(&wrong_mode, fs::Permissions::from_mode(0o1700))?;
        assert_eq!(fs::symlink_metadata(&wrong_mode)?.mode() & 0o7777, 0o1700);
        assert!(RecordGuard::publish(&wrong_mode.join("record"), b"mode").is_err());
        let private = directory.0.join("private");
        fs::create_dir(&private)?;
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700))?;
        let target_record = private.join("record");
        fs::write(&target_record, b"")?;
        fs::set_permissions(&target_record, fs::Permissions::from_mode(0o600))?;
        let linked = directory.0.join("linked");
        std::os::unix::fs::symlink(&private, &linked)?;
        assert!(RecordGuard::publish(&linked.join("record"), b"link").is_err());
        assert!(fs::read(&target_record)?.is_empty());

        let socket_path = directory.0.join("orbit.sock");
        let (listener, guard) = create_listener(&socket_path)?;
        let endpoint = endpoint_identity(&socket_path)?;
        assert_eq!(endpoint.path, socket_path.as_os_str().as_bytes());
        let client = UnixStream::connect(&socket_path)?;
        let (server, _) = listener.accept()?;
        assert_eq!(peer_uid(&server)?, current_uid());
        assert_eq!(peer_uid(&client)?, current_uid());
        assert!(process_start_identity()? > 0);
        drop((server, client, listener, guard));

        let stale = UnixListener::bind(&socket_path)?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o640))?;
        drop(stale);
        assert!(create_listener(&socket_path).is_err());
        assert!(socket_path.exists(), "untrusted stale socket was removed");
        Ok(())
    }
}
