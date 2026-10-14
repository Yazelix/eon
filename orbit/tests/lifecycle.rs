use std::{
    fs::{self, OpenOptions},
    io::{BufReader, Read, Write},
    net::Shutdown,
    os::unix::{
        ffi::OsStringExt,
        fs::{MetadataExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Barrier,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use orbit_protocol::management::{
    self as management, ClientMessage as ManagementClientMessage,
    FailureCode as ManagementFailureCode, LiveIdentity, ObjectIdentity, ProcessOutcome,
    Record as ManagementRecord, ServerMessage as ManagementServerMessage, TerminationReason,
};
use orbit_protocol::{
    Frame, Rgb, Screen,
    session::{
        self, ClientMessage, FailureCode, FocusEvent, KeyAction, KeyEvent, Modifiers, PhysicalKey,
        SelectionAction, SelectionPosition, ServerMessage, SurfaceSize, decode_server_message,
        encode_client_message,
    },
};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new(name: &str) -> TestResult<Self> {
        let path = std::env::temp_dir().join(format!(
            "orbit-{name}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
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

struct Server(Child);

impl Server {
    fn wait(mut self) -> TestResult<ExitStatus> {
        wait_bounded(&mut self)
    }

    fn shutdown(mut self) -> TestResult<ExitStatus> {
        terminate(&self)?;
        wait_bounded(&mut self)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = terminate(self);
            let _ = wait_bounded(self);
        }
    }
}

struct ProcessCleanup(Option<(u32, u64)>);

impl ProcessCleanup {
    fn disarm(&mut self) {
        self.0 = None;
    }
}

impl Drop for ProcessCleanup {
    fn drop(&mut self) {
        let Some((pid, start)) = self.0 else {
            return;
        };
        if !process_identity_matches(pid, start) {
            return;
        }
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
        if wait_process_gone(pid).is_err() && process_identity_matches(pid, start) {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
            let _ = wait_process_gone(pid);
        }
    }
}

struct ManagementClient {
    reader: BufReader<UnixStream>,
}

impl ManagementClient {
    fn request(
        &mut self,
        message: &ManagementClientMessage,
    ) -> TestResult<ManagementServerMessage> {
        self.reader
            .get_mut()
            .write_all(&management::encode_client_message(message)?)?;
        read_management_message(&mut self.reader)
    }
}

struct Client {
    reader: BufReader<UnixStream>,
    frame: Frame,
}

struct MetadataObserver {
    reader: BufReader<UnixStream>,
    metadata: session::Metadata,
}

impl MetadataObserver {
    fn attach(socket: &Path) -> TestResult<Self> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let observe = encode_client_message(&ClientMessage::ObserveMetadata)?;
        loop {
            let stream = connect_with_timeouts(socket, deadline)?;
            let mut reader = BufReader::new(stream);
            reader.get_mut().write_all(&observe)?;
            let mut observing = false;
            loop {
                match read_message(&mut reader) {
                    Ok(ServerMessage::ObservingMetadata) if !observing => observing = true,
                    Ok(ServerMessage::Metadata(metadata)) if observing => {
                        return Ok(Self { reader, metadata });
                    }
                    Ok(ServerMessage::Busy) if !observing && Instant::now() < deadline => break,
                    Ok(message) => {
                        return Err(format!("unexpected observe response: {message:?}").into());
                    }
                    Err(error) if Instant::now() < deadline && error.is::<std::io::Error>() => {
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            thread::yield_now();
        }
    }

    fn wait_metadata(&mut self, title: &str, working_directory: &str) -> TestResult {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.metadata.title == title && self.metadata.working_directory == working_directory
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "metadata never became ({title:?}, {working_directory:?}); last: {:?}",
                    self.metadata
                )
                .into());
            }
            match read_message(&mut self.reader)? {
                ServerMessage::Metadata(metadata) => {
                    assert!(metadata.revision > self.metadata.revision);
                    self.metadata = metadata;
                }
                ServerMessage::Exited { code } => {
                    return Err(format!("shell exited with {code}").into());
                }
                message => return Err(format!("unexpected observer message: {message:?}").into()),
            }
        }
    }
}

impl Client {
    fn attach(socket: &Path) -> TestResult<Self> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let hello = encode_client_message(&ClientMessage::Hello)?;
        loop {
            let stream = connect_with_timeouts(socket, deadline)?;
            let mut reader = BufReader::new(stream);
            if let Err(error) = reader.get_mut().write_all(&hello) {
                if Instant::now() >= deadline {
                    return Err(error.into());
                }
                thread::yield_now();
                continue;
            }
            let mut attached = false;
            loop {
                match read_message(&mut reader) {
                    Ok(ServerMessage::Attached) if !attached => attached = true,
                    Ok(ServerMessage::Frame(frame)) if attached => {
                        return Ok(Self {
                            reader,
                            frame: *frame,
                        });
                    }
                    Ok(ServerMessage::Busy) if !attached && Instant::now() < deadline => break,
                    Ok(message) => {
                        return Err(format!("unexpected attach response: {message:?}").into());
                    }
                    Err(error) if Instant::now() < deadline && error.is::<std::io::Error>() => {
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            thread::yield_now();
        }
    }

    fn request(&mut self, request: &ClientMessage) -> TestResult {
        write_message(self.reader.get_mut(), request)?;
        loop {
            match read_message(&mut self.reader)? {
                ServerMessage::Accepted => return Ok(()),
                ServerMessage::Frame(frame) => self.frame = *frame,
                ServerMessage::Failure(failure) => {
                    return Err(format!("request failed: {failure:?}").into());
                }
                message => return Err(format!("unexpected response: {message:?}").into()),
            }
        }
    }

    fn request_frame(&mut self, request: &ClientMessage) -> TestResult {
        let previous_revision = self.frame.revision;
        write_message(self.reader.get_mut(), request)?;
        let mut accepted = false;
        loop {
            match read_message(&mut self.reader)? {
                ServerMessage::Accepted => accepted = true,
                ServerMessage::Frame(frame) => {
                    assert!(frame.revision > self.frame.revision);
                    self.frame = *frame;
                    if accepted && self.frame.revision > previous_revision {
                        return Ok(());
                    }
                }
                ServerMessage::Failure(failure) => {
                    return Err(format!("request failed: {failure:?}").into());
                }
                message => return Err(format!("unexpected response: {message:?}").into()),
            }
        }
    }

    fn paste(&mut self, bytes: impl Into<Vec<u8>>) -> TestResult {
        self.request(&ClientMessage::Paste(bytes.into()))
    }

    fn key(&mut self, action: KeyAction, key: PhysicalKey, modifiers: Modifiers) -> TestResult {
        self.request(&ClientMessage::Key(KeyEvent {
            action,
            key,
            modifiers,
            consumed_modifiers: Modifiers::empty(),
            composing: false,
            text: None,
            unshifted_codepoint: None,
        }))
    }

    fn enter(&mut self) -> TestResult {
        self.key(KeyAction::Press, PhysicalKey::ENTER, Modifiers::empty())
    }

    fn text(&mut self, text: impl Into<String>, consumed_modifiers: Modifiers) -> TestResult {
        self.request(&ClientMessage::Key(KeyEvent {
            action: KeyAction::Press,
            key: PhysicalKey::A,
            modifiers: consumed_modifiers,
            consumed_modifiers,
            composing: false,
            text: Some(text.into()),
            unshifted_codepoint: Some('a'),
        }))
    }

    fn wheel(&mut self, button: session::MouseButton) -> TestResult {
        let previous_revision = self.frame.revision;
        write_message(
            self.reader.get_mut(),
            &ClientMessage::Mouse(session::MouseEvent {
                action: session::MouseAction::Press,
                button: Some(button),
                modifiers: Modifiers::empty(),
                x: 1.0,
                y: 1.0,
            }),
        )?;
        match read_message(&mut self.reader)? {
            ServerMessage::WheelOutcome(session::WheelOutcome::Viewport {
                applied_rows: -1..=1,
                frame,
            }) => {
                assert!(frame.revision > previous_revision);
                self.frame = *frame;
                Ok(())
            }
            ServerMessage::Failure(failure) => Err(format!("request failed: {failure:?}").into()),
            message => Err(format!("unexpected wheel response: {message:?}").into()),
        }
    }

    fn preview(
        &mut self,
        direction: session::VerticalDirection,
    ) -> TestResult<session::VerticalPreview> {
        let frame_revision = self.frame.revision;
        write_message(
            self.reader.get_mut(),
            &ClientMessage::PreviewVertical {
                frame_revision,
                direction,
            },
        )?;
        match read_message(&mut self.reader)? {
            ServerMessage::VerticalPreview(preview) => {
                assert_eq!(preview.frame_revision, frame_revision);
                Ok(preview)
            }
            ServerMessage::Failure(failure) => Err(format!("preview failed: {failure:?}").into()),
            message => Err(format!("unexpected preview response: {message:?}").into()),
        }
    }

    fn select(&mut self, action: SelectionAction) -> TestResult {
        let finishing = matches!(action, SelectionAction::Finish { .. });
        if !finishing {
            return self.request_frame(&ClientMessage::Selection(action));
        }

        write_message(self.reader.get_mut(), &ClientMessage::Selection(action))?;
        let ServerMessage::SelectionFinished {
            frame_revision: revision,
        } = read_message(&mut self.reader)?
        else {
            return Err("host selection finish did not name its frame".into());
        };
        let ServerMessage::Frame(frame) = read_message(&mut self.reader)? else {
            return Err("host selection finish did not publish its frame".into());
        };
        assert_eq!(frame.revision, revision);
        assert!(frame.revision > self.frame.revision);
        self.frame = *frame;
        assert!(matches!(
            read_message(&mut self.reader)?,
            ServerMessage::CopiedText {
                location: session::ClipboardLocation::Selection,
                ..
            }
        ));
        Ok(())
    }

    fn copy(&mut self) -> TestResult<Result<String, FailureCode>> {
        write_message(
            self.reader.get_mut(),
            &ClientMessage::Selection(SelectionAction::Copy),
        )?;
        loop {
            match read_message(&mut self.reader)? {
                ServerMessage::CopiedText {
                    location: session::ClipboardLocation::Standard,
                    text,
                } => return Ok(Ok(text)),
                ServerMessage::Frame(frame) => self.frame = *frame,
                ServerMessage::Failure(failure) => return Ok(Err(failure.code)),
                message => return Err(format!("unexpected copy response: {message:?}").into()),
            }
        }
    }

    fn wait_title(&mut self, expected: &str) -> TestResult {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if self.frame.title == expected {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "title never became {expected:?}; last title: {:?}",
                    self.frame.title
                )
                .into());
            }
            match read_message(&mut self.reader)? {
                ServerMessage::Frame(frame) => self.frame = *frame,
                ServerMessage::Exited { code } => {
                    return Err(format!("shell exited with {code}").into());
                }
                _ => {}
            }
        }
    }
}

fn write_message(writer: &mut impl Write, message: &ClientMessage) -> TestResult {
    writer.write_all(&encode_client_message(message)?)?;
    Ok(())
}

fn read_message(reader: &mut impl Read) -> TestResult<ServerMessage> {
    let mut header = [0; session::HEADER_BYTES];
    reader.read_exact(&mut header)?;
    let length = session::server_message_len(&header)?
        .expect("complete header declares its server-message length");
    let mut framed = Vec::with_capacity(length);
    framed.extend_from_slice(&header);
    framed.resize(length, 0);
    reader.read_exact(&mut framed[session::HEADER_BYTES..])?;
    Ok(decode_server_message(&framed)?)
}

fn read_management_message(reader: &mut impl Read) -> TestResult<ManagementServerMessage> {
    let mut header = [0; management::HEADER_BYTES];
    reader.read_exact(&mut header)?;
    let length = management::server_message_len(&header)?
        .expect("complete header declares its management-message length");
    let mut framed = Vec::with_capacity(length);
    framed.extend_from_slice(&header);
    framed.resize(length, 0);
    reader.read_exact(&mut framed[management::HEADER_BYTES..])?;
    Ok(management::decode_server_message(&framed)?)
}

fn wait_management_record(path: &Path) -> TestResult<ManagementRecord> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(bytes) = fs::read(path)
            && bytes.len() <= management::MAX_RECORD_BYTES
            && let Ok(record) = management::decode_record(&bytes)
        {
            return Ok(record);
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for record {}", path.display()).into());
        }
        thread::yield_now();
    }
}

