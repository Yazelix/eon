//! AppKit event-path proof driver (`harness = false`, eon-33cww.4.2).
//! Parent supervisor owns isolation, Orbit, trace socket, deadline, and PID-pinned teardown
//! (surviving `rune::fast_exit`); child worker hosts Venus or Rune on AppKit's main thread.

#[path = "appkit_event_driver/judge.rs"]
mod judge;

use judge::*;
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    net::sockopt::set_socket_recv_buffer_size,
    process::{Pid, Signal, kill_process},
    termios::{OptionalActions, tcgetattr, tcgetwinsize, tcsetattr},
};
use serde_json::Value;
use std::{
    env, fs,
    io::{self, Read, Write},
    os::unix::{
        fs::DirBuilderExt,
        net::{UnixDatagram, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const WORKER_FLAGS: [&str; 2] = ["--cursor-effect-v1", "none"];

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}
fn err<E: std::fmt::Display>(ctx: &str) -> impl FnOnce(E) -> String + '_ {
    move |e| format!("{ctx}: {e}")
}

struct CellGuard {
    run_dir: PathBuf,
    children: Vec<Child>,
}

impl CellGuard {
    fn reap(child: &mut Child) {
        if !matches!(child.try_wait(), Ok(None)) {
            return;
        }
        // `child` is unreaped, so its PID stays kernel-pinned until `try_wait()` returns `Ok(Some(_))`.
        if let Some(pid) = Pid::from_raw(child.id() as i32) {
            let _ = kill_process(pid, Signal::TERM);
        }
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end && matches!(child.try_wait(), Ok(None)) {
            thread::sleep(ms(25));
        }
        if matches!(child.try_wait(), Ok(None)) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for CellGuard {
    fn drop(&mut self) {
        let _ = fs::write(self.run_dir.join("finish"), "");
        self.children.iter_mut().rev().for_each(Self::reap);
        let _ = fs::remove_dir_all(&self.run_dir);
    }
}

struct TraceCollector {
    stop: Arc<AtomicBool>,
    lines: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl TraceCollector {
    fn bind(path: &Path, ack_sock: Option<PathBuf>) -> Result<Self, String> {
        let sock = UnixDatagram::bind(path).map_err(err("bind trace"))?;
        set_socket_recv_buffer_size(&sock, 256 * 1024).map_err(err("trace rcvbuf"))?;
        sock.set_read_timeout(Some(ms(50)))
            .map_err(err("trace rto"))?;
        sock.set_write_timeout(Some(ms(50)))
            .map_err(err("trace wto"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let lines = Arc::new(Mutex::new(Vec::new()));
        let (s2, l2) = (stop.clone(), lines.clone());
        let handle = thread::spawn(move || {
            let (mut buf, mut storm, mut kind, mut idx) = ([0u8; 4096], false, None::<char>, 0u64);
            while let Ok(n) = sock.recv(&mut buf).or_else(|e| {
                if s2.load(Ordering::Relaxed) {
                    Err(e)
                } else {
                    Ok(0)
                }
            }) {
                if n == 0 {
                    continue;
                }
                let line = String::from_utf8_lossy(&buf[..n]).into_owned();
                if let Some(ack) = &ack_sock {
                    let mut p = line.splitn(7, '\t');
                    match (p.nth(5), p.next()) {
                        (Some("driver_open"), Some("storm")) => {
                            (storm, kind, idx) = (true, None, 0)
                        }
                        (Some("driver_close"), Some("storm")) => (storm, kind) = (false, None),
                        (Some("input"), Some("modifier Pressed")) if storm => kind = Some('P'),
                        (Some("input"), Some("modifier Released")) if storm => kind = Some('R'),
                        (Some("input_end"), _) if storm => {
                            if let Some(k) = kind.take() {
                                let _ = sock.send_to(format!("{idx}:{k}").as_bytes(), ack);
                                idx += 1;
                            }
                        }
                        _ => {}
                    }
                }
                l2.lock().unwrap().push(line);
            }
        });
        Ok(Self {
            stop,
            lines,
            handle: Some(handle),
        })
    }

    fn finish(self) -> Vec<String> {
        let lines = self.lines.clone();
        drop(self);
        std::mem::take(&mut *lines.lock().unwrap())
    }
}

impl Drop for TraceCollector {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        return ExitCode::SUCCESS;
    }
    if args.first().map(String::as_str) == Some("--pty-payload") {
        return match pty_payload(&args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("pty-payload: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if env::var_os("EON_APPKIT_DRIVER_WORKER").is_some() {
        #[cfg(target_os = "macos")]
        return appkit::run_worker();
        #[cfg(not(target_os = "macos"))]
        return ExitCode::FAILURE;
    }
    let out = driver_flag(&args, "--out").or_else(|| {
        env::var("EON_APPKIT_PROOF_OUT")
            .ok()
            .filter(|s| !s.is_empty())
    });
    let Some(out) = out else {
        if has_driver_flag(&args) {
            eprintln!("appkit-event-driver: --out DIR or EON_APPKIT_PROOF_OUT is required");
            return ExitCode::FAILURE;
        }
        return ExitCode::SUCCESS;
    };
    match run_parent(&args, Path::new(&out)) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("appkit-event-driver: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_parent(args: &[String], out: &Path) -> Result<bool, String> {
    if !cfg!(target_os = "macos") {
        return Err("AppKit event driver requires macOS".into());
    }
    let host = driver_flag(args, "--host").unwrap_or_else(|| "venus".into());
    let orbit = driver_flag(args, "--orbit").ok_or("--orbit PATH is required")?;
    let mac_proof = driver_flag(args, "--mac-proof").ok_or("--mac-proof PATH is required")?;
    let secs: u64 = driver_flag(args, "--deadline")
        .unwrap_or_else(|| "180".into())
        .parse()
        .map_err(err("--deadline"))?;
    if !(30..=600).contains(&secs) {
        return Err("--deadline must be 30..=600".into());
    }
    let only = driver_flag(args, "--only");
    let fixtures: Vec<&str> = match only.as_deref() {
        Some(l) => l.split(',').filter(|s| !s.is_empty()).collect(),
        None => FIXTURES.iter().map(|(f, _)| *f).collect(),
    };
    fs::create_dir_all(out).map_err(err("mkdir"))?;
    let exe = env::current_exe().map_err(err("exe"))?;
    let (platform, mut cells, dl) = (host_platform(), Vec::new(), Duration::from_secs(secs));
    let paths = (exe.as_path(), Path::new(&orbit), Path::new(&mac_proof));
    let put = |p: PathBuf, v: &Value| {
        fs::write(p, serde_json::to_string_pretty(v).unwrap()).map_err(err("write"))
    };
    for (idx, fixture) in fixtures.into_iter().enumerate() {
        let cell_out = out.join(format!("{fixture}-{host}"));
        fs::create_dir_all(&cell_out).map_err(err("mkdir"))?;
        let (res, fails, counts) = match run_cell(paths, &host, dl, fixture, idx, &cell_out) {
            Ok((c, f)) if f.is_empty() => ("pass", f, c),
            Ok((c, f)) => ("fail", f, c),
            Err(e) => ("error", vec![e], Value::Null),
        };
        let cell = receipt_json(platform, fixture, &host, res, &fails, counts);
        put(cell_out.join("receipt.json"), &cell)?;
        cells.push(cell);
    }
    let summary = result_json(platform, &host, &WORKER_FLAGS, cells);
    put(out.join("result.json"), &summary)?;
    Ok(summary["result"] == "pass")
}

fn run_cell(
    (exe, orbit, mac_proof): (&Path, &Path, &Path),
    host: &str,
    timeout: Duration,
    fixture: &str,
    idx: usize,
    cell_out: &Path,
) -> Result<(Value, Vec<String>), String> {
    let run_dir = match env::var_os("EON_APPKIT_RUN_ROOT").filter(|s| !s.is_empty()) {
        Some(root) => PathBuf::from(root).join(idx.to_string()),
        None => PathBuf::from(format!("/tmp/ev-{}-{idx}", std::process::id())),
    };
    let _ = fs::remove_dir_all(&run_dir);
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&run_dir)
        .map_err(err("tmp"))?;
    let mut guard = CellGuard {
        run_dir: run_dir.clone(),
        children: Vec::new(),
    };
    let (trace_sock, orbit_sock) = (run_dir.join("t.sock"), run_dir.join("o.sock"));
    let ack_sock = (fixture == "modifier-storm").then(|| run_dir.join("m.sock"));
    let collector = TraceCollector::bind(&trace_sock, ack_sock)?;
    let start_orbit = |g: &mut CellGuard| -> Result<(), String> {
        let log = fs::File::create(cell_out.join("orbit.stderr.log")).map_err(err("orbit log"))?;
        let s = orbit_sock.to_string_lossy();
        let (e, d) = (exe.to_string_lossy(), run_dir.to_string_lossy());
        let mut cmd = Command::new(orbit);
        cmd.args(["serve", &s, "--", &e, "--pty-payload", fixture, &d]);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::from(log));
        let i = g.children.len();
        g.children.push(cmd.spawn().map_err(err("orbit"))?);
        let end = Instant::now() + Duration::from_secs(10);
        while !orbit_sock.exists() && Instant::now() < end {
            if let Ok(Some(st)) = g.children[i].try_wait() {
                return Err(format!("orbit exited {st} before publishing socket"));
            }
            thread::sleep(ms(20));
        }
        orbit_sock
            .exists()
            .then_some(())
            .ok_or_else(|| "orbit socket did not appear".into())
    };
    let mut need_orbit = fixture.starts_with("moving-caret-");
    if !need_orbit {
        start_orbit(&mut guard)?;
    }
    let stderr_path = cell_out.join("venus.stderr.log");
    let metrics_path = cell_out.join("metrics.json");
    let stderr_file = fs::File::create(&stderr_path).map_err(err("stderr"))?;
    let worker_idx = guard.children.len();
    let mut cmd = Command::new(exe);
    cmd.args(WORKER_FLAGS).arg(&orbit_sock).envs([
        ("EON_APPKIT_DRIVER_WORKER", Path::new("1")),
        ("EON_APPKIT_FIXTURE", Path::new(fixture)),
        ("EON_APPKIT_HOST", Path::new(host)),
        ("EON_APPKIT_DIR", &run_dir),
        ("EON_APPKIT_METRICS", &metrics_path),
        ("EON_APPKIT_MAC_PROOF", mac_proof),
        ("EON_BENCH_TRACE", &trace_sock),
    ]);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_file));
    guard.children.push(cmd.spawn().map_err(err("worker"))?);
    let end = Instant::now() + timeout;
    let status = loop {
        if need_orbit && run_dir.join("primed").exists() {
            start_orbit(&mut guard)?;
            need_orbit = false;
        }
        match guard.children[worker_idx].try_wait() {
            Ok(Some(st)) => break Ok(st),
            Ok(None) if Instant::now() < end => thread::sleep(ms(25)),
            _ => break Err("cell timed out".to_owned()),
        }
    }?;
    drop(guard);
    let trace = parse_trace(&collector.finish(), expected_windows(fixture));
    let stderr = fs::read_to_string(&stderr_path).unwrap_or_default();
    let raw = fs::read_to_string(&metrics_path).unwrap_or_default();
    let v: Value = serde_json::from_str(&raw)
        .map_err(|_| format!("worker exited {status} without metrics.json"))?;
    let (counts, mut failures) = evaluate(fixture, &v, &trace, &stderr);
    if !status.success() && failures.is_empty() {
        failures.push(format!("worker exited {status}"));
    }
    Ok((counts, failures))
}

fn atomic_write(path: &Path, bytes: &[u8]) {
    let tmp = path.with_extension("tmp");
    let _ = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, path));
}

fn pty_payload(args: &[String]) -> Result<(), String> {
    let [fixture, dir] = args else {
        return Err("need FIXTURE DIR".into());
    };
    let dir = Path::new(dir);
    let life = match env::var_os("EON_APPKIT_RUN_ROOT").filter(|s| !s.is_empty()) {
        Some(r) => {
            let s = UnixStream::connect(PathBuf::from(r).join("life.sock"))
                .map_err(|e| format!("life.sock: {e}"))?;
            let _ = s.set_nonblocking(true);
            Some(s)
        }
        None => None,
    };
    let life_dead = || {
        life.as_ref()
            .is_some_and(|mut s| match s.read(&mut [0; 1]) {
                Ok(1..) => false,
                Err(e) => !matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ),
                Ok(0) => true,
            })
    };
    let done = || !dir.exists() || dir.join("finish").exists() || life_dead();
    let (stdin, mut out) = (io::stdin(), io::stdout());
    if let Ok(mut mode) = tcgetattr(&stdin) {
        mode.make_raw();
        let _ = tcsetattr(&stdin, OptionalActions::Now, &mode);
    }
    let winsize = || tcgetwinsize(&stdin).ok().map(|w| (w.ws_row, w.ws_col));
    let mut put = |s: &str| {
        let _ = out.write_all(s.as_bytes()).and_then(|()| out.flush());
    };
    let wait_done = || {
        while !done() {
            thread::sleep(ms(20));
        }
    };
    put(match fixture.as_str() {
        "moving-caret-30" | "moving-caret-60" => "\x1b[?25h\x1b[2 q\x1b[2J\x1b[H",
        "configure-storm" => "\x1b[?25h\x1b[2 q\x1b[2J\x1b[999;1H",
        _ => "\x1b[?25h\x1b[2 q\x1b[2J\x1b[HREADY",
    });
    if fixture == "held-repeat" {
        atomic_write(&dir.join("count.txt"), b"0");
    }
    let _ = fs::write(dir.join("ready"), "");
    match fixture.as_str() {
        "static-echo" | "held-repeat" => {
            let (mut acc, mut count, mut buf) = (Vec::new(), 0u64, [0u8; 128]);
            let wait = Timespec {
                tv_sec: 0,
                tv_nsec: 20_000_000,
            };
            let hangup = PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL;
            while !done() {
                let mut fds = [PollFd::new(&stdin, PollFlags::IN)];
                match poll(&mut fds, Some(&wait)) {
                    Ok(1..) if fds[0].revents().contains(PollFlags::IN) => {
                        let Ok(n @ 1..) = rustix::io::read(&stdin, &mut buf) else {
                            break;
                        };
                        if fixture == "static-echo" {
                            acc.extend_from_slice(&buf[..n]);
                            atomic_write(&dir.join("committed.txt"), &acc);
                        } else {
                            count += buf[..n].iter().filter(|&&b| b == b'1').count() as u64;
                            atomic_write(&dir.join("count.txt"), count.to_string().as_bytes());
                            put(&format!("\r{count} "));
                        }
                    }
                    Ok(1..) if fds[0].revents().intersects(hangup) => break,
                    Err(_) => break,
                    _ => {}
                }
            }
        }
        "moving-caret-30" | "moving-caret-60" => {
            while !dir.join("go").exists() && !done() {
                thread::sleep(ms(5));
            }
            let rows = winsize().map(|(r, _)| r).filter(|&r| r > 0).unwrap_or(24);
            let ns = if fixture.ends_with("30") {
                33_333_333
            } else {
                16_666_667
            };
            let (step, start) = (Duration::from_nanos(ns), Instant::now());
            for n in 0..CARET_TICKS {
                if done() {
                    return Ok(());
                }
                put(&format!("\x1b[{rows};1H\x1b[2K{n:03}"));
                for k in 0..CARET_MOVES {
                    put(&format!("\x1b[{rows};{}H", 6 + (CARET_MOVES * n + k) % 60));
                    thread::sleep(ms(2));
                }
                let next = start + step * (n as u32 + 1);
                if let Some(w) = next.checked_duration_since(Instant::now()) {
                    thread::sleep(w);
                }
            }
            put(&format!("\x1b[{rows};1H"));
            let _ = fs::write(dir.join("ticked"), "");
            wait_done();
        }
        "configure-storm" => {
            let mut last = winsize();
            while !done() {
                let cur = winsize();
                if cur != last {
                    last = cur;
                    put("\x1b[999;1H");
                }
                thread::sleep(ms(5));
            }
        }
        _ => wait_done(),
    }
    Ok(())
}

#[cfg(target_os = "macos")]
mod appkit {
    use super::*;
    use dispatch2::DispatchQueue;
    use objc2::{
        MainThreadMarker, msg_send,
        rc::Retained,
        runtime::{AnyObject, NSObjectProtocol},
        sel,
    };
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSEvent, NSEventModifierFlags, NSEventType,
        NSResponder, NSView, NSWindow,
    };
    use objc2_foundation::{
        NSNotFound, NSNotificationCenter, NSPoint, NSProcessInfo, NSRange, NSRect, NSSize, NSString,
    };
    use serde_json::json;
    use std::{
        collections::BTreeSet,
        io::{BufRead, BufReader},
    };

    type Rect = [i64; 4];

    /// Responder verified on the main thread to implement the 4 `NSTextInputClient` selectors.
    struct TextInputClient(Retained<NSResponder>);

    fn main_window(mtm: MainThreadMarker) -> Option<Retained<NSWindow>> {
        let app = NSApplication::sharedApplication(mtm);
        app.keyWindow()
            .or_else(|| app.mainWindow())
            .or_else(|| app.windows().firstObject())
    }

    fn focused_client(
        mtm: MainThreadMarker,
    ) -> Result<(Retained<NSWindow>, TextInputClient), String> {
        let win = main_window(mtm).ok_or("no NSWindow")?;
        let has_ime = |r: &NSResponder| {
            [
                sel!(setMarkedText:selectedRange:replacementRange:),
                sel!(unmarkText),
                sel!(insertText:replacementRange:),
                sel!(firstRectForCharacterRange:actualRange:),
            ]
            .into_iter()
            .all(|s| r.respondsToSelector(s))
        };
        let resp = win
            .firstResponder()
            .filter(|r| has_ime(r))
            .or_else(|| {
                win.contentView()
                    .map(Retained::into_super)
                    .filter(|r| has_ime(r))
            })
            .ok_or("no NSTextInputClient responder")?;
        Ok((win, TextInputClient(resp)))
    }

    fn activate_and_focus(mtm: MainThreadMarker) -> Result<bool, String> {
        let app = NSApplication::sharedApplication(mtm);
        let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
        let (win, client) = focused_client(mtm)?;
        win.makeKeyAndOrderFront(None);
        let _ = win.makeFirstResponder(Some(&client.0));
        Ok(win.isKeyWindow())
    }

    fn set_marked(client: &TextInputClient, _mtm: MainThreadMarker, text: &str, sel_len: usize) {
        let (s, sr) = (NSString::from_str(text), NSRange::new(0, sel_len));
        let rr = NSRange::new(NSNotFound as usize, 0);
        #[allow(unsafe_code)]
        // SAFETY: `MainThreadMarker` proves main thread; `TextInputClient` verified selector; `&*s` is valid NSString.
        let _: () = unsafe {
            msg_send![&*client.0, setMarkedText: &*s, selectedRange: sr, replacementRange: rr]
        };
    }

    fn unmark(client: &TextInputClient, _mtm: MainThreadMarker) {
        #[allow(unsafe_code)]
        // SAFETY: `MainThreadMarker` proves main thread; `TextInputClient` verified `unmarkText`.
        let _: () = unsafe { msg_send![&*client.0, unmarkText] };
    }

    fn insert_text(client: &TextInputClient, _mtm: MainThreadMarker, text: &str) {
        let s = NSString::from_str(text);
        let rr = NSRange::new(NSNotFound as usize, 0);
        #[allow(unsafe_code)]
        // SAFETY: `MainThreadMarker` proves main thread; `TextInputClient` verified selector; `&*s` is valid NSString.
        let _: () = unsafe { msg_send![&*client.0, insertText: &*s, replacementRange: rr] };
    }

    fn first_rect(client: &TextInputClient, _mtm: MainThreadMarker) -> Rect {
        let (r, null) = (NSRange::new(0, 0), std::ptr::null_mut::<NSRange>());
        #[allow(unsafe_code)]
        // SAFETY: `MainThreadMarker` proves main thread; `TextInputClient` verified selector; `actualRange` is nullable.
        let rect: NSRect =
            unsafe { msg_send![&*client.0, firstRectForCharacterRange: r, actualRange: null] };
        let (o, s) = (rect.origin, rect.size);
        [o.x, o.y, s.width, s.height].map(|v| v.round() as i64)
    }

    fn post_frame_did_change(view: &NSView, _mtm: MainThreadMarker) {
        let (obj, name): (&AnyObject, _) =
            (view, NSString::from_str("NSViewFrameDidChangeNotification"));
        #[allow(unsafe_code)]
        // SAFETY: `MainThreadMarker` proves main thread; `view` is `win.contentView()` matching `NSViewFrameDidChangeNotification`'s object type.
        unsafe {
            NSNotificationCenter::defaultCenter().postNotificationName_object(&name, Some(obj))
        };
    }

    fn on_main<R: Send, F: FnOnce(MainThreadMarker) -> R + Send>(f: F) -> R {
        let mut out = None;
        DispatchQueue::main()
            .exec_sync(|| out = Some(f(MainThreadMarker::new().expect("main queue"))));
        out.expect("main queue ran")
    }

    fn drain(t: u64) {
        thread::sleep(ms(t));
        on_main(|_| ());
    }

    fn trace_window<R>(phase: &str, f: impl FnOnce() -> R) -> R {
        on_main(|_| yazelix_venus::bench_trace("driver_open", phase));
        let out = f();
        drain(80);
        on_main(|_| yazelix_venus::bench_trace("driver_close", phase));
        out
    }

    fn caret_rect(fallback: Rect) -> Rect {
        on_main(|m| focused_client(m).map_or(fallback, |(_, c)| first_rect(&c, m)))
    }

    fn wait_until(limit: Duration, step: Duration, mut pred: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + limit;
        while Instant::now() < end && !pred() {
            thread::sleep(step);
        }
        pred()
    }

    fn wait_ready(dir: &Path) -> Result<(), String> {
        let ready = || dir.join("ready").exists() && caret_rect([0; 4])[2..].iter().all(|&v| v > 0);
        if !wait_until(Duration::from_secs(10), ms(25), ready) {
            return Err("Orbit/IME cursor area did not become ready within 10s".into());
        }
        drain(50);
        Ok(())
    }

    fn read_count(dir: &Path) -> u64 {
        fs::read_to_string(dir.join("count.txt"))
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    fn measure_dwell(dir: &Path) -> (u64, u64) {
        thread::sleep(ms(SETTLE_MS));
        let before = read_count(dir);
        thread::sleep(ms(420));
        (before, read_count(dir).saturating_sub(before))
    }

    fn resize_content_no_display(win: &NSWindow, size: NSSize) {
        let cur = win.frame();
        let mut next = win.frameRectForContentRect(NSRect::new(NSPoint::new(0.0, 0.0), size));
        next.origin = NSPoint::new(
            cur.origin.x,
            cur.origin.y + cur.size.height - next.size.height,
        );
        win.setFrame_display(next, false);
    }

    pub fn run_worker() -> ExitCode {
        let env_s = |k| env::var(k).unwrap_or_default();
        let (fixture, dir) = (
            env_s("EON_APPKIT_FIXTURE"),
            PathBuf::from(env_s("EON_APPKIT_DIR")),
        );
        let mac_proof = PathBuf::from(env_s("EON_APPKIT_MAC_PROOF"));
        let out_metrics = PathBuf::from(env_s("EON_APPKIT_METRICS"));
        let host = env::var("EON_APPKIT_HOST").unwrap_or_else(|_| "venus".into());
        thread::spawn(move || {
            let ready = wait_until(Duration::from_secs(15), ms(25), || {
                matches!(on_main(activate_and_focus), Ok(true))
            });
            let metrics = if ready {
                let pid = std::process::id().to_string();
                let res = match fixture.as_str() {
                    "static-echo" => drive_echo(&dir),
                    "moving-caret-30" | "moving-caret-60" => drive_caret(&dir),
                    "held-repeat" => drive_repeat(&dir, &mac_proof, &pid),
                    "configure-storm" => drive_configure(&dir),
                    "modifier-storm" => drive_modifiers(&dir, &mac_proof, &pid),
                    other => Err(format!("unknown fixture {other}")),
                };
                res.unwrap_or_else(|e| json!({"driver_error": e}))
            } else {
                let d = on_main(activate_and_focus)
                    .err()
                    .unwrap_or_else(|| "not key window".into());
                json!({"driver_error": format!("startup timeout (15s): {d}")})
            };
            let _ = fs::write(&out_metrics, serde_json::to_string(&metrics).unwrap());
            on_main(|m| match main_window(m) {
                Some(w) => w.performClose(None),
                None => NSApplication::sharedApplication(m).terminate(None),
            });
        });
        let res = match host.as_str() {
            #[cfg(feature = "candidate-rune")]
            "rune" => yazelix_venus::run_rune_candidate(),
            "venus" => yazelix_venus::run_venus(),
            other => Err(format!("unsupported host {other}").into()),
        };
        if let Err(e) = res {
            eprintln!("worker host error: {e}");
            return ExitCode::FAILURE;
        }
        ExitCode::SUCCESS
    }

    fn drive_echo(dir: &Path) -> Result<Value, String> {
        wait_ready(dir)?;
        let (mut calls, mut changes, mut last) = (0u64, 0u64, caret_rect([0; 4]));
        trace_window("echo", || {
            for _ in 0..(ECHO_COMMITS / 100) {
                on_main(|m| {
                    let Ok((_, c)) = focused_client(m) else {
                        return;
                    };
                    for _ in 0..100 {
                        set_marked(&c, m, "", 0);
                        unmark(&c, m);
                        calls += 1;
                        let r = first_rect(&c, m);
                        if r != last {
                            (changes, last) = (changes + 1, r);
                        }
                    }
                });
                thread::sleep(ms(5));
            }
        });
        on_main(|m| {
            if let Ok((_, c)) = focused_client(m) {
                set_marked(&c, m, "にほん", 3);
                insert_text(&c, m, "日本");
            }
        });
        let committed = dir.join("committed.txt");
        let got = || fs::read_to_string(&committed).is_ok_and(|s| !s.is_empty());
        wait_until(Duration::from_secs(3), ms(20), got);
        drain(150);
        thread::sleep(ms(250));
        let text = fs::read_to_string(&committed).unwrap_or_default();
        Ok(json!({"echo_calls": calls, "rect_changes": changes, "committed": text}))
    }

    fn drive_caret(dir: &Path) -> Result<Value, String> {
        let prime_rect = on_main(|m| {
            let (_, c) = focused_client(m)?;
            set_marked(&c, m, "a", 1);
            let r = first_rect(&c, m);
            unmark(&c, m);
            yazelix_venus::bench_trace("driver_prime", "pre-draw");
            Ok::<_, String>(r)
        })?;
        fs::write(dir.join("primed"), "").map_err(err("write primed"))?;
        wait_ready(dir)?;
        let col1_x = caret_rect([0; 4])[0];
        let (mut seen, mut changes, mut last) = (BTreeSet::new(), 0u64, [0i64; 4]);
        let (mut ticked, mut motion_dwelled) = (false, false);
        trace_window("motion", || {
            let _ = fs::write(dir.join("go"), "");
            let sample = |seen: &mut BTreeSet<Rect>, ch: &mut u64, last: &mut Rect| {
                let r = caret_rect(*last);
                if seen.is_empty() || r != *last {
                    *ch += u64::from(!seen.is_empty());
                    seen.insert(r);
                    *last = r;
                }
            };
            let end = Instant::now() + Duration::from_secs(30);
            while !dir.join("ticked").exists() && Instant::now() < end {
                sample(&mut seen, &mut changes, &mut last);
                thread::sleep(ms(8));
            }
            ticked = dir.join("ticked").exists();
            let (park_end, mut at_col1) = (Instant::now() + Duration::from_secs(3), None);
            while ticked && Instant::now() < park_end {
                sample(&mut seen, &mut changes, &mut last);
                if last[0] == col1_x && seen.iter().any(|r| r[0] > col1_x) {
                    if at_col1.get_or_insert_with(Instant::now).elapsed() >= ms(80) {
                        motion_dwelled = true;
                        break;
                    }
                } else {
                    at_col1 = None;
                }
                thread::sleep(ms(8));
            }
        });
        drain(150);
        let tail_start = caret_rect([0; 4]);
        let parked = ticked
            && motion_dwelled
            && tail_start[2] > 0
            && tail_start[3] > 0
            && tail_start[0] == col1_x;
        last = tail_start;
        let mut tail_rc = 0u64;
        trace_window("tail", || {
            let end = Instant::now() + Duration::from_secs(1);
            while Instant::now() < end {
                let r = caret_rect([0; 4]);
                if r != last {
                    (tail_rc, last) = (tail_rc + 1, r);
                }
                thread::sleep(ms(25));
            }
        });
        let moved = seen.iter().any(|r| *r != prime_rect);
        Ok(json!({
            "ticked": ticked, "parked": parked, "prime_moved": moved,
            "distinct_rects": seen.len() as u64, "rect_changes": changes, "tail_rect_changes": tail_rc,
        }))
    }

    fn drive_repeat(dir: &Path, mac_proof: &Path, pid: &str) -> Result<Value, String> {
        wait_ready(dir)?;
        let (rs, ds, is) = (
            HOLD_REPEATS.to_string(),
            REPEAT_DELAY_MS.to_string(),
            REPEAT_INTERVAL_MS.to_string(),
        );
        let args = [
            "hold-repeat",
            pid,
            "18",
            "0",
            &rs,
            &ds,
            &is,
            "--cancel-on-eof",
        ];
        let run_rep = |tag: &'static str, cancel_at: Option<u64>| -> Result<(bool, u64), String> {
            let mut child = Command::new(mac_proof)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .map_err(err(tag))?;
            let (saw, keep) = match cancel_at {
                Some(target) => {
                    let s = wait_until(ms(1500), ms(20), || read_count(dir) >= target);
                    drop(child.stdin.take());
                    (s, None)
                }
                None => (false, child.stdin.take()),
            };
            let o = child.wait_with_output().map_err(err(tag))?;
            drop(keep);
            if !o.status.success() {
                return Err(format!("{tag} exited {}", o.status));
            }
            drain(100);
            let j: Value = serde_json::from_slice(&o.stdout).map_err(err(tag))?;
            Ok((saw, j["sent_repeats"].as_u64().unwrap_or(0)))
        };
        let (_, hold_sent) = trace_window("hold", || run_rep("hold-repeat", None))?;
        let (held, dwell) = measure_dwell(dir);
        let before = read_count(dir);
        let (saw_input, cancel_sent) =
            trace_window("cancel", || run_rep("cancel", Some(before + 3)))?;
        let (after_settle, c_dwell) = measure_dwell(dir);
        Ok(json!({
            "hold_sent": hold_sent, "held_bytes": held, "dwell_bytes": dwell,
            "cancel_saw_input": saw_input, "cancel_bytes": after_settle.saturating_sub(before),
            "cancel_sent": cancel_sent, "cancel_dwell_bytes": c_dwell,
        }))
    }

    fn drive_configure(dir: &Path) -> Result<Value, String> {
        wait_ready(dir)?;
        on_main(|m| {
            main_window(m)
                .into_iter()
                .for_each(|w| w.setContentSize(NSSize::new(960.0, 600.0)));
        });
        drain(150);
        let (mut same_rc, mut last) = (0u64, caret_rect([0; 4]));
        trace_window("same", || {
            for _ in 0..TOGGLES {
                if let Some(r) = on_main(|m| {
                    let (w, c) = focused_client(m).ok()?;
                    let view = w.contentView()?;
                    post_frame_did_change(&view, m);
                    Some(first_rect(&c, m))
                }) && r != last
                {
                    (same_rc, last) = (same_rc + 1, r);
                }
                thread::sleep(ms(5));
            }
        });
        let mut resize_rc = 0u64;
        trace_window("resize", || -> Result<(), String> {
            for i in 0..MODES {
                let h = if i.is_multiple_of(2) { 520.0 } else { 600.0 };
                on_main(|m| {
                    let (w, _) = focused_client(m)?;
                    let scale = w.backingScaleFactor();
                    let (pw, ph) = ((960.0 * scale).round() as u32, (h * scale).round() as u32);
                    yazelix_venus::bench_trace("driver_probe", format_args!("{i} {pw}x{ph}"));
                    resize_content_no_display(&w, NSSize::new(960.0, h));
                    let ts = NSProcessInfo::processInfo().systemUptime();
                    for ty in [NSEventType::RightMouseDown, NSEventType::RightMouseUp] {
                        let ev = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                            ty, NSPoint::new(100.0, 100.0), NSEventModifierFlags::empty(), ts,
                            w.windowNumber(), None, 0, 1, 1.0,
                        ).ok_or("no RightMouse NSEvent")?;
                        let button = ev.buttonNumber();
                        if button != 1 {
                            return Err(format!("RightMouse NSEvent had buttonNumber {button}"));
                        }
                        w.sendEvent(&ev);
                    }
                    Ok::<_, String>(())
                })?;
                let (end, mut changed_at) = (Instant::now() + ms(1500), None::<Instant>);
                while Instant::now() < end {
                    let r = caret_rect(last);
                    if r != last {
                        (last, resize_rc, changed_at) = (r, resize_rc + 1, Some(Instant::now()));
                    } else if changed_at.is_some_and(|t| t.elapsed() >= ms(80)) {
                        break;
                    }
                    thread::sleep(ms(10));
                }
            }
            Ok(())
        })?;
        Ok(json!({"same_rect_changes": same_rc, "resize_rect_changes": resize_rc}))
    }

    fn drive_modifiers(dir: &Path, mac_proof: &Path, pid: &str) -> Result<Value, String> {
        wait_ready(dir)?;
        let ack = UnixDatagram::bind(dir.join("m.sock")).map_err(err("bind m.sock"))?;
        ack.set_read_timeout(Some(ms(500)))
            .map_err(err("m.sock timeout"))?;
        let (mut changes, mut last, pairs) = (0u64, caret_rect([0; 4]), MODIFIER_PAIRS.to_string());
        let (mut missed_acks, mut extra_acks, mut driver_err) = (0u64, 0u64, None::<String>);
        let drain_extra = |buf: &mut [u8; 16]| -> Result<u64, String> {
            ack.set_nonblocking(true).map_err(err("m.sock nonblock"))?;
            let mut n = 0u64;
            while ack.recv(buf).is_ok() {
                n += 1;
            }
            ack.set_nonblocking(false).map_err(err("m.sock block"))?;
            Ok(n)
        };
        trace_window("storm", || -> Result<(), String> {
            let mut child = Command::new(mac_proof)
                .args(["flags-changed", pid, &pairs, "--step"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .map_err(err("flags-changed"))?;
            let mut stdin = child.stdin.take().ok_or("flags-changed stdin")?;
            let mut stdout = BufReader::new(child.stdout.take().ok_or("flags-changed stdout")?);
            let (mut line, mut buf) = (String::new(), [0u8; 16]);
            let stepped = (|| -> Result<(), String> {
                for step in 0..(2 * MODIFIER_PAIRS) {
                    let stale = drain_extra(&mut buf)?;
                    if stale > 0 {
                        extra_acks += stale;
                        return Err(format!("modifier-storm: {stale} extra acks before {step}"));
                    }
                    line.clear();
                    let n = stdin
                        .write_all(b"\n")
                        .and_then(|()| stdin.flush())
                        .and_then(|()| stdout.read_line(&mut line))
                        .map_err(err("flags-changed io"))?;
                    if n == 0 {
                        return Err("flags-changed exited early".into());
                    }
                    let exp = format!("{step}:{}", if step.is_multiple_of(2) { 'P' } else { 'R' });
                    match ack.recv(&mut buf) {
                        Ok(n) if &buf[..n] == exp.as_bytes() => {}
                        Ok(n) => {
                            extra_acks += 1;
                            let got = String::from_utf8_lossy(&buf[..n]);
                            return Err(format!("modifier-storm: ack {got:?} != {exp:?}"));
                        }
                        Err(_) => {
                            missed_acks += 1;
                            return Err(format!("modifier-storm: missed ack {exp}"));
                        }
                    }
                    let r = caret_rect([0; 4]);
                    if r != last {
                        (changes, last) = (changes + 1, r);
                    }
                }
                Ok(())
            })();
            drop(stdin);
            let status = child.wait().map_err(err("flags-changed wait"))?;
            if let Err(e) = stepped {
                driver_err = Some(e);
            } else if !status.success() {
                driver_err = Some(format!("mac-proof flags-changed exited {status}"));
            } else {
                drain(80);
                extra_acks += drain_extra(&mut buf)?;
                changes += u64::from(caret_rect([0; 4]) != last);
            }
            Ok(())
        })?;
        let mut out =
            json!({"rect_changes": changes, "missed_acks": missed_acks, "extra_acks": extra_acks});
        if let Some(e) = driver_err {
            out["driver_error"] = e.into();
        }
        Ok(out)
    }
}
