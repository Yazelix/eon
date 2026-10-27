//! The ORBS v14 client and server harness the real-PTY checks share, on the
//! canonical `orbit-protocol` codec.

use orbit_protocol::{
    Frame,
    session::{
        self, ClientMessage, FailureCode, KeyAction, KeyEvent, Metadata, Modifiers, MouseAction,
        MouseButton, MouseEvent, PhysicalKey, PreviewOutcome, ScrollOutcome, SelectionAction,
        SelectionPosition, ServerMessage, VerticalDirection, WheelOutcome,
    },
};
use std::{
    collections::HashMap,
    env,
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, PermissionsExt, symlink},
        io::{AsRawFd, FromRawFd},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// The longest a check may run, past the longest nix/orbit-checks.nix timeout.
const WATCHDOG: Duration = Duration::from_secs(180);

/// The processes the checks started and have not reaped.
fn running() -> MutexGuard<'static, Vec<i32>> {
    static RUNNING: Mutex<Vec<i32>> = Mutex::new(Vec::new());
    RUNNING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Runs one check at a time, so checks that measure time never share the
/// machine with another session even when libtest runs tests in parallel.
/// A check that hangs past the watchdog terminates the servers it started
/// and ends the test process.
pub fn serial(name: &'static str, check: fn()) {
    static SERIAL: Mutex<()> = Mutex::new(());
    let _turn = SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        env::var_os("ORBIT_BIN").is_some(),
        "the real-PTY checks run the Orbit at ORBIT_BIN"
    );
    let (finished, watched) = mpsc::channel::<()>();
    thread::spawn(move || {
        if watched.recv_timeout(WATCHDOG) == Err(mpsc::RecvTimeoutError::Timeout) {
            eprintln!("{name} did not finish within {} s", WATCHDOG.as_secs());
            for &pid in running().iter() {
                // SAFETY: sending a signal has no memory effects.
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
            std::process::exit(124);
        }
    });
    check();
    drop(finished);
}

pub fn seconds(value: f64) -> Duration {
    Duration::from_secs_f64(value)
}

pub fn sleep(value: f64) {
    thread::sleep(seconds(value));
}

/// A private temporary directory, removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        loop {
            let name = format!(
                "borb-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = env::temp_dir().join(name);
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("{}: {error}", path.display()),
            }
        }
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The bash that runs child scripts: `ORBIT_BASH`, or the first on PATH.
pub fn bash() -> PathBuf {
    if let Some(path) = env::var_os("ORBIT_BASH") {
        return path.into();
    }
    env::split_paths(&env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join("bash"))
        .find(|path| path.is_file())
        .expect("bash on PATH")
}

/// Writes an executable script into `directory`.
pub fn executable(directory: &Path, name: &str, text: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).expect("chmod");
    path
}

/// The child script tests/NAME.sh: the path in `variable`, as nix/orbit-checks.nix sets
/// it, or else the source tree's copy written into `directory` with a bash
/// shebang, as `writeShellScript` makes it.
pub fn script(directory: &Path, variable: &str, name: &str) -> PathBuf {
    if let Some(path) = env::var_os(variable) {
        return path.into();
    }
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(format!("{name}.sh"));
    let text =
        fs::read_to_string(&source).unwrap_or_else(|error| panic!("{}: {error}", source.display()));
    executable(directory, name, &format!("#!{}\n{text}", bash().display()))
}

/// The named answers a child script recorded, one `NAME REPLY` per line.
pub fn replies(path: &Path) -> HashMap<String, String> {
    fs::read_to_string(path)
        .expect("replies")
        .lines()
        .map(|line| {
            let (name, reply) = line.split_once(' ').expect("a named reply");
            (name.to_string(), reply.to_string())
        })
        .collect()
}

/// Waits up to ten seconds for a path to exist.
pub fn wait_for(path: &Path) {
    let deadline = Instant::now() + seconds(10.0);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "{} never appeared",
            path.display()
        );
        sleep(0.02);
    }
}

pub fn read_pid(path: &Path) -> i32 {
    let text = fs::read_to_string(path).expect("PID file");
    text.trim().parse().expect("a PID")
}