fn acquire_management(
    record_path: &Path,
    identity: &LiveIdentity,
) -> TestResult<(ManagementServerMessage, ManagementClient)> {
    let metadata = fs::symlink_metadata(record_path)?;
    let record = ObjectIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    };
    let management_path = PathBuf::from(std::ffi::OsString::from_vec(
        identity.management.path.clone(),
    ));
    let mut stream = connect_bounded(&management_path, Instant::now() + Duration::from_secs(5))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let write = stream.write_all(&management::encode_client_message(
        &ManagementClientMessage::Acquire {
            expected: identity.clone(),
            record,
        },
    )?);
    let mut client = ManagementClient {
        reader: BufReader::new(stream),
    };
    let response = match read_management_message(&mut client.reader) {
        Ok(response) => response,
        Err(error) => {
            write?;
            return Err(error);
        }
    };
    Ok((response, client))
}

fn connect_bounded(socket: &Path, deadline: Instant) -> TestResult<UnixStream> {
    loop {
        match UnixStream::connect(socket) {
            Ok(stream) => return Ok(stream),
            Err(error)
                if Instant::now() < deadline
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                    ) =>
            {
                thread::yield_now();
            }
            Err(error) => return Err(error.into()),
        }
    }
}

fn connect_with_timeouts(socket: &Path, deadline: Instant) -> TestResult<UnixStream> {
    loop {
        let stream = connect_bounded(socket, deadline)?;
        match stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(2))))
        {
            Ok(()) => return Ok(stream),
            Err(_) if Instant::now() < deadline => thread::yield_now(),
            Err(error) => return Err(error.into()),
        }
    }
}

fn wait_file_text(path: &Path) -> TestResult<String> {
    wait_file_text_matching(path, |text| !text.trim().is_empty())
}

fn wait_file_text_matching(path: &Path, ready: impl Fn(&str) -> bool) -> TestResult<String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = fs::read_to_string(path)
            && ready(&text)
        {
            return Ok(text);
        }
        if Instant::now() >= deadline {
            return Err(format!("timed out waiting for content in {}", path.display()).into());
        }
        thread::yield_now();
    }
}

fn server_command() -> Command {
    let binary = env!("CARGO_BIN_EXE_yazelix-orbit");
    let mut command = if let Some(valgrind) = std::env::var_os("ORBIT_MEMCHECK") {
        let mut command = Command::new(valgrind);
        command.args([
            "--tool=memcheck",
            "--leak-check=full",
            "--show-leak-kinds=all",
            "--errors-for-leak-kinds=definite,indirect",
            "--track-fds=yes",
            "--error-exitcode=97",
            "--log-file=target/orbit-memcheck.%p.log",
            binary,
        ]);
        command
    } else {
        Command::new(binary)
    };
    command
        .arg("serve")
        .env("PS1", "")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn gated_server_command(ready: &Path, release: &Path) -> Command {
    let server = server_command();
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg(
            "printf ready > \"$ORBIT_READY\"; \
             while [ ! -e \"$ORBIT_RELEASE\" ]; do sleep 0.01; done; exec \"$@\"",
        )
        .arg("orbit-claim")
        .arg(server.get_program())
        .args(server.get_args())
        .envs(
            server
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        )
        .env("ORBIT_READY", ready)
        .env("ORBIT_RELEASE", release)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

fn spawn_server(socket: &Path) -> TestResult<Server> {
    Ok(Server(
        server_command()
            .arg(socket)
            .arg("--")
            .arg("/bin/sh")
            .spawn()?,
    ))
}

fn spawn_default_server(runtime: &Path) -> TestResult<Server> {
    Ok(Server(
        server_command()
            .arg("--")
            .arg("/bin/sh")
            .env("XDG_RUNTIME_DIR", runtime)
            .spawn()?,
    ))
}

fn wait_bounded(server: &mut Server) -> TestResult<ExitStatus> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = server.0.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            server.0.kill()?;
            let _ = server.0.wait();
            return Err("server did not exit within five seconds".into());
        }
        thread::yield_now();
    }
}

fn wait_process_gone(pid: u32) -> TestResult {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("process {pid} remained alive").into());
        }
        thread::yield_now();
    }
}

#[cfg(target_os = "linux")]
fn process_start_identity(pid: u32) -> TestResult<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let fields = stat.rsplit_once(')').ok_or("malformed process stat")?.1;
    Ok(fields
        .split_whitespace()
        .nth(19)
        .ok_or("process stat is missing start identity")?
        .parse()?)
}

#[cfg(target_os = "macos")]
fn process_start_identity(pid: u32) -> TestResult<u64> {
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
    if read != size || info.pbi_pid != pid || info.pbi_start_tvusec >= 1_000_000 {
        return Err("invalid macOS process start identity".into());
    }
    info.pbi_start_tvsec
        .checked_mul(1_000_000)
        .and_then(|seconds| seconds.checked_add(info.pbi_start_tvusec))
        .filter(|identity| *identity != 0)
        .ok_or_else(|| "invalid macOS process start identity".into())
}

fn process_identity_matches(pid: u32, start: u64) -> bool {
    process_start_identity(pid).is_ok_and(|current| current == start)
}

#[cfg(target_os = "linux")]
fn process_group_and_session(pid: u32) -> TestResult<(u32, u32)> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or("process stat is missing its command terminator")?;
    let mut fields = fields.split_whitespace().skip(2);
    Ok((
        fields
            .next()
            .ok_or("process stat is missing pgrp")?
            .parse()?,
        fields
            .next()
            .ok_or("process stat is missing session")?
            .parse()?,
    ))
}

#[cfg(target_os = "linux")]
fn process_cpu_ticks(pid: u32) -> TestResult<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or("process stat is missing its command terminator")?;
    let mut fields = fields.split_whitespace();
    let user = fields.nth(11).ok_or("process stat is missing user CPU")?;
    let system = fields.next().ok_or("process stat is missing system CPU")?;
    Ok(user.parse::<u64>()? + system.parse::<u64>()?)
}

fn terminate(server: &Server) -> std::io::Result<()> {
    match unsafe { libc::kill(server.0.id() as libc::pid_t, libc::SIGTERM) } {
        0 => Ok(()),
        _ => Err(std::io::Error::last_os_error()),
    }
}

fn frame_text(frame: &Frame) -> String {
    frame
        .rows
        .iter()
        .flat_map(|row| {
            row.cells
                .iter()
                .map(|cell| cell.text.as_str())
                .chain(["\n"])
        })
        .collect()
}

