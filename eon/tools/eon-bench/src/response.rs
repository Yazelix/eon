//! On-demand native input-to-visible-response proof on a private headless Sway.

use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::{Read, Write},
    os::unix::{fs::DirBuilderExt, net::UnixDatagram},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use crate::{
    environment,
    json::Value,
    launcher::{BusPolicy, Headless, WindowMode, live_identity},
    result::utc_timestamp,
    sha256::Sha256,
    stats::Summary,
};

type Result<T = ()> = std::result::Result<T, String>;

pub const HELP: &str = "\
eon-bench response --eon /nix/store/.../bin/eon --out DIR [OPTIONS]

Runs idle native clicks and keys in its own headless Sway, HOME, D-Bus and
Eon roots. Requires sway, swaymsg, wdotool, grim, dbus-daemon and timeout on
PATH. Never connects to the ambient display or Eon runtime. Linux only.

  --samples N       repetitions per case (default 100; 1..1000)
  --bound-ms N      warm input-to-visible budget (default 250; 1..10000)
  --sensitivity S   normal (default), drop, missing, delay or invalid: typing controls

Cases: tab click/key, pane click/key, popup open, Shortcuts button, inbox
filter click and typing. A raw terminal fixture supplies a visible identity
and response counter. Local dialogs use calibrated opposite-state image
regions. Captures bracket visibility; model/present timestamps alone never
pass. Writes DIR/result.json, trace.tsv, logs and PPM captures, including
failed samples. DIR must not exist. No stress, builds or installed-profile
changes. Exit 0: all attempted inputs pass; 1: failure/inconclusive; 2: usage.
Sensitivity runs are expected to exit 1: drop explicitly discards one key;
missing omits its effect without refusal;
delay holds its response for twice the budget; invalid suppresses one driver
send, which must report unobserved ingress. Not part of default eon-test.
See docs/DEVELOPMENT.md for the isolated candidate recipe.";

pub struct Options {
    eon: PathBuf,
    out: PathBuf,
    samples: usize,
    bound: Duration,
    sensitivity: String,
}

pub fn options(arguments: &[String]) -> Result<Options> {
    let mut values = HashMap::new();
    let mut arguments = arguments.iter();
    while let Some(flag) = arguments.next() {
        if matches!(flag.as_str(), "--help" | "-h") {
            println!("{HELP}");
            return Err(String::new());
        }
        if !matches!(
            flag.as_str(),
            "--eon" | "--out" | "--samples" | "--bound-ms" | "--sensitivity"
        ) {
            return Err(format!("unknown argument {flag}\n{HELP}"));
        }
        let value = arguments.next().ok_or(format!("{flag} needs a value"))?;
        if values.insert(flag.as_str(), value.as_str()).is_some() {
            return Err(format!("{flag} is repeated"));
        }
    }
    let number = |key, default, max| -> Result<usize> {
        values
            .get(key)
            .copied()
            .unwrap_or(default)
            .parse::<usize>()
            .ok()
            .filter(|&n| n > 0 && n <= max)
            .ok_or(format!("invalid {key}"))
    };
    let sensitivity = values.get("--sensitivity").copied().unwrap_or("normal");
    if !matches!(
        sensitivity,
        "normal" | "drop" | "missing" | "delay" | "invalid"
    ) {
        return Err("--sensitivity must be normal, drop, missing, delay or invalid".into());
    }
    Ok(Options {
        eon: PathBuf::from(values.get("--eon").ok_or("--eon is required")?),
        out: PathBuf::from(values.get("--out").ok_or("--out is required")?),
        samples: number("--samples", "100", 1000)?,
        bound: Duration::from_millis(number("--bound-ms", "250", 10000)? as u64),
        sensitivity: sensitivity.into(),
    })
}

fn failure(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn object(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}
fn nanos() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: time is a writable timespec for the process-independent monotonic clock.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &raw mut time) },
        0
    );
    time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64
}

#[derive(Clone, Debug)]
struct Event {
    owner: String,
    pid: u32,
    seq: u64,
    at: u64,
    lost: u64,
    stage: String,
    detail: String,
}
impl Event {
    fn parse(line: &str) -> Result<Self> {
        let fields: Vec<_> = line.splitn(7, '\t').collect();
        if fields.len() != 7 {
            return Err("incomplete trace packet".into());
        }
        Ok(Self {
            owner: fields[0].into(),
            pid: fields[1].parse().map_err(failure)?,
            seq: fields[2].parse().map_err(failure)?,
            at: fields[3].parse().map_err(failure)?,
            lost: fields[4].parse().map_err(failure)?,
            stage: fields[5].into(),
            detail: fields[6].into(),
        })
    }
}

fn trace_complete(events: &[Event]) -> Result {
    let mut processes = BTreeMap::<u32, Vec<&Event>>::new();
    for event in events {
        processes.entry(event.pid).or_default().push(event);
    }
    for owner in ["venus", "runtime", "orbit", "fixture"] {
        if !events.iter().any(|event| event.owner == owner) {
            return Err(format!("missing {owner} instrumentation"));
        }
    }
    for (pid, mut events) in processes {
        events.sort_by_key(|event| (event.at, event.seq));
        let mut lifetimes: Vec<Vec<&Event>> = Vec::new();
        for event in events {
            if event.stage == "start" && event.seq == 0 {
                lifetimes.push(Vec::new());
            }
            lifetimes
                .last_mut()
                .ok_or(format!("missing trace start for {pid}"))?
                .push(event);
        }
        for mut lifetime in lifetimes {
            lifetime.sort_by_key(|event| event.seq);
            let owner = &lifetime[0].owner;
            for (expected, event) in lifetime.iter().enumerate() {
                if event.seq != expected as u64
                    || event.lost != 0
                    || event.owner != *owner
                    || (expected != 0 && event.stage == "start")
                {
                    return Err(format!(
                        "trace loss/duplicate for {pid} at sequence {expected}"
                    ));
                }
            }
            // exec ends this producer; the same PID can then start another instrumented program.
            if !matches!(lifetime.last().unwrap().stage.as_str(), "end" | "exec") {
                return Err(format!("incomplete trace lifecycle for {pid}"));
            }
        }
    }
    Ok(())
}

fn fixture_accounted(events: &[Event], pid: u32, next: u64, lost: u64) -> bool {
    let mut records: Vec<_> = events
        .iter()
        .filter(|e| e.owner == "fixture" && e.pid == pid)
        .collect();
    records.sort_by_key(|e| e.seq);
    lost == 0
        && records.len() as u64 == next
        && records
            .iter()
            .enumerate()
            .all(|(seq, e)| e.seq == seq as u64 && e.lost == 0)
        && records.last().is_some_and(|e| e.stage == "end")
}
fn primary_inputs<'a>(events: &'a [Event], class: &str) -> Vec<&'a Event> {
    events
        .iter()
        .filter(|e| e.owner == "venus" && e.stage == "input" && e.detail == class)
        .collect()
}
fn input_has_stage(events: &[Event], input: &Event, stage: &str) -> bool {
    let end = events
        .iter()
        .filter(|e| {
            e.owner == "venus"
                && e.pid == input.pid
                && e.seq > input.seq
                && matches!(e.stage.as_str(), "input" | "input_end")
        })
        .min_by_key(|e| e.seq);
    end.is_some_and(|end| {
        end.stage == "input_end"
            && events.iter().any(|e| {
                e.owner == "venus"
                    && e.pid == input.pid
                    && e.seq > input.seq
                    && e.seq < end.seq
                    && e.stage == stage
            })
    })
}
fn receipts_match(events: &[Event], pid: u32, count: u32, expected: usize) -> bool {
    let receipts: Vec<_> = events
        .iter()
        .filter(|e| e.owner == "fixture" && e.stage == "fixture_input")
        .collect();
    receipts.len() == expected
        && receipts.iter().all(|e| e.pid == pid)
        && receipts
            .last()
            .is_some_and(|e| e.detail.parse::<u32>().ok() == Some(count))
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Verdict {
    OnTime,
    Late,
    Inconclusive,
    Uncorrelated,
    Unobserved,
    Rejected,
    FixtureDropped,
    Missing,
}
impl Verdict {
    fn name(self) -> &'static str {
        match self {
            Self::OnTime => "on-time",
            Self::Late => "late",
            Self::Inconclusive | Self::Uncorrelated => "inconclusive",
            Self::Unobserved => "unobserved-ingress",
            Self::Rejected => "rejected",
            Self::FixtureDropped => "fixture-dropped",
            Self::Missing => "missing-response",
        }
    }
}
fn classify(lower: u64, upper: Option<u64>, bound: u64, rejected: bool) -> Verdict {
    match upper {
        Some(upper) if upper <= bound => Verdict::OnTime,
        Some(_) if lower > bound => Verdict::Late,
        Some(_) => Verdict::Inconclusive,
        None if rejected => Verdict::Rejected,
        None => Verdict::Missing,
    }
}