/// A FIFO the child blocks on until the check releases it.
pub struct Gate(PathBuf);

impl Gate {
    pub fn new(directory: &Path, name: &str) -> Self {
        let path = directory.join(name);
        let raw = std::ffi::CString::new(path.as_os_str().as_bytes()).expect("path");
        // SAFETY: `raw` is a NUL-terminated path that outlives the call.
        let made = unsafe { libc::mkfifo(raw.as_ptr(), 0o600) };
        assert_eq!(made, 0, "mkfifo {}", path.display());
        Self(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    pub fn release(&self) {
        let mut opened = fs::OpenOptions::new()
            .write(true)
            .open(&self.0)
            .expect("open gate");
        opened.write_all(b"go\n").expect("release gate");
    }
}

pub fn encode(message: &ClientMessage) -> Vec<u8> {
    session::encode_client_message(message).expect("encodable message")
}

pub fn paste(data: &[u8]) -> ClientMessage {
    ClientMessage::Paste(data.to_vec())
}

pub fn scroll(frame_revision: u64, rows: i16) -> ClientMessage {
    ClientMessage::ScrollVertical {
        frame_revision,
        rows,
    }
}

pub fn preview(frame_revision: u64, direction: VerticalDirection) -> ClientMessage {
    ClientMessage::PreviewVertical {
        frame_revision,
        direction,
    }
}

pub fn position(x: f32, y: f32) -> SelectionPosition {
    SelectionPosition { x, y }
}

/// An upward selection tick with no host gesture, which Bend answers with
/// Failure(InvalidInput) without moving anything or ending the attachment.
pub fn idle_tick() -> ClientMessage {
    ClientMessage::Selection(SelectionAction::AutoscrollUp {
        position: position(5.0, 9.0),
    })
}

/// A Key event for one lowercase ASCII letter, with no modifiers.
pub fn key(letter: char, action: KeyAction) -> ClientMessage {
    let code = PhysicalKey::A.raw() + (letter as u16 - 'a' as u16);
    ClientMessage::Key(KeyEvent {
        action,
        key: PhysicalKey::from_raw(code).expect("a letter key"),
        modifiers: Modifiers::empty(),
        consumed_modifiers: Modifiers::empty(),
        composing: false,
        text: Some(letter.to_string()),
        unshifted_codepoint: Some(letter),
    })
}

/// A Mouse event with no modifiers in column 3, row 2 of the 10x18-pixel cells.
pub fn mouse(action: MouseAction, button: Option<MouseButton>) -> ClientMessage {
    ClientMessage::Mouse(MouseEvent {
        action,
        button,
        modifiers: Modifiers::empty(),
        x: 25.0,
        y: 20.0,
    })
}

/// Whether a frame names this text in its title or working directory, where
/// the checks' children write it.
pub fn mentions(frame: &Frame, text: &str) -> bool {
    frame.title.contains(text) || frame.working_directory.contains(text)
}

/// Whether the frame presents a host selection.
pub fn selected(frame: &Frame) -> bool {
    frame
        .rows
        .iter()
        .flat_map(|row| &row.cells)
        .any(|cell| cell.style.selected)
}

/// A server message in one line, without a frame's rows.
pub fn brief(message: &ServerMessage) -> String {
    fn frame(frame: &Frame) -> String {
        format!(
            "frame revision {} title {:?} rows_from_live {}",
            frame.revision, frame.title, frame.scroll_position.rows_from_live
        )
    }
    match message {
        ServerMessage::Frame(current) => frame(current),
        ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows,
            frame: current,
        }) => format!("wheel {applied_rows} with {}", frame(current)),
        ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
            requested_rows,
            applied_rows,
            frame: current,
            ..
        }) => format!(
            "scroll {requested_rows} applied {applied_rows} with {}",
            frame(current)
        ),
        ServerMessage::VerticalPreview(preview) => format!(
            "preview {:?} at {} {}",
            preview.direction,
            preview.frame_revision,
            match &preview.outcome {
                PreviewOutcome::TerminalRouted => "terminal routed".to_string(),
                PreviewOutcome::Viewport {
                    edge_reached, rows, ..
                } => format!("edge {edge_reached} rows {}", rows.len()),
            }
        ),
        other => {
            let mut text = format!("{other:?}");
            text.truncate(200);
            text
        }
    }
}