#[test]
fn ansi_palette_launch_is_visible_and_survives_reattachment() -> TestResult {
    const PALETTE: &str = "000102,101112,202122,303132,404142,505152,606162,707172,808182,909192,a0a1a2,b0b1b2,c0c1c2,d0d1d2,e0e1e2,f0f1f2";
    const fn rgb(r: u8, g: u8, b: u8) -> Rgb {
        Rgb { r, g, b }
    }

    let dir = TestDir::new("ansi-palette")?;
    let socket = dir.0.join("orbit.sock");
    let server = Server(
        server_command()
            .arg(&socket)
            .arg("--ansi-palette-v1")
            .arg(PALETTE)
            .arg("--")
            .arg("/bin/sh")
            .spawn()?,
    );
    let first = Client::attach(&socket)?;
    assert_eq!(first.frame.colors.palette[0], rgb(0, 1, 2));
    assert_eq!(first.frame.colors.palette[15], rgb(0xf0, 0xf1, 0xf2));
    assert_eq!(first.frame.colors.palette[16], Rgb::BLACK);
    drop(first);

    let mut second = Client::attach(&socket)?;
    assert_eq!(second.frame.colors.palette[1], rgb(0x10, 0x11, 0x12));
    second.paste("printf '\\033]4;1;rgb:01/02/03\\033\\\\'; printf '\\033]2;override\\033\\\\'")?;
    second.enter()?;
    second.wait_title("override")?;
    assert_eq!(second.frame.colors.palette[1], rgb(1, 2, 3));
    second.paste("printf '\\033]104;1\\033\\\\'; printf '\\033]2;reset\\033\\\\'")?;
    second.enter()?;
    second.wait_title("reset")?;
    assert_eq!(second.frame.colors.palette[1], rgb(0x10, 0x11, 0x12));
    drop(second);
    assert!(server.shutdown()?.success());

    let invalid_socket = dir.0.join("invalid.sock");
    let child_marker = dir.0.join("child-started");
    let status = server_command()
        .arg(&invalid_socket)
        .arg("--ansi-palette-v1")
        .arg("incomplete")
        .arg("--")
        .arg("/bin/sh")
        .arg("-c")
        .arg(format!(": > {}", child_marker.display()))
        .status()?;
    assert!(!status.success());
    assert!(!invalid_socket.exists());
    assert!(!child_marker.exists());
    Ok(())
}

#[test]
fn attachment_negotiation_and_races_recover_for_canonical_client() -> TestResult {
    let dir = TestDir::new("negotiation")?;
    let socket = dir.0.join("orbit.sock");
    let server = spawn_server(&socket)?;

    let mut incompatible = connect_with_timeouts(&socket, Instant::now() + Duration::from_secs(5))?;
    let mut unsupported = encode_client_message(&ClientMessage::Hello)?;
    unsupported[4..6].copy_from_slice(&(session::VERSION + 1).to_le_bytes());
    incompatible.write_all(&unsupported)?;
    assert_eq!(incompatible.read(&mut [0; 1])?, 0);
    drop(incompatible);

    let mut unordered = connect_with_timeouts(&socket, Instant::now() + Duration::from_secs(5))?;
    write_message(&mut unordered, &ClientMessage::Focus(FocusEvent::Gained))?;
    match read_message(&mut unordered)? {
        ServerMessage::Failure(failure) => assert_eq!(failure.code, FailureCode::Protocol),
        message => return Err(format!("unexpected unordered response: {message:?}").into()),
    }
    drop(unordered);

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut first = connect_with_timeouts(&socket, deadline)?;
    let mut second = connect_with_timeouts(&socket, deadline)?;

    assert_eq!(read_message(&mut second)?, ServerMessage::Busy);
    drop(second);

    drop(Client::attach(&socket)?);
    assert_eq!(first.read(&mut [0; 1])?, 0);
    assert!(server.shutdown()?.success());
    Ok(())
}

#[test]
fn metadata_observer_streams_inactive_title_and_cwd_without_owning_input() -> TestResult {
    let dir = TestDir::new("metadata-observer")?;
    let socket = dir.0.join("orbit.sock");
    let ready = dir.0.join("ready");
    let begin = dir.0.join("begin");
    let partial = dir.0.join("partial");
    let complete = dir.0.join("complete");
    let slow = dir.0.join("slow");
    let slow_done = dir.0.join("slow-done");
    let exit = dir.0.join("exit");
    let server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(
                "printf '\\033]2;initial\\033\\\\\\033]7;file:///tmp/eon-initial\\033\\\\'; \
                 printf ready > \"$ORBIT_READY\"; \
                 while [ ! -e \"$ORBIT_BEGIN\" ]; do sleep 0.01; done; \
                 printf '\\033]2;split'; printf partial > \"$ORBIT_PARTIAL\"; \
                 while [ ! -e \"$ORBIT_COMPLETE\" ]; do sleep 0.01; done; \
                 printf '%b' '-complete\\033\\\\'; \
                 i=0; while [ \"$i\" -lt 300 ]; do \
                   printf '\\033]2;rapid-%03d\\033\\\\' \"$i\"; i=$((i + 1)); \
                 done; \
                 printf '\\033]2;rapid-final\\033\\\\\\033]7;file:///tmp/eon-rapid\\033\\\\'; \
                 while [ ! -e \"$ORBIT_SLOW\" ]; do sleep 0.01; done; \
                 padding=$(printf '%01000d' 0); i=0; \
                 while [ \"$i\" -lt 1000 ]; do \
                   printf '\\033]2;slow-%04d-%s\\033\\\\' \"$i\" \"$padding\"; i=$((i + 1)); \
                 done; \
                 printf '\\033]2;slow-final\\033\\\\\\033]7;file:///tmp/eon-slow\\033\\\\'; \
                 printf done > \"$ORBIT_SLOW_DONE\"; \
                 while [ ! -e \"$ORBIT_EXIT\" ]; do sleep 0.01; done; exit 17",
            )
            .env("ORBIT_READY", &ready)
            .env("ORBIT_BEGIN", &begin)
            .env("ORBIT_PARTIAL", &partial)
            .env("ORBIT_COMPLETE", &complete)
            .env("ORBIT_SLOW", &slow)
            .env("ORBIT_SLOW_DONE", &slow_done)
            .env("ORBIT_EXIT", &exit)
            .spawn()?,
    );
    assert_eq!(wait_file_text(&ready)?, "ready");

    let mut interactive = Client::attach(&socket)?;
    interactive.wait_title("initial")?;
    let mut rejected = MetadataObserver::attach(&socket)?;
    assert_eq!(
        (
            rejected.metadata.title.as_str(),
            rejected.metadata.working_directory.as_str(),
        ),
        ("initial", "file:///tmp/eon-initial")
    );
    write_message(
        rejected.reader.get_mut(),
        &ClientMessage::Focus(FocusEvent::Gained),
    )?;
    match read_message(&mut rejected.reader)? {
        ServerMessage::Failure(failure) => assert_eq!(failure.code, FailureCode::Protocol),
        message => return Err(format!("observer input was not rejected: {message:?}").into()),
    }
    drop(rejected);

    let mut observer = MetadataObserver::attach(&socket)?;
    let mut excess = connect_with_timeouts(&socket, Instant::now() + Duration::from_secs(5))?;
    let _ = write_message(&mut excess, &ClientMessage::ObserveMetadata);
    assert_eq!(read_message(&mut excess)?, ServerMessage::Busy);

    fs::write(&begin, b"begin")?;
    assert_eq!(wait_file_text(&partial)?, "partial");
    observer
        .reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(150)))?;
    let error = read_message(&mut observer.reader).expect_err("split OSC published metadata");
    assert!(error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        )
    }));
    observer
        .reader
        .get_mut()
        .set_read_timeout(Some(Duration::from_secs(5)))?;

    let interactive = thread::spawn(move || {
        interactive
            .wait_title("slow-final")
            .map(|_| interactive)
            .map_err(|error| error.to_string())
    });
    fs::write(&complete, b"complete")?;
    observer.wait_metadata("rapid-final", "file:///tmp/eon-rapid")?;

    fs::write(&slow, b"slow")?;
    assert_eq!(wait_file_text(&slow_done)?, "done");
    let _interactive = interactive
        .join()
        .map_err(|_| "interactive metadata-pressure reader panicked")?
        .map_err(|error| format!("interactive metadata-pressure reader failed: {error}"))?;
    observer.wait_metadata("slow-final", "file:///tmp/eon-slow")?;

    fs::write(&exit, b"exit")?;
    loop {
        match read_message(&mut observer.reader)? {
            ServerMessage::Metadata(metadata) => observer.metadata = metadata,
            ServerMessage::Exited { code } => {
                assert_eq!(code, 17);
                break;
            }
            message => return Err(format!("unexpected observer exit message: {message:?}").into()),
        }
    }
    assert_eq!(server.wait()?.code(), Some(17));
    assert!(UnixStream::connect(&socket).is_err());
    Ok(())
}

