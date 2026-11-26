//! Event-path fixtures (eon-33cww.4): input-method echo, a moving caret,
//! held repeat, configure and modifier storms, judged only from Venus's own
//! `WAYLAND_DEBUG=client` protocol log.
// Off Linux only the CLI parses; the fixtures and their judge are Linux
// display code that the tests still exercise everywhere.
#![cfg_attr(not(target_os = "linux"), allow(dead_code, unused_assignments))]
use super::*;
#[cfg(target_os = "linux")]
use native::{directory_workers, json, pause, wait};
#[cfg(target_os = "linux")]
use std::io::Write;

const USAGE: &str = "eon-bench events --eon LABEL=PATH --revision LABEL=COMMIT --candidate-tree LABEL=TREE --source-tree BENCH_TREE --out NEW_DIR [OPTIONS]";
const FIXTURES: [&str; 6] = [
    "static-echo",
    "moving-caret-30",
    "moving-caret-60",
    "held-repeat",
    "configure-storm",
    "modifier-storm",
];
/// Proactive echo commits; P0's pane reached 12,000 in one epoch.
const ECHO_COMMITS: u64 = 2000;
const CARET_TICKS: u64 = 120;
const HOLD_MS: u64 = 2000;
/// Grace after the logged release for repeat bytes already in flight.
const SETTLE_MS: u64 = 300;
/// Caret moves per tick, flushed apart, so updates outnumber frames.
const CARET_MOVES: u64 = 4;
const TOGGLES: u64 = 20;
const MODES: u64 = 10;
const MODIFIER_PAIRS: u64 = 200;
// evdev key codes.
const KEY_X: u32 = 45;
const KEY_ENTER: u32 = 28;
const KEY_LEFTSHIFT: u32 = 42;
const KEY_LEFTCTRL: u32 = 29;

pub struct Options {
    settings: suite::Settings,
    candidates: Vec<Candidate>,
    renderer: Renderer,
    #[cfg(target_os = "linux")]
    source_tree: String,
    #[cfg(any(target_os = "linux", test))]
    fixtures: Vec<String>,
    #[cfg(target_os = "linux")]
    deadline: Duration,
}

fn help() -> String {
    format!(
        "{USAGE}\n\n\
Linux/Wayland only. Private Sway with real fcitx5 (an echo input method for\n\
static-echo); never the desktop or profile.\n\
  --eon, --revision, --candidate-tree, --source-tree, --renderer  as in frames\n\
  --only FIXTURE           repeat; default all: {}\n\
  --deadline SECONDS       30..600 per cell (default 180)\n\n\
Each cell writes receipt.json with pass, fail or error and window counts from\n\
Venus's protocol log; result.json gathers them. A cell that does not pass fails\n\
the run; one that leaves private processes behind stops the remaining cells.\n\n{}",
        FIXTURES.join(", "),
        suite::settings_help()
    )
}

