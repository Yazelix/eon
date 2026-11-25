use super::*;
use crate::supervisor::temporary_directory;
use std::{
    cell::Cell,
    fs,
    io::Read,
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::PathBuf,
    rc::Rc,
    sync::{Arc, mpsc},
};

/// A source that yields one byte per read and advances a shared clock by
/// `step` per read, so a test controls time without sleeping.
struct Drip {
    bytes: Vec<u8>,
    clock: Rc<Cell<Instant>>,
    step: Duration,
    reads: usize,
}

impl Source for Drip {
    fn read_within(&mut self, buffer: &mut [u8], timeout: Duration) -> io::Result<usize> {
        assert!(!timeout.is_zero(), "a read was asked to wait no time");
        self.reads += 1;
        self.clock.set(self.clock.get() + self.step);
        match self.bytes.first() {
            None => Ok(0),
            Some(&byte) => {
                buffer[0] = byte;
                self.bytes.remove(0);
                Ok(1)
            }
        }
    }
}

fn drip(bytes: &[u8], step: Duration) -> (Drip, Rc<Cell<Instant>>) {
    let clock = Rc::new(Cell::new(Instant::now()));
    let source = Drip {
        bytes: bytes.to_vec(),
        clock: Rc::clone(&clock),
        step,
        reads: 0,
    };
    (source, clock)
}