#[test]
fn pty_pressure_disconnect_ignores_stale_client_readiness() -> TestResult {
    let dir = TestDir::new("stale-client-readiness")?;
    let socket = dir.0.join("orbit.sock");
    let emitted = dir.0.join("emitted");

    // One more than the pending-effect bound disconnects during PTY processing.
    let effects = "\\033]52;c;eA==\\033\\\\".repeat(65);
    let server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(
                "stty -echo; read orbit_trigger; printf \"$ORBIT_EFFECTS\"; \
                 printf emitted > \"$ORBIT_EMITTED\"; read orbit_stop",
            )
            .env("ORBIT_EFFECTS", effects)
            .env("ORBIT_EMITTED", &emitted)
            .spawn()?,
    );
    let mut first = Client::attach(&socket)?;

    let mut input = encode_client_message(&ClientMessage::Paste(b"trigger\n".to_vec()))?;
    // The complete first message releases the child; the incomplete second
    // message keeps client input ready without producing another reply.
    let incomplete =
        encode_client_message(&ClientMessage::Paste(vec![b'x'; session::MAX_PASTE_BYTES]))?;
    input.extend_from_slice(&incomplete[..incomplete.len() - 1]);
    let _ = first.reader.get_mut().write_all(&input);
    assert_eq!(wait_file_text(&emitted)?, "emitted");

    let _second = Client::attach(&socket)?;
    assert!(server.shutdown()?.success());
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn transient_pty_eio_recovers_when_the_live_child_reopens_the_terminal() -> TestResult {
    let dir = TestDir::new("transient-pty-eio")?;
    let socket = dir.0.join("orbit.sock");
    let marker = dir.0.join("lifecycle");
    let input = dir.0.join("input");
    let release = dir.0.join("release");
    let mut server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(
                "exec 3>\"$ORBIT_MARKER\"; exec 0<&- 1>&- 2>&-; printf closed >&3; \
                 while [ ! -e \"$ORBIT_RELEASE\" ]; do sleep 0.01; done; sleep 1; \
                 exec 0<>/dev/tty 1>&0 2>&0; printf ',reopened' >&3; \
                 printf '\\033]2;reopened\\033\\\\'; IFS= read -r input; printf '%s' \"$input\" > \"$ORBIT_INPUT\"",
            )
            .env("ORBIT_MARKER", &marker)
            .env("ORBIT_INPUT", &input)
            .env("ORBIT_RELEASE", &release)
            .spawn()?,
    );

    assert_eq!(wait_file_text(&marker)?, "closed");
    let mut client = Client::attach(&socket)?;
    assert!(server.0.try_wait()?.is_none());
    let cpu_before = std::env::var_os("ORBIT_MEMCHECK")
        .is_none()
        .then(|| process_cpu_ticks(server.0.id()))
        .transpose()?;
    fs::write(&release, b"release")?;
    assert_eq!(
        wait_file_text_matching(&marker, |text| text == "closed,reopened")?,
        "closed,reopened"
    );
    if let Some(cpu_before) = cpu_before {
        let cpu_ticks = process_cpu_ticks(server.0.id())? - cpu_before;
        let clock_ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        assert!(clock_ticks > 0, "sysconf(_SC_CLK_TCK) failed");
        assert!(
            cpu_ticks.saturating_mul(5) < clock_ticks as u64,
            "Orbit consumed {cpu_ticks}/{clock_ticks} CPU ticks during a one-second PTY slave gap"
        );
    }
    client.wait_title("reopened")?;
    client.paste("after-reopen\n")?;
    assert_eq!(wait_file_text(&input)?, "after-reopen");
    assert!(wait_bounded(&mut server)?.success());
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn writing_descendant_cannot_hold_server_open_after_known_child_exit() -> TestResult {
    let dir = TestDir::new("writing-descendant")?;
    let socket = dir.0.join("orbit.sock");
    let descendant_pid_file = dir.0.join("descendant.pid");
    let mut server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(
                "sh -c 'trap \"\" HUP; printf \"%s\\n\" \"$$\" > \"$ORBIT_DESCENDANT_PID\"; while printf orbit; do sleep 0.05; done' & while [ ! -s \"$ORBIT_DESCENDANT_PID\" ]; do :; done",
            )
            .env("ORBIT_DESCENDANT_PID", &descendant_pid_file)
            .spawn()?,
    );

    let descendant_pid = wait_file_text(&descendant_pid_file)?.trim().parse()?;
    let _cleanup = ProcessCleanup(Some((
        descendant_pid,
        process_start_identity(descendant_pid)?,
    )));

    assert!(wait_bounded(&mut server)?.success());
    assert!(!socket.exists());
    wait_process_gone(descendant_pid)?;
    Ok(())
}

#[test]
fn shell_survives_detach_and_one_client_reattaches() -> TestResult {
    let dir = TestDir::new("lifecycle")?;
    let socket = dir.0.join("orbit.sock");
    let fifo = dir.0.join("release.fifo");
    let first_pid_file = dir.0.join("first-shell.pid");
    let second_pid_file = dir.0.join("second-shell.pid");
    let key_file = dir.0.join("key-input");
    let multiline_file = dir.0.join("multiline-paste");
    let status = Command::new("mkfifo").arg(&fifo).status()?;
    assert!(status.success());

    let mut server = spawn_server(&socket)?;
    let mut first = Client::attach(&socket)?;
    first.paste(format!(
        "printf '%s\\n' \"$$\" > {}",
        first_pid_file.display()
    ))?;
    first.enter()?;
    let shell_pid = wait_file_text(&first_pid_file)?.trim().parse()?;

    first.text(
        format!("printf key > {}", key_file.display()),
        Modifiers::empty(),
    )?;
    first.enter()?;
    wait_file_text_matching(&key_file, |text| text == "key")?;
    first.paste(format!(
        "printf first > {}\nprintf second >> {}",
        multiline_file.display(),
        multiline_file.display()
    ))?;
    first.enter()?;
    wait_file_text_matching(&multiline_file, |text| text == "firstsecond")?;
    first.request(&ClientMessage::Mouse(session::MouseEvent {
        action: session::MouseAction::Motion,
        button: Some(session::MouseButton::Left),
        modifiers: Modifiers::empty(),
        x: 1.0,
        y: 1.0,
    }))?;

    first.request(&ClientMessage::Resize(SurfaceSize {
        cols: 100,
        rows: 40,
        screen_width: 920,
        screen_height: 740,
        cell_width: 9,
        cell_height: 18,
        padding_top: 10,
        padding_bottom: 10,
        padding_left: 10,
        padding_right: 10,
    }))?;
    first.paste("printf '\\033]2;size-%s\\033\\\\' \"$(stty size)\"")?;
    first.enter()?;
    first.wait_title("size-40 100")?;

    let mut rejected = UnixStream::connect(&socket)?;
    rejected.set_read_timeout(Some(Duration::from_secs(2)))?;
    write_message(&mut rejected, &ClientMessage::Hello)?;
    assert_eq!(read_message(&mut rejected)?, ServerMessage::Busy);
    first.request(&ClientMessage::Focus(FocusEvent::Gained))?;

    first.paste(format!(
        "printf '\\033]2;ready\\033\\\\'; read orbit_release < {}; printf '\\033]2;detached\\033\\\\'",
        fifo.display()
    ))?;
    first.enter()?;
    first.wait_title("ready")?;
    drop(first);

    writeln!(OpenOptions::new().write(true).open(&fifo)?, "go")?;

    let mut second = Client::attach(&socket)?;
    assert_eq!(
        (second.frame.dimensions.cols, second.frame.dimensions.rows),
        (100, 40)
    );
    second.paste(format!(
        "printf '%s\\n' \"$$\" > {}",
        second_pid_file.display()
    ))?;
    second.enter()?;
    assert_eq!(
        wait_file_text(&second_pid_file)?.trim().parse::<u32>()?,
        shell_pid
    );
    second.wait_title("detached")?;
    second.paste("exit")?;
    second.enter()?;
    drop(second);

    assert!(wait_bounded(&mut server)?.success());
    assert!(!socket.exists());
    wait_process_gone(shell_pid)?;
    Ok(())
}