/// The whole server messages at the start of `data`, decoded, and the length
/// of a cut message after them.
pub fn split(data: &[u8]) -> (Vec<ServerMessage>, usize) {
    let mut messages = Vec::new();
    let mut offset = 0;
    while let Some(length) =
        session::server_message_len(&data[offset..]).expect("a valid ORBS header")
    {
        if data.len() - offset < length {
            break;
        }
        let message = session::decode_server_message(&data[offset..offset + length])
            .expect("a canonical server message");
        messages.push(message);
        offset += length;
    }
    (messages, data.len() - offset)
}

/// One client socket, read through one buffer of whole ORBS messages.
pub struct Peer {
    pub stream: UnixStream,
    buffer: Vec<u8>,
    start: usize,
    scratch: Vec<u8>,
}

impl Peer {
    fn new(stream: UnixStream, timeout: f64) -> Self {
        let mut peer = Self {
            stream,
            buffer: Vec::new(),
            start: 0,
            scratch: vec![0; 1 << 20],
        };
        peer.set_timeout(timeout);
        peer
    }

    pub fn connect(path: &Path, timeout: f64) -> Self {
        let stream = UnixStream::connect(path)
            .unwrap_or_else(|error| panic!("connect {}: {error}", path.display()));
        Self::new(stream, timeout)
    }