#[derive(Clone, Copy, Debug)]
struct Rect {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}
impl Rect {
    fn parse(text: &str) -> Option<Self> {
        let n: Vec<f64> = text
            .split_whitespace()
            .map(str::parse)
            .collect::<std::result::Result<_, _>>()
            .ok()?;
        if n.len() != 4 || n.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return None;
        }
        Some(Self {
            x: n[0].round() as u32,
            y: n[1].round() as u32,
            w: n[2].round() as u32,
            h: n[3].round() as u32,
        })
    }
}
struct Picture {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}
impl Picture {
    fn read(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).map_err(failure)?;
        let mut position = 0;
        let mut token = || -> Result<&[u8]> {
            while bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
                position += 1;
            }
            let begin = position;
            while bytes
                .get(position)
                .is_some_and(|b| !b.is_ascii_whitespace())
            {
                position += 1;
            }
            if position == begin {
                return Err("short PPM header".into());
            }
            Ok(&bytes[begin..position])
        };
        if token()? != b"P6" {
            return Err("capture is not P6 PPM".into());
        }
        let width = std::str::from_utf8(token()?)
            .map_err(failure)?
            .parse::<usize>()
            .map_err(failure)?;
        let height = std::str::from_utf8(token()?)
            .map_err(failure)?
            .parse::<usize>()
            .map_err(failure)?;
        if token()? != b"255" {
            return Err("capture depth is not 255".into());
        }
        if !bytes.get(position).is_some_and(u8::is_ascii_whitespace) {
            return Err("bad PPM separator".into());
        }
        position += 1;
        if width == 0
            || height == 0
            || width > 4096
            || height > 4096
            || bytes.len() - position != width * height * 3
        {
            return Err("invalid PPM dimensions/length".into());
        }
        Ok(Self {
            width,
            height,
            pixels: bytes[position..].to_vec(),
        })
    }
    fn pixel(&self, x: usize, y: usize) -> Option<&[u8]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let begin = (y * self.width + x) * 3;
        Some(&self.pixels[begin..begin + 3])
    }
    fn crop(&self, rect: Rect) -> Result<Vec<u8>> {
        if rect.w == 0
            || rect.h == 0
            || u64::from(rect.x) + u64::from(rect.w) > self.width as u64
            || u64::from(rect.y) + u64::from(rect.h) > self.height as u64
        {
            return Err("oracle region is outside the capture".into());
        }
        Ok((rect.y..rect.y + rect.h)
            .flat_map(|y| {
                let start = (y as usize * self.width + rect.x as usize) * 3;
                self.pixels[start..start + rect.w as usize * 3]
                    .iter()
                    .copied()
            })
            .collect())
    }
    fn barcode(&self, rect: Rect, grid: [f64; 3]) -> Option<u32> {
        let [width, height, padding] = grid;
        let mut code = 0;
        for bit in 0..32 {
            let x = (f64::from(rect.x) + padding + (f64::from(bit) + 0.5) * width).floor() as usize;
            let y = (f64::from(rect.y) + padding + 2.5 * height).floor() as usize;
            let pixel = self.pixel(x, y)?;
            let value = if pixel.iter().all(|&v| v > 220) {
                1
            } else if pixel.iter().all(|&v| v < 30) {
                0
            } else {
                return None;
            };
            code = (code << 1) | value;
        }
        Some(code)
    }
}