#[test]
fn conformance_c9_selection_copy_is_authoritative_bounded_and_client_scoped() -> TestResult {
    let dir = TestDir::new("selection")?;
    let socket = dir.0.join("orbit.sock");
    let release = dir.0.join("release");
    let activity = dir.0.join("activity");
    let stop = dir.0.join("stop");
    let script = format!(
        "stty -echo; \
         while [ ! -e '{release}' ]; do sleep 0.01; done; \
         i=0; while [ \"$i\" -lt 6 ]; do printf 'history-%02d\\n' \"$i\"; i=$((i + 1)); done; \
         printf 'first-界-é-second'; printf '\\033]2;selection-ready\\033\\\\'; \
         while [ ! -e '{activity}' ]; do sleep 0.01; done; \
         printf '\\033[?1049hhidden\\033[?1049l\\033]2;selection-activity\\033\\\\'; \
         while [ ! -e '{stop}' ]; do sleep 0.01; done",
        release = release.display(),
        activity = activity.display(),
        stop = stop.display(),
    );
    let mut server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(script)
            .spawn()?,
    );

    let mut first = Client::attach(&socket)?;
    let surface = SurfaceSize {
        cols: 20,
        rows: 4,
        cell_width: 8,
        cell_height: 16,
        screen_width: 160,
        screen_height: 64,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    };
    let position = |x: u16, y: u16| SelectionPosition {
        x: (u32::from(x) * surface.cell_width) as f32 + surface.cell_width as f32 / 2.0,
        y: (u32::from(y) * surface.cell_height) as f32 + surface.cell_height as f32 / 2.0,
    };
    first.request_frame(&ClientMessage::Resize(surface))?;
    fs::write(&release, b"release")?;
    first.wait_title("selection-ready")?;
    let live = first.frame.clone();
    let preview = first.preview(session::VerticalDirection::Up)?;
    let session::PreviewOutcome::Viewport { cols: 20, rows, .. } = preview.outcome else {
        return Err("upward preview did not expose a row window".into());
    };
    assert!(!rows.is_empty() && rows.len() <= usize::from(surface.rows));
    let entering_top = rows
        .into_iter()
        .next()
        .ok_or("upward preview row window was empty")?;
    assert_eq!(first.frame, live);
    first.wheel(session::MouseButton::Four)?;
    assert_ne!(first.frame.rows, live.rows);
    assert_eq!(first.frame.rows[0], entering_top);
    let preview = first.preview(session::VerticalDirection::Down)?;
    let session::PreviewOutcome::Viewport { cols: 20, rows, .. } = preview.outcome else {
        return Err("downward preview did not expose a row window".into());
    };
    assert!(!rows.is_empty() && rows.len() <= usize::from(surface.rows));
    let entering_bottom = rows
        .into_iter()
        .next()
        .ok_or("downward preview row window was empty")?;
    first.wheel(session::MouseButton::Five)?;
    assert_eq!(first.frame.rows, live.rows);
    assert_eq!(first.frame.rows.last(), Some(&entering_bottom));

    first.select(SelectionAction::Begin {
        frame_revision: first.frame.revision,
        position: position(17, 3),
        time_ns: 1_000_000_000,
        modifiers: session::Modifiers::empty(),
    })?;
    first.select(SelectionAction::Update {
        position: position(0, 3),
        modifiers: session::Modifiers::empty(),
    })?;
    first.select(SelectionAction::Finish {
        position: position(0, 3),
        modifiers: session::Modifiers::empty(),
    })?;
    assert!(
        first
            .frame
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| cell.style.selected)
    );
    assert_eq!(first.copy()?, Ok("first-界-é-second".into()));

    let selected = first.frame.clone();
    drop(first);
    let mut second = Client::attach(&socket)?;
    assert!(second.frame.revision > selected.revision);
    assert_eq!(frame_text(&second.frame), frame_text(&selected));
    assert!(
        !second
            .frame
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| cell.style.selected)
    );
    assert_eq!(second.copy()?, Err(FailureCode::InvalidInput));

    second.select(SelectionAction::Begin {
        frame_revision: second.frame.revision,
        position: position(17, 3),
        time_ns: 1_000_000_000,
        modifiers: session::Modifiers::empty(),
    })?;
    second.select(SelectionAction::Update {
        position: position(0, 3),
        modifiers: session::Modifiers::empty(),
    })?;
    second.select(SelectionAction::Finish {
        position: position(0, 3),
        modifiers: session::Modifiers::empty(),
    })?;
    fs::write(&activity, b"activity")?;
    second.wait_title("selection-activity")?;
    assert!(
        !second
            .frame
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| cell.style.selected)
    );
    assert_eq!(second.copy()?, Ok("first-界-é-second".into()));

    second.request_frame(&ClientMessage::Resize(SurfaceSize {
        cols: 21,
        screen_width: 168,
        ..surface
    }))?;
    assert_eq!(second.copy()?, Ok("first-界-é-second".into()));

    fs::write(&stop, b"stop")?;
    drop(second);
    assert!(wait_bounded(&mut server)?.success());
    assert!(!socket.exists());
    Ok(())
}

#[test]
fn authoritative_viewport_survives_detach_and_slow_reader_pressure() -> TestResult {
    let dir = TestDir::new("viewport")?;
    let socket = dir.0.join("orbit.sock");
    let preview = dir.0.join("preview");
    let commit = dir.0.join("commit");
    let release = dir.0.join("release");
    let clear = dir.0.join("clear");
    let pressure = dir.0.join("pressure");
    let stop = dir.0.join("stop");
    let script = format!(
        "stty -echo; \
         i=0; while [ \"$i\" -lt 48 ]; do printf 'history-%02d\\n' \"$i\"; i=$((i + 1)); done; \
         printf '\\033]2;history-ready\\033\\\\'; \
         while [ ! -e '{preview}' ] && [ ! -e '{stop}' ]; do sleep 0.01; done; \
         [ -e '{stop}' ] && exit; \
         i=0; while [ \"$i\" -lt 8 ]; do printf '\\033[Hpreview-update-%02d' \"$i\"; i=$((i + 1)); done; \
         printf '\\033[999;1H\\033]2;preview-update\\033\\\\'; \
         while [ ! -e '{commit}' ] && [ ! -e '{stop}' ]; do sleep 0.01; done; \
         [ -e '{stop}' ] && exit; \
         i=0; while [ \"$i\" -lt 8 ]; do printf '\\033[Hcommit-update-%02d' \"$i\"; i=$((i + 1)); done; \
         printf '\\033[999;1H\\033]2;commit-update\\033\\\\'; \
         while [ ! -e '{release}' ] && [ ! -e '{stop}' ]; do sleep 0.01; done; \
         [ -e '{stop}' ] && exit; \
         printf 'tail-after-pin\\n\\033]2;pinned-output\\033\\\\'; \
         while [ ! -e '{clear}' ] && [ ! -e '{stop}' ]; do sleep 0.01; done; \
         [ -e '{stop}' ] && exit; \
         printf '\\033[3J'; \
         i=0; while [ \"$i\" -lt 1100 ]; do printf 'prune-%04d-abcdefghijklmnopqrstuvwxyz-ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789\\n' \"$i\"; i=$((i + 1)); done; \
         printf '\\033]2;pruned\\033\\\\'; \
         while [ ! -e '{pressure}' ] && [ ! -e '{stop}' ]; do sleep 0.01; done; \
         [ -e '{stop}' ] && exit; \
         printf '\\033]2;pressure-complete\\033\\\\'; \
         while [ ! -e '{stop}' ]; do sleep 0.01; done",
        preview = preview.display(),
        commit = commit.display(),
        release = release.display(),
        clear = clear.display(),
        pressure = pressure.display(),
        stop = stop.display(),
    );
    let mut server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(script)
            .spawn()?,
    );

    let mut first = Client::attach(&socket)?;
    first.wait_title("history-ready")?;
    let presented_revision = first.frame.revision;
    fs::write(&preview, b"preview")?;
    first.wait_title("preview-update")?;
    assert!(first.frame.revision > presented_revision);
    write_message(
        first.reader.get_mut(),
        &ClientMessage::PreviewVertical {
            frame_revision: presented_revision,
            direction: session::VerticalDirection::Up,
        },
    )?;
    let ServerMessage::VerticalPreview(preview) = read_message(&mut first.reader)? else {
        return Err("continuous output prevented the stale preview from converging".into());
    };
    assert_eq!(preview.frame_revision, first.frame.revision);

    fs::write(&commit, b"commit")?;
    first.wait_title("commit-update")?;
    assert!(first.frame.revision > preview.frame_revision);
    write_message(
        first.reader.get_mut(),
        &ClientMessage::ScrollVertical {
            frame_revision: preview.frame_revision,
            rows: -6,
        },
    )?;
    let ServerMessage::ScrollOutcome(session::ScrollOutcome::Viewport {
        requested_rows: -6,
        applied_rows: -6,
        frame,
        ..
    }) = read_message(&mut first.reader)?
    else {
        return Err("large viewport movement expanded beyond one result".into());
    };
    assert!(frame.revision > first.frame.revision);
    first.frame = *frame;
    let pinned = first.frame.clone();

    fs::write(&release, b"release")?;
    first.wait_title("pinned-output")?;
    assert_eq!(first.frame.rows, pinned.rows);
    assert!(!frame_text(&first.frame).contains("tail-after-pin"));
    let detached = first.frame.clone();
    drop(first);

    let mut second = Client::attach(&socket)?;
    assert_eq!(second.frame, detached);
    second.request_frame(&ClientMessage::Key(KeyEvent {
        action: KeyAction::Press,
        key: PhysicalKey::A,
        modifiers: Modifiers::empty(),
        consumed_modifiers: Modifiers::empty(),
        composing: false,
        text: Some("x".into()),
        unshifted_codepoint: Some('x'),
    }))?;
    assert!(frame_text(&second.frame).contains("tail-after-pin"));

    fs::write(&clear, b"clear")?;
    second.wait_title("pruned")?;
    second.request_frame(&ClientMessage::Resize(SurfaceSize {
        cols: 40,
        rows: 12,
        screen_width: 320,
        screen_height: 192,
        cell_width: 8,
        cell_height: 16,
        padding_top: 0,
        padding_bottom: 0,
        padding_left: 0,
        padding_right: 0,
    }))?;
    assert_eq!(
        (second.frame.dimensions.cols, second.frame.dimensions.rows),
        (40, 12)
    );

    let preview = second.preview(session::VerticalDirection::Up)?;
    let session::PreviewOutcome::Viewport {
        cols: 40,
        edge_reached: false,
        rows,
    } = preview.outcome
    else {
        return Err("reflowed preview did not expose a row window".into());
    };
    assert_eq!(rows.len(), 12);
    let entering_top = rows
        .into_iter()
        .next()
        .ok_or("reflowed preview row window was empty")?;
    second.wheel(session::MouseButton::Four)?;
    assert_eq!(second.frame.rows[0], entering_top);

    let started = Instant::now();
    for _ in 0..31 {
        second.wheel(session::MouseButton::Four)?;
    }
    let wheel_elapsed = started.elapsed();
    let frame_bytes =
        session::encode_server_message(&ServerMessage::Frame(Box::new(second.frame.clone())))?
            .len();
    eprintln!(
        "viewport measurement: 32 wheel round trips in {wheel_elapsed:?}; complete ORBS frame {frame_bytes} bytes"
    );
    assert!(wheel_elapsed < Duration::from_secs(5));

    let wheel = encode_client_message(&ClientMessage::Mouse(session::MouseEvent {
        action: session::MouseAction::Press,
        button: Some(session::MouseButton::Four),
        modifiers: Modifiers::empty(),
        x: 1.0,
        y: 1.0,
    }))?;
    let _ = second.reader.get_mut().write_all(&wheel.repeat(4096));
    fs::write(&pressure, b"pressure")?;
    let mut busy = connect_with_timeouts(&socket, Instant::now() + Duration::from_secs(5))?;
    write_message(&mut busy, &ClientMessage::Hello)?;
    assert_eq!(read_message(&mut busy)?, ServerMessage::Busy);
    drop(busy);
    drop(second);
    let mut third = Client::attach(&socket)?;
    third.wait_title("pressure-complete")?;

    fs::write(&stop, b"stop")?;
    drop(third);
    assert!(wait_bounded(&mut server)?.success());
    assert!(!socket.exists());
    Ok(())
}