    /// Connects with this receive buffer, set before the connection as a
    /// client that sizes its socket does.
    pub fn connect_with_receive_buffer(path: &Path, bytes: libc::c_int, timeout: f64) -> Self {
        // SAFETY: the descriptor is fresh and owned by the stream from here on;
        // the address is zeroed and filled within its bounds.
        unsafe {
            let fd = libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0);
            assert!(fd >= 0, "socket: {}", io::Error::last_os_error());
            let stream = UnixStream::from_raw_fd(fd);
            let set = libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                (&bytes as *const libc::c_int).cast(),
                size_of::<libc::c_int>() as libc::socklen_t,
            );
            assert_eq!(set, 0, "SO_RCVBUF: {}", io::Error::last_os_error());
            let mut address: libc::sockaddr_un = std::mem::zeroed();
            address.sun_family = libc::AF_UNIX as libc::sa_family_t;
            let raw = path.as_os_str().as_bytes();
            assert!(raw.len() < address.sun_path.len(), "socket path too long");
            for (slot, byte) in address.sun_path.iter_mut().zip(raw) {
                *slot = *byte as libc::c_char;
            }
            let connected = libc::connect(
                fd,
                (&address as *const libc::sockaddr_un).cast(),
                size_of::<libc::sockaddr_un>() as libc::socklen_t,
            );
            assert_eq!(connected, 0, "connect: {}", io::Error::last_os_error());
            Self::new(stream, timeout)
        }
    }

    /// Connects and negotiates the interactive role; the initial frame is unread.
    ///
    /// A previous client that just closed may hold the slot until the server
    /// reads its close, so Busy is retried for two seconds.
    pub fn attach(path: &Path, timeout: f64) -> Self {
        let deadline = Instant::now() + seconds(2.0);
        loop {
            let mut peer = Self::connect(path, timeout);
            peer.send(&ClientMessage::Hello);
            let answer = peer.receive();
            if answer == ServerMessage::Busy && Instant::now() < deadline {
                drop(peer);
                sleep(0.02);
                continue;
            }
            assert_eq!(answer, ServerMessage::Attached, "attach");
            return peer;
        }
    }

    /// Sets the read and write timeouts. macOS refuses socket options on a
    /// connection the server has closed, whose reads then end at once, so a
    /// refusal is ignored.
    pub fn set_timeout(&mut self, timeout: f64) {
        let _ = self.stream.set_read_timeout(Some(seconds(timeout)));
        let _ = self.stream.set_write_timeout(Some(seconds(timeout)));
    }

    pub fn send(&mut self, message: &ClientMessage) {
        self.send_bytes(&encode(message));
    }

    pub fn send_bytes(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).expect("send");
    }

    pub fn shutdown_write(&self) {
        self.stream
            .shutdown(std::net::Shutdown::Write)
            .expect("shutdown");
    }

    fn pending(&self) -> &[u8] {
        &self.buffer[self.start..]
    }

    /// Reads once into the buffer: the number of bytes, 0 at end of stream.
    fn fill(&mut self) -> io::Result<usize> {
        if self.start > 0 {
            self.buffer.drain(..self.start);
            self.start = 0;
        }
        loop {
            match self.stream.read(&mut self.scratch) {
                Ok(read) => {
                    self.buffer.extend_from_slice(&self.scratch[..read]);
                    return Ok(read);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }

    /// The next whole message, or None if the read timed out first.
    pub fn receive_within(&mut self) -> Option<ServerMessage> {
        loop {
            let pending = self.pending();
            if let Some(length) = session::server_message_len(pending).expect("a valid ORBS header")
                && pending.len() >= length
            {
                let message = session::decode_server_message(&pending[..length])
                    .expect("a canonical server message");
                self.start += length;
                return Some(message);
            }
            match self.fill() {
                Ok(0) => panic!("peer closed before a complete ORBS message"),
                Ok(_) => {}
                Err(error) if timed_out(&error) => return None,
                Err(error) => panic!("receive: {error}"),
            }
        }
    }

    /// Reads one whole ORBS v14 message.
    pub fn receive(&mut self) -> ServerMessage {
        self.receive_within()
            .expect("timed out waiting for a message")
    }

    pub fn expect(&mut self, expected: ServerMessage) {
        let message = self.receive();
        assert!(
            message == expected,
            "expected {}, received {}",
            brief(&expected),
            brief(&message)
        );
    }

    /// Reads one message, which must be a frame.
    pub fn frame(&mut self) -> Frame {
        match self.receive() {
            ServerMessage::Frame(frame) => *frame,
            other => panic!("expected a frame, received {}", brief(&other)),
        }
    }

    /// Reads frames until one has exactly this title.
    pub fn frame_titled(&mut self, title: &str) -> Frame {
        loop {
            let current = self.frame();
            if current.title == title {
                return current;
            }
        }
    }

    /// Reads frames until one mentions this text.
    pub fn frame_with(&mut self, text: &str) -> Frame {
        loop {
            let current = self.frame();
            if mentions(&current, text) {
                return current;
            }
        }
    }

    /// Reads one message, which must be Metadata.
    pub fn metadata(&mut self) -> Metadata {
        match self.receive() {
            ServerMessage::Metadata(metadata) => metadata,
            other => panic!("expected metadata, received {}", brief(&other)),
        }
    }

    pub fn failure(&mut self, code: FailureCode) {
        match self.receive() {
            ServerMessage::Failure(failure) if failure.code == code => {}
            other => panic!("expected Failure({code:?}), received {}", brief(&other)),
        }
    }

    /// Reads a ScrollOutcome::Viewport; returns requested, applied and its frame.
    pub fn scroll_outcome(&mut self) -> (i16, i16, Frame) {
        match self.receive() {
            ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
                requested_rows,
                applied_rows,
                frame,
                ..
            }) => (requested_rows, applied_rows, *frame),
            other => panic!("expected a scroll outcome, received {}", brief(&other)),
        }
    }

    /// Asserts that nothing arrives for a while, then sets ten-second timeouts.
    pub fn quiet(&mut self, timeout: f64, what: &str) {
        self.quiet_then(timeout, what, 10.0);
    }

    /// Asserts that nothing arrives for a while, then sets `restore` timeouts.
    pub fn quiet_then(&mut self, timeout: f64, what: &str, restore: f64) {
        assert!(self.pending().is_empty(), "{what}: a message had arrived");
        let _ = self.stream.set_read_timeout(Some(seconds(timeout)));
        let read = self.fill();
        self.set_timeout(restore);
        match read {
            Err(error) if timed_out(&error) => {}
            Ok(0) => panic!("{what}: peer closed before a complete ORBS message"),
            Ok(_) => panic!("{what}: unexpected {}", brief(&self.receive())),
            Err(error) => panic!("{what}: {error}"),
        }
    }

    /// Reads frames until Exited with this code; returns how many frames came.
    pub fn exited(&mut self, code: i32) -> usize {
        let mut frames = 0;
        loop {
            match self.receive() {
                ServerMessage::Exited { code: received } => {
                    assert_eq!(received, code, "exit code");
                    return frames;
                }
                ServerMessage::Frame(_) => frames += 1,
                other => panic!("expected a frame or Exited, received {}", brief(&other)),
            }
        }
    }

    /// Asserts that the next thing the peer reads is the end of the stream.
    pub fn expect_closed(&mut self) {
        assert!(
            self.pending().is_empty(),
            "data arrived before the end of the stream"
        );
        match self.fill() {
            Ok(0) => {}
            Ok(read) => panic!("{read} bytes arrived before the end of the stream"),
            Err(error) => panic!("expected the end of the stream: {error}"),
        }
    }

    /// Discards everything until the end of the stream.
    pub fn drain(&mut self) {
        while self.fill().expect("drain") > 0 {}
        self.buffer.clear();
        self.start = 0;
    }

    /// Everything unread until the end of the stream, or until a reset when
    /// `reset_ends` holds; also returns whether a reset ended it.
    pub fn read_until_closed(&mut self, reset_ends: bool) -> (Vec<u8>, bool) {
        let reset = loop {
            match self.fill() {
                Ok(0) => break false,
                Ok(_) => {}
                Err(error) if reset_ends && error.kind() == io::ErrorKind::ConnectionReset => {
                    break true;
                }
                Err(error) => panic!("read until closed: {error}"),
            }
        };
        let data = self.pending().to_vec();
        self.buffer.clear();
        self.start = 0;
        (data, reset)
    }

    /// Peeks at the socket for one byte without waiting: 1, or 0 at the end
    /// of the stream.
    fn peek(&self) -> io::Result<usize> {
        let mut byte = 0u8;
        loop {
            // SAFETY: the buffer is one writable byte that outlives the call.
            let read = unsafe {
                libc::recv(
                    self.stream.as_raw_fd(),
                    (&mut byte as *mut u8).cast(),
                    1,
                    libc::MSG_PEEK | libc::MSG_DONTWAIT,
                )
            };
            if read >= 0 {
                return Ok(read as usize);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }

    /// Asserts that the peer is still open: a byte waits to be read, or none
    /// arrives within the timeout, but the stream has not ended. It polls
    /// rather than peek with a read timeout, which macOS does not apply to a
    /// peek.
    pub fn expect_open(&mut self, timeout: f64, what: &str) {
        if !self.pending().is_empty() {
            return;
        }
        let mut ready = libc::pollfd {
            fd: self.stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for the duration of the call.
        match unsafe { libc::poll(&mut ready, 1, (timeout * 1000.0) as libc::c_int) } {
            0 => {}
            1 => match self.peek() {
                Ok(0) => panic!("{what}"),
                Ok(_) => {}
                Err(error) => panic!("{what}: {error}"),
            },
            _ => panic!("poll: {}", io::Error::last_os_error()),
        }
    }

    /// Asserts that nothing waits to be read and the stream has not ended,
    /// without waiting.
    pub fn expect_nothing_yet(&mut self, what: &str) {
        assert!(self.pending().is_empty(), "{what}");
        match self.peek() {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Ok(_) => panic!("{what}"),
            Err(error) => panic!("{what}: {error}"),
        }
    }
}

fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Whether a process exists, a zombie included.
pub fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only checks that the process exists.
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn signal(pid: i32, signal: libc::c_int) {
    // SAFETY: sending a signal has no memory effects.
    assert_eq!(unsafe { libc::kill(pid, signal) }, 0, "kill {pid}");
}

/// The process IDs of a process's children.
pub fn children(pid: i32) -> Vec<i32> {
    let listed = Command::new("pgrep")
        .arg("-P")
        .arg(pid.to_string())
        .output()
        .expect("pgrep");
    String::from_utf8_lossy(&listed.stdout)
        .split_whitespace()
        .map(|child| child.parse().expect("a PID"))
        .collect()
}

/// A process's command line, or an empty string once it is gone.
pub fn command(pid: i32) -> String {
    let listed = Command::new("ps")
        .args(["-o", "command=", "-p"])
        .arg(pid.to_string())
        .output()
        .expect("ps");
    String::from_utf8_lossy(&listed.stdout).trim().to_string()
}

/// The server's PTY child once it is spawned: its child other than the Bend
/// worker.
fn pty_child(server: i32) -> Option<i32> {
    children(server).into_iter().find(|&child| {
        let line = command(child);
        !line.is_empty() && !line.contains("yazelix-orbit-worker")
    })
}

/// Orbit's command line for one session, with `options`, such as
/// `--management-v3` and its identities, before the command.
pub fn server_command(path: &Path, command: &[&OsStr], options: &[&str]) -> Vec<OsString> {
    let binary = env::var_os("ORBIT_BIN").expect("ORBIT_BIN");
    let mut line = vec![binary, "serve".into(), path.into()];
    line.extend(options.iter().map(OsString::from));
    line.push("--".into());
    line.extend(command.iter().map(OsString::from));
    line
}

/// A finished process: its status and everything it printed.
pub struct Finished {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl Finished {
    pub fn code(&self) -> Option<i32> {
        self.status.code()
    }

    /// Asserts the code and that nothing was printed.
    pub fn assert_silent(&self, code: i32) {
        assert!(
            self.code() == Some(code) && self.stdout.is_empty() && self.stderr.is_empty(),
            "status {:?}, stdout {:?}, stderr {:?}; expected {code} and no output",
            self.status,
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr),
        );
    }

    /// Asserts status 1 and an `orbit: ` message.
    pub fn assert_refused(&self) {
        assert!(
            self.code() == Some(1) && self.stderr.starts_with(b"orbit: "),
            "status {:?}, stderr {:?}; expected a refusal",
            self.status,
            String::from_utf8_lossy(&self.stderr),
        );
    }
}

/// The threads collecting a process's standard output and error.
type Output = (JoinHandle<Vec<u8>>, JoinHandle<Vec<u8>>);

/// A child process whose output is collected as it runs.
struct Process {
    child: Child,
    output: Option<Output>,
}

impl Process {
    fn spawn(line: &[OsString], env: &[(&str, &Path)]) -> Self {
        let mut child = Command::new(&line[0])
            .args(&line[1..])
            .envs(env.iter().map(|(name, value)| (name, value)))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {:?}: {error}", line[0]));
        let collect = |mut pipe: Box<dyn Read + Send>| {
            thread::spawn(move || {
                let mut data = Vec::new();
                let _ = pipe.read_to_end(&mut data);
                data
            })
        };
        let stdout = collect(Box::new(child.stdout.take().expect("stdout")));
        let stderr = collect(Box::new(child.stderr.take().expect("stderr")));
        running().push(child.id() as i32);
        Self {
            child,
            output: Some((stdout, stderr)),
        }
    }

    fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn reaped(&self) {
        let pid = self.pid();
        running().retain(|&running| running != pid);
    }

    /// Waits for the process and the end of its output, or None at the deadline.
    fn wait_until(&mut self, deadline: Instant) -> Option<Finished> {
        loop {
            let finished = self
                .output
                .as_ref()
                .is_some_and(|(stdout, stderr)| stdout.is_finished() && stderr.is_finished());
            if finished && let Some(status) = self.child.try_wait().expect("wait") {
                self.reaped();
                let (stdout, stderr) = self.output.take().expect("output");
                return Some(Finished {
                    status,
                    stdout: stdout.join().expect("stdout"),
                    stderr: stderr.join().expect("stderr"),
                });
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Waits for the process to exit and close its output.
    fn communicate(&mut self, timeout: f64) -> Finished {
        self.wait_until(Instant::now() + seconds(timeout))
            .unwrap_or_else(|| panic!("the process did not finish within {timeout} s"))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if self.output.is_some() && self.running() {
            // SAFETY: the child is ours and not yet reaped.
            unsafe { libc::kill(self.pid(), libc::SIGTERM) };
            if self.wait_until(Instant::now() + seconds(10.0)).is_none() {
                let _ = self.child.kill();
            }
        }
        let _ = self.child.wait();
        self.reaped();
    }
}

/// Runs a command to completion, with its output collected.
pub fn run_command(line: &[OsString], env: &[(&str, &Path)], timeout: f64) -> Finished {
    Process::spawn(line, env).communicate(timeout)
}

/// Runs a copy of the host under test in `directory/bin`, with the
/// `yazelix-orbit-worker` the caller put in `directory` linked where the host
/// finds its package's worker. Returns how the host finished, once it neither
/// bound the socket nor started the child.
pub fn run_unopened(directory: &Path) -> Finished {
    let libexec = directory.join("libexec/yazelix-orbit");
    fs::create_dir_all(&libexec).expect("create libexec");
    symlink(
        directory.join("yazelix-orbit-worker"),
        libexec.join("yazelix-orbit-worker"),
    )
    .expect("link worker");
    fs::create_dir(directory.join("bin")).expect("create bin");
    let host = directory.join("bin/yazelix-orbit");
    fs::copy(env::var_os("ORBIT_BIN").expect("ORBIT_BIN"), &host).expect("copy host");
    let socket = directory.join("orbit.sock");
    let started = directory.join("started");
    let mut line = server_command(
        &socket,
        &[
            OsStr::new("/bin/sh"),
            OsStr::new("-c"),
            OsStr::new(": >\"$0\""),
            started.as_os_str(),
        ],
        &[],
    );
    line[0] = host.into();
    let finished = run_command(&line, &[], 10.0);
    assert!(!socket.exists(), "the socket was bound");
    assert!(!started.exists(), "the child started");
    finished
}

/// The Orbit server on a private socket in a directory, running a
/// command as its PTY child through Orbit's command line.
pub struct Server {
    pub path: PathBuf,
    pub child: i32,
    process: Process,
}

impl Server {
    /// Starts a session whose child is `shell`; `env` adds child environment.
    pub fn start(directory: &Path, shell: &Path, env: &[(&str, &Path)]) -> Self {
        Self::launch(directory, &[shell.as_os_str()], env, &[], &[])
    }

    /// Starts a session; `wait` lists files, besides the socket, that must
    /// exist before the session counts as open, and `options` precede `--`.
    pub fn launch(
        directory: &Path,
        command: &[&OsStr],
        env: &[(&str, &Path)],
        wait: &[&Path],
        options: &[&str],
    ) -> Self {
        let path = directory.join("orbit.sock");
        let line = server_command(&path, command, options);
        Self::open(path, &line, env, wait)
    }

    /// Starts a session on a command line that binds `path`.
    pub fn open(path: PathBuf, line: &[OsString], env: &[(&str, &Path)], wait: &[&Path]) -> Self {
        let mut process = Process::spawn(line, env);
        let mut child = None;
        for _ in 0..200 {
            if child.is_none() {
                child = pty_child(process.pid());
            }
            if let Some(child) = child
                && path.exists()
                && wait.iter().all(|file| file.exists())
            {
                return Self {
                    path,
                    child,
                    process,
                };
            }
            assert!(
                process.running(),
                "server exited before opening the session"
            );
            sleep(0.02);
        }
        drop(process);
        panic!("server did not open the session");
    }

    pub fn pid(&self) -> i32 {
        self.process.pid()
    }

    pub fn alive(&mut self) -> bool {
        self.process.running()
    }

    pub fn communicate(&mut self, timeout: f64) -> Finished {
        self.process.communicate(timeout)
    }

    /// Waits for a clean finish with the child's code; returns the child PID.
    pub fn passed(&mut self, code: i32, timeout: f64) -> i32 {
        let finished = self.communicate(timeout);
        self.check_passed(&finished, code)
    }

    /// The server exits with the child's code and prints nothing, the child is
    /// reaped and the socket pathname is gone.
    pub fn check_passed(&self, finished: &Finished, code: i32) -> i32 {
        finished.assert_silent(code);
        assert!(!alive(self.child), "the child outlived the server");
        assert!(!self.path.exists(), "the socket pathname remains");
        self.child
    }
}