pub fn options(args: &[String]) -> Result<Options> {
    let (settings, flags) = suite::parse(
        args,
        USAGE,
        help,
        &[
            "--eon",
            "--revision",
            "--candidate-tree",
            "--source-tree",
            "--renderer",
            "--only",
            "--deadline",
        ],
    )?;
    let (candidates, source_tree, renderer, flags) = super::candidates(flags)?;
    let (mut fixtures, mut deadline) = (Vec::new(), None);
    for (flag, value) in flags {
        if flag == "--only" {
            if !FIXTURES.contains(&value.as_str()) || fixtures.contains(&value) {
                return Err("unknown or repeated fixture".into());
            }
            fixtures.push(value);
        } else if deadline.replace(suite::number(&flag, &value)?).is_some() {
            return Err("repeated --deadline".into());
        }
    }
    let deadline = deadline.unwrap_or(180);
    if !(30..=600).contains(&deadline) {
        return Err("--deadline must be 30..600".into());
    }
    if fixtures.is_empty() {
        fixtures = FIXTURES.iter().map(|s| (*s).into()).collect();
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (source_tree, deadline);
    Ok(Options {
        settings,
        candidates,
        renderer,
        #[cfg(target_os = "linux")]
        source_tree,
        #[cfg(any(target_os = "linux", test))]
        fixtures,
        #[cfg(target_os = "linux")]
        deadline: Duration::from_secs(deadline),
    })
}

pub fn run(args: &[String], options: &Options) -> Result {
    isolate(
        args,
        &options.settings,
        &options.candidates,
        &options.renderer,
        "events",
    )
}

pub fn run_inner(options: &Options) -> Result {
    #[cfg(target_os = "linux")]
    {
        let out = &options.settings.out;
        let mut cells = Vec::new();
        let (mut failed, mut aborted) = (0, None);
        'cells: for fixture in &options.fixtures {
            for candidate in &options.candidates {
                let id = format!("{fixture}-{}", candidate.label);
                let dir = out.join(&id);
                fs::create_dir(&dir).map_err(error)?;
                eprintln!("events: {id}");
                let (result, counts, failures) = match sample(options, candidate, fixture, &dir) {
                    Ok(Ok((counts, failures))) if failures.is_empty() => ("pass", counts, failures),
                    Ok(Ok((counts, failures))) => ("fail", counts, failures),
                    Ok(Err(e)) => ("error", Value::Null, vec![e]),
                    Err(e) => {
                        aborted = Some(format!("{id}: {e}"));
                        ("error", Value::Null, vec![e])
                    }
                };
                failed += usize::from(result != "pass");
                eprintln!("events: {id} {result} {failures:?}");
                let cell = object([
                    ("run", id.as_str().into()),
                    ("fixture", fixture.as_str().into()),
                    ("candidate", candidate.label.as_str().into()),
                    ("candidate_revision", candidate.revision.as_str().into()),
                    ("candidate_tree", candidate.tree.as_str().into()),
                    ("renderer", options.renderer.0.as_str().into()),
                    (
                        "input_method",
                        match fixture.as_str() {
                            "static-echo" => "echo peer".into(),
                            _ => fs::read_to_string(dir.join("fcitx5-version.txt"))
                                .map_or("fcitx5".into(), |v| format!("fcitx5 {}", v.trim())),
                        }
                        .into(),
                    ),
                    ("result", result.into()),
                    (
                        "failures",
                        Value::Array(failures.into_iter().map(Value::from).collect()),
                    ),
                    ("counts", counts),
                ]);
                fs::write(dir.join("receipt.json"), cell.to_pretty()).map_err(error)?;
                cells.push(cell);
                if aborted.is_some() {
                    break 'cells;
                }
            }
        }
        let result = object([
            ("schema", 1u64.into()),
            ("tool", "eon-bench-events".into()),
            ("suite", "events".into()),
            ("result", if failed == 0 { "pass" } else { "fail" }.into()),
            ("source_tree", options.source_tree.as_str().into()),
            ("requested_renderer", options.renderer.0.as_str().into()),
            (
                "conditions",
                object([
                    ("compositor", "sway".into()),
                    ("echo_commits", ECHO_COMMITS.into()),
                    ("caret_ticks", CARET_TICKS.into()),
                    ("caret_moves_per_tick", CARET_MOVES.into()),
                    ("repeat_settle_ms", SETTLE_MS.into()),
                    ("hold_ms", HOLD_MS.into()),
                    ("repeat", "delay 300 ms, rate 25 Hz".into()),
                    ("fullscreen_toggles", TOGGLES.into()),
                    ("mode_changes", MODES.into()),
                    ("modifier_pairs", MODIFIER_PAIRS.into()),
                ]),
            ),
            ("cells", Value::Array(cells)),
        ]);
        fs::write(out.join("result.json"), result.to_pretty()).map_err(error)?;
        native::write_manifest(out)?;
        if let Some(cause) = aborted {
            return Err(format!("stopped after an unclean cell: {cause}"));
        }
        if failed > 0 {
            return Err(format!("{failed} event cell(s) did not pass"));
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = options;
        Err("events requires Linux/Wayland".into())
    }
}

/// The pane child's part of a fixture, after the frames workload's `go`.
#[cfg(target_os = "linux")]
pub(super) fn payload(name: &str, root: &Path, dir: &Path, deadline: Instant) -> Option<Result> {
    let finished = || root.join("finish").is_file();
    let mut out = std::io::stdout();
    Some(match name {
        "moving-caret-30" | "moving-caret-60" => (|| {
            let (rows, _) = child::grid().ok_or("pane has no grid")?;
            let step = if name.ends_with("30") {
                33_333_333
            } else {
                16_666_667
            };
            let start = environment::monotonic_ns();
            let mut put = |text: String| write!(out, "{text}").and_then(|()| out.flush());
            for n in 0..CARET_TICKS {
                put(format!("\x1b[{rows};1H\x1b[2K{n:03}")).map_err(error)?;
                for k in 0..CARET_MOVES {
                    put(format!("\x1b[{rows};{}H", 6 + (CARET_MOVES * n + k) % 60))
                        .map_err(error)?;
                    std::thread::sleep(Duration::from_millis(2));
                }
                pause(start + (n + 1) * step, deadline)?;
            }
            put(format!("\x1b[{rows};1H")).map_err(error)?;
            child::publish(&dir.join("ticked"), "")
        })(),
        "held-repeat" => (|| {
            let stdin = std::io::stdin();
            let mut mode = rustix::termios::tcgetattr(&stdin).map_err(error)?;
            mode.make_raw();
            rustix::termios::tcsetattr(&stdin, rustix::termios::OptionalActions::Now, &mode)
                .map_err(error)?;
            let (mut count, mut bytes) = (0, [0; 256]);
            child::publish(&dir.join("count"), "0")?;
            while !finished() && Instant::now() < deadline {
                let mut fd = [rustix::event::PollFd::new(
                    &stdin,
                    rustix::event::PollFlags::IN,
                )];
                let wait = rustix::event::Timespec {
                    tv_sec: 0,
                    tv_nsec: 20_000_000,
                };
                if rustix::event::poll(&mut fd, Some(&wait)).unwrap_or(0) > 0 {
                    let n = rustix::io::read(&stdin, &mut bytes).map_err(error)?;
                    count += bytes[..n].iter().filter(|b| **b == b'x').count();
                    child::publish(&dir.join("count"), &count.to_string())?;
                    write!(out, "\r{count} ")
                        .and_then(|()| out.flush())
                        .map_err(error)?;
                }
            }
            Ok(())
        })(),
        // Keep the caret on the bottom row, so every height change moves it.
        "configure-storm" => (|| {
            let mut grid = child::grid();
            while !finished() && Instant::now() < deadline {
                if child::grid() != grid {
                    grid = child::grid();
                    write!(out, "\x1b[999;1H")
                        .and_then(|()| out.flush())
                        .map_err(error)?;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(())
        })(),
        _ => return None,
    })
}

#[cfg(target_os = "linux")]
fn sample(
    options: &Options,
    candidate: &Candidate,
    fixture: &str,
    dir: &Path,
) -> Result<Result<(Value, Vec<String>)>> {
    // The outer error means the environment is no longer clean, so later
    // cells must not run; the inner one is this cell's own error.
    let deadline = Instant::now() + options.deadline;
    let mut launch = Headless::new(dir, &candidate.path, WindowMode::Fullscreen)?;
    options.renderer.apply(&mut launch)?;
    let fonts = terminals::tool("frames_fontconfig")?;
    launch
        .env
        .insert("FONTCONFIG_FILE".into(), fonts.display().to_string());
    launch.env.insert("WAYLAND_DEBUG".into(), "client".into());
    let mut sway = fs::OpenOptions::new()
        .append(true)
        .open(dir.join("sway.conf"))
        .map_err(error)?;
    sway.write_all(b"input type:keyboard repeat_delay 300\ninput type:keyboard repeat_rate 25\n")
        .map_err(error)?;
    let shell = [
        std::env::current_exe()
            .map_err(error)?
            .display()
            .to_string(),
        "frames-workload".into(),
        dir.display().to_string(),
        fixture.into(),
        "1".into(),
        "-".into(),
        "-".into(),
    ];
    let shell = Value::Array(shell.into_iter().map(Value::from).collect()).to_pretty();
    fs::write(dir.join("config/config.toml"), format!("[shell]\ncommand = {}\n[anima]\nenabled = false\n[session]\nanimate_working_badge = false\n[bar]\nwidgets = []\n[terminal]\nfont_family = \"DejaVu Sans Mono\"\nfont_size = 16\n", shell.trim())).map_err(error)?;
    fs::write(
        dir.join("payload.vt"),
        "\x1b[0m\x1b[2J\x1b[HEON EVENTS READY\x1b[999;1H\x1b[?25h\x1b[2 q",
    )
    .map_err(error)?;
    let measured = drive(&mut launch, fixture, deadline);
    let _ = child::publish(&dir.join("finish"), "");
    let _ = child::publish(&dir.join("release"), "");
    let stopped = launch.stop_eon();
    // Daemonized children leave cleanup's tree; remember every marked one.
    for entry in fs::read_dir("/proc").map_err(error)?.flatten() {
        if let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) {
            launch.processes.remember(pid);
        }
    }
    let cleaned = launch.processes.cleanup();
    let own = std::process::id();
    let left = || -> Result<Vec<String>> {
        Ok(crate::launcher::namespace_processes()?
            .into_keys()
            .filter(|pid| *pid != 1 && *pid != own)
            .map(|pid| {
                let line = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
                format!(
                    "{pid} {}",
                    String::from_utf8_lossy(&line).replace('\0', " ")
                )
            })
            .collect())
    };
    // Killed processes may still be exiting; give them the same second.
    let settled = Instant::now() + Duration::from_secs(1);
    while !left()?.is_empty() && Instant::now() < settled {
        std::thread::sleep(Duration::from_millis(10));
    }
    let left = left()?;
    if !cleaned || !left.is_empty() {
        return Err(format!("private processes remain after cleanup: {left:?}"));
    }
    Ok(measured.and_then(|m| stopped.map(|()| m)))
}

/// Waits until the log stops growing across two polls 100 ms apart.
#[cfg(target_os = "linux")]
fn quiet(log: &Path, deadline: Instant) -> Result<usize> {
    let size = || fs::metadata(log).map(|m| m.len() as usize).map_err(error);
    let mut last = size()?;
    loop {
        pause(environment::monotonic_ns() + 100_000_000, deadline)?;
        let now = size()?;
        if now == last {
            return Ok(now);
        }
        last = now;
    }
}

#[cfg(target_os = "linux")]
fn drive(launch: &mut Headless, fixture: &str, deadline: Instant) -> Result<(Value, Vec<String>)> {
    let dir = launch.out.clone();
    let path = dir.join("eon.log");
    let log = || -> Result<Log> { Ok(Log::parse(&fs::read_to_string(&path).map_err(error)?)) };
    let size = || fs::metadata(&path).map(|m| m.len() as usize).map_err(error);
    launch.start_compositor_until(BusPolicy::Private, deadline)?;
    let outputs = terminals::sway(launch, deadline, &["-t", "get_outputs", "-r"])?;
    launch.output_size.check_outputs(&outputs)?;
    // The headless seat has no keyboard until this one joins; without it Venus
    // gets no wl_keyboard and no text input.
    let mut keyboard = seat::Keyboard::new(launch)?;
    launch.start_eon()?;
    // The workspace starts its shell once the CLI names the tab's directory.
    let here = dir.to_str().ok_or("non-UTF8 output")?;
    wait(deadline, "candidate workspace CLI", || {
        Ok(native::eon(
            launch,
            deadline,
            &["tab", "directory", "t1", "--json", "--", here],
        )
        .is_ok())
    })?;
    wait(deadline, "pane workload", || {
        Ok(directory_workers(&dir)?.len() == 1)
    })?;
    child::publish(&dir.join("setup"), "")?;
    let worker = directory_workers(&dir)?.remove(0);
    wait(deadline, "pane payload", || {
        Ok(worker.join("ready").is_file())
    })?;
    // As in P0: focus away, start the input method, focus back, so keyboard
    // focus and the text input's enter arrive together once attached (Sway
    // enters text inputs only while an input method exists; 0.1.28 enables
    // its IME only on that focus).
    wait(deadline, "the Venus window", || {
        let tree = terminals::sway(launch, deadline, &["-t", "get_tree", "-r"])?;
        Ok(terminals::window(&tree).is_some())
    })?;
    terminals::sway(launch, deadline, &["workspace away"])?;
    let mut echo = None;
    if fixture == "static-echo" {
        echo = Some(seat::Echo::new(launch)?);
    } else {
        let profile = dir.join("fcitx/fcitx5");
        fs::create_dir_all(&profile).map_err(error)?;
        fs::write(profile.join("profile"), "[Groups/0]\nName=Default\nDefault Layout=us\nDefaultIM=keyboard-us\n\n[Groups/0/Items/0]\nName=keyboard-us\nLayout=\n\n[GroupOrder]\n0=Default\n").map_err(error)?;
        let fcitx5 = terminals::tool("fcitx5")?;
        let version = terminals::call(launch, deadline, &fcitx5, &["--version"])?;
        fs::write(dir.join("fcitx5-version.txt"), version).map_err(error)?;
        launch.env.insert(
            "XDG_CONFIG_HOME".into(),
            dir.join("fcitx").display().to_string(),
        );
        let started = launch.spawn(
            fcitx5.to_str().ok_or("non-UTF8 fcitx5 path")?,
            &[
                "--disable",
                "all",
                "--enable",
                "wayland,waylandim,keyboard,dbus",
            ],
            "fcitx5.log",
        );
        launch.env.remove("XDG_CONFIG_HOME");
        started?;
        wait(deadline, "fcitx5 input method", || {
            Ok(fs::read_to_string(dir.join("fcitx5.log"))
                .is_ok_and(|t| t.contains(".get_input_method(")))
        })?;
    }
    terminals::sway(launch, deadline, &["workspace back_and_forth"])?;
    // 0.1.28 applies IME focus only when it draws (P0's pane was streaming),
    // so one echoed Enter makes it draw before any window opens.
    keyboard.key(KEY_ENTER, true)?;
    keyboard.key(KEY_ENTER, false)?;
    // Startup readiness: the IME is active and Venus has published its caret.
    wait(deadline, "active input method", || match &mut echo {
        Some(echo) => echo.active(),
        None => Ok(fs::read_to_string(dir.join("fcitx5.log")).is_ok_and(|t| activated(&t))),
    })?;
    wait(deadline, "Venus caret publication", || Ok(log()?.ready()))?;
    let start = quiet(&path, deadline)?;
    let rows = || -> Result<u64> {
        json(launch, deadline, &["pane", "read", "p1", "--json"])?
            .get("rows")
            .and_then(Value::as_u64)
            .ok_or("candidate lacks pane rows".into())
    };
    // held-repeat's sink counts: (held, leaked after the dwell).
    let mut repeat = None;
    let windows = match fixture {
        "static-echo" => {
            let echo = echo.as_mut().ok_or("missing echo")?;
            echo.burst(ECHO_COMMITS as u32, Duration::from_millis(2))?;
            vec![start, quiet(&path, deadline)?]
        }
        "moving-caret-30" | "moving-caret-60" => {
            child::publish(&dir.join("go"), "")?;
            // The payload parks the caret in column 1 after its last tick, so
            // the motion has been presented once that publication is logged.
            wait(deadline, "the parked caret", || {
                Ok(worker.join("ticked").is_file() && log()?.parked(start))
            })?;
            let tail = quiet(&path, deadline)?;
            pause(environment::monotonic_ns() + 1_000_000_000, deadline)?;
            vec![start, tail, quiet(&path, deadline)?]
        }
        "held-repeat" => {
            child::publish(&dir.join("go"), "")?;
            wait(deadline, "raw sink", || Ok(worker.join("count").is_file()))?;
            let count = || -> Result<u64> {
                fs::read_to_string(worker.join("count"))
                    .map_err(error)?
                    .parse()
                    .map_err(error)
            };
            let start = quiet(&path, deadline)?;
            keyboard.key(KEY_X, true)?;
            pause(environment::monotonic_ns() + HOLD_MS * 1_000_000, deadline)?;
            keyboard.key(KEY_X, false)?;
            keyboard.roundtrip()?;
            wait(deadline, "logged key release", || {
                Ok(log()?.released(start, KEY_X))
            })?;
            // A bounded settle for bytes in flight at the release: a stuck
            // repeat never settles, so it must show up in the dwell instead.
            pause(
                environment::monotonic_ns() + SETTLE_MS * 1_000_000,
                deadline,
            )?;
            let fence = size()?;
            let info = log()?.repeat(start).filter(|r| r.0 > 0);
            let (held, info) = (count()?, info.ok_or("no repeat_info")?);
            let dwell = info.1 + 3 * 1000 / info.0;
            pause(environment::monotonic_ns() + dwell * 1_000_000, deadline)?;
            repeat = Some((held, count()?.saturating_sub(held), info));
            vec![start, fence]
        }
        "configure-storm" => {
            child::publish(&dir.join("go"), "")?;
            for _ in 0..TOGGLES {
                let from = size()?;
                terminals::sway(launch, deadline, &["fullscreen toggle"])?;
                wait(deadline, "an acknowledged same-size configure", || {
                    Ok(log()?.configured(from, 800))
                })?;
            }
            let middle = quiet(&path, deadline)?;
            let mut end = middle;
            for step in 0..MODES {
                let height = if step % 2 == 0 { 760 } else { 800 };
                let (rows_before, from) = (rows()?, size()?);
                let frames = log()?.window(from, usize::MAX).frames;
                terminals::sway(launch, deadline, &[&format!("output * mode 1280x{height}")])?;
                wait(deadline, "a resized grid and frame", || {
                    let log = log()?;
                    Ok(rows()? != rows_before
                        && log.window(from, usize::MAX).frames > frames
                        && log.configured(from, height))
                })?;
                end = quiet(&path, deadline)?;
            }
            vec![start, middle, end]
        }
        "modifier-storm" => {
            // A virtual keyboard's keys do not move the seat's modifier
            // state; the depressed mask (Shift 1, Ctrl 4) has to be sent.
            for pair in 0..MODIFIER_PAIRS {
                let (key, mask) = if pair % 2 == 0 {
                    (KEY_LEFTSHIFT, 1)
                } else {
                    (KEY_LEFTCTRL, 4)
                };
                keyboard.key(key, true)?;
                keyboard.modifiers(mask)?;
                keyboard.key(key, false)?;
                keyboard.modifiers(0)?;
            }
            keyboard.roundtrip()?;
            wait(deadline, "every modifier event", || {
                Ok(log()?.window(start, usize::MAX).modifiers >= 2 * MODIFIER_PAIRS)
            })?;
            vec![start, quiet(&path, deadline)?]
        }
        _ => return Err("unknown fixture".into()),
    };
    let (mut counts, failures) = match repeat {
        Some((held, leaked, info)) => {
            held_repeat(&log()?, windows[0], windows[1], held, leaked, info)
        }
        None => judge(fixture, &log()?, &windows),
    };
    // Readiness evidence: the eon.log byte offsets bounding each window; the
    // first is where the startup barrier released.
    if let Value::Object(fields) = &mut counts {
        let bytes = windows.iter().map(|&b| Value::from(b as u64)).collect();
        fields.push(("window_bytes".into(), Value::Array(bytes)));
    }
    Ok((counts, failures))
}

/// fcitx5's own protocol log shows its input method bound and activated.
fn activated(log: &str) -> bool {
    let mut bound = false;
    log.lines()
        .filter_map(message)
        .any(|(request, interface, _, name, _)| {
            bound |=
                request && interface == "zwp_input_method_manager_v2" && name == "get_input_method";
            bound && !request && interface == "zwp_input_method_v2" && name == "activate"
        })
}

/// One fixture verdict: named window counts and every failed requirement.
struct Verdict(Vec<(String, Value)>, Vec<String>);
impl Verdict {
    fn new(log: &Log) -> Self {
        Self(Vec::new(), log.error.iter().cloned().collect())
    }
    fn tally(&mut self, name: &str, tally: &Tally) {
        let fields = [
            ("frames", tally.frames),
            ("publications", tally.publications),
            ("most_per_epoch", tally.most_per_epoch),
            ("distinct_rectangles", tally.distinct),
            ("dones", tally.dones),
            ("enables", tally.enables),
            ("modifiers", tally.modifiers),
            ("configures", tally.configures),
        ];
        self.0
            .push((name.into(), object(fields.map(|(k, v)| (k, v.into())))));
    }
    /// No caret publication and no re-enable inside `w`.
    fn silent(&mut self, w: &Tally, when: &str) {
        self.require(
            w.publications == 0,
            format!("{} publications {when}", w.publications),
        );
        self.require(w.enables == 0, format!("{} enables {when}", w.enables));
    }
    fn require(&mut self, ok: bool, what: String) {
        if !ok {
            self.1.push(what);
        }
    }
    fn done(self) -> (Value, Vec<String>) {
        (Value::Object(self.0), self.1)
    }
}

fn judge(fixture: &str, log: &Log, windows: &[usize]) -> (Value, Vec<String>) {
    let mut v = Verdict::new(log);
    let window = |n: usize| log.window(windows[n], windows[n + 1]);
    match fixture {
        "static-echo" => {
            let w = window(0);
            v.tally("echo", &w);
            v.require(
                w.dones >= ECHO_COMMITS / 2,
                format!("{} done < {}", w.dones, ECHO_COMMITS / 2),
            );
            v.require(
                w.most_per_epoch <= 1,
                format!("{} publications in one epoch", w.most_per_epoch),
            );
            v.require(
                w.enables == 0,
                format!("{} enables during the echo", w.enables),
            );
        }
        "configure-storm" => {
            let (same, changing) = (window(0), window(1));
            v.tally("unchanged", &same);
            v.tally("changing", &changing);
            v.require(
                same.configures >= TOGGLES,
                format!("{} same-size configures < {TOGGLES}", same.configures),
            );
            v.require(
                changing.configures >= MODES,
                format!("{} resize configures < {MODES}", changing.configures),
            );
            v.silent(&same, "on unchanged configures");
            let bound = changing.frames.min(MODES + 1);
            v.require(
                (1..=bound).contains(&changing.publications),
                format!(
                    "{} resize publications outside 1..={bound}",
                    changing.publications
                ),
            );
        }
        "modifier-storm" => {
            let w = window(0);
            v.tally("storm", &w);
            v.require(
                w.modifiers >= 2 * MODIFIER_PAIRS,
                format!("{} modifiers < {}", w.modifiers, 2 * MODIFIER_PAIRS),
            );
            v.silent(&w, "during modifiers");
        }
        _ => {
            let (motion, tail) = (window(0), window(1));
            v.tally("motion", &motion);
            v.tally("tail", &tail);
            v.require(
                motion.distinct >= 2,
                format!("{} distinct rectangles < 2", motion.distinct),
            );
            v.require(
                motion.publications <= motion.frames + 1,
                format!(
                    "{} publications > {} frames + 1",
                    motion.publications, motion.frames
                ),
            );
            v.silent(&tail, "in the static tail");
        }
    }
    v.done()
}

fn held_repeat(
    log: &Log,
    start: usize,
    fence: usize,
    held: u64,
    leaked: u64,
    (rate, delay): (u64, u64),
) -> (Value, Vec<String>) {
    let mut v = Verdict::new(log);
    let hold = log.window(start, fence);
    v.tally("hold", &hold);
    let floor = 1 + HOLD_MS.saturating_sub(delay) * rate / 1000 / 2;
    v.0.extend([
        ("repeat_rate".into(), rate.into()),
        ("repeat_delay_ms".into(), delay.into()),
        ("held_bytes".into(), held.into()),
        ("dwell_bytes".into(), leaked.into()),
    ]);
    v.require(held >= floor, format!("{held} repeated bytes < {floor}"));
    v.require(hold.frames > 0, "no frame during the hold".into());
    v.require(leaked == 0, format!("{leaked} bytes after the release"));
    v.done()
}

#[derive(Debug, PartialEq)]
enum Kind {
    Frame,
    /// `zwp_text_input_v3.enable`; each opens an epoch.
    Enable,
    /// A committed text-input state in an enable epoch; the enable commit
    /// counts only when it carries a rectangle.
    Publication(u64, Option<[i64; 4]>),
    Done,
    Modifiers,
    Configure(i64),
    Ack,
    /// `wl_keyboard.key`: (evdev code, pressed).
    Key(u64, bool),
    Repeat(u64, u64),
}

/// Venus's protocol log, by byte offset of each message's line end.
#[derive(Default)]
struct Log {
    events: Vec<(usize, Kind)>,
    error: Option<String>,
}

#[derive(Default, Debug, PartialEq)]
struct Tally {
    frames: u64,
    publications: u64,
    most_per_epoch: u64,
    distinct: u64,
    dones: u64,
    enables: u64,
    modifiers: u64,
    configures: u64,
}

/// `[ts] {queue}  -> interface#id.message(args)`: (request, interface, id, message, args).
fn message(line: &str) -> Option<(bool, &str, &str, &str, &str)> {
    let rest = line.split_once("] ")?.1.trim_start();
    let rest = match rest.strip_prefix('{') {
        Some(queued) => queued.split_once('}')?.1.trim_start(),
        None => rest,
    };
    let (request, rest) = match rest.strip_prefix("-> ") {
        Some(rest) => (true, rest),
        None => (false, rest),
    };
    let (interface, rest) = rest.split_once('#')?;
    let (id, rest) = rest.split_once('.')?;
    let (name, args) = rest.split_once('(')?;
    Some((request, interface, id, name, args.strip_suffix(')')?))
}

impl Log {
    fn parse(text: &str) -> Self {
        let mut log = Log::default();
        let (mut surface, mut attached, mut open, mut enabling) = (None, false, false, false);
        let (mut rect, mut offset, mut epochs) = (None, 0, 0);
        let numbers = |args: &str| -> Vec<i64> {
            args.split(", ")
                .map(|a| a.trim().parse().unwrap_or(-1))
                .collect()
        };
        for line in text.split_inclusive('\n') {
            offset += line.len();
            let Some((request, interface, id, name, args)) = message(line.trim_end()) else {
                continue;
            };
            let kind = match (request, interface, name) {
                (true, "xdg_wm_base", "get_xdg_surface") if surface.is_none() => {
                    surface = args.rsplit_once("wl_surface#").map(|(_, s)| s.to_owned());
                    None
                }
                (true, "wl_surface", "attach") if surface.as_deref() == Some(id) => {
                    attached = !args.starts_with("nil");
                    None
                }
                (true, "wl_surface", "commit") if surface.as_deref() == Some(id) => {
                    std::mem::take(&mut attached).then_some(Kind::Frame)
                }
                (true, "zwp_text_input_v3", "enable") => {
                    if rect.is_some() {
                        log.error
                            .get_or_insert("uncommitted cursor rectangle at re-enable".into());
                    }
                    (epochs, open, enabling, rect) = (epochs + 1, true, true, None);
                    Some(Kind::Enable)
                }
                (true, "zwp_text_input_v3", "set_cursor_rectangle") => {
                    let n = numbers(args);
                    rect = (n.len() == 4).then(|| [n[0], n[1], n[2], n[3]]);
                    None
                }
                // The enable commit publishes only a rectangle sent with it.
                (true, "zwp_text_input_v3", "commit") if std::mem::take(&mut enabling) => {
                    rect.take().map(|r| Kind::Publication(epochs, Some(r)))
                }
                (true, "zwp_text_input_v3", "commit") if open => {
                    Some(Kind::Publication(epochs, rect.take()))
                }
                (true, "zwp_text_input_v3", "disable") | (false, "zwp_text_input_v3", "leave") => {
                    if rect.is_some() {
                        log.error
                            .get_or_insert("uncommitted cursor rectangle at epoch end".into());
                    }
                    (open, enabling) = (false, false);
                    None
                }
                (false, "zwp_text_input_v3", "done") => Some(Kind::Done),
                (false, "wl_keyboard", "modifiers") => Some(Kind::Modifiers),
                (false, "wl_keyboard", "key") => {
                    let n = numbers(args);
                    (n.len() == 4 && (0..=1).contains(&n[3]))
                        .then(|| Kind::Key(n[2] as u64, n[3] == 1))
                }
                (false, "wl_keyboard", "repeat_info") => {
                    let n = numbers(args);
                    (n.len() == 2).then(|| Kind::Repeat(n[0].max(0) as u64, n[1].max(0) as u64))
                }
                (false, "xdg_toplevel", "configure") => {
                    Some(Kind::Configure(numbers(args).get(1).copied().unwrap_or(-1)))
                }
                (true, "xdg_surface", "ack_configure") => Some(Kind::Ack),
                _ => None,
            };
            if let Some(kind) = kind {
                log.events.push((offset, kind));
            }
        }
        if rect.is_some() {
            log.error
                .get_or_insert("uncommitted cursor rectangle at the end of the log".into());
        }
        log
    }
    fn between(&self, from: usize, to: usize) -> impl Iterator<Item = &Kind> {
        // An event belongs to the window its line ends in: from < at <= to.
        self.events
            .iter()
            .filter(move |(at, _)| *at > from && *at <= to)
            .map(|(_, k)| k)
    }
    fn window(&self, from: usize, to: usize) -> Tally {
        let mut tally = Tally::default();
        let (mut epochs, mut rects) = (BTreeMap::new(), std::collections::BTreeSet::new());
        for kind in self.between(from, to) {
            match kind {
                Kind::Frame => tally.frames += 1,
                Kind::Publication(epoch, rect) => {
                    tally.publications += 1;
                    *epochs.entry(epoch).or_insert(0) += 1;
                    rects.extend(*rect);
                }
                Kind::Done => tally.dones += 1,
                Kind::Enable => tally.enables += 1,
                Kind::Modifiers => tally.modifiers += 1,
                Kind::Configure(_) => tally.configures += 1,
                _ => {}
            }
        }
        tally.most_per_epoch = epochs.into_values().max().unwrap_or(0);
        tally.distinct = rects.len() as u64;
        tally
    }
    /// An enable epoch has published a caret rectangle.
    fn ready(&self) -> bool {
        self.events
            .iter()
            .any(|(_, k)| matches!(k, Kind::Publication(_, Some(_))))
    }
    /// The last caret rectangle after `from` is its leftmost, after motion.
    fn parked(&self, from: usize) -> bool {
        let xs: Vec<_> = self
            .between(from, usize::MAX)
            .filter_map(|k| match k {
                Kind::Publication(_, Some(r)) => Some(r[0]),
                _ => None,
            })
            .collect();
        xs.iter().min() == xs.last() && xs.first() != xs.last()
    }
    fn released(&self, from: usize, key: u32) -> bool {
        // A release is `wl_keyboard.key(serial, time, key, 0)`.
        self.between(from, usize::MAX)
            .any(|k| *k == Kind::Key(key.into(), false))
    }
    /// A configure to `height` after `from`, acknowledged.
    fn configured(&self, from: usize, height: i64) -> bool {
        let mut seen = false;
        self.between(from, usize::MAX).any(|k| {
            seen |= *k == Kind::Configure(height);
            seen && *k == Kind::Ack
        })
    }
    /// The last (rate, delay ms) before `before`.
    fn repeat(&self, before: usize) -> Option<(u64, u64)> {
        self.between(0, before)
            .filter_map(|k| match k {
                Kind::Repeat(rate, delay) => Some((*rate, *delay)),
                _ => None,
            })
            .last()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENABLE: &str = "[1.0]  -> xdg_wm_base#14.get_xdg_surface(new id xdg_surface#29, wl_surface#28)\n[1.1]  -> zwp_text_input_v3#21.enable()\n[1.2]  -> zwp_text_input_v3#21.set_content_type(0, 0)\n[1.3]  -> zwp_text_input_v3#21.commit()\n";
    fn rect(y: u32) -> String {
        format!(
            "[2.0]  -> zwp_text_input_v3#21.set_cursor_rectangle(8, {y}, 10, 19)\n[2.1]  -> zwp_text_input_v3#21.commit()\n"
        )
    }
    const FRAME: &str = "[3.0] {mesa vk display queue}  -> wl_surface#28.attach(wl_buffer#58, 0, 0)\n[3.1] {mesa vk display queue}  -> wl_surface#28.commit()\n";
    const DONE: &str = "[4.0] zwp_text_input_v3#21.done(7)\n";
    /// A re-enable that publishes its rectangle in the enable commit.
    const REENABLE: &str = "[4.1]  -> zwp_text_input_v3#21.enable()\n[4.2]  -> zwp_text_input_v3#21.set_cursor_rectangle(8, 10, 10, 19)\n[4.3]  -> zwp_text_input_v3#21.commit()\n";
    fn judged(fixture: &str, text: &str, windows: &[usize]) -> Vec<String> {
        judge(fixture, &Log::parse(text), windows).1
    }

    #[test]
    fn static_echo_limits_publications_per_epoch() {
        let mut text = format!("{ENABLE}{}", rect(10));
        let start = text.len();
        text += &DONE.repeat(1000);
        assert!(judged("static-echo", &text, &[start, text.len()]).is_empty());
        text += &(rect(10) + &rect(10));
        assert_eq!(
            judged("static-echo", &text, &[start, text.len()]),
            ["2 publications in one epoch"]
        );
        assert_eq!(
            judged("static-echo", ENABLE, &[ENABLE.len(), ENABLE.len()]),
            ["0 done < 1000"]
        );
        // Re-enable churn: one publication per epoch, each in an enable commit.
        let mut churn = format!("{ENABLE}{}", rect(10));
        let start = churn.len();
        churn += &(DONE.repeat(1000) + &REENABLE.repeat(2));
        assert_eq!(
            judged("static-echo", &churn, &[start, churn.len()]),
            ["2 enables during the echo"]
        );
        assert_eq!(
            Log::parse(&churn).window(start, churn.len()).publications,
            2
        );
    }

    #[test]
    fn publication_counts_commit_once_and_tracks_epoch_across_window_start() {
        let text = format!("{ENABLE}{}{}", rect(10), rect(30));
        let start = ENABLE.len() + rect(10).len();
        let log = Log::parse(&text);
        assert!(log.ready());
        assert_eq!(log.window(0, text.len()).publications, 2);
        assert_eq!(log.window(start, text.len()).publications, 1);
        assert_eq!(log.window(start, text.len()).most_per_epoch, 1);
        assert!(!Log::parse(ENABLE).ready());
        let dangling =
            format!("{ENABLE}[5.0]  -> zwp_text_input_v3#21.set_cursor_rectangle(1, 2, 3, 4)\n");
        assert!(Log::parse(&dangling).error.is_some());
        let closed = format!("{dangling}[5.1]  -> zwp_text_input_v3#21.disable()\n");
        assert_eq!(
            Log::parse(&closed).error.as_deref(),
            Some("uncommitted cursor rectangle at epoch end")
        );
        let reopened = format!("{dangling}{REENABLE}");
        assert_eq!(
            Log::parse(&reopened).error.as_deref(),
            Some("uncommitted cursor rectangle at re-enable")
        );
        assert!(Log::parse(&format!("{ENABLE}{REENABLE}")).ready());
    }

    #[test]
    fn moving_caret_bounds_by_frames_and_requires_motion() {
        let start = ENABLE.len();
        let tail = |t: &str| [start, t.len(), t.len()];
        let good = format!("{ENABLE}{FRAME}{}{FRAME}{}", rect(10), rect(30));
        assert!(judged("moving-caret-30", &good, &tail(&good)).is_empty());
        let flood = format!("{good}{}", rect(30).repeat(2));
        assert_eq!(
            judged("moving-caret-30", &flood, &tail(&flood)),
            ["4 publications > 2 frames + 1"]
        );
        let still = format!("{ENABLE}{FRAME}{}", rect(10));
        assert_eq!(
            judged("moving-caret-60", &still, &tail(&still)),
            ["1 distinct rectangles < 2"]
        );
    }

    #[test]
    fn moving_caret_waits_for_the_parked_caret() {
        let at = |x| {
            format!(
                "[2.2]  -> zwp_text_input_v3#21.set_cursor_rectangle({x}, 30, 10, 19)\n[2.3]  -> zwp_text_input_v3#21.commit()\n"
            )
        };
        let motion = format!("{ENABLE}{}{}", at(8), at(16));
        assert!(!Log::parse(&motion).parked(0));
        assert!(Log::parse(&format!("{motion}{}", at(0))).parked(0));
        assert!(!Log::parse(&format!("{ENABLE}{}", at(0))).parked(0));
    }

    #[test]
    fn moving_caret_static_tail_must_be_silent() {
        let motion = format!("{ENABLE}{FRAME}{}{FRAME}{}", rect(10), rect(30));
        let text = format!("{motion}{DONE}{}", rect(30));
        assert_eq!(
            judged(
                "moving-caret-30",
                &text,
                &[ENABLE.len(), motion.len(), text.len()]
            ),
            ["1 publications in the static tail"]
        );
    }

    #[test]
    fn presented_frames_count_main_surface_attach_commit_only() {
        let noise = "[6.0]  -> wl_surface#40.attach(wl_buffer#9, 0, 0)\n[6.1]  -> wl_surface#40.commit()\n[6.2]  -> wl_surface#28.attach(nil, 0, 0)\n[6.3]  -> wl_surface#28.commit()\n[6.4]  -> wl_surface#28.commit()\n[6.5] wl_callback#50.done(1)\n[6.6] wl_display#1.delete_id(50)\n";
        let text = format!("{ENABLE}{FRAME}{noise}");
        assert_eq!(Log::parse(&text).window(0, text.len()).frames, 1);
    }

    #[test]
    fn held_repeat_fails_on_starved_repeats_and_on_dwell_bytes() {
        let text = format!("[0.1] wl_keyboard#20.repeat_info(25, 300)\n{ENABLE}{FRAME}");
        let log = Log::parse(&text);
        let (start, info) = (text.len() - FRAME.len(), (25, 300));
        assert_eq!(log.repeat(start), Some(info));
        assert!(
            held_repeat(&log, start, text.len(), 22, 0, info)
                .1
                .is_empty()
        );
        assert_eq!(
            held_repeat(&log, start, text.len(), 21, 0, info).1,
            ["21 repeated bytes < 22"]
        );
        assert_eq!(
            held_repeat(&log, start, text.len(), 40, 3, info).1,
            ["3 bytes after the release"]
        );
        assert_eq!(
            held_repeat(&log, start, start, 40, 0, info).1,
            ["no frame during the hold"]
        );
    }

    #[test]
    fn configure_storm_requires_observed_configures_and_silent_unchanged_phase() {
        let configure = |h: u32| {
            format!(
                "[7.0] xdg_toplevel#30.configure(1280, {h}, array[4])\n[7.1]  -> xdg_surface#29.ack_configure(9)\n"
            )
        };
        let same = format!("{ENABLE}{}", configure(800).repeat(TOGGLES as usize));
        let resized = format!(
            "{same}{}",
            format!("{}{FRAME}", configure(760)).repeat(MODES as usize)
        );
        let good = format!("{resized}{}", rect(40));
        let windows = [ENABLE.len(), same.len(), good.len()];
        assert!(judged("configure-storm", &good, &windows).is_empty());
        assert!(Log::parse(&good).configured(same.len(), 760));
        assert!(!Log::parse(&good).configured(same.len(), 700));
        assert_eq!(
            judged(
                "configure-storm",
                &resized,
                &[ENABLE.len(), same.len(), resized.len()]
            ),
            ["0 resize publications outside 1..=10"]
        );
        let doubled = format!(
            "{same}{}",
            format!("{}{FRAME}{}{}", configure(760), rect(40), rect(50)).repeat(MODES as usize)
        );
        assert_eq!(
            judged(
                "configure-storm",
                &doubled,
                &[ENABLE.len(), same.len(), doubled.len()]
            ),
            ["20 resize publications outside 1..=10"]
        );
        let configs = &same[ENABLE.len()..];
        let noisy = format!("{ENABLE}{}{configs}{}", rect(40), &good[same.len()..]);
        let middle = ENABLE.len() + rect(40).len() + configs.len();
        assert_eq!(
            judged(
                "configure-storm",
                &noisy,
                &[ENABLE.len(), middle, noisy.len()]
            ),
            ["1 publications on unchanged configures"]
        );
        let short = format!("{ENABLE}{}", configure(800));
        assert_eq!(
            judged(
                "configure-storm",
                &short,
                &[ENABLE.len(), short.len(), short.len()]
            )
            .len(),
            3
        );
    }

    #[test]
    fn modifier_storm_counts_inside_window_and_requires_2n() {
        let modifier = "[8.0] wl_keyboard#20.modifiers(9, 1, 0, 0, 0)\n";
        let key = format!("[8.1] wl_keyboard#20.key(9, 1, {KEY_LEFTSHIFT}, 1)\n");
        let text = format!(
            "{ENABLE}{modifier}{}{}",
            modifier.repeat(399),
            key.repeat(2)
        );
        let from = ENABLE.len() + modifier.len();
        let windows = [from, text.len()];
        assert_eq!(
            judged("modifier-storm", &text, &windows),
            ["399 modifiers < 400"]
        );
        let full = format!("{text}{modifier}{}", rect(10));
        assert_eq!(
            judged("modifier-storm", &full, &[from, full.len()]),
            ["1 publications during modifiers"]
        );
        let churn = format!("{text}{modifier}{REENABLE}");
        assert_eq!(
            judged("modifier-storm", &churn, &[from, churn.len()]),
            [
                "1 publications during modifiers",
                "1 enables during modifiers"
            ]
        );
    }

    #[test]
    fn fcitx5_readiness_requires_bound_and_activated_input_method() {
        let bound = "[1.0]  -> zwp_input_method_manager_v2#9.get_input_method(wl_seat#3, new id zwp_input_method_v2#10)\n";
        let active = "[2.0] zwp_input_method_v2#10.activate()\n";
        assert!(activated(&format!("{bound}{active}")));
        assert!(!activated(bound));
        assert!(!activated(active));
        assert!(!activated(&format!(
            "{bound}[2.0]  -> zwp_input_method_v2#10.commit(0)\n"
        )));
    }

    #[test]
    fn events_options_select_known_fixtures_and_bound_the_deadline() {
        let hash = "0".repeat(40);
        let args = |extra: &str| {
            format!("--out /x --eon edge=/nix/store/c/bin/eon --revision edge={hash} --candidate-tree edge={hash} --source-tree {hash} {extra}").split_whitespace().map(str::to_owned).collect::<Vec<_>>()
        };
        assert_eq!(options(&args("")).unwrap().fixtures.len(), 6);
        assert_eq!(
            options(&args("--only held-repeat")).unwrap().fixtures,
            ["held-repeat"]
        );
        assert!(options(&args("--only typing")).is_err());
        assert!(options(&args("--deadline 5")).is_err());
    }
}