#[test]
fn progress_never_renews_the_request_deadline() {
    // Every read makes progress, yet 100 ms per byte cannot fill 8 bytes in 250 ms.
    let (mut source, clock) = drip(&[7; 8], Duration::from_millis(100));
    let deadline = clock.get() + Duration::from_millis(250);
    let mut buffer = [0; 8];
    let error = read_exact_before(&mut source, &mut buffer, deadline, || clock.get()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(
        source.reads, 3,
        "the reader stopped at the deadline, not later"
    );
}

#[test]
fn a_prompt_source_fills_and_an_early_end_is_eof() {
    let (mut source, clock) = drip(b"EONU\x01\x00", Duration::ZERO);
    let deadline = clock.get() + Duration::from_millis(250);
    let mut buffer = [0; 6];
    read_exact_before(&mut source, &mut buffer, deadline, || clock.get()).unwrap();
    assert_eq!(&buffer, b"EONU\x01\x00");
    let (mut short, clock) = drip(b"EON", Duration::ZERO);
    let error = read_exact_before(&mut short, &mut buffer, deadline, || clock.get()).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn a_real_socket_read_times_out_by_the_deadline() {
    let (mut window, mut client) = UnixStream::pair().unwrap();
    std::io::Write::write_all(&mut client, b"EON").unwrap();
    let deadline = Instant::now() + Duration::from_millis(100);
    let mut buffer = [0; 6];
    let error = read_exact_before(&mut window, &mut buffer, deadline, Instant::now).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    // A generous bound only catches a hang, never a slow scheduler.
    assert!(Instant::now() < deadline + Duration::from_secs(30));
}

fn private_runtime() -> PathBuf {
    let runtime = temporary_directory();
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    runtime
}

fn log(runtime: &Path) -> Arc<DiagnosticLog> {
    DiagnosticLog::open_in(Ok(runtime.join("logs")), "g1-test", "eon")
}

fn logged(runtime: &Path) -> String {
    fs::read_dir(runtime.join("logs"))
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| fs::read_to_string(entry.path()).unwrap_or_default())
        .collect()
}

#[test]
fn update_socket_is_owned_0600_and_removed_on_drop_only_if_ours() {
    let runtime = private_runtime();
    let log = log(&runtime);
    let listener = UpdateListener::bind(&runtime, &log).expect("tests serve update.sock");
    let path = runtime.join(SOCKET);
    assert_eq!(listener.path(), path);
    let metadata = fs::symlink_metadata(&path).unwrap();
    assert!(metadata.file_type().is_socket());
    assert_eq!(metadata.mode() & 0o777, 0o600);
    assert_eq!(metadata.uid(), crate::supervisor::effective_uid());
    drop(listener);
    assert!(!path.exists());

    // A socket that replaced ours survives our drop.
    let listener = UpdateListener::bind(&runtime, &log).unwrap();
    fs::remove_file(&path).unwrap();
    let other = std::os::unix::net::UnixListener::bind(&path).unwrap();
    drop(listener);
    assert!(path.exists());
    drop(other);
    fs::remove_dir_all(runtime).unwrap();
}

#[test]
fn a_failed_bind_leaves_the_window_without_an_endpoint() {
    // A regular file, a live listener and an overlong path each give no
    // endpoint and a logged reason, never an error.
    let runtime = private_runtime();
    let log = log(&runtime);
    fs::write(runtime.join(SOCKET), b"not a socket").unwrap();
    assert!(UpdateListener::bind(&runtime, &log).is_none());
    assert!(
        runtime.join(SOCKET).is_file(),
        "a foreign object is left alone"
    );
    fs::remove_file(runtime.join(SOCKET)).unwrap();

    let live = std::os::unix::net::UnixListener::bind(runtime.join(SOCKET)).unwrap();
    assert!(UpdateListener::bind(&runtime, &log).is_none());
    drop(live);
    fs::remove_file(runtime.join(SOCKET)).unwrap();

    let long = runtime.join("x".repeat(crate::platform::SOCKET_PATH_BYTES));
    fs::create_dir(&long).unwrap();
    assert!(UpdateListener::bind(&long, &log).is_none());
    let notes = logged(&runtime);
    assert!(notes.contains("must be an owned Unix socket"), "{notes}");
    assert!(notes.contains("already active"), "{notes}");
    assert!(notes.contains("socket path limit"), "{notes}");
    fs::remove_dir_all(runtime).unwrap();
}

#[test]
fn an_update_socket_path_at_the_limit_binds() {
    // The longest path the platform accepts binds; one byte more is refused.
    let base = private_runtime();
    let log = log(&base);
    let room = crate::platform::SOCKET_PATH_BYTES
        .checked_sub(base.as_os_str().len() + 2 + SOCKET.len())
        .expect("the test root leaves room for a directory");
    for (extra, binds) in [(0, true), (1, false)] {
        let runtime = base.join("d".repeat(room + extra));
        fs::create_dir(&runtime).unwrap();
        assert_eq!(
            runtime.join(SOCKET).as_os_str().len(),
            crate::platform::SOCKET_PATH_BYTES + extra
        );
        assert_eq!(UpdateListener::bind(&runtime, &log).is_some(), binds);
    }
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn absent_endpoint_refuses_with_manual_move() {
    let runtime = private_runtime();
    let refusal = endpoint(&runtime).unwrap_err();
    assert_eq!(refusal, Refusal::Absent);
    let text = refusal.to_string();
    assert!(text.contains("cannot be updated in place"), "{text}");
    assert!(text.contains(MANUAL_MOVE), "{text}");
    fs::remove_dir_all(runtime).unwrap();
}

#[test]
fn client_refuses_unsafe_directories_and_endpoint_objects() {
    let runtime = private_runtime();
    let path = runtime.join(SOCKET);
    // A good endpoint passes and reports the socket's identity.
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let (found, identity) = endpoint(&runtime).unwrap();
    assert_eq!(found, path);
    assert_eq!(
        identity,
        socket_identity_from(&fs::symlink_metadata(&path).unwrap())
    );
    // Another mode on the socket.
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(endpoint(&runtime), Err(Refusal::Unsafe(_))));
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    // A directory others can enter.
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(endpoint(&runtime), Err(Refusal::Unsafe(_))));
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    drop(listener);
    // A symlink to a good socket, and a regular file.
    let elsewhere = private_runtime();
    let target = elsewhere.join("real.sock");
    let real = std::os::unix::net::UnixListener::bind(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(matches!(endpoint(&runtime), Err(Refusal::Unsafe(_))));
    fs::remove_file(&path).unwrap();
    fs::write(&path, b"not a socket").unwrap();
    assert!(matches!(endpoint(&runtime), Err(Refusal::Unsafe(_))));
    drop(real);
    fs::remove_dir_all(runtime).unwrap();
    fs::remove_dir_all(elsewhere).unwrap();
}

#[test]
fn an_endpoint_past_the_socket_path_limit_is_not_absent() {
    // A socket moved into a directory too deep to connect to is reported as
    // such, never as a window without an endpoint.
    let base = private_runtime();
    let deep = base.join("d".repeat(crate::platform::SOCKET_PATH_BYTES));
    fs::create_dir(&deep).unwrap();
    fs::set_permissions(&deep, fs::Permissions::from_mode(0o700)).unwrap();
    let short = base.join("s.sock");
    let listener = std::os::unix::net::UnixListener::bind(&short).unwrap();
    fs::set_permissions(&short, fs::Permissions::from_mode(0o600)).unwrap();
    fs::rename(&short, deep.join(SOCKET)).unwrap();
    assert_eq!(endpoint(&deep).unwrap_err(), Refusal::PathTooLong);
    drop(listener);
    fs::remove_dir_all(base).unwrap();
}

#[test]
fn a_namespace_alias_is_the_same_directory() {
    // The /tmp and /private/tmp shape: one directory, two spellings.
    let root = private_runtime();
    let alias = PathBuf::from(format!("{}-alias", root.display()));
    std::os::unix::fs::symlink(&root, &alias).unwrap();
    assert!(same_directory(&root, &alias));
    assert!(same_directory(&alias.join("."), &root));
    let other = private_runtime();
    assert!(!same_directory(&root, &other));
    assert!(!same_directory(&root, &root.join("missing")));
    fs::remove_file(&alias).unwrap();
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(other).unwrap();
}

// The exchange, in process: `serve_connection` on one end of a pair or a
// fixture listener, the client or raw bytes on the other.

use eon_workspace_protocol::update::{Host, Presentation};
use std::{os::unix::net::UnixListener, thread};

const LIFETIME: &str = "g1-0123456789abcdef0123456789abcdef";
const BUILD: &str = "g1-fedcba9876543210fedcba9876543210";

fn fixture(request: RequestId, namespace: &Path, lifetime: &str, pid: u32) -> Inspected {
    Inspected {
        request,
        pid,
        namespace: namespace.as_os_str().as_encoded_bytes().to_vec(),
        lifetime: lifetime.into(),
        created_build: BUILD.into(),
        active_build: BUILD.into(),
        active_launcher: None,
        eonw: 46,
        eonr: 1,
        update_schema: 1,
        presentation: Presentation::Detached,
        hosts: vec![Host {
            terminal: "terminal-1".into(),
            run: "run-1".into(),
            origin: None,
            management: 9,
            record_generation: 1,
            orbs: None,
            orbf: None,
        }],
        last: None,
    }
}

fn raw(version: u16, kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = b"EONU".to_vec();
    frame.extend_from_slice(&version.to_le_bytes());
    frame.extend_from_slice(&[kind, 0]);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// Serves `client`'s bytes on a pair; returns how the window ended it and
/// the reply bytes the client read until EOF.
fn exchange(sent: &[u8], hold_open: bool, deadline: Duration) -> (Served, Vec<u8>) {
    let (window, mut client) = UnixStream::pair().unwrap();
    client.write_all(sent).unwrap();
    if !hold_open {
        client.shutdown(Shutdown::Write).unwrap();
    }
    let deadline = Instant::now() + deadline;
    // The caller's injected fault applies to the window's thread.
    let point = INJECTED.with(|injected| injected.get());
    let window = thread::spawn(move || {
        inject(point);
        serve_connection(window, deadline, |request| {
            Ok(fixture(request, Path::new("/"), LIFETIME, 1))
        })
    });
    // Read until the window stops writing, then close, as a client does; a
    // window that closed with nothing written may reset the connection.
    let mut reply = Vec::new();
    let _ = client.read_to_end(&mut reply);
    drop(client);
    (window.join().unwrap(), reply)
}

fn refused_code(reply: &[u8]) -> Code {
    Refused::decode(reply, Direction::Reply).unwrap().code
}

const LONG: Duration = Duration::from_secs(30);

#[test]
fn inspect_is_answered_with_the_echoed_request() {
    let request = [7; 16];
    let (served, reply) = exchange(&Inspect { request }.encode().unwrap(), true, LONG);
    assert_eq!(served, Served::Replied(Kind::Inspected, None));
    let inspected = Inspected::decode(&reply).unwrap();
    assert_eq!(inspected.request, request);
    assert_eq!(inspected.eonw, 46);
}

#[test]
fn another_version_is_refused_from_the_prefix_alone() {
    // Exactly the 6-byte prefix, socket held open; then a short 8-byte frame.
    for sent in [b"EONU\x02\x00".to_vec(), b"EONU\x02\x00\x01\x00".to_vec()] {
        let (served, reply) = exchange(&sent, true, LONG);
        assert_eq!(served, Served::Replied(Kind::Refused, Some(Code::Version)));
        assert_eq!(&reply[..6], b"EONU\x01\x00", "answered in v1");
        let refused = Refused::decode(&reply, Direction::Reply).unwrap();
        assert_eq!(refused.code, Code::Version);
        // The approved page's text, with X the window's and Y the client's.
        assert_eq!(
            refused.detail,
            "this window speaks Update v1; this build speaks v2; move the window by hand once"
        );
    }
}

#[test]
fn a_refusal_before_the_end_of_a_request_still_arrives() {
    // A v2 header and payload: refused after 6 bytes, the rest unread. The
    // drain keeps the reply readable, ending in EOF and not a reset, every
    // time.
    let sent = raw(2, 1, &[9; 4000]);
    for _ in 0..50 {
        let (window, mut client) = UnixStream::pair().unwrap();
        client.write_all(&sent).unwrap();
        client.shutdown(Shutdown::Write).unwrap();
        let served = serve_connection(window, Instant::now() + LONG, |_| unreachable!());
        assert_eq!(served, Served::Replied(Kind::Refused, Some(Code::Version)));
        let mut reply = Vec::new();
        client
            .read_to_end(&mut reply)
            .expect("a clean end, not a reset");
        assert_eq!(refused_code(&reply), Code::Version);
    }
}

#[test]
fn bad_magic_flags_kind_and_direction_are_malformed() {
    let mut flags = raw(1, 1, &[0; 16]);
    flags[7] = 1;
    // A Reserved Stop and an EONW request reach update.sock.
    let stop = b"EONW\x00\x00\x01\x00\x25\x00\x00\x00".to_vec();
    for sent in [
        b"EONX\x01\x00".to_vec(),
        stop,
        flags,
        raw(1, 4, &[]),
        raw(1, 129, &[]),
        raw(1, 64, &[]),
        raw(1, 65, &[]),
    ] {
        let (served, reply) = exchange(&sent, true, LONG);
        assert_eq!(
            served,
            Served::Replied(Kind::Refused, Some(Code::Malformed)),
            "{sent:?}"
        );
        assert_eq!(refused_code(&reply), Code::Malformed);
    }
}

#[test]
fn an_oversized_declaration_is_refused_without_waiting_for_its_payload() {
    // The client sends only the header and keeps the socket open; without
    // the header check the window would wait the 30 s and close silently.
    let mut header = raw(1, 2, &[]);
    header[8..12].copy_from_slice(&(8 * 1024 + 1u32).to_le_bytes());
    let (served, reply) = exchange(&header, true, LONG);
    assert_eq!(
        served,
        Served::Replied(Kind::Refused, Some(Code::Malformed))
    );
    assert_eq!(refused_code(&reply), Code::Malformed);
}

#[test]
fn a_partial_request_closes_without_a_reply() {
    let whole = Inspect { request: [1; 16] }.encode().unwrap();
    for hold_open in [true, false] {
        let (served, reply) = exchange(&whole[..20], hold_open, Duration::from_millis(100));
        assert_eq!(served, Served::Partial, "held open: {hold_open}");
        assert!(reply.is_empty());
    }
}

#[test]
fn a_dripping_client_is_closed_at_the_request_deadline() {
    // One byte every 100 ms of an 8 KiB PREFLIGHT would take 13 minutes.
    let (window, mut client) = UnixStream::pair().unwrap();
    let frame = raw(1, 2, &[0; 8192]);
    let dripper = thread::spawn(move || {
        for byte in frame {
            if client.write_all(&[byte]).is_err() {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    });
    let started = Instant::now();
    let served = serve_connection(
        window,
        Instant::now() + Duration::from_millis(250),
        |_| unreachable!(),
    );
    assert_eq!(served, Served::Partial);
    assert!(started.elapsed() < LONG, "{:?}", started.elapsed());
    dripper.join().unwrap();
}

#[test]
fn requests_this_slice_does_not_serve_are_unsupported() {
    for kind in [2, 3] {
        let (served, reply) = exchange(&raw(1, kind, &[5; 100]), true, LONG);
        assert_eq!(
            served,
            Served::Replied(Kind::Refused, Some(Code::Unsupported))
        );
        let refused = Refused::decode(&reply, Direction::Reply).unwrap();
        assert_eq!(refused.request, [5; 16], "the request is echoed");
    }
}

#[test]
fn an_unreadable_or_foreign_peer_gets_no_byte() {
    for points in [&["peer-unreadable"][..], &["peer-uid"]] {
        let point = points[0];
        inject(points);
        let (served, reply) = exchange(&Inspect { request: [1; 16] }.encode().unwrap(), true, LONG);
        inject(&[]);
        assert_eq!(served, Served::Peer, "{point}");
        assert!(reply.is_empty());
    }
}

#[test]
fn a_reply_that_cannot_be_encoded_is_an_internal_refusal() {
    inject(&["encode"]);
    let (served, reply) = exchange(&Inspect { request: [3; 16] }.encode().unwrap(), true, LONG);
    inject(&[]);
    assert_eq!(served, Served::Replied(Kind::Refused, Some(Code::Internal)));
    assert_eq!(
        Refused::decode(&reply, Direction::Reply).unwrap().request,
        [3; 16]
    );
}

/// Restores thread-local test hooks on drop, so an early return or panic never
/// leaks injected faults or a pending release sender into later tests.
///
/// Dropped in `Fixture::inspect_under` before joining the fixture server thread,
/// which drops any unsent `Sender` armed via `release_before_hangup` so a
/// fixture blocked in `recv()` unblocks without a timer. Together with
/// `hold_until_eof`, the server end is closed only after the client has passed
/// `peer(&stream)` or hung up (its stream dropped by `inspect` returning, or
/// its `ClientHooks` drop dropping the `Sender`), never by a timer, so the
/// client's peer credential read always happens on a connected socket in every
/// interleaving; a stalled test is bounded by the test runner's own timeout.
struct ClientHooks;

impl ClientHooks {
    fn arm(points: &'static [&'static str], release: Option<mpsc::Sender<()>>) -> Self {
        inject(points);
        release_before_hangup(release);
        Self
    }
}

impl Drop for ClientHooks {
    fn drop(&mut self) {
        inject(&[]);
        release_before_hangup(None);
    }
}

/// Holds the server end of a fixture connection open until the client hangs up,
/// discarding any bytes the client writes.
///
/// A fixture that closes right after `accept()`—or on a timer—races the
/// client's peer credential read: on macOS `LOCAL_PEERPID` fails with
/// `ENOTCONN` once the peer has closed. Because `hold_until_eof` has no overall
/// deadline and closes only after the client has hung up (its stream dropped by
/// `inspect` returning), never by a timer, the client's peer credential read
/// always happens on a connected socket in every interleaving; a stalled test
/// is bounded by the test runner's own timeout. Reading via `read_within` (and
/// continuing on `Interrupted`, `WouldBlock`, or `TimedOut`) keeps the no-panic
/// handling of `EINVAL` from `set_read_timeout` if the client already closed.
fn hold_until_eof(mut stream: UnixStream) {
    use std::io::ErrorKind::{Interrupted, TimedOut, WouldBlock};
    let mut sink = [0; 256];
    loop {
        match stream.read_within(&mut sink, LONG) {
            Ok(0) => return,
            Ok(_) => {}
            Err(error) if matches!(error.kind(), Interrupted | WouldBlock | TimedOut) => {}
            Err(_) => return,
        }
    }
}

/// A lifetime directory `<root>/generations/<LIFETIME>` with a fixture
/// window on its update.sock that answers each connection with `answer`.
struct Fixture {
    root: PathBuf,
    lifetime: PathBuf,
    control: UnixListener,
    server: Option<thread::JoinHandle<()>>,
}

impl Fixture {
    fn new(answer: impl FnOnce(UnixStream, &Path) + Send + 'static) -> Self {
        let root = private_runtime();
        let lifetime = root.join("generations").join(LIFETIME);
        fs::create_dir_all(&lifetime).unwrap();
        fs::set_permissions(root.join("generations"), fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&lifetime, fs::Permissions::from_mode(0o700)).unwrap();
        // A window's eon.sock the client must never touch.
        let control = UnixListener::bind(lifetime.join("eon.sock")).unwrap();
        control.set_nonblocking(true).unwrap();
        let path = lifetime.join(SOCKET);
        let listener = UnixListener::bind(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let directory = lifetime.clone();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            answer(stream, &directory);
        });
        Self {
            root,
            lifetime,
            control,
            server: Some(server),
        }
    }

    /// Runs `inspect` against this fixture with no client-side faults.
    /// Callers that need a client-side fault use `inspect_with`, because
    /// `inspect` clears the client thread's hooks.
    fn inspect(self) -> Result<Inspected, Refusal> {
        self.inspect_under(&[], None)
    }

    fn inspect_under(
        mut self,
        points: &'static [&'static str],
        release: Option<mpsc::Sender<()>>,
    ) -> Result<Inspected, Refusal> {
        let result = {
            let _hooks = ClientHooks::arm(points, release);
            inspect(&self.lifetime, Instant::now() + LONG)
        };
        self.server.take().unwrap().join().unwrap();
        // Zero eon.sock connections: inspecting never speaks EONW.
        assert_eq!(
            self.control
                .accept()
                .map(drop)
                .map_err(|error| error.kind()),
            Err(io::ErrorKind::WouldBlock)
        );
        fs::remove_dir_all(&self.root).unwrap();
        result
    }
}

/// A fixture window that answers with `fixture`, changed by `edit`.
fn answering(edit: fn(&mut Inspected, &Path)) -> impl FnOnce(UnixStream, &Path) + Send {
    move |stream, lifetime| {
        let root = lifetime.parent().unwrap().parent().unwrap().to_owned();
        serve_connection(stream, Instant::now() + LONG, |request| {
            let mut inspected = fixture(request, &root, LIFETIME, std::process::id());
            edit(&mut inspected, lifetime);
            Ok(inspected)
        });
    }
}

#[test]
fn the_client_inspects_a_window_of_another_eonw() {
    let inspected = Fixture::new(answering(|_, _| {})).inspect().unwrap();
    assert_eq!(inspected.eonw, 46);
    assert_eq!(inspected.lifetime, LIFETIME);
}

#[test]
fn the_client_accepts_the_namespace_through_an_alias() {
    // The window reports its root through a symlink, as /tmp and
    // /private/tmp on macOS.
    let inspected = Fixture::new(answering(|inspected, lifetime| {
        let root = lifetime.parent().unwrap().parent().unwrap();
        let alias = PathBuf::from(format!("{}-alias", root.display()));
        let _ = std::os::unix::fs::symlink(root, &alias);
        inspected.namespace = alias.as_os_str().as_encoded_bytes().to_vec();
    }))
    .inspect();
    assert!(inspected.is_ok(), "{inspected:?}");
}

#[test]
fn the_client_refuses_another_window_or_process() {
    type Edit = fn(&mut Inspected, &Path);
    let cases: [(Edit, &str); 3] = [
        (
            |inspected, _| inspected.lifetime = BUILD.into(),
            "another lifetime",
        ),
        (
            |inspected, _| inspected.namespace = b"/".to_vec(),
            "another namespace",
        ),
        (
            |inspected, _| inspected.pid = inspected.pid.wrapping_add(1),
            "another process",
        ),
    ];
    for (edit, reason) in cases {
        match Fixture::new(answering(edit)).inspect() {
            Err(Refusal::NotThisWindow(found)) => assert!(found.contains(reason), "{found}"),
            other => panic!("{reason}: {other:?}"),
        }
    }
}

#[test]
fn the_client_refuses_a_reply_to_another_request() {
    let result = Fixture::new(answering(|inspected, _| inspected.request = [0xee; 16])).inspect();
    assert!(matches!(result, Err(Refusal::Malformed(_))), "{result:?}");
}

#[test]
fn the_client_refuses_an_endpoint_swapped_while_it_answered() {
    let result = Fixture::new(|stream, lifetime: &Path| {
        let swapped = lifetime.join("swap.sock");
        let _new = UnixListener::bind(&swapped).unwrap();
        fs::set_permissions(&swapped, fs::Permissions::from_mode(0o600)).unwrap();
        fs::rename(&swapped, lifetime.join(SOCKET)).unwrap();
        let root = lifetime.parent().unwrap().parent().unwrap().to_owned();
        serve_connection(stream, Instant::now() + LONG, |request| {
            Ok(fixture(request, &root, LIFETIME, std::process::id()))
        });
    })
    .inspect();
    assert_eq!(result, Err(Refusal::EndpointChanged));
}

#[test]
fn the_client_reports_another_version_with_the_manual_move() {
    let result = Fixture::new(|mut stream, _: &Path| {
        let mut request = [0; 28];
        let _ = stream.read_exact(&mut request);
        let _ = stream.write_all(&raw(2, 130, &[0; 19]));
    })
    .inspect();
    assert_eq!(result, Err(Refusal::Version { window: 2, ours: 1 }));
    assert_eq!(
        Refusal::Version { window: 2, ours: 1 }.to_string(),
        "this window speaks Update v2; this build speaks v1; move the window by hand once"
    );
}

#[test]
fn the_client_refuses_a_peer_of_another_user() {
    let result = inspect_with(
        Fixture::new(|stream, _: &Path| hold_until_eof(stream)),
        &["peer-uid"],
    );
    match result {
        Err(Refusal::Unsafe(reason)) => assert!(reason.contains("another user"), "{reason}"),
        other => panic!("expected Unsafe(another user), got {other:?}"),
    }
}

#[test]
fn the_client_passes_a_window_refusal_through() {
    let result = Fixture::new(|stream, _: &Path| {
        inject(&["encode"]);
        serve_connection(stream, Instant::now() + LONG, |request| {
            Ok(fixture(request, Path::new("/"), LIFETIME, 1))
        });
    })
    .inspect();
    assert!(
        matches!(
            result,
            Err(Refusal::Refused {
                code: Code::Internal,
                ..
            })
        ),
        "{result:?}"
    );
}

/// A fixture window that reads the request and answers with `reply(request)`.
fn replying(
    reply: impl FnOnce(RequestId) -> Vec<u8> + Send + 'static,
) -> impl FnOnce(UnixStream, &Path) + Send {
    move |mut stream, _| {
        let mut request = [0; 28];
        stream.read_exact(&mut request).unwrap();
        let id: RequestId = request[HEADER_BYTES..].try_into().unwrap();
        let _ = stream.write_all(&reply(id));
    }
}

fn refused(request: RequestId) -> Vec<u8> {
    Refused {
        request,
        code: Code::Busy,
        detail: "busy".into(),
    }
    .encode()
    .unwrap()
}

#[test]
fn the_client_checks_a_refusal_answers_this_request() {
    let result = Fixture::new(replying(|_| refused([0xee; 16]))).inspect();
    assert!(matches!(result, Err(Refusal::Malformed(_))), "{result:?}");
    // All zero: the window could not read the request.
    let result = Fixture::new(replying(|_| refused([0; 16]))).inspect();
    assert!(
        matches!(
            result,
            Err(Refusal::Refused {
                code: Code::Busy,
                ..
            })
        ),
        "{result:?}"
    );
    let result = Fixture::new(replying(refused)).inspect();
    assert!(
        matches!(
            result,
            Err(Refusal::Refused {
                code: Code::Busy,
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn a_refusal_from_a_swapped_endpoint_is_not_this_windows() {
    let result = Fixture::new(|mut stream, lifetime: &Path| {
        let mut request = [0; 28];
        stream.read_exact(&mut request).unwrap();
        let swapped = lifetime.join("swap.sock");
        let _new = UnixListener::bind(&swapped).unwrap();
        fs::set_permissions(&swapped, fs::Permissions::from_mode(0o600)).unwrap();
        fs::rename(&swapped, lifetime.join(SOCKET)).unwrap();
        let id: RequestId = request[HEADER_BYTES..].try_into().unwrap();
        stream.write_all(&refused(id)).unwrap();
    })
    .inspect();
    assert_eq!(result, Err(Refusal::EndpointChanged));
}

/// `inspect` with `points` injected on this, the client's, thread only.
fn inspect_with(fixture: Fixture, points: &'static [&'static str]) -> Result<Inspected, Refusal> {
    fixture.inspect_under(points, None)
}

#[test]
fn a_reply_buffered_before_the_window_closed_is_read() {
    // The window reads the request, writes its whole reply and closes. The
    // client waits until it has seen the hang-up, then sets its read
    // timeout, which fails with EINVAL as on Darwin after a peer close;
    // `bound_io` must still let it read the buffered reply.
    let points = &["await-close-before-read", "einval-on-read-timeout"];
    let complete = Fixture::new(|mut stream, lifetime: &Path| {
        let root = lifetime.parent().unwrap().parent().unwrap().to_owned();
        let mut request = [0; 28];
        stream.read_exact(&mut request).unwrap();
        let id: RequestId = request[HEADER_BYTES..].try_into().unwrap();
        let reply = fixture(id, &root, LIFETIME, std::process::id());
        stream.write_all(&reply.encode().unwrap()).unwrap();
    });
    let result = inspect_with(complete, points);
    assert!(result.is_ok(), "{result:?}");
    let other_version = Fixture::new(|mut stream, _: &Path| {
        let mut request = [0; 28];
        stream.read_exact(&mut request).unwrap();
        stream.write_all(&raw(2, 130, &[0; 19])).unwrap();
    });
    assert_eq!(
        inspect_with(other_version, points),
        Err(Refusal::Version { window: 2, ours: 1 })
    );
}

#[test]
fn a_reply_from_a_window_that_closed_before_the_request_is_read() {
    // The window answers without reading and closes before the client sets
    // its write timeout: both options fail with EINVAL, as on Darwin, the
    // request write fails, and the reply must still be read. The fixture
    // blocks on `release_before_hangup` without a timer before writing and
    // closing so the client's `peer(&stream)` always sees a connected socket
    // in every interleaving.
    let points = &[
        "await-close-before-write",
        "einval-on-write-timeout",
        "einval-on-read-timeout",
    ];
    let inspect_early_reply = |reply: Vec<u8>| {
        let (release_tx, release_rx) = mpsc::channel();
        let fixture = Fixture::new(move |mut stream, _: &Path| {
            if release_rx.recv().is_ok() {
                let _ = stream.write_all(&reply);
            }
        });
        fixture.inspect_under(points, Some(release_tx))
    };
    assert_eq!(
        inspect_early_reply(raw(2, 130, &[0; 19])),
        Err(Refusal::Version { window: 2, ours: 1 })
    );
    // The approved all-zero ID: the window could not read the request.
    let result = inspect_early_reply(refused([0; 16]));
    assert!(
        matches!(
            result,
            Err(Refusal::Refused {
                code: Code::Busy,
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn a_window_that_cannot_read_its_own_record_refuses_internally() {
    let (window, mut client) = UnixStream::pair().unwrap();
    client
        .write_all(&Inspect { request: [6; 16] }.encode().unwrap())
        .unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let served = serve_connection(window, Instant::now() + LONG, |_| {
        Err("cannot read the identity record".into())
    });
    assert_eq!(served, Served::Replied(Kind::Refused, Some(Code::Internal)));
    let mut reply = Vec::new();
    client.read_to_end(&mut reply).unwrap();
    let refused = Refused::decode(&reply, Direction::Reply).unwrap();
    assert_eq!(refused.request, [6; 16]);
    assert!(
        refused.detail.contains("identity record"),
        "{}",
        refused.detail
    );
}

#[test]
fn the_endpoint_module_names_no_workspace_module() {
    // The Runtime side of EONU never learns EONW either: no versioned
    // module path (`v51::`, `crate::v51`, any version) in update.rs.
    let source = include_str!("../update.rs");
    let bytes = source.as_bytes();
    for (at, _) in source.match_indices('v') {
        let digits = bytes[at + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        assert!(
            digits == 0 || !source[at + 1 + digits..].starts_with("::"),
            "update.rs names a versioned module at byte {at}"
        );
    }
    assert!(!source.contains("crate::v"));
}

#[test]
fn a_window_still_open_fails_the_wait_before_any_option_call() {
    // The window holds its end open until the client hangs up after the 1 s
    // wait expires. The client must stop at the wait, before its option call:
    // with the EINVAL injected, reaching the option would refuse with a
    // different text.
    let held = |read_first: bool| {
        move |mut stream: UnixStream, _: &Path| {
            if read_first {
                let mut request = [0; 28];
                if stream.read_exact(&mut request).is_ok() {
                    let _ = stream.write_all(&raw(2, 130, &[0; 19]));
                }
            }
            hold_until_eof(stream);
        }
    };
    for (read_first, points) in [
        (
            false,
            &["await-close-before-write", "einval-on-write-timeout"][..],
        ),
        (true, &["await-close-before-read", "einval-on-read-timeout"]),
    ] {
        let result = inspect_with(Fixture::new(held(read_first)), points);
        match result {
            Err(Refusal::Unavailable(reason)) => {
                assert!(reason.contains(NO_HANGUP), "{read_first}: {reason}")
            }
            other => panic!("{read_first}: {other:?}"),
        }
    }
}