#[test]
fn conformance_c5_authoritative_input_modes_and_resize_survive_detach() -> TestResult {
    let dir = TestDir::new("input-modes")?;
    let socket = dir.0.join("orbit.sock");
    let first_done = dir.0.join("first-done");
    let kitty_done = dir.0.join("kitty-done");
    let release = dir.0.join("release");
    let modes_off = dir.0.join("modes-off");
    let size = dir.0.join("size");
    let result = dir.0.join("result");
    let mut server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(
                "stty -echo -icanon -icrnl min 1 time 0; \
                 printf '\\033[?1h\\033[?1000h\\033[?1006h\\033[?1004h\\033[?2004h\\033]2;input-modes-on\\033\\\\'; \
                 dd bs=1 count=34 of=\"$ORBIT_DIR/first\" 2>/dev/null; \
                 stty size > \"$ORBIT_SIZE\"; printf done > \"$ORBIT_FIRST_DONE\"; \
                 printf '\\033[>11u\\033]2;kitty-on\\033\\\\'; \
                 dd bs=1 count=9 of=\"$ORBIT_DIR/kitty\" 2>/dev/null; \
                 printf done > \"$ORBIT_KITTY_DONE\"; \
                 while [ ! -e \"$ORBIT_RELEASE\" ]; do sleep 0.01; done; \
                 printf '\\033[<u\\033[?1l\\033[?1000l\\033[?1006l\\033[?1004l\\033[?2004l\\033]2;input-modes-off\\033\\\\'; \
                 printf done > \"$ORBIT_MODES_OFF\"; \
                 dd bs=1 count=7 of=\"$ORBIT_DIR/second\" 2>/dev/null; \
                 stty min 0 time 1; dd bs=1 count=1 of=\"$ORBIT_DIR/extra\" 2>/dev/null; \
                 if [ -s \"$ORBIT_DIR/extra\" ]; then printf duplicate > \"$ORBIT_RESULT\"; exit 1; fi; \
                 printf ok > \"$ORBIT_RESULT\"",
            )
            .env("ORBIT_DIR", &dir.0)
            .env("ORBIT_FIRST_DONE", &first_done)
            .env("ORBIT_KITTY_DONE", &kitty_done)
            .env("ORBIT_RELEASE", &release)
            .env("ORBIT_MODES_OFF", &modes_off)
            .env("ORBIT_SIZE", &size)
            .env("ORBIT_RESULT", &result)
            .spawn()?,
    );

    let mut first = Client::attach(&socket)?;
    first.wait_title("input-modes-on")?;
    first.request(&ClientMessage::Resize(SurfaceSize {
        cols: 100,
        rows: 40,
        screen_width: 920,
        screen_height: 740,
        cell_width: 9,
        cell_height: 17,
        padding_top: 30,
        padding_bottom: 30,
        padding_left: 10,
        padding_right: 10,
    }))?;
    first.text("q", Modifiers::ALT)?;
    first.key(KeyAction::Press, PhysicalKey::ARROW_UP, Modifiers::empty())?;
    first.request(&ClientMessage::Mouse(session::MouseEvent {
        action: session::MouseAction::Press,
        button: Some(session::MouseButton::Left),
        modifiers: Modifiers::SHIFT,
        x: 915.0,
        y: 725.0,
    }))?;
    first.request(&ClientMessage::Focus(FocusEvent::Gained))?;
    first.paste(b"a\nb".to_vec())?;
    assert_eq!(wait_file_text(&first_done)?, "done");
    assert_eq!(
        fs::read(dir.0.join("first"))?,
        b"q\x1bOA\x1b[<4;100;40M\x1b[I\x1b[200~a\nb\x1b[201~"
    );
    assert_eq!(wait_file_text(&size)?.trim(), "40 100");

    first.wait_title("kitty-on")?;
    first.key(KeyAction::Release, PhysicalKey::ENTER, Modifiers::SHIFT)?;
    assert_eq!(wait_file_text(&kitty_done)?, "done");
    assert_eq!(fs::read(dir.0.join("kitty"))?, b"\x1b[13;2:3u");
    drop(first);

    fs::write(&release, b"release")?;
    assert_eq!(wait_file_text(&modes_off)?, "done");
    let mut second = Client::attach(&socket)?;
    second.wait_title("input-modes-off")?;
    assert_eq!(
        (second.frame.dimensions.cols, second.frame.dimensions.rows),
        (100, 40)
    );
    second.text("z", Modifiers::empty())?;
    second.key(KeyAction::Press, PhysicalKey::ARROW_UP, Modifiers::empty())?;
    second.request(&ClientMessage::Mouse(session::MouseEvent {
        action: session::MouseAction::Press,
        button: Some(session::MouseButton::Left),
        modifiers: Modifiers::empty(),
        x: 1.0,
        y: 1.0,
    }))?;
    second.request(&ClientMessage::Focus(FocusEvent::Gained))?;
    second.key(KeyAction::Release, PhysicalKey::ENTER, Modifiers::empty())?;
    second.paste(b"c\nd".to_vec())?;

    assert_eq!(wait_file_text(&result)?, "ok");
    assert_eq!(fs::read(dir.0.join("second"))?, b"z\x1b[Ac\rd");
    drop(second);
    assert!(wait_bounded(&mut server)?.success());
    assert!(!socket.exists());
    Ok(())
}

#[test]
fn conformance_c2_parser_state_and_terminal_replies_survive_client_failure() -> TestResult {
    let dir = TestDir::new("terminal-authority")?;
    let socket = dir.0.join("orbit.sock");
    let begin = dir.0.join("begin");
    let partial = dir.0.join("partial");
    let release = dir.0.join("release");
    let answered = dir.0.join("answered");
    let check_duplicate = dir.0.join("check-duplicate");
    let result = dir.0.join("result");
    let stop = dir.0.join("stop");
    let mut server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(
                "stty -echo -icanon min 1 time 0; \
                 while [ ! -e \"$ORBIT_BEGIN\" ]; do sleep 0.01; done; \
                 printf '\\033[?104'; printf partial > \"$ORBIT_PARTIAL\"; \
                 while [ ! -e \"$ORBIT_RELEASE\" ]; do sleep 0.01; done; \
                 printf '9h\\033[?25l\\033[H\\033[6n'; \
                 reply=$(dd bs=1 count=6 2>/dev/null); \
                 if [ \"$reply\" != \"$(printf '\\033[1;1R')\" ]; then printf wrong > \"$ORBIT_RESULT\"; exit 1; fi; \
                 printf answered > \"$ORBIT_ANSWERED\"; printf '\\033]2;authoritative-reply\\033\\\\'; \
                 while [ ! -e \"$ORBIT_CHECK_DUPLICATE\" ]; do sleep 0.01; done; \
                 stty min 0 time 1; extra=$(dd bs=1 count=1 2>/dev/null); stty sane; \
                 if [ -n \"$extra\" ]; then printf duplicate > \"$ORBIT_RESULT\"; exit 1; fi; \
                 printf once > \"$ORBIT_RESULT\"; \
                 while [ ! -e \"$ORBIT_STOP\" ]; do sleep 0.01; done; \
                 printf '\\033[?1049l'",
            )
            .env("ORBIT_BEGIN", &begin)
            .env("ORBIT_PARTIAL", &partial)
            .env("ORBIT_RELEASE", &release)
            .env("ORBIT_ANSWERED", &answered)
            .env("ORBIT_CHECK_DUPLICATE", &check_duplicate)
            .env("ORBIT_RESULT", &result)
            .env("ORBIT_STOP", &stop)
            .spawn()?,
    );

    let mut first = Client::attach(&socket)?;
    let initial_revision = first.frame.revision;
    fs::write(&begin, b"begin")?;
    assert_eq!(wait_file_text(&partial)?, "partial");
    while first.frame.revision == initial_revision {
        if let ServerMessage::Frame(frame) = read_message(&mut first.reader)? {
            first.frame = *frame;
        }
    }
    drop(first);

    fs::write(&release, b"release")?;
    assert_eq!(wait_file_text(&answered)?, "answered");
    let mut second = Client::attach(&socket)?;
    second.wait_title("authoritative-reply")?;
    assert_eq!(second.frame.screen, Screen::Alternate);
    assert!(!second.frame.cursor.visible);
    fs::write(&check_duplicate, b"check")?;
    assert_eq!(wait_file_text(&result)?, "once");

    fs::write(&stop, b"stop")?;
    let exit_code = loop {
        match read_message(&mut second.reader)? {
            ServerMessage::Frame(frame) => second.frame = *frame,
            ServerMessage::Exited { code } => break code,
            _ => {}
        }
    };
    assert_eq!(exit_code, 0);
    assert_eq!(second.frame.screen, Screen::Primary);
    drop(second);
    assert!(wait_bounded(&mut server)?.success());
    assert!(!socket.exists());
    Ok(())
}