struct Collector {
    receiver: mpsc::Receiver<Event>,
    alive: Arc<AtomicBool>,
    errors: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Collector {
    fn start(socket: UnixDatagram, file: File) -> Result<Self> {
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .map_err(failure)?;
        let (sender, receiver) = mpsc::sync_channel(65536);
        let alive = Arc::new(AtomicBool::new(true));
        let errors = Arc::new(AtomicU64::new(0));
        let (running, failed) = (Arc::clone(&alive), Arc::clone(&errors));
        let thread = thread::spawn(move || {
            let mut file = std::io::BufWriter::new(file);
            let mut bytes = [0; 4097];
            loop {
                match socket.recv(&mut bytes) {
                    Ok(length) => {
                        let event = std::str::from_utf8(&bytes[..length])
                            .map_err(failure)
                            .and_then(|text| {
                                if length > 4096 {
                                    return Err("oversize trace packet".into());
                                }
                                writeln!(file, "{text}").map_err(failure)?;
                                Event::parse(text)
                            });
                        match event {
                            Ok(event) => {
                                if sender.try_send(event).is_err() {
                                    failed.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                            Err(_) => {
                                failed.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                        ) =>
                    {
                        if !running.load(Ordering::Relaxed) {
                            break;
                        }
                    }
                    Err(_) => {
                        failed.fetch_add(1, Ordering::Relaxed);
                        break;
                    }
                }
            }
            if file.flush().is_err() {
                failed.fetch_add(1, Ordering::Relaxed);
            }
        });
        Ok(Self {
            receiver,
            alive,
            errors,
            thread: Some(thread),
        })
    }
    fn finish(&mut self) -> Result {
        self.alive.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| "trace collector panicked")?;
        }
        if self.errors.load(Ordering::Relaxed) != 0 {
            return Err("trace collector lost records or could not write its log".into());
        }
        Ok(())
    }
}
impl Drop for Collector {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

struct Session {
    launch: Headless,
    collector: Collector,
    deadline: Instant,
    events: Vec<Event>,
    targets: HashMap<String, Rect>,
    grid: Option<[f64; 3]>,
}
impl Session {
    fn command(&self, program: impl AsRef<std::ffi::OsStr>) -> Command {
        self.launch.command(program)
    }
    fn call(&mut self, program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result<String> {
        if CANCELLED.load(Ordering::Relaxed) {
            return Err("response run cancelled".into());
        }
        if Instant::now() >= self.deadline {
            return Err("idle response run exceeded 15 minutes".into());
        }
        let mut command = self.command("timeout");
        let result = command
            .arg("10s")
            .arg(program)
            .args(args)
            .output()
            .map_err(failure)?;
        self.pump()?;
        if !result.status.success() {
            return Err(format!(
                "command failed ({}): {}",
                result.status,
                String::from_utf8_lossy(&result.stderr)
            ));
        }
        String::from_utf8(result.stdout).map_err(failure)
    }
    fn eon(&mut self, args: &[&str]) -> Result<Value> {
        let program = self.launch.eon.clone();
        let output = self.call(program, args)?;
        Value::parse(&output)
    }
    fn native(&mut self, args: &[&str]) -> Result {
        let mut all = vec!["--backend", "wlr-protocols"];
        all.extend_from_slice(args);
        self.call("wdotool", &all).map(|_| ())
    }
    fn pump(&mut self) -> Result {
        for event in self.collector.receiver.try_iter() {
            if self.events.len() >= 250000 {
                return Err("trace record bound exceeded".into());
            }
            if event.owner == "venus"
                && event.stage == "target"
                && let Some((name, rect)) = event.detail.split_once(' ')
            {
                self.targets.insert(
                    name.into(),
                    Rect::parse(rect).ok_or("bad target rectangle")?,
                );
            }
            if event.owner == "venus" && event.stage == "grid" {
                let numbers: Vec<f64> = event
                    .detail
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()
                    .map_err(failure)?;
                self.grid = Some(numbers.try_into().map_err(|_| "bad grid")?);
            }
            // Retain private instrumented descendants before their parent can exit.
            if !self.launch.processes.known.contains_key(&event.pid) {
                self.launch.processes.remember(event.pid);
            }
            self.events.push(event);
        }
        if self.collector.errors.load(Ordering::Relaxed) != 0 {
            return Err("trace collector failed".into());
        }
        Ok(())
    }
    fn fixture_command(&mut self, pid: u32, message: &str) -> Result<String> {
        if !self
            .launch
            .processes
            .known
            .get(&pid)
            .is_some_and(|identity| live_identity(pid, identity))
        {
            return Err(format!("fixture {pid} is no longer owned/live"));
        }
        let directory = Path::new(&self.launch.env["XDG_RUNTIME_DIR"]);
        let reply_path = directory.join("reply");
        let target = directory.join(format!("f{pid}"));
        let _ = fs::remove_file(&reply_path);
        let socket = UnixDatagram::bind(&reply_path).map_err(failure)?;
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .map_err(failure)?;
        socket
            .set_write_timeout(Some(Duration::from_millis(100)))
            .map_err(failure)?;
        socket
            .send_to(message.as_bytes(), &target)
            .map_err(failure)?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut bytes = [0; 128];
        loop {
            self.pump()?;
            match socket.recv_from(&mut bytes) {
                Ok((length, source)) if source.as_pathname() == Some(target.as_path()) => {
                    return std::str::from_utf8(&bytes[..length])
                        .map(str::to_owned)
                        .map_err(failure);
                }
                Ok(_) => return Err("unexpected fixture control peer".into()),
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) => {}
                Err(e) => return Err(failure(e)),
            }
            if Instant::now() >= deadline {
                return Err(format!("fixture {pid} {message} handshake timed out"));
            }
        }
    }
    fn finish_fixtures(&mut self) -> Result {
        self.pump()?;
        let pids: Vec<_> = self
            .events
            .iter()
            .filter(|e| e.owner == "fixture" && e.stage == "start")
            .map(|e| e.pid)
            .collect();
        for pid in pids {
            let ack = self.fixture_command(pid, "finish")?;
            let numbers = ack
                .split_whitespace()
                .map(str::parse::<u64>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(failure)?;
            let [ack_pid, next, lost] = numbers.as_slice() else {
                return Err("invalid fixture final accounting".into());
            };
            if *ack_pid != u64::from(pid) || *lost != 0 {
                return Err(format!("fixture {pid} final trace loss"));
            }
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                self.pump()?;
                if fixture_accounted(&self.events, pid, *next, *lost) {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(format!("fixture {pid} final trace missing"));
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    }
    fn wait(&mut self, after: u64, expected: &str) -> Result {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.pump()?;
            let has_frame = self
                .events
                .iter()
                .rev()
                .find(|event| event.owner == "venus" && event.stage == "present_revision")
                .is_some_and(|event| !event.detail.ends_with(" None"));
            if has_frame
                && self
                    .events
                    .iter()
                    .rev()
                    .find(|event| event.owner == "venus" && event.stage == "present")
                    .is_some_and(|event| {
                        event.at >= after
                            && event.detail.split_whitespace().any(|word| {
                                if expected == "popup=+" {
                                    word.starts_with("popup=") && word != "popup=-"
                                } else {
                                    word == expected
                                }
                            })
                    })
            {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("no presented {expected}"));
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
    fn focus(&mut self, target: &str) -> Result {
        self.eon(&["focus", target, "--json"])?;
        self.wait(
            0,
            &format!(
                "{}={target}",
                if target.starts_with('t') {
                    "tab"
                } else {
                    "pane"
                }
            ),
        )
    }
    fn verify_focus(&mut self) -> Result {
        fn focused(value: &Value) -> Option<&Value> {
            if value.get("focused").and_then(Value::as_bool) == Some(true) {
                return Some(value);
            }
            for key in ["nodes", "floating_nodes"] {
                for node in value.get(key).and_then(Value::as_array).unwrap_or(&[]) {
                    if let Some(node) = focused(node) {
                        return Some(node);
                    }
                }
            }
            None
        }
        let tree = Value::parse(&self.call("swaymsg", &["-t", "get_tree", "-r"])?)?;
        let node = focused(&tree).ok_or("private compositor has no focused surface")?;
        let pid = node
            .get("pid")
            .and_then(Value::as_u64)
            .ok_or("private focus is not a client")?;
        if !self
            .events
            .iter()
            .any(|e| e.owner == "venus" && u64::from(e.pid) == pid)
        {
            return Err("private focused surface is not the instrumented Venus".into());
        }
        Ok(())
    }
    fn key(&mut self, key: &str) -> Result {
        self.native(&["key", key])
    }
    fn click(&mut self, target: &str) -> Result {
        let rect = *self
            .targets
            .get(target)
            .ok_or(format!("no native target {target}"))?;
        let x = (rect.x + rect.w / 2).to_string();
        let y = (rect.y + rect.h / 2).to_string();
        self.native(&["mousemove", &x, &y])?;
        self.native(&["click", "1"])
    }
    fn capture(&mut self, name: &str, rect: Option<Rect>) -> Result<(Picture, u64, u64)> {
        let path = self.launch.out.join(name);
        let geometry = rect.map(|r| format!("{},{} {}x{}", r.x, r.y, r.w, r.h));
        let mut args = vec!["-t", "ppm"];
        if let Some(geometry) = &geometry {
            args.extend(["-g", geometry.as_str()]);
        }
        args.push(path.to_str().ok_or("non-UTF8 output path")?);
        let begin = nanos();
        self.call("grim", &args)?;
        let end = nanos();
        let picture = Picture::read(&path)?;
        if rect.is_some_and(|r| picture.width != r.w as usize || picture.height != r.h as usize) {
            return Err("capture region dimensions changed".into());
        }
        Ok((picture, begin, end))
    }
}

fn start(options: &Options) -> Result<Session> {
    let mut launch = Headless::new(&options.out, &options.eon, WindowMode::Fullscreen)?;
    let out = launch.out.clone();
    let runtime = PathBuf::from(&launch.env["XDG_RUNTIME_DIR"]);
    let bench = std::env::current_exe().map_err(failure)?;
    let socket_path = runtime.join("trace");
    let argv = Value::Array(vec![
        bench.display().to_string().into(),
        "response-fixture".into(),
        socket_path.display().to_string().into(),
        runtime.display().to_string().into(),
        options.sensitivity.as_str().into(),
        (options.bound.as_millis() * 2).to_string().into(),
    ])
    .to_pretty();
    let config = format!(
        "[shell]\ncommand = {}\n[anima]\nenabled = false\n[terminal]\nbackground_opacity = 1.0\n[popups.git]\ncommand = {}\nkeep_alive = true\n",
        argv.trim(),
        argv.trim()
    );
    fs::write(out.join("config/config.toml"), config).map_err(failure)?;
    let socket = UnixDatagram::bind(&socket_path).map_err(failure)?;
    launch
        .env
        .insert("EON_BENCH_TRACE".into(), socket_path.display().to_string());
    let mut session = Session {
        launch,
        collector: Collector::start(
            socket,
            File::create(out.join("trace.tsv")).map_err(failure)?,
        )?,
        deadline: Instant::now() + Duration::from_secs(15 * 60),
        events: Vec::new(),
        targets: HashMap::new(),
        grid: None,
    };
    let executable = session.launch.eon.clone();
    let versions = session.call(executable, &["versions"])?;
    fs::write(out.join("versions.txt"), versions).map_err(failure)?;
    session.launch.start_compositor(BusPolicy::Session)?;
    session.launch.spawn("wdotool", &["prime"], "prime.log")?;
    session.launch.start_eon()?;
    session.wait(0, "popup=+")?;
    // Escape is the documented startup-picker dismissal; no shell/zoxide timing dependency.
    session.key("Escape")?;
    session.wait(0, "popup=-")?;
    Ok(session)
}

#[derive(Clone)]
enum Stimulus {
    Burst(usize),
    Key(&'static str),
    Click(String),
}
enum Oracle {
    Terminal { pid: u32, count: u32, rect: Rect },
    Region { rect: Rect, pixels: Vec<u8> },
}
struct Case {
    name: &'static str,
    initial: String,
    expected: String,
    stimulus: Stimulus,
    oracle: Oracle,
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or(format!("missing string {key}"))
}
fn active_tab(snapshot: &Value) -> Result<&Value> {
    let selected = field(snapshot, "active_tab")?;
    snapshot
        .get("tabs")
        .and_then(Value::as_array)
        .and_then(|tabs| {
            tabs.iter()
                .find(|tab| field(tab, "id").ok() == Some(selected))
        })
        .ok_or("no active tab".into())
}
fn terminal_pid(session: &mut Session, pane: Option<&str>) -> Result<u32> {
    let snapshot = session.eon(&["session", "--json"])?;
    let tab = active_tab(&snapshot)?;
    let terminal = if let Some(pane) = pane {
        let pane = tab
            .get("panes")
            .and_then(Value::as_array)
            .and_then(|panes| panes.iter().find(|p| field(p, "id").ok() == Some(pane)))
            .ok_or("no expected pane")?;
        field(pane, "terminal")?
    } else {
        let selected = field(tab, "selected_popup")?;
        let popup = tab
            .get("popups")
            .and_then(Value::as_array)
            .and_then(|popups| {
                popups
                    .iter()
                    .find(|p| field(p, "id").ok() == Some(selected))
            })
            .ok_or("no expected popup")?;
        field(popup, "terminal")?
    };
    let prefix = format!("{terminal}.");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        session.pump()?;
        if let Some(event) = session
            .events
            .iter()
            .find(|e| e.owner == "fixture" && e.stage == "start" && e.detail.starts_with(&prefix))
        {
            return Ok(event.pid);
        }
        if Instant::now() >= deadline {
            return Err(format!("no fixture for {terminal}"));
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn latest_count(session: &Session, pid: u32) -> u32 {
    session
        .events
        .iter()
        .rev()
        .find(|e| e.pid == pid && e.stage == "fixture_input")
        .and_then(|e| e.detail.parse().ok())
        .unwrap_or(0)
}
fn barcode(pid: u32, count: u32) -> u32 {
    ((pid & 65535) << 16) | count
}

fn event_value(event: &Event) -> Value {
    object([
        ("owner", event.owner.as_str().into()),
        ("pid", u64::from(event.pid).into()),
        ("sequence", event.seq.into()),
        ("at_ns", event.at.to_string().into()),
        ("stage", event.stage.as_str().into()),
        ("detail", event.detail.as_str().into()),
    ])
}

fn measure(
    session: &mut Session,
    options: &Options,
    case: &Case,
    ordinal: usize,
) -> Result<(Value, Verdict)> {
    if let Stimulus::Click(target) = &case.stimulus {
        let rect = *session
            .targets
            .get(target)
            .ok_or(format!("no click target {target}"))?;
        session.native(&[
            "mousemove",
            &(rect.x + rect.w / 2).to_string(),
            &(rect.y + rect.h / 2).to_string(),
        ])?;
    }
    session.verify_focus()?;
    let rect = match &case.oracle {
        Oracle::Terminal { rect, .. } => {
            let [width, height, padding] = session.grid.ok_or("missing grid metrics")?;
            let w = (padding + 32.0 * width).ceil() as u32;
            let h = (padding + 3.0 * height).ceil() as u32;
            if w > rect.w || h > rect.h {
                return Err("fixture barcode does not fit the terminal viewport".into());
            }
            Rect { w, h, ..*rect }
        }
        Oracle::Region { rect, .. } => *rect,
    };
    let before = session.capture("before.ppm", Some(rect))?.0;
    let local = Rect { x: 0, y: 0, ..rect };
    let matches = |picture: &Picture, session: &Session| -> Result<bool> {
        match &case.oracle {
            Oracle::Terminal { pid, count, .. } => Ok(picture
                .barcode(local, session.grid.ok_or("missing grid metrics")?)
                == Some(barcode(*pid, *count))),
            Oracle::Region { pixels, .. } => Ok(picture.crop(local)? == *pixels),
        }
    };
    if matches(&before, session)? {
        return Err(format!("{} has no opposite-state visual oracle", case.name));
    }
    if case.name == "typing" && matches!(options.sensitivity.as_str(), "drop" | "missing" | "delay")
    {
        let Oracle::Terminal { pid, .. } = case.oracle else {
            return Err("typing requires fixture".into());
        };
        if session.fixture_command(pid, "arm")? != "armed" {
            return Err("fixture did not arm".into());
        }
    }
    let first_event = session.events.len();
    let begin = nanos();
    let mut sent = Vec::new();
    let injected = !(options.sensitivity == "invalid" && case.name == "typing");
    match &case.stimulus {
        Stimulus::Key(key) => {
            if injected {
                session.key(key)?;
            }
        }
        Stimulus::Click(_) => {
            session.native(&["click", "1"])?;
        }
        Stimulus::Burst(count) => {
            for _ in 0..*count {
                let at = nanos();
                session.key("x")?;
                sent.push(object([
                    ("sent_ns", at.to_string().into()),
                    ("send_completed_ns", nanos().to_string().into()),
                ]));
            }
        }
    }
    let send_end = nanos();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut lower = 0;
    let mut upper = None;
    let mut capture_times = Vec::new();
    loop {
        let (picture, requested, captured) = session.capture("after.ppm", Some(rect))?;
        capture_times.push(object([
            ("requested_ns", requested.to_string().into()),
            ("completed_ns", captured.to_string().into()),
        ]));
        if matches(&picture, session)? {
            upper = Some(captured - begin);
            break;
        }
        lower = requested.saturating_sub(begin);
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }
    // In idle tests a missing response still gets a post-deadline drain; never retry input.
    if upper.is_none() {
        let drain = Instant::now() + Duration::from_secs(30);
        while Instant::now() < drain {
            let (picture, requested, captured) = session.capture("after.ppm", Some(rect))?;
            capture_times.push(object([
                ("requested_ns", requested.to_string().into()),
                ("completed_ns", captured.to_string().into()),
            ]));
            if matches(&picture, session)? {
                upper = Some(captured - begin);
                break;
            }
            lower = requested.saturating_sub(begin);
            thread::sleep(Duration::from_millis(100));
        }
    }
    session.pump()?;
    let end = nanos();
    let events: Vec<_> = session.events[first_event..]
        .iter()
        .filter(|event| event.at >= begin && event.at <= end)
        .cloned()
        .collect();
    let expected_inputs = if let Stimulus::Burst(count) = case.stimulus {
        count
    } else {
        1
    };
    let primary = if matches!(case.stimulus, Stimulus::Click(_)) {
        "pointer Pressed"
    } else {
        "key Pressed"
    };
    let ingress = primary_inputs(&events, primary);
    let rejected = ingress
        .iter()
        .any(|input| input_has_stage(&events, input, "input_rejected"));
    let mut verdict = classify(lower, upper, options.bound.as_nanos() as u64, rejected);
    if upper.is_none()
        && events
            .iter()
            .any(|e| e.owner == "fixture" && e.stage == "fixture_drop")
    {
        verdict = Verdict::FixtureDropped;
    }
    if upper.is_some() {
        if ingress.len() != expected_inputs {
            verdict = Verdict::Uncorrelated;
        }
        let expected = session
            .events
            .iter()
            .rev()
            .find(|e| e.owner == "venus" && e.stage == "present")
            .is_some_and(|e| {
                e.at >= begin
                    && e.detail
                        .split_whitespace()
                        .any(|word| word == case.expected)
            });
        if !expected || ingress.is_empty() {
            verdict = Verdict::Uncorrelated;
        }
        // State from the independent CLI must agree with the native response.
        let snapshot = session.eon(&["session", "--json"])?;
        let (key, value) = case.expected.split_once('=').ok_or("bad expected state")?;
        if key == "tab" && field(&snapshot, "active_tab")? != value
            || key == "pane" && field(active_tab(&snapshot)?, "selected_pane")? != value
        {
            return Err(format!("{} responded on the wrong target", case.name));
        }
        if let Oracle::Terminal { pid, count, .. } = case.oracle
            && matches!(case.stimulus, Stimulus::Key("x") | Stimulus::Burst(_))
            && (!receipts_match(&events, pid, count, expected_inputs)
                || latest_count(session, pid) != count)
        {
            verdict = Verdict::Uncorrelated;
        }
    }
    if let Oracle::Terminal { pid, .. } = case.oracle
        && matches!(case.stimulus, Stimulus::Key("x") | Stimulus::Burst(_))
        && events
            .iter()
            .any(|e| e.owner == "fixture" && e.stage == "fixture_input" && e.pid != pid)
    {
        verdict = Verdict::Uncorrelated;
    }
    let stage_values = events.iter().map(event_value).collect();
    for input in &mut sent {
        let at = field(input, "sent_ns")?.parse::<u64>().map_err(failure)?;
        if let Value::Object(fields) = input {
            fields.push((
                "visible_upper_ns".into(),
                upper.map(|duration| begin + duration - at).into(),
            ));
        }
    }
    if ingress.is_empty() && upper.is_none() {
        verdict = Verdict::Unobserved;
    }
    let sample = object([
        ("case", case.name.into()),
        ("ordinal", (ordinal as u64).into()),
        ("status", verdict.name().into()),
        ("attempted_ns", begin.to_string().into()),
        ("injected", injected.into()),
        ("sent_ns", injected.then(|| begin.to_string()).into()),
        ("send_completed_ns", send_end.to_string().into()),
        ("burst_inputs", Value::Array(sent)),
        (
            "capture_region",
            object([
                ("x", u64::from(rect.x).into()),
                ("y", u64::from(rect.y).into()),
                ("width", u64::from(rect.w).into()),
                ("height", u64::from(rect.h).into()),
            ]),
        ),
        ("visible_lower_ns", lower.into()),
        ("visible_upper_ns", upper.into()),
        ("captures", Value::Array(capture_times)),
        ("stages", Value::Array(stage_values)),
    ]);
    let prefix = format!("{}-{ordinal}", case.name);
    if verdict != Verdict::OnTime {
        fs::copy(
            session.launch.out.join("before.ppm"),
            session.launch.out.join(format!("{prefix}-before.ppm")),
        )
        .map_err(failure)?;
        fs::copy(
            session.launch.out.join("after.ppm"),
            session.launch.out.join(format!("{prefix}-after.ppm")),
        )
        .map_err(failure)?;
    }
    Ok((sample, verdict))
}

fn attempt(
    session: &mut Session,
    options: &Options,
    case: &Case,
    ordinal: usize,
    samples: &mut Vec<Value>,
) -> Result<Verdict> {
    let attempted = nanos();
    match measure(session, options, case, ordinal) {
        Ok((sample, verdict)) => {
            samples.push(sample);
            Ok(verdict)
        }
        Err(error) => {
            samples.push(object([
                ("case", case.name.into()),
                ("ordinal", (ordinal as u64).into()),
                ("status", "harness-error".into()),
                ("attempted_ns", attempted.to_string().into()),
                ("error", error.as_str().into()),
            ]));
            Err(error)
        }
    }
}

fn route_after_switch(
    session: &mut Session,
    options: &Options,
    name: &'static str,
    expected: &str,
    pid: u32,
    ordinal: usize,
    samples: &mut Vec<Value>,
) -> Result<Verdict> {
    // Header navigation keeps chrome focus; Escape changes only keyboard focus, not attachment.
    session.key("Escape")?;
    let rect = *session
        .targets
        .get("terminal")
        .ok_or("missing switched-to terminal")?;
    let case = Case {
        name,
        initial: String::new(),
        expected: expected.into(),
        stimulus: Stimulus::Key("x"),
        oracle: Oracle::Terminal {
            pid,
            count: latest_count(session, pid) + 1,
            rect,
        },
    };
    attempt(session, options, &case, ordinal, samples)
}
fn count_verdict(verdict: Verdict, failures: &mut usize, name: &str, ordinal: usize) -> Result {
    match verdict {
        Verdict::OnTime => Ok(()),
        Verdict::Late | Verdict::Inconclusive => {
            *failures += 1;
            Ok(())
        }
        _ => Err(format!("{name} sample {ordinal}: {}", verdict.name())),
    }
}

fn cases(session: &mut Session, options: &Options, samples: &mut Vec<Value>) -> Result {
    let mut timing_failures = 0;
    session.eon(&["pane", "create", "--json"])?;
    let directory = session
        .launch
        .out
        .to_str()
        .ok_or("non-UTF8 output path")?
        .to_owned();
    session.eon(&["tab", "create", "--dir", &directory, "--json"])?;
    session.focus("p1")?;
    let p1 = terminal_pid(session, Some("p1"))?;
    let r1 = *session.targets.get("terminal").ok_or("missing p1 region")?;
    session.focus("p2")?;
    let p2 = terminal_pid(session, Some("p2"))?;
    let r2 = *session.targets.get("terminal").ok_or("missing p2 region")?;
    session.focus("t2")?;
    let p3 = terminal_pid(session, Some("p3"))?;
    let r3 = *session.targets.get("terminal").ok_or("missing p3 region")?;
    let mut cases = vec![
        Case {
            name: "tab-click",
            initial: "t1".into(),
            expected: "tab=t2".into(),
            stimulus: Stimulus::Click("tab-t2".into()),
            oracle: Oracle::Terminal {
                pid: p3,
                count: 0,
                rect: r3,
            },
        },
        Case {
            name: "tab-key",
            initial: "t1".into(),
            expected: "tab=t2".into(),
            stimulus: Stimulus::Key("ctrl+Tab"),
            oracle: Oracle::Terminal {
                pid: p3,
                count: 0,
                rect: r3,
            },
        },
        Case {
            name: "pane-click",
            initial: "p1".into(),
            expected: "pane=p2".into(),
            stimulus: Stimulus::Click("pane-p2".into()),
            oracle: Oracle::Terminal {
                pid: p2,
                count: 0,
                rect: r2,
            },
        },
        Case {
            name: "pane-key",
            initial: "p1".into(),
            expected: "pane=p2".into(),
            stimulus: Stimulus::Key("alt+j"),
            oracle: Oracle::Terminal {
                pid: p2,
                count: 0,
                rect: r2,
            },
        },
    ];
    session.focus("p1")?;
    let cold_begin = nanos();
    let cold_event = session.events.len();
    session.key("alt+shift+j")?;
    session.wait(0, "popup=+")?;
    let popup = terminal_pid(session, None)?;
    let mut identities = [p1, p2, p3, popup].map(|pid| barcode(pid, 0));
    identities.sort_unstable();
    if identities.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(
            "fixture identities alias in the visible barcode; rerun with fresh PIDs".into(),
        );
    }
    let popup_rect = *session
        .targets
        .get("terminal")
        .ok_or("missing popup region")?;
    let snapshot = session.eon(&["session", "--json"])?;
    let popup_id = field(active_tab(&snapshot)?, "selected_popup")?.to_owned();
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut cold_lower = 0;
    let (requested, captured, cold_visible) = loop {
        let (picture, requested, captured) = session.capture("cold-popup.ppm", None)?;
        let visible = picture.barcode(popup_rect, session.grid.ok_or("missing popup grid")?)
            == Some(barcode(popup, 0));
        if visible || Instant::now() >= deadline {
            break (requested, captured, visible);
        }
        cold_lower = requested.saturating_sub(cold_begin);
        thread::sleep(Duration::from_millis(20));
    };
    let cold_verdict = if cold_visible {
        classify(
            cold_lower,
            Some(captured - cold_begin),
            6_000_000_000,
            false,
        )
    } else {
        Verdict::Inconclusive
    };
    samples.push(object([
        ("case", "popup-cold".into()),
        ("ordinal", 0u64.into()),
        ("status", cold_verdict.name().into()),
        ("sent_ns", cold_begin.to_string().into()),
        ("bound_ms", 6000u64.into()),
        ("visible_lower_ns", cold_lower.into()),
        ("visible_upper_ns", Some(captured - cold_begin).into()),
        ("capture_requested_ns", requested.to_string().into()),
        ("capture_completed_ns", captured.to_string().into()),
        (
            "stages",
            Value::Array(
                session.events[cold_event..]
                    .iter()
                    .map(event_value)
                    .collect(),
            ),
        ),
    ]));
    if !cold_visible {
        return Err("cold popup did not visibly respond within six seconds".into());
    }
    timing_failures += usize::from(cold_verdict != Verdict::OnTime);
    session.key("alt+shift+j")?;
    session.wait(0, "popup=-")?;
    cases.push(Case {
        name: "popup",
        initial: "p1".into(),
        expected: format!("popup={popup_id}"),
        stimulus: Stimulus::Key("alt+shift+j"),
        oracle: Oracle::Terminal {
            pid: popup,
            count: 0,
            rect: popup_rect,
        },
    });
    session.key("alt+slash")?;
    session.wait(0, "viewer=true")?;
    let rect = *session
        .targets
        .get("dialog")
        .ok_or("missing viewer rectangle")?;
    // The static title band, clear of scrolling contents and the terminal behind it.
    let region = Rect {
        x: rect.x + 10,
        y: rect.y + 10,
        w: rect.w.saturating_sub(20),
        h: 30,
    };
    let reference = session
        .capture("viewer-reference.ppm", None)?
        .0
        .crop(region)?;
    let mut controls = Vec::new();
    let mut captured = true;
    for (name, primary) in [("key", "key Pressed"), ("inside-click", "pointer Pressed")] {
        session.pump()?;
        let first_event = session.events.len();
        let begin = nanos();
        if name == "key" {
            session.key("x")?;
        } else {
            session.click("dialog")?;
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            session.pump()?;
            let events = &session.events[first_event..];
            let inputs = primary_inputs(events, primary);
            if inputs.len() == 1 && input_has_stage(events, inputs[0], "input_consumed") {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let (control, _, end) = session.capture(&format!("capture-control-{name}.ppm"), None)?;
        let events: Vec<_> = session.events[first_event..]
            .iter()
            .filter(|e| e.at >= begin && e.at <= end)
            .cloned()
            .collect();
        let inputs = primary_inputs(&events, primary);
        let consumed = inputs.len() == 1 && input_has_stage(&events, inputs[0], "input_consumed");
        let leaked = events
            .iter()
            .any(|e| e.stage == "fixture_input" || e.stage == "terminal_queue");
        let ok = control.crop(region)? == reference && !leaked && consumed;
        captured &= ok;
        controls.push(object([
            ("stimulus", name.into()),
            ("captured", ok.into()),
            ("sent_ns", begin.to_string().into()),
            ("checked_ns", end.to_string().into()),
            (
                "stages",
                Value::Array(events.iter().map(event_value).collect()),
            ),
        ]));
    }
    samples.push(object([
        ("case", "dialog-capture-control".into()),
        ("ordinal", 0u64.into()),
        (
            "status",
            if captured { "captured" } else { "inconclusive" }.into(),
        ),
        ("stimuli", Value::Array(controls)),
    ]));
    if !captured {
        return Err("dialog did not separately capture both control inputs".into());
    }
    session.key("Escape")?;
    session.wait(0, "viewer=false")?;
    cases.push(Case {
        name: "shortcuts-key",
        initial: "p1".into(),
        expected: "viewer=true".into(),
        stimulus: Stimulus::Key("alt+slash"),
        oracle: Oracle::Region {
            rect: region,
            pixels: reference.clone(),
        },
    });
    cases.push(Case {
        name: "shortcuts-click",
        initial: "p1".into(),
        expected: "viewer=true".into(),
        stimulus: Stimulus::Click("control-Shortcuts".into()),
        oracle: Oracle::Region {
            rect: region,
            pixels: reference,
        },
    });
    session.key("alt+q")?;
    session.wait(0, "inbox=true")?;
    let filter = *session
        .targets
        .get("filter-NeedsInput")
        .ok_or("missing inbox filter rectangle")?;
    session.click("filter-NeedsInput")?;
    session.wait(0, "filter=NeedsInput")?;
    let reference = session
        .capture("filter-reference.ppm", None)?
        .0
        .crop(filter)?;
    session.key("Left")?;
    session.wait(0, "filter=All")?;
    session.key("Escape")?;
    session.wait(0, "inbox=false")?;
    cases.push(Case {
        name: "menu-click",
        initial: "p1".into(),
        expected: "filter=NeedsInput".into(),
        stimulus: Stimulus::Click("filter-NeedsInput".into()),
        oracle: Oracle::Region {
            rect: filter,
            pixels: reference,
        },
    });
    cases.push(Case {
        name: "typing",
        initial: "p1".into(),
        expected: "pane=p1".into(),
        stimulus: Stimulus::Key("x"),
        oracle: Oracle::Terminal {
            pid: p1,
            count: 1,
            rect: r1,
        },
    });
    cases.push(Case {
        name: "typing-burst",
        initial: "p1".into(),
        expected: "pane=p1".into(),
        stimulus: Stimulus::Burst(8),
        oracle: Oracle::Terminal {
            pid: p1,
            count: 8,
            rect: r1,
        },
    });
    for mut case in cases {
        let initial_failures = timing_failures;
        for ordinal in 0..options.samples {
            session.focus(&case.initial)?;
            session.key("Escape")?;
            if case.name == "menu-click" {
                session.key("alt+q")?;
                session.wait(0, "inbox=true")?;
                session.wait(0, "filter=All")?;
            }
            if let Oracle::Terminal {
                pid, ref mut count, ..
            } = case.oracle
            {
                *count = latest_count(session, pid)
                    + match case.stimulus {
                        Stimulus::Burst(n) => n as u32,
                        _ => u32::from(case.name == "typing"),
                    };
            }
            let verdict = attempt(session, options, &case, ordinal, samples)?;
            count_verdict(verdict, &mut timing_failures, case.name, ordinal)?;
            let routing = match case.name {
                "tab-click" => Some(("tab-click-route", p3)),
                "tab-key" => Some(("tab-key-route", p3)),
                "pane-click" => Some(("pane-click-route", p2)),
                "pane-key" => Some(("pane-key-route", p2)),
                "popup" => Some(("popup-route", popup)),
                _ => None,
            };
            if let Some((name, pid)) = routing {
                let verdict = route_after_switch(
                    session,
                    options,
                    name,
                    &case.expected,
                    pid,
                    ordinal,
                    samples,
                )?;
                count_verdict(verdict, &mut timing_failures, name, ordinal)?;
            }
            match case.name {
                "popup" => {
                    session.key("alt+shift+j")?;
                    session.wait(0, "popup=-")?;
                }
                "shortcuts-click" | "shortcuts-key" => {
                    session.key("Escape")?;
                    session.wait(0, "viewer=false")?;
                }
                "menu-click" => {
                    session.key("Escape")?;
                    session.wait(0, "inbox=false")?;
                }
                _ => {}
            }
        }
        println!(
            "{}: {} samples, {} timing failures",
            case.name,
            options.samples,
            timing_failures - initial_failures
        );
    }
    session.focus("p2")?;
    session.eon(&["pane", "move", "right", "--json"])?;
    let deadline = Instant::now() + Duration::from_secs(15);
    while !session.targets.contains_key("column-0") {
        session.pump()?;
        if Instant::now() >= deadline {
            return Err("new column layout was not presented".into());
        }
        thread::sleep(Duration::from_millis(10));
    }
    session.focus("p2")?;
    // Both expanded terminals are already visible; focus changes the pane header.
    let rect = *session
        .targets
        .get("pane-p2")
        .ok_or("missing column header")?;
    let reference = session
        .capture("column-reference.ppm", None)?
        .0
        .crop(rect)?;
    for stimulus in [Stimulus::Click("column-1".into()), Stimulus::Key("alt+l")] {
        let case = Case {
            name: if matches!(stimulus, Stimulus::Click(_)) {
                "column-click"
            } else {
                "column-key"
            },
            initial: "p1".into(),
            expected: "pane=p2".into(),
            stimulus,
            oracle: Oracle::Region {
                rect,
                pixels: reference.clone(),
            },
        };
        for ordinal in 0..options.samples {
            session.focus("p1")?;
            session.key("Escape")?;
            let verdict = attempt(session, options, &case, ordinal, samples)?;
            count_verdict(verdict, &mut timing_failures, case.name, ordinal)?;
            let name = if case.name == "column-click" {
                "column-click-route"
            } else {
                "column-key-route"
            };
            let verdict =
                route_after_switch(session, options, name, &case.expected, p2, ordinal, samples)?;
            count_verdict(verdict, &mut timing_failures, name, ordinal)?;
        }
    }
    if timing_failures == 0 {
        Ok(())
    } else {
        Err(format!(
            "{timing_failures} late or inconclusive timing samples; see result.json"
        ))
    }
}

static CANCELLED: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "linux")]
struct Interrupts([libc::sighandler_t; 2]);
#[cfg(target_os = "linux")]
impl Interrupts {
    fn enter() -> Result<Self> {
        extern "C" fn cancel(_: libc::c_int) {
            CANCELLED.store(true, Ordering::Relaxed);
        }
        CANCELLED.store(false, Ordering::Relaxed);
        // SAFETY: the handler only stores a lock-free flag; the guard restores both handlers.
        let previous = unsafe {
            [
                libc::signal(libc::SIGINT, cancel as *const () as libc::sighandler_t),
                libc::signal(libc::SIGTERM, cancel as *const () as libc::sighandler_t),
            ]
        };
        let guard = Self(previous);
        if guard.0.contains(&libc::SIG_ERR) {
            return Err("cannot install response cleanup handlers".into());
        }
        Ok(guard)
    }
}
#[cfg(target_os = "linux")]
impl Drop for Interrupts {
    fn drop(&mut self) {
        for (signal, handler) in [libc::SIGINT, libc::SIGTERM].into_iter().zip(self.0) {
            if handler != libc::SIG_ERR {
                // SAFETY: each successful handler came from signal for this exact signal.
                unsafe {
                    libc::signal(signal, handler);
                }
            }
        }
    }
}

pub fn run(options: &Options) -> Result {
    if !cfg!(target_os = "linux") {
        return Err("response currently requires Linux/Wayland".into());
    }
    #[cfg(target_os = "linux")]
    let _interrupts = Interrupts::enter()?;
    let eon = crate::launcher::candidate(&options.eon)?;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&options.out)
        .map_err(failure)?;
    let started = utc_timestamp(SystemTime::now());
    let load = environment::load()?;
    let mut samples = Vec::new();
    let mut events = Vec::new();
    let outcome = (|| {
        let mut session = start(options)?;
        let outcome = cases(&mut session, options, &mut samples);
        let teardown = (|| -> Result {
            // Explicitly stop only the private root. The Child/PID cleanup guard handles failure.
            // Teardown must still run after cancellation or the measurement deadline.
            let fixtures_finished = session.finish_fixtures();
            let stopped = session.launch.stop_eon();
            if !session.launch.processes.cleanup() {
                return Err(
                    "private process cleanup did not finish; runtime roots retained".into(),
                );
            }
            let collected = session.collector.finish();
            session.pump()?;
            events = session.events.clone();
            collected?;
            trace_complete(&events)?;
            fixtures_finished?;
            stopped?;
            Ok(())
        })();
        match (outcome, teardown) {
            (Err(measurement), Err(teardown)) => Err(format!("{measurement}; {teardown}")),
            (Err(error), _) | (_, Err(error)) => Err(error),
            _ => Ok(()),
        }
    })();
    let build = environment::build()?;
    let summary = Summary::of(
        &samples
            .iter()
            .filter_map(|sample| {
                sample
                    .get("visible_upper_ns")
                    .and_then(Value::as_u64)
                    .map(|ns| ns as f64 / 1_000_000.0)
            })
            .collect::<Vec<_>>(),
    );
    let mut grouped = BTreeMap::<&str, Vec<&Value>>::new();
    for sample in &samples {
        grouped
            .entry(field(sample, "case")?)
            .or_default()
            .push(sample);
    }
    let cells = grouped
        .into_iter()
        .map(|(case, samples)| {
            let values = samples
                .iter()
                .filter_map(|sample| {
                    sample
                        .get("visible_upper_ns")
                        .and_then(Value::as_u64)
                        .map(|n| n as f64 / 1_000_000.0)
                })
                .collect::<Vec<_>>();
            let stats = Summary::of(&values).map(|s| {
                object([
                    ("count", (s.n as u64).into()),
                    ("median", s.median.into()),
                    ("p95", s.p95.into()),
                    ("min", s.min.into()),
                    ("max", s.max.into()),
                ])
            });
            let counts = [
                "on-time",
                "late",
                "inconclusive",
                "rejected",
                "fixture-dropped",
                "missing-response",
                "unobserved-ingress",
                "harness-error",
                "captured",
            ]
            .into_iter()
            .map(|status| {
                (
                    status,
                    (samples
                        .iter()
                        .filter(|sample| {
                            sample.get("status").and_then(Value::as_str) == Some(status)
                        })
                        .count() as u64)
                        .into(),
                )
            });
            object([
                ("case", case.into()),
                ("attempted", (samples.len() as u64).into()),
                ("outcomes", object(counts)),
                ("visible_upper_ms", stats.into()),
            ])
        })
        .collect();
    let result = object([
        ("schema", 1u64.into()),
        ("tool", "eon-bench-response".into()),
        ("tool_version", env!("CARGO_PKG_VERSION").into()),
        ("suite", "idle-response".into()),
        ("started_at", started.into()),
        ("finished_at", utc_timestamp(SystemTime::now()).into()),
        (
            "settings",
            object([
                ("samples_per_case", (options.samples as u64).into()),
                ("bound_ms", (options.bound.as_millis() as u64).into()),
                ("sensitivity", options.sensitivity.as_str().into()),
                ("compositor_renderer", "headless-pixman".into()),
                ("warm_capture", "response region only".into()),
            ]),
        ),
        (
            "environment",
            object([
                ("kernel", environment::kernel().into()),
                ("os", environment::os().into()),
                ("cpu", environment::cpu_model().into()),
                ("cpus_online", environment::cpus_online().into()),
                ("load_start", load.into()),
                ("load_end", environment::load()?.into()),
                (
                    "gpu_adapters",
                    Value::Array(
                        events
                            .iter()
                            .filter(|e| e.owner == "venus" && e.stage == "gpu")
                            .map(|e| {
                                object([
                                    ("pid", u64::from(e.pid).into()),
                                    ("software", e.detail.starts_with("software=true ").into()),
                                    (
                                        "adapter",
                                        e.detail
                                            .split_once(' ')
                                            .map(|(_, info)| info)
                                            .unwrap_or(&e.detail)
                                            .into(),
                                    ),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ]),
        ),
        (
            "build",
            object([
                ("bench", build.path.into()),
                ("bench_sha256", build.sha256.into()),
                ("bench_profile", build.profile.into()),
                (
                    "bench_source",
                    build
                        .source
                        .map(|source| {
                            object([
                                ("commit", source.commit.into()),
                                ("dirty", source.dirty.into()),
                                ("from", source.from.into()),
                            ])
                        })
                        .into(),
                ),
                ("eon", eon.display().to_string().into()),
                ("eon_sha256", hash(&eon)?.into()),
            ]),
        ),
        (
            "result",
            if outcome.is_ok() { "pass" } else { "fail" }.into(),
        ),
        ("error", outcome.as_ref().err().map(String::as_str).into()),
        ("cells", Value::Array(cells)),
        ("samples", Value::Array(samples)),
        ("trace_records", (events.len() as u64).into()),
        (
            "tools",
            Value::Array(
                [
                    "sway",
                    "swaymsg",
                    "wdotool",
                    "grim",
                    "dbus-daemon",
                    "timeout",
                ]
                .into_iter()
                .map(|name| {
                    let path = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                        .map(|directory| directory.join(name))
                        .find(|path| path.is_file())
                        .and_then(|path| fs::canonicalize(path).ok());
                    object([
                        ("name", name.into()),
                        (
                            "path",
                            path.as_ref().map(|path| path.display().to_string()).into(),
                        ),
                        (
                            "sha256",
                            path.as_ref().and_then(|path| hash(path).ok()).into(),
                        ),
                    ])
                })
                .collect(),
            ),
        ),
        (
            "visible_upper_ms",
            summary
                .map(|s| {
                    object([
                        ("count", (s.n as u64).into()),
                        ("median", s.median.into()),
                        ("p95", s.p95.into()),
                        ("min", s.min.into()),
                        ("max", s.max.into()),
                    ])
                })
                .into(),
        ),
    ]);
    fs::write(options.out.join("result.json"), result.to_pretty()).map_err(failure)?;
    outcome
}
fn hash(path: &Path) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(&fs::read(path).map_err(failure)?);
    Ok(hash.hex())
}

/// The private fixture: deterministic text plus an independently decoded visible counter.
pub fn fixture(arguments: &[String]) -> Result {
    let [collector, directory, fault, delay] = arguments else {
        return Err("fixture needs collector, control directory, fault and delay".into());
    };
    let input = File::from(
        std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(failure)?,
    );
    let _raw = crate::writer::RawInput::enter(&input).map_err(failure)?;
    let socket = UnixDatagram::unbound().map_err(failure)?;
    socket.connect(collector).map_err(failure)?;
    socket.set_nonblocking(true).map_err(failure)?;
    let control = UnixDatagram::bind(Path::new(directory).join(format!("f{}", std::process::id())))
        .map_err(failure)?;
    control
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(failure)?;
    let mut sequence = 0;
    let mut lost = 0;
    let mut trace = |stage: &str, detail: &str| {
        let packet = format!(
            "fixture\t{}\t{sequence}\t{}\t{lost}\t{stage}\t{detail}",
            std::process::id(),
            nanos()
        );
        sequence += 1;
        if socket.send(packet.as_bytes()).is_err() {
            lost += 1;
        }
        (sequence, lost)
    };
    trace("start", &std::env::var("EON_TERMINAL").map_err(failure)?);
    let delay = delay.parse::<u64>().map_err(failure)?;
    let mut armed = false;
    let mut finished = false;
    let mut output = File::from(
        std::io::stdout()
            .as_fd()
            .try_clone_to_owned()
            .map_err(failure)?,
    );
    let draw = |output: &mut File, count: u32| -> Result {
        write!(
            output,
            "\x1b[?25l\x1b[Hfixture {}\x1b[2;1Hkey: {} count: {count:05}\x1b[3;1H",
            std::process::id(),
            if count == 0 { '-' } else { 'x' }
        )
        .map_err(failure)?;
        let code = barcode(std::process::id(), count);
        for bit in (0..32).rev() {
            let c = if code & (1 << bit) == 0 { 0 } else { 255 };
            write!(output, "\x1b[48;2;{c};{c};{c}m ").map_err(failure)?;
        }
        write!(output, "\x1b[0m").map_err(failure)
    };
    draw(&mut output, 0)?;
    let mut count = 0;
    loop {
        use std::os::fd::AsRawFd;
        let mut poll = [
            libc::pollfd {
                fd: input.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: control.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        // SAFETY: poll contains two valid descriptors and is writable for its full length.
        if unsafe { libc::poll(poll.as_mut_ptr(), 2, 1000) } < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(failure(std::io::Error::last_os_error()));
        }
        if poll[1].revents != 0 {
            let mut bytes = [0; 32];
            let (length, peer) = control.recv_from(&mut bytes).map_err(failure)?;
            let peer = peer
                .as_pathname()
                .ok_or("fixture command needs a reply path")?;
            match &bytes[..length] {
                b"arm" if !finished => {
                    armed = true;
                    control.send_to(b"armed", peer).map_err(failure)?;
                }
                b"finish" if !finished => {
                    let (next, lost) = trace("end", "");
                    control
                        .send_to(
                            format!("{} {next} {lost}", std::process::id()).as_bytes(),
                            peer,
                        )
                        .map_err(failure)?;
                    // Keep the PTY alive until product Stop; finishing one must not close a tab's peers.
                    finished = true;
                }
                _ => return Err("invalid fixture command".into()),
            }
        }
        if poll[0].revents != 0 {
            let mut bytes = [0; 64];
            let length = (&input).read(&mut bytes).map_err(failure)?;
            if length == 0 {
                return if finished {
                    Ok(())
                } else {
                    Err("fixture EOF before finish handshake".into())
                };
            }
            if finished {
                continue;
            }
            for &byte in &bytes[..length] {
                if byte != b'x' {
                    continue;
                }
                count += 1;
                trace("fixture_input", &count.to_string());
                let faulty = std::mem::take(&mut armed);
                if faulty && fault == "drop" {
                    trace("fixture_drop", &count.to_string());
                    continue;
                }
                if faulty && fault == "missing" {
                    continue;
                }
                if faulty && fault == "delay" {
                    thread::sleep(Duration::from_millis(delay));
                }
                draw(&mut output, count)?;
                trace("fixture_output", &count.to_string());
            }
        }
    }
}

use std::os::fd::AsFd;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classification_keeps_late_missing_and_uncertain_separate() {
        assert_eq!(classify(20, Some(40), 50, false), Verdict::OnTime);
        assert_eq!(classify(60, Some(80), 50, false), Verdict::Late);
        assert_eq!(classify(40, Some(80), 50, false), Verdict::Inconclusive);
        assert_eq!(classify(60, None, 50, true), Verdict::Rejected);
        assert_eq!(classify(60, None, 50, false), Verdict::Missing);
        assert_eq!(classify(60, Some(80), 50, true), Verdict::Late);
    }
    #[test]
    fn complete_trace_needs_all_owners_no_gaps_and_an_end() {
        let mut events = Vec::new();
        for (pid, owner) in ["venus", "runtime", "orbit", "fixture"].iter().enumerate() {
            for (seq, stage) in ["start", "end"].iter().enumerate() {
                events.push(
                    Event::parse(&format!("{owner}\t{pid}\t{seq}\t100\t0\t{stage}\t")).unwrap(),
                );
            }
        }
        assert!(trace_complete(&events).is_ok());
        let mut missing = events.clone();
        missing.remove(1);
        assert!(trace_complete(&missing).is_err());
        let fixture_pid = events.iter().find(|e| e.owner == "fixture").unwrap().pid;
        assert!(fixture_accounted(&events, fixture_pid, 2, 0));
        let mut last_lost = events.clone();
        last_lost.retain(|e| !(e.pid == fixture_pid && e.stage == "end"));
        assert!(trace_complete(&last_lost).is_err());
        assert!(!fixture_accounted(&last_lost, fixture_pid, 2, 0));
        assert!(!fixture_accounted(&events, fixture_pid, 3, 0));
        assert!(!fixture_accounted(&events, fixture_pid, 2, 1));
        let mut gap = events.clone();
        gap[1].seq = 2;
        assert!(trace_complete(&gap).is_err());
        let mut loss = events.clone();
        loss[1].lost = 1;
        assert!(trace_complete(&loss).is_err());
        events.reverse();
        assert!(trace_complete(&events).is_ok());
        let runtime = events
            .iter_mut()
            .find(|event| event.owner == "runtime" && event.stage == "end")
            .unwrap();
        runtime.stage = "exec".into();
        let pid = runtime.pid;
        events.push(Event::parse(&format!("runtime\t{pid}\t0\t200\t0\tstart\t")).unwrap());
        events.push(Event::parse(&format!("runtime\t{pid}\t1\t201\t0\tend\t")).unwrap());
        assert!(trace_complete(&events).is_ok());
        events.retain(|event| event.stage != "exec");
        assert!(trace_complete(&events).is_err());
    }
    #[test]
    fn each_modal_stimulus_needs_its_own_consumption_and_routes_reject_other_receipts() {
        let event = |seq, stage, detail| {
            Event::parse(&format!("venus\t1\t{seq}\t100\t0\t{stage}\t{detail}")).unwrap()
        };
        let events = vec![
            event(0, "input", "key Pressed"),
            event(1, "input_end", ""),
            event(2, "input", "pointer Pressed"),
            event(3, "input_consumed", "dialog-pointer"),
            event(4, "input_end", ""),
        ];
        let key = primary_inputs(&events, "key Pressed");
        let pointer = primary_inputs(&events, "pointer Pressed");
        assert!(!input_has_stage(&events, key[0], "input_consumed"));
        assert!(input_has_stage(&events, pointer[0], "input_consumed"));
        let mut no_key = events.clone();
        no_key.remove(0);
        assert!(primary_inputs(&no_key, "key Pressed").is_empty());
        let mut released = events.clone();
        released.extend([
            event(5, "input", "pointer Released"),
            event(6, "input_rejected", "geometry"),
            event(7, "input_end", ""),
        ]);
        assert!(!input_has_stage(&released, pointer[0], "input_rejected"));
        let mut receipts = vec![Event::parse("fixture\t2\t1\t100\t0\tfixture_input\t7").unwrap()];
        assert!(receipts_match(&receipts, 2, 7, 1));
        assert!(!receipts_match(&receipts, 3, 7, 1));
        receipts.push(Event::parse("fixture\t3\t1\t101\t0\tfixture_input\t1").unwrap());
        assert!(!receipts_match(&receipts, 2, 7, 1));
        assert_ne!(Verdict::FixtureDropped.name(), Verdict::Missing.name());
    }
    #[test]
    fn visible_barcode_and_regions_detect_the_wrong_identity_counter_and_crop() {
        let mut picture = Picture {
            width: 32,
            height: 3,
            pixels: vec![0; 32 * 3 * 3],
        };
        let code = barcode(123, 7);
        for bit in 0..32 {
            if code & (1 << (31 - bit)) != 0 {
                picture.pixels[(64 + bit) * 3..(65 + bit) * 3].fill(255);
            }
        }
        let rect = Rect {
            x: 0,
            y: 0,
            w: 32,
            h: 3,
        };
        assert_eq!(picture.barcode(rect, [1.0, 1.0, 0.0]), Some(code));
        assert_ne!(
            picture.barcode(rect, [1.0, 1.0, 0.0]),
            Some(barcode(124, 7))
        );
        assert_ne!(
            picture.barcode(rect, [1.0, 1.0, 0.0]),
            Some(barcode(123, 8))
        );
        let mut canvas = Picture {
            width: 40,
            height: 10,
            pixels: vec![0; 40 * 10 * 3],
        };
        for y in 0..3 {
            for x in 0..32 {
                let at = ((y + 4) * 40 + x + 3) * 3;
                canvas.pixels[at..at + 3].copy_from_slice(picture.pixel(x, y).unwrap());
            }
        }
        let crop = Picture {
            width: 32,
            height: 3,
            pixels: canvas.crop(Rect { x: 3, y: 4, ..rect }).unwrap(),
        };
        assert_eq!(crop.barcode(rect, [1.0, 1.0, 0.0]), Some(code));
        assert!(
            picture
                .crop(Rect {
                    x: 31,
                    y: 0,
                    w: 2,
                    h: 1
                })
                .is_err()
        );
    }
}