#[test]
fn stale_socket_and_safe_signal_shutdown() -> TestResult {
    let dir = TestDir::new("cleanup")?;
    let runtime = dir.0.join("yazelix-orbit");
    fs::create_dir(&runtime)?;
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))?;
    let socket = runtime.join("orbit.sock");
    drop(UnixListener::bind(&socket)?);
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;

    let server = spawn_default_server(&dir.0)?;
    let aborted = connect_bounded(&socket, Instant::now() + Duration::from_secs(5))?;
    aborted.shutdown(Shutdown::Read)?;
    drop(aborted);
    let mut client = Client::attach(&socket)?;
    let shell_pid_file = dir.0.join("shell.pid");
    client.paste(format!(
        "printf '%s\\n' \"$$\" > {}",
        shell_pid_file.display()
    ))?;
    client.enter()?;
    let shell_pid = wait_file_text(&shell_pid_file)?.trim().parse()?;
    let foreground_pid_file = dir.0.join("foreground.pid");
    client.paste(format!(
        "sh -c 'trap \"\" HUP; echo $$ > {}; printf \"\\033]2;foreground-ready\\033\\\\\"; exec sleep 2'",
        foreground_pid_file.display()
    ))?;
    client.enter()?;
    client.wait_title("foreground-ready")?;
    let foreground_pid = fs::read_to_string(&foreground_pid_file)?
        .trim()
        .parse::<u32>()?;
    let mode = fs::metadata(&socket)?.permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    assert!(server.shutdown()?.success());
    assert!(!socket.exists());
    wait_process_gone(shell_pid)?;
    assert_eq!(unsafe { libc::kill(foreground_pid as libc::pid_t, 0) }, 0);
    wait_process_gone(foreground_pid)?;

    fs::write(&socket, b"do not remove")?;
    let mut refused = spawn_default_server(&dir.0)?;
    assert!(!wait_bounded(&mut refused)?.success());
    assert_eq!(fs::read(&socket)?, b"do not remove");

    fs::remove_file(&socket)?;
    let replaced = spawn_default_server(&dir.0)?;
    drop(Client::attach(&socket)?);
    fs::remove_file(&socket)?;
    let _replacement = UnixListener::bind(&socket)?;
    assert!(replaced.shutdown()?.success());
    assert!(socket.exists(), "server removed a replacement socket");
    Ok(())
}

#[test]
fn shutdown_hups_the_unreaped_direct_child() -> TestResult {
    let dir = TestDir::new("direct-child-hup")?;
    let socket = dir.0.join("orbit.sock");
    let child_pid_file = dir.0.join("child.pid");
    let hup_marker = dir.0.join("hup");
    let server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/sh")
            .arg("-c")
            .arg(
                "trap 'printf hup > \"$ORBIT_HUP_MARKER\"; exit 0' HUP; \
                 printf '%s\n' \"$$\" > \"$ORBIT_CHILD_PID\"; \
                 while :; do :; done",
            )
            .env("ORBIT_CHILD_PID", &child_pid_file)
            .env("ORBIT_HUP_MARKER", &hup_marker)
            .spawn()?,
    );

    wait_file_text(&child_pid_file)?;
    assert!(server.shutdown()?.success());
    assert_eq!(fs::read_to_string(&hup_marker)?, "hup");
    Ok(())
}

#[test]
#[cfg(target_os = "linux")]
fn shutdown_escalates_foreground_and_permits_a_detached_process() -> TestResult {
    let dir = TestDir::new("portable-shutdown")?;
    let socket = dir.0.join("orbit.sock");
    let initial_pid_file = dir.0.join("initial.pid");
    let detached_pid_file = dir.0.join("detached.pid");
    let foreground_pid_file = dir.0.join("foreground.pid");
    let mut unrelated = Server(Command::new("/bin/sleep").arg("60").spawn()?);
    let server = Server(
        server_command()
            .arg(&socket)
            .arg("--")
            .arg("/bin/bash")
            .arg("-c")
            .arg(
                "set -m; printf '%s\\n' \"$$\" > \"$ORBIT_INITIAL_PID\"; \
                 /usr/bin/setsid --fork /bin/sh -c 'printf \"%s\\n\" \"$$\" > \"$ORBIT_DETACHED_PID\"; exec sleep 60' </dev/null >/dev/null 2>&1; \
                 while [ ! -s \"$ORBIT_DETACHED_PID\" ]; do :; done; \
                 /bin/sh -c 'trap \"\" HUP; printf \"%s\\n\" \"$$\" > \"$ORBIT_FOREGROUND_PID\"; exec sleep 60'",
            )
            .env("ORBIT_INITIAL_PID", &initial_pid_file)
            .env("ORBIT_DETACHED_PID", &detached_pid_file)
            .env("ORBIT_FOREGROUND_PID", &foreground_pid_file)
            .spawn()?,
    );

    let initial_pid = wait_file_text(&initial_pid_file)?.trim().parse()?;
    let detached_pid = wait_file_text(&detached_pid_file)?.trim().parse()?;
    let foreground_pid = wait_file_text(&foreground_pid_file)?.trim().parse()?;
    let detached_start = process_start_identity(detached_pid)?;
    let _detached_cleanup = ProcessCleanup(Some((detached_pid, detached_start)));
    assert_eq!(
        process_group_and_session(initial_pid)?,
        (initial_pid, initial_pid)
    );
    assert_eq!(process_group_and_session(foreground_pid)?.0, foreground_pid);
    assert_eq!(process_group_and_session(detached_pid)?.1, detached_pid);
    assert!(server.shutdown()?.success());
    wait_process_gone(initial_pid)?;
    wait_process_gone(foreground_pid)?;
    assert!(process_identity_matches(detached_pid, detached_start));
    assert!(unrelated.0.try_wait()?.is_none());
    Ok(())
}

#[test]
fn simultaneous_stale_socket_claim_has_one_reachable_owner() -> TestResult {
    let dir = TestDir::new("stale-socket-claim")?;
    let attempts = if std::env::var_os("ORBIT_MEMCHECK").is_some() {
        1
    } else {
        25
    };

    for attempt in 0..attempts {
        let socket = dir.0.join(format!("orbit-{attempt}.sock"));
        let first_ready = dir.0.join(format!("first-{attempt}.ready"));
        let second_ready = dir.0.join(format!("second-{attempt}.ready"));
        let release = dir.0.join(format!("release-{attempt}"));
        drop(UnixListener::bind(&socket)?);
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;

        let mut first = Server(
            gated_server_command(&first_ready, &release)
                .arg(&socket)
                .arg("--")
                .arg("/bin/sh")
                .spawn()?,
        );
        let mut second = Server(
            gated_server_command(&second_ready, &release)
                .arg(&socket)
                .arg("--")
                .arg("/bin/sh")
                .spawn()?,
        );
        assert_eq!(wait_file_text(&first_ready)?, "ready");
        assert_eq!(wait_file_text(&second_ready)?, "ready");
        fs::write(&release, b"release")?;

        let deadline = Instant::now() + Duration::from_secs(5);
        let (loser_status, loser, winner) = loop {
            if let Some(status) = first.0.try_wait()? {
                break (status, &mut first, &mut second);
            }
            if let Some(status) = second.0.try_wait()? {
                break (status, &mut second, &mut first);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "both stale-socket contenders remained alive on attempt {attempt}"
                )
                .into());
            }
            thread::yield_now();
        };
        assert!(
            !loser_status.success(),
            "losing stale-socket contender succeeded"
        );
        let mut error = String::new();
        loser
            .0
            .stderr
            .take()
            .ok_or("losing stale-socket contender has no stderr")?
            .read_to_string(&mut error)?;
        assert!(
            error.contains("socket claim already in progress")
                || error.contains("server already listening"),
            "unexpected stale-socket loser error: {error:?}"
        );
        drop(Client::attach(&socket)?);
        terminate(winner)?;
        assert!(wait_bounded(winner)?.success());
        assert!(!socket.exists());
    }
    Ok(())
}

#[test]
fn signal_during_startup_removes_socket() -> TestResult {
    let dir = TestDir::new("startup-signal")?;
    let socket = dir.0.join("orbit.sock");
    let server = spawn_server(&socket)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while !socket.exists() {
        if Instant::now() >= deadline {
            return Err("server socket did not appear within five seconds".into());
        }
        thread::yield_now();
    }

    assert!(server.shutdown()?.success());
    assert!(!socket.exists());
    Ok(())
}

#[test]
fn managed_run_survives_launcher_loss_and_has_one_replacement_owner() -> TestResult {
    let dir = TestDir::new("managed-owner")?;
    let socket = dir.0.join("orbit.sock");
    let mut record_path = socket.as_os_str().to_os_string();
    record_path.push(".record");
    let record_path = PathBuf::from(record_path);
    let orbit_pid_path = dir.0.join("orbit.pid");
    let pty_pid_path = dir.0.join("pty.pid");
    let work = dir.0.join("work");
    let done = dir.0.join("done");

    let mut orbit = server_command();
    orbit
        .arg(&socket)
        .args([
            "--management-v1",
            "session-1",
            "run-1",
            "component-1",
            "--",
            "/bin/sh",
            "-c",
            "printf '%s' \"$$\" > \"$ORBIT_PTY_PID\"; \
             while [ ! -e \"$ORBIT_WORK\" ]; do sleep 0.01; done; \
             printf '\\033]2;survived\\033\\\\'; printf survived > \"$ORBIT_DONE\"; \
             while :; do sleep 1; done",
        ])
        .env("ORBIT_PTY_PID", &pty_pid_path)
        .env("ORBIT_WORK", &work)
        .env("ORBIT_DONE", &done);

    let mut launcher = Command::new("/bin/sh");
    launcher
        .arg("-c")
        .arg("\"$@\" & orbit=$!; printf '%s' \"$orbit\" > \"$ORBIT_PID\"; wait \"$orbit\"")
        .arg("orbit-launcher")
        .arg(orbit.get_program())
        .args(orbit.get_args())
        .envs(
            orbit
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        )
        .env("ORBIT_PID", &orbit_pid_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut launcher = launcher.spawn()?;

    let orbit_pid = wait_file_text(&orbit_pid_path)?.parse::<u32>()?;
    let mut cleanup = ProcessCleanup(None);
    let identity = match wait_management_record(&record_path)? {
        ManagementRecord::Live(identity) => identity,
        record => return Err(format!("expected live management record, got {record:?}").into()),
    };
    assert_eq!(identity.process_id, orbit_pid);
    assert!(process_identity_matches(orbit_pid, identity.process_start));
    cleanup.0 = Some((orbit_pid, identity.process_start));
    drop(ProcessCleanup(Some((
        orbit_pid,
        identity.process_start + 1,
    ))));
    let pty_pid = wait_file_text(&pty_pid_path)?.parse::<u32>()?;
    #[cfg(target_os = "linux")]
    {
        let inherited_sockets = [
            format!("socket:[{}]", identity.presentation.object.inode),
            format!("socket:[{}]", identity.management.object.inode),
        ];
        for descriptor in fs::read_dir(format!("/proc/{pty_pid}/fd"))? {
            let target = fs::read_link(descriptor?.path())?;
            assert!(
                !inherited_sockets
                    .iter()
                    .any(|socket| target == Path::new(socket)),
                "PTY child inherited an Orbit listener: {}",
                target.display()
            );
        }
    }

    let mut wrong_identity = identity.clone();
    wrong_identity.process_start += 1;
    let (response, rejected) = acquire_management(&record_path, &wrong_identity)?;
    assert!(matches!(
        response,
        ManagementServerMessage::Failure(ref failure)
            if failure.code == ManagementFailureCode::InvalidIdentity
    ));
    drop(rejected);
    thread::sleep(Duration::from_millis(50));

    let (response, initial_manager) = acquire_management(&record_path, &identity)?;
    assert_eq!(response, ManagementServerMessage::Lease(identity.clone()));
    let initial_revision = Client::attach(&socket)?.frame.revision;

    assert_eq!(
        unsafe { libc::kill(launcher.id() as libc::pid_t, libc::SIGKILL) },
        0
    );
    assert!(!launcher.wait()?.success());
    drop(initial_manager);
    thread::sleep(Duration::from_millis(150));

    fs::write(&work, b"continue")?;
    assert_eq!(wait_file_text(&done)?, "survived");

    let barrier = Arc::new(Barrier::new(3));
    let mut attempts = Vec::new();
    for _ in 0..2 {
        let barrier = Arc::clone(&barrier);
        let record_path = record_path.clone();
        let identity = identity.clone();
        attempts.push(thread::spawn(move || {
            barrier.wait();
            acquire_management(&record_path, &identity).unwrap()
        }));
    }
    barrier.wait();

    let mut winner = None;
    let mut busy = 0;
    for attempt in attempts {
        let (response, client) = attempt.join().map_err(|_| "management race panicked")?;
        match response {
            ManagementServerMessage::Lease(current) => {
                assert_eq!(current, identity);
                assert!(winner.replace(client).is_none(), "two leases were granted");
            }
            ManagementServerMessage::Busy => busy += 1,
            response => return Err(format!("unexpected lease-race response: {response:?}").into()),
        }
    }
    assert_eq!(busy, 1);
    let mut winner = winner.ok_or("management race produced no lease winner")?;
    assert_eq!(
        winner.request(&ManagementClientMessage::Status)?,
        ManagementServerMessage::Status(identity.clone())
    );

    let mut presentation = Client::attach(&socket)?;
    presentation.wait_title("survived")?;
    assert!(presentation.frame.revision > initial_revision);
    assert_eq!(identity.process_id, orbit_pid);
    assert!(process_identity_matches(orbit_pid, identity.process_start));
    assert_eq!(unsafe { libc::kill(pty_pid as libc::pid_t, 0) }, 0);

    let stop = management::encode_client_message(&ManagementClientMessage::Stop)?;
    winner.reader.get_mut().write_all(&stop)?;
    drop(winner);
    wait_process_gone(orbit_pid)?;
    let ManagementRecord::Tombstone(stopped) = wait_management_record(&record_path)? else {
        return Err("accepted stop did not publish a tombstone".into());
    };
    assert_eq!(stopped.identity, identity);
    assert_eq!(stopped.reason, TerminationReason::ExplicitStop);
    assert_eq!(stopped.outcome, ProcessOutcome::Signal(libc::SIGHUP));
    wait_process_gone(pty_pid)?;
    cleanup.disarm();
    assert!(!socket.exists());
    assert!(!identity.management.path.is_empty());
    assert!(!PathBuf::from(std::ffi::OsString::from_vec(identity.management.path)).exists());
    assert!(record_path.exists());
    Ok(())
}

#[test]
fn management_authority_negatives_fail_closed_without_stopping_session() -> TestResult {
    let dir = TestDir::new("management-deny")?;
    let socket = dir.0.join("orbit.sock");
    let mut record_path = socket.as_os_str().to_os_string();
    record_path.push(".record");
    let record_path = PathBuf::from(record_path);
    let natural_exit = dir.0.join("natural-exit");
    let server = Server(
        server_command()
            .arg(&socket)
            .args([
                "--management-v1",
                "session-negative",
                "run-negative",
                "component-negative",
                "--",
                "/bin/sh",
                "-c",
                "while [ ! -e \"$ORBIT_NATURAL_EXIT\" ]; do sleep 0.01; done; exit 23",
            ])
            .env("ORBIT_NATURAL_EXIT", &natural_exit)
            .spawn()?,
    );
    let identity = match wait_management_record(&record_path)
        .map_err(|error| format!("initial management record: {error}"))?
    {
        ManagementRecord::Live(identity) => identity,
        record => return Err(format!("expected live management record, got {record:?}").into()),
    };
    let management_path = PathBuf::from(std::ffi::OsString::from_vec(
        identity.management.path.clone(),
    ));

    let silent = connect_bounded(&management_path, Instant::now() + Duration::from_secs(5))?;
    thread::sleep(Duration::from_millis(1_150));
    let (response, lease) = acquire_management(&record_path, &identity)?;
    assert_eq!(response, ManagementServerMessage::Lease(identity.clone()));
    drop(silent);
    drop(lease);
    thread::sleep(Duration::from_millis(150));

    let mut malformed = connect_bounded(&management_path, Instant::now() + Duration::from_secs(5))?;
    malformed.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut oversized = [0; management::HEADER_BYTES];
    oversized[..4].copy_from_slice(b"ORBM");
    oversized[4..6].copy_from_slice(&management::VERSION.to_le_bytes());
    oversized[6] = 2;
    oversized[8..12].copy_from_slice(&(management::MAX_MESSAGE_BYTES as u32).to_le_bytes());
    malformed.write_all(&oversized)?;
    assert!(matches!(
        read_management_message(&mut malformed)?,
        ManagementServerMessage::Failure(ref failure)
            if failure.code == ManagementFailureCode::InvalidRequest
    ));
    drop(malformed);
    thread::sleep(Duration::from_millis(150));

    fs::set_permissions(&record_path, fs::Permissions::from_mode(0o640))?;
    let (response, rejected) = acquire_management(&record_path, &identity)?;
    assert!(matches!(
        response,
        ManagementServerMessage::Failure(ref failure)
            if failure.code == ManagementFailureCode::InvalidIdentity
    ));
    drop(rejected);
    fs::set_permissions(&record_path, fs::Permissions::from_mode(0o600))?;
    thread::sleep(Duration::from_millis(150));

    let (response, lease) = acquire_management(&record_path, &identity)?;
    assert_eq!(response, ManagementServerMessage::Lease(identity.clone()));
    drop(lease);
    thread::sleep(Duration::from_millis(150));

    fs::remove_file(&socket)?;
    let replacement = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    let (response, rejected) = acquire_management(&record_path, &identity)?;
    assert!(matches!(
        response,
        ManagementServerMessage::Failure(ref failure)
            if failure.code == ManagementFailureCode::InvalidIdentity
    ));
    drop(rejected);

    fs::write(&natural_exit, b"exit")?;
    assert_eq!(server.wait()?.code(), Some(23));
    assert!(socket.exists(), "replacement endpoint was removed");
    assert!(!management_path.exists());
    let ManagementRecord::Tombstone(tombstone) = wait_management_record(&record_path)
        .map_err(|error| format!("terminal management record: {error}"))?
    else {
        return Err("natural exit did not publish a tombstone".into());
    };
    assert_eq!(tombstone.reason, TerminationReason::NaturalExit);
    assert_eq!(tombstone.outcome, ProcessOutcome::ExitCode(23));
    drop(replacement);
    Ok(())
}
