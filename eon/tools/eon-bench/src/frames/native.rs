use super::*;
use std::io::{BufRead, BufReader, Write};

const HISTORY_LINES: u64 = 3000;
const CADENCE_NS: u64 = 33_333_333;
const TAIL_NS: u64 = 2_000_000_000;

fn load_guard(mode: &str, limit: f64, start: f64, end: f64) -> Result {
    let load = start.max(end);
    if mode != "check" && load > limit {
        return Err(format!(
            "environment load {load:.2} exceeds {limit}; refuses to wait inside a heavy slot"
        ));
    }
    Ok(())
}

fn wait(deadline: Instant, what: &str, mut ready: impl FnMut() -> Result<bool>) -> Result {
    loop {
        suite::interrupt::check()?;
        if ready()? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("deadline waiting for {what}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn pause(until: u64, deadline: Instant) -> Result {
    loop {
        suite::interrupt::check()?;
        let now = environment::monotonic_ns();
        if now >= until {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("measurement window exceeded sample deadline".into());
        }
        std::thread::sleep(Duration::from_nanos(until - now).min(Duration::from_millis(20)));
    }
}
fn eon(launch: &Headless, deadline: Instant, args: &[&str]) -> Result<String> {
    terminals::call(launch, deadline, &launch.eon, args)
}
fn json(launch: &Headless, deadline: Instant, args: &[&str]) -> Result<Value> {
    Value::parse(&eon(launch, deadline, args)?)
}
fn directory_workers(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = fs::read_dir(dir)
        .map_err(error)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("worker-"))
        })
        .map(|e| e.path())
        .collect::<Vec<_>>();
    dirs.sort();
    Ok(dirs)
}
fn payload(blink: bool, dense: bool) -> String {
    let mut text = "\x1b[0m\x1b[2J\x1b[H".to_owned();
    if dense {
        for n in 0..HISTORY_LINES {
            text.push_str(&format!(
                "\x1b[38;5;{}m{n:06} 漢字中文 日本語 \u{f120} \u{f07c} abcXYZ\x1b[0m\r\n",
                16 + n % 216
            ));
        }
    } else {
        text.push_str("EON FRAMES READY");
    }
    text.push_str(if blink {
        "\x1b[?25h\x1b[1 q"
    } else {
        "\x1b[?25h\x1b[2 q"
    });
    text
}

/// Terminal child, also used by default-shell panes created through the Eon CLI.
pub fn workload(args: &[String]) -> Result {
    let [root, name, repetitions, kitten, cat64] = args else {
        return Err("frames-workload needs DIR NAME REPETITIONS KITTEN CAT64".into());
    };
    let root = Path::new(root);
    let dir = root.join(format!("worker-{}", std::process::id()));
    fs::create_dir(&dir).map_err(error)?;
    child::publish(&dir.join("spawned"), &std::process::id().to_string())?;
    let deadline = Instant::now() + Duration::from_secs(600);
    wait(deadline, "fixture setup", || {
        child::publish(&dir.join("grid"), &format!("{:?}", child::grid()))?;
        Ok(root.join("setup").is_file())
    })?;
    crate::writer::run(&root.join("payload.vt"), &dir.join("setup.json"))?;
    child::publish(&dir.join("grid"), &format!("{:?}", child::grid()))?;
    child::publish(&dir.join("ready"), "")?;
    wait(deadline, "fixture go", || Ok(root.join("go").is_file()))?;
    let result = if name == "cat64" || name.starts_with("kitten-") {
        fs::write(dir.join("setup.vt"), b"\x1b[0m\x1b[2J\x1b[HEON FRAMES GO").map_err(error)?;
        terminals::workload(&[
            dir.display().to_string(),
            name.clone(),
            repetitions.clone(),
            kitten.clone(),
            cat64.clone(),
        ])
    } else {
        wait(deadline, "fixture finish", || {
            Ok(root.join("finish").is_file())
        })
    };
    child::publish(&dir.join("done"), if result.is_ok() { "0" } else { "1" })?;
    wait(deadline, "fixture release", || {
        Ok(root.join("release").is_file())
    })?;
    result
}

#[derive(Clone, Copy)]
struct Cpu {
    start: u64,
    user: u64,
    system: u64,
}
impl Cpu {
    fn at(pid: u32) -> Result<Self> {
        let text = fs::read_to_string(format!("/proc/{pid}/stat")).map_err(error)?;
        let fields = text
            .rsplit_once(") ")
            .ok_or("invalid Venus proc stat")?
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        if fields.first().is_none_or(|s| matches!(*s, "Z" | "X")) {
            return Err("Venus exited during sample".into());
        }
        let n = |index: usize| {
            fields
                .get(index)
                .ok_or("missing CPU field")?
                .parse::<u64>()
                .map_err(error)
        };
        Ok(Self {
            start: n(19)?,
            user: n(11)?,
            system: n(12)?,
        })
    }
    fn json(self) -> Value {
        object([
            ("start_ticks", self.start.into()),
            ("utime_ticks", self.user.into()),
            ("stime_ticks", self.system.into()),
        ])
    }
}
fn cpu_percent(before: Cpu, after: Cpu, elapsed_ns: u64, ticks: u64) -> Result<f64> {
    if before.start != after.start || elapsed_ns == 0 || ticks == 0 {
        return Err("invalid CPU measurement identity/window".into());
    }
    let used = after
        .user
        .checked_sub(before.user)
        .and_then(|n| {
            after
                .system
                .checked_sub(before.system)
                .and_then(|s| n.checked_add(s))
        })
        .ok_or("CPU ticks moved backwards")?;
    Ok(100.0 * used as f64 / ticks as f64 / (elapsed_ns as f64 / 1e9))
}
fn native_pid(value: &Value) -> Option<u32> {
    if value.get("app_id").and_then(Value::as_str) == Some("eon") {
        return value.get("pid")?.as_u64()?.try_into().ok();
    }
    for key in ["nodes", "floating_nodes"] {
        for node in value.get(key).and_then(Value::as_array).unwrap_or(&[]) {
            if let Some(pid) = native_pid(node) {
                return Some(pid);
            }
        }
    }
    None
}
fn capture(launch: &Headless, deadline: Instant, cell: &Cell, name: &str) -> Result<String> {
    let col_width = cell.width / cell.columns;
    let geometry = format!(
        "{},{} {}x{}",
        cell.width - col_width + 10,
        cell.height / 3,
        col_width - 20,
        cell.height / 3
    );
    let path = launch.out.join(name);
    terminals::call(
        launch,
        deadline,
        Path::new("grim"),
        &[
            "-t",
            "ppm",
            "-g",
            &geometry,
            path.to_str().ok_or("non-UTF8 capture")?,
        ],
    )?;
    hash(&path)
}
fn retained_lines(history: &Value) -> Result<u64> {
    if history.get("screen").and_then(Value::as_str) != Some("primary")
        || history.get("cut_bytes").and_then(Value::as_u64) != Some(0)
    {
        return Err("candidate lacks complete primary-screen history JSON".into());
    }
    let rows = history
        .get("rows")
        .and_then(Value::as_u64)
        .ok_or("candidate lacks history rows")?;
    let text = history
        .get("text")
        .and_then(Value::as_str)
        .ok_or("candidate lacks history text")?;
    Ok((text
        .lines()
        .filter(|line| {
            line.as_bytes()
                .get(..6)
                .is_some_and(|n| n.iter().all(u8::is_ascii_digit))
        })
        .count() as u64)
        .saturating_sub(rows))
}
fn scroll_bound(steps: u64) -> u64 {
    // Inspected both targets: 3 rows/wheel; pixel gain 2, min 16px row,
    // capped 8000px/s fling / decay 4. Reserve 256 rows beyond input.
    steps * 3 + 256
}

fn verify_columns(tab: &Value, panes: &[String]) -> Result {
    let actual = tab
        .get("panes")
        .and_then(Value::as_array)
        .ok_or("candidate lacks flat CLI pane layout")?;
    if actual.len() != panes.len()
        || tab.get("selected_pane").and_then(Value::as_str) != panes.last().map(String::as_str)
        || tab.get("selected_popup") != Some(&Value::Null)
        || actual
            .iter()
            .zip(panes)
            .enumerate()
            .any(|(i, (actual, expected))| {
                actual.get("id").and_then(Value::as_str) != Some(expected.as_str())
                    || actual.get("column").and_then(Value::as_u64) != Some(i as u64 + 1)
                    || actual.get("expanded").and_then(Value::as_bool) != Some(true)
            })
    {
        return Err(
            "workspace needs one expanded fixture pane per column, selected last, without a popup"
                .into(),
        );
    }
    Ok(())
}

struct Session {
    launch: Headless,
    deadline: Instant,
}
impl Session {
    fn new(options: &Options, candidate: &Candidate, cell: &Cell, dir: &Path) -> Result<Self> {
        let mut launch = Headless::new(
            dir,
            &candidate.path,
            if cell.throughput() {
                WindowMode::Floating
            } else {
                WindowMode::Fullscreen
            },
        )?;
        options.renderer.apply(&mut launch)?;
        launch.env.insert(
            "FONTCONFIG_FILE".into(),
            terminals::tool("frames_fontconfig")?.display().to_string(),
        );
        fs::write(dir.join("sway.conf"),format!("output * mode {}x{}\noutput * scale 1\noutput * bg #202020 solid_color\nseat seat0 fallback true\nxwayland disable\ndefault_border none\nfocus_follows_mouse no\nfor_window [app_id=\"eon\"] {} enable\n",cell.width,cell.height,if cell.throughput() { "floating" } else { "fullscreen" })).map_err(error)?;
        if options.mode != "control" {
            launch.env.insert(
                "EON_VENUS_FRAME_TIMINGS".into(),
                dir.join("frames.jsonl").display().to_string(),
            );
        }
        if options.mode == "check" {
            launch.env.insert("WAYLAND_DEBUG".into(), "client".into());
        }
        let exe = std::env::current_exe().map_err(error)?;
        let argv = [
            exe.display().to_string(),
            "frames-workload".into(),
            dir.display().to_string(),
            cell.workload.clone(),
            options.repetitions.to_string(),
            terminals::tool("kitten")?.display().to_string(),
            options
                .settings
                .out
                .join("corpus/cat64.vt")
                .display()
                .to_string(),
        ];
        let argv = Value::Array(argv.into_iter().map(Value::from).collect()).to_pretty();
        fs::write(dir.join("config/config.toml"),format!("[shell]\ncommand = {}\n[anima]\nenabled = false\n[session]\nanimate_working_badge = false\n[bar]\nwidgets = []\n[terminal]\nfont_family = \"DejaVu Sans Mono\"\nfont_size = 16\nline_height = 1.2\ncolumns = 80\nrows = 24\nbackground_opacity = 1.0\n",argv.trim())).map_err(error)?;
        fs::write(
            dir.join("payload.vt"),
            payload(cell.workload == "idle-blink", !cell.throughput()),
        )
        .map_err(error)?;
        Ok(Self {
            launch,
            deadline: Instant::now() + options.deadline,
        })
    }
    fn measure(&mut self, options: &Options, cell: &Cell) -> Result<Value> {
        let dir = self.launch.out.clone();
        let launch = &mut self.launch;
        fs::write(
            dir.join("versions.txt"),
            eon(launch, self.deadline, &["versions"])?,
        )
        .map_err(error)?;
        launch.start_compositor_until(BusPolicy::Private, self.deadline)?;
        fs::write(
            dir.join("compositor-version.txt"),
            terminals::call(launch, self.deadline, Path::new("sway"), &["--version"])?,
        )
        .map_err(error)?;
        let outputs = terminals::sway(launch, self.deadline, &["-t", "get_outputs", "-r"])?;
        let active = outputs
            .as_array()
            .ok_or("missing Sway outputs")?
            .iter()
            .filter(|o| o.get("active").and_then(Value::as_bool) == Some(true))
            .collect::<Vec<_>>();
        if active.len() != 1
            || active[0].get("current_mode").is_none_or(|m| {
                m.get("width").and_then(Value::as_u64) != Some(cell.width)
                    || m.get("height").and_then(Value::as_u64) != Some(cell.height)
            })
            || active[0].get("scale").and_then(Value::as_f64) != Some(1.0)
        {
            return Err("incorrect private output dimensions/scale".into());
        }
        launch.start_eon()?;
        wait(self.deadline, "candidate workspace CLI", || {
            Ok(eon(
                launch,
                self.deadline,
                &[
                    "tab",
                    "directory",
                    "t1",
                    "--json",
                    "--",
                    dir.to_str().ok_or("non-UTF8 output")?,
                ],
            )
            .is_ok())
        })?;
        let mut panes = vec!["p1".to_owned()];
        for _ in 1..cell.columns {
            let reply = json(launch, self.deadline, &["pane", "create", "--json"])?;
            panes.push(
                reply
                    .get("created")
                    .and_then(|c| c.get("pane"))
                    .and_then(Value::as_str)
                    .ok_or("candidate lacks created pane JSON")?
                    .into(),
            );
            eon(launch, self.deadline, &["pane", "move", "right", "--json"])?;
        }
        wait(self.deadline, "workload children", || {
            Ok(directory_workers(&dir)?.len() == cell.columns as usize)
        })?;
        if cell.throughput() {
            let mut settled = None;
            let mut last_resize = Instant::now() - Duration::from_secs(1);
            wait(self.deadline, "80x24 grid", || {
                let workers = directory_workers(&dir)?;
                let actual = fs::read_to_string(workers[0].join("grid"))
                    .ok()
                    .and_then(|s| terminals::grid(&s).ok());
                let native = terminals::window(&terminals::sway(
                    launch,
                    self.deadline,
                    &["-t", "get_tree", "-r"],
                )?);
                if actual == Some((24, 80)) && native.is_some() {
                    let since = settled.get_or_insert_with(Instant::now);
                    return Ok(since.elapsed() >= Duration::from_secs(1));
                }
                settled = None;
                if last_resize.elapsed() >= Duration::from_millis(150)
                    && let (Some((rows, cols)), Some((id, w, h))) = (actual, native)
                {
                    terminals::sway(
                        launch,
                        self.deadline,
                        &[&format!(
                            "[con_id={id}] resize set width {} px height {} px, move position center",
                            (w + (80 - cols) * 10).clamp(100, 1260),
                            (h + (24 - rows) * 19).clamp(100, 780)
                        )],
                    )?;
                    last_resize = Instant::now();
                }
                Ok(false)
            })?;
        }
        pause(environment::monotonic_ns() + 1_000_000_000, self.deadline)?;
        child::publish(&dir.join("setup"), "")?;
        wait(self.deadline, "fixture payload completion", || {
            Ok(directory_workers(&dir)?
                .iter()
                .all(|d| d.join("ready").is_file()))
        })?;
        let mut grids = Vec::new();
        for pane in &panes {
            let screen = json(launch, self.deadline, &["pane", "read", pane, "--json"])?;
            fs::write(dir.join(format!("{pane}-before.json")), screen.to_pretty())
                .map_err(error)?;
            if screen
                .get("rows")
                .and_then(Value::as_u64)
                .is_none_or(|n| n == 0)
                || screen
                    .get("columns")
                    .and_then(Value::as_u64)
                    .is_none_or(|n| n == 0)
                || (cell.throughput()
                    && (screen.get("rows").and_then(Value::as_u64) != Some(24)
                        || screen.get("columns").and_then(Value::as_u64) != Some(80)))
            {
                return Err(format!(
                    "candidate grid mismatch: rows={:?}, columns={:?}; throughput requires 24x80",
                    screen.get("rows").and_then(Value::as_u64),
                    screen.get("columns").and_then(Value::as_u64)
                ));
            }
            if cell.scroll() {
                let history = json(launch, self.deadline, &["pane", "history", pane, "--json"])?;
                let retained = retained_lines(&history)?;
                if retained <= scroll_bound(options.scroll_seconds * 1_000_000_000 / CADENCE_NS + 1)
                {
                    return Err(format!(
                        "history too shallow: {retained} retained logical lines; refuses a scroll that could reach its top"
                    ));
                }
                fs::write(
                    dir.join(format!("{pane}-history.json")),
                    history.to_pretty(),
                )
                .map_err(error)?;
            }
            grids.push(screen);
        }
        let session = json(launch, self.deadline, &["session", "--json"])?;
        fs::write(dir.join("session.json"), session.to_pretty()).map_err(error)?;
        let tab = session
            .get("tabs")
            .and_then(Value::as_array)
            .and_then(|t| t.iter().find(|t| t.get("id") == session.get("active_tab")))
            .ok_or("candidate lacks active tab JSON")?;
        verify_columns(tab, &panes)?;
        let mut pid = None;
        wait(self.deadline, "private Venus surface", || {
            let tree = terminals::sway(launch, self.deadline, &["-t", "get_tree", "-r"])?;
            pid = native_pid(&tree);
            Ok(pid.is_some())
        })?;
        let pid = pid.ok_or("missing private Venus PID")?;
        if launch.processes.remember(pid).is_none() {
            return Err("Venus PID lacks private runtime ownership".into());
        }
        let setup_revision = grids
            .last()
            .and_then(|s| s.get("revision"))
            .and_then(Value::as_u64)
            .ok_or("candidate lacks selected terminal revision")?;
        let pointer = if cell.scroll() {
            Some(input::Pointer::new(
                launch,
                cell.columns,
                cell.width,
                cell.height,
            )?)
        } else {
            None
        };
        pause(environment::monotonic_ns() + 1_000_000_000, self.deadline)?;
        let before_image = if cell.scroll() {
            Some(capture(launch, self.deadline, cell, "before.ppm")?)
        } else {
            None
        };
        if options.mode == "check" {
            let picture = dir.join("setup.png");
            terminals::call(
                &self.launch,
                self.deadline,
                Path::new("grim"),
                &[picture.to_str().ok_or("non-UTF8 capture")?],
            )?;
        }
        let ticks = rustix::param::clock_ticks_per_second();
        let before = Cpu::at(pid)?;
        let begin = environment::monotonic_ns();
        child::publish(&dir.join("go"), "")?;
        let stop;
        if let Some(pointer) = &pointer {
            let mut sent =
                std::io::BufWriter::new(fs::File::create(dir.join("input.tsv")).map_err(error)?);
            let until = begin + options.scroll_seconds * 1_000_000_000;
            let mut next = begin;
            while next < until {
                pause(next, self.deadline)?;
                if environment::monotonic_ns() > next + CADENCE_NS {
                    return Err(
                        "input missed a full cadence interval; invalid scroll conditions".into(),
                    );
                }
                pointer.scroll(cell.workload == "scroll-pixel")?;
                writeln!(sent, "{next}\t{}", environment::monotonic_ns()).map_err(error)?;
                next += CADENCE_NS;
            }
            pause(until, self.deadline)?;
            if cell.workload == "scroll-pixel" {
                pointer.stop()?;
            }
            stop = environment::monotonic_ns();
            sent.flush().map_err(error)?;
            pause(stop + TAIL_NS, self.deadline)?;
        } else if cell.idle() {
            pause(begin + options.idle_seconds * 1_000_000_000, self.deadline)?;
            stop = environment::monotonic_ns();
        } else {
            wait(self.deadline, "throughput completion", || {
                Ok(directory_workers(&dir)?
                    .iter()
                    .all(|d| d.join("done").is_file()))
            })?;
            stop = environment::monotonic_ns();
            pause(stop + 200_000_000, self.deadline)?;
        }
        let end = environment::monotonic_ns();
        let after = Cpu::at(pid)?;
        let percent = if options.mode == "check" {
            None
        } else {
            Some(cpu_percent(before, after, end - begin, ticks)?)
        };
        let venus_executable = fs::read_link(format!("/proc/{pid}/exe")).map_err(error)?;
        child::publish(&dir.join("finish"), "")?;
        wait(self.deadline, "successful workload completion", || {
            let dirs = directory_workers(&dir)?;
            for dir in &dirs {
                if let Ok(status) = fs::read_to_string(dir.join("done"))
                    && status != "0"
                {
                    return Err("terminal workload failed".into());
                }
            }
            Ok(dirs.iter().all(|d| d.join("done").is_file()))
        })?;
        let after_image = if cell.scroll() {
            Some(capture(&self.launch, self.deadline, cell, "after.ppm")?)
        } else {
            None
        };
        if cell.scroll() && before_image == after_image {
            return Err("scroll did not change the terminal body capture".into());
        }
        let mut completion_revision = None;
        for (index, pane) in panes.iter().enumerate() {
            let final_screen = json(
                &self.launch,
                self.deadline,
                &["pane", "read", pane, "--json"],
            )?;
            if ["rows", "columns"]
                .iter()
                .any(|key| final_screen.get(key) != grids[index].get(key))
            {
                return Err("terminal grid changed during the workload".into());
            }
            if cell.throughput() {
                if !final_screen
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|t| t.contains("EON BENCH COMPLETE"))
                {
                    return Err("throughput final screen marker absent".into());
                }
                completion_revision = Some(
                    final_screen
                        .get("revision")
                        .and_then(Value::as_u64)
                        .ok_or("missing throughput completion revision")?,
                );
            }
            fs::write(
                dir.join(format!("{pane}-after.json")),
                final_screen.to_pretty(),
            )
            .map_err(error)?;
        }
        Ok(object([
            ("begin_ns", begin.into()),
            ("input_stop_ns", stop.into()),
            ("end_ns", end.into()),
            ("cpu_percent_one_core", percent.into()),
            ("clock_ticks_per_second", ticks.into()),
            (
                "cpu_before",
                if options.mode == "check" {
                    Value::Null
                } else {
                    before.json()
                },
            ),
            (
                "cpu_after",
                if options.mode == "check" {
                    Value::Null
                } else {
                    after.json()
                },
            ),
            ("venus_pid", u64::from(pid).into()),
            (
                "venus_executable",
                venus_executable.display().to_string().into(),
            ),
            ("venus_sha256", hash(&venus_executable)?.into()),
            ("setup_revision", setup_revision.into()),
            ("completion_revision", completion_revision.into()),
            ("outputs", outputs),
            ("grids", Value::Array(grids)),
            (
                "compositor_renderer",
                self.launch.env["WLR_RENDERER"].as_str().into(),
            ),
            (
                "vulkan_icd",
                self.launch.env.get("VK_DRIVER_FILES").cloned().into(),
            ),
            ("before_capture_sha256", before_image.into()),
            ("after_capture_sha256", after_image.into()),
        ]))
    }
}

fn protocol_check(path: &Path, pixels: bool) -> Result {
    let mut axis = false;
    let mut discrete = false;
    let mut stop = false;
    for line in BufReader::new(fs::File::open(path).map_err(error)?).lines() {
        let line = line.map_err(error)?;
        if line.contains("wl_pointer") {
            axis |= line.contains(".axis(");
            discrete |= line.contains(".axis_discrete(") || line.contains(".axis_value120(");
            stop |= line.contains(".axis_stop(");
        }
    }
    if !axis || (pixels && (!stop || discrete)) || (!pixels && !discrete) {
        return Err(
            "native Wayland observation did not prove the requested wheel/pixel+stop input".into(),
        );
    }
    Ok(())
}
fn check_stderr(path: &Path) -> Result {
    for line in BufReader::new(fs::File::open(path).map_err(error)?).lines() {
        let line = line.map_err(error)?;
        if line.contains("Venus frame timings disabled")
            || line.contains("Venus frame timings writer failed")
        {
            return Err(format!("candidate diagnostic failure: {line}"));
        }
    }
    Ok(())
}
fn sample(options: &Options, candidate: &Candidate, cell: &Cell, dir: &Path) -> Result<Value> {
    let mut session = Session::new(options, candidate, cell, dir)?;
    let measured = session.measure(options, cell);
    let stopped = session.launch.stop_eon();
    let cleaned = session.launch.processes.cleanup();
    let mut detail = measured?;
    stopped?;
    if !cleaned {
        return Err("private processes remain after cleanup".into());
    }
    check_stderr(&dir.join("eon.log"))?;
    if options.mode == "check" && cell.scroll() {
        protocol_check(&dir.join("eon.log"), cell.workload == "scroll-pixel")?;
    }
    if options.mode != "control" {
        let window = observation_window(dir, &detail)?;
        let stats = diagnostics::timings(
            &window,
            cell.scroll() || cell.throughput() || cell.workload == "idle-blink",
        )?;
        let gpu = stats
            .get("adapter")
            .and_then(Value::as_str)
            .ok_or("missing adapter")?;
        if !gpu.starts_with(if options.renderer.0 == "lavapipe" {
            "software=true "
        } else {
            "software=false "
        }) || !gpu.contains("backend: Vulkan")
        {
            return Err(format!("renderer mismatch: {gpu}"));
        }
        if let Value::Object(fields) = &mut detail {
            fields.push(("venus_gpu".into(), gpu.into()));
            fields.push((
                "frame_statistics".into(),
                if options.mode == "check" {
                    Value::Null
                } else {
                    stats
                },
            ));
        }
    }
    Ok(detail)
}

fn observation_window(dir: &Path, observed: &Value) -> Result<diagnostics::Window> {
    let n = |key| {
        observed
            .get(key)
            .and_then(Value::as_u64)
            .ok_or(format!("missing {key}"))
    };
    Ok(diagnostics::Window {
        path: dir.join("frames.jsonl"),
        pid: u32::try_from(n("venus_pid")?).map_err(error)?,
        revision: n("setup_revision")?,
        completion_revision: match observed.get("completion_revision") {
            Some(Value::Null) => None,
            Some(value) => Some(value.as_u64().ok_or("invalid completion revision")?),
            None => return Err("missing completion revision".into()),
        },
        begin: n("begin_ns")?,
        stop: n("input_stop_ns")?,
        end: n("end_ns")?,
    })
}

fn font(launch: &Headless, deadline: Instant, pattern: &str, point: u32) -> Result<Value> {
    let report = terminals::call(
        launch,
        deadline,
        &terminals::tool("fc_match")?,
        &["-f", "%{file}\n%{charset}", pattern],
    )?;
    let (file, charset) = report.split_once('\n').ok_or("invalid fontconfig match")?;
    let covered = charset.split_whitespace().any(|range| {
        let (first, last) = range.split_once('-').unwrap_or((range, range));
        u32::from_str_radix(first, 16)
            .ok()
            .zip(u32::from_str_radix(last, 16).ok())
            .is_some_and(|(a, b)| point >= a && point <= b)
    });
    if !covered {
        return Err(format!("pinned font lacks U+{point:04X}: {file}"));
    }
    Ok(object([
        ("pattern", pattern.into()),
        ("path", file.into()),
        ("sha256", hash(Path::new(file))?.into()),
    ]))
}

pub(super) fn run(options: &Options) -> Result {
    let settings = &options.settings;
    if let Some(cpus) = &settings.cpus {
        environment::pin(cpus)?;
    }
    let mut result = RunResult {
        suite: "frames".into(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        quick: settings.quick,
        corpus_version: corpus::CORPUS_VERSION.into(),
        warmups: settings.warmups,
        samples: settings.samples,
        started_at: utc_timestamp(SystemTime::now()),
        finished_at: String::new(),
        environment: suite::environment(
            settings,
            environment::kernel(),
            environment::load()?,
            Grid { cols: 80, rows: 24 },
            Vec::new(),
            environment::build()?,
        )?,
        cells: Vec::new(),
    }
    .to_json();
    if let Some(Value::Object(fields)) = match &mut result {
        Value::Object(fields) => fields
            .iter_mut()
            .find(|(k, _)| k == "environment")
            .map(|(_, v)| v),
        _ => None,
    } && let Some((_, grid)) = fields.iter_mut().find(|(k, _)| k == "grid")
    {
        *grid = Value::Null;
    }
    if let Value::Object(fields) = &mut result {
        if let Some((_, tool)) = fields.iter_mut().find(|(k, _)| k == "tool") {
            *tool = "eon-bench-frames".into();
        }
        fields.retain(|(k, _)| !["cells", "finished_at"].contains(&k.as_str()));
        fields.extend([
            ("mode".into(),options.mode.as_str().into()),("source_tree".into(),options.source_tree.as_str().into()),
            ("requested_renderer".into(),options.renderer.0.as_str().into()),
            ("conditions".into(),object([("fixture_version",1u64.into()),("scroll_seconds",options.scroll_seconds.into()),("idle_seconds",options.idle_seconds.into()),("cadence_ns",CADENCE_NS.into()),("scroll_tail_ns",TAIL_NS.into()),("kitten_repetitions",options.repetitions.into()),("scrollback_budget_bytes",16_777_216u64.into()),("frame_metric","host scene+render+submit elapsed ns; acquisition separate; not scanout latency".into()),("instrumentation","measure/check: frame logger v2; control: logger disabled".into())])),
        ]);
    }
    let mut records = Vec::new();
    let mut execution = (|| -> Result {
        if options.mode != "check" && cfg!(debug_assertions) {
            return Err(
                "measurement/control requires release eon-bench, not a debug test binary".into(),
            );
        }
        let preflight = settings.out.join("fonts");
        fs::create_dir(&preflight).map_err(error)?;
        let mut launch = Headless::new(
            &preflight,
            &options.candidates[0].path,
            WindowMode::Fullscreen,
        )?;
        launch.env.insert(
            "FONTCONFIG_FILE".into(),
            terminals::tool("frames_fontconfig")?.display().to_string(),
        );
        let fonts = [
            font(
                &launch,
                Instant::now() + Duration::from_secs(10),
                "DejaVu Sans Mono:style=Book",
                0x41,
            )?,
            font(
                &launch,
                Instant::now() + Duration::from_secs(10),
                "DejaVu Sans Mono:charset=4e2d",
                0x4e2d,
            )?,
            font(
                &launch,
                Instant::now() + Duration::from_secs(10),
                "DejaVu Sans Mono:charset=f120",
                0xf120,
            )?,
        ];
        fs::write(
            settings.out.join("fonts.json"),
            Value::Array(fonts.into()).to_pretty(),
        )
        .map_err(error)?;
        if options.cells.iter().any(|c| c.workload == "cat64") {
            corpus::write_files(
                &settings.out.join("corpus"),
                &[corpus::find("cat64").ok_or("missing cat64")?],
            )?;
        }
        for round in 0..settings.warmups + settings.samples {
            for offset in 0..options.cells.len() {
                let cell = &options.cells[(offset + round as usize) % options.cells.len()];
                for candidate_offset in 0..options.candidates.len() {
                    let candidate = &options.candidates
                        [(candidate_offset + round as usize) % options.candidates.len()];
                    let load = environment::load()?;
                    load_guard(&options.mode, settings.max_load, load, load)?;
                    let id = format!("{round:02}-{}-{}", cell.id(), candidate.label);
                    let dir = settings.out.join(&id);
                    fs::create_dir(&dir).map_err(error)?;
                    eprintln!("frames: {id} mode={} load={load:.2}", options.mode);
                    let outcome = sample(options, candidate, cell, &dir);
                    let load_end = environment::load()?;
                    let outcome = outcome.and_then(|observed| {
                        load_guard(&options.mode, settings.max_load, load, load_end)?;
                        Ok(observed)
                    });
                    let error_message = outcome.as_ref().err().cloned();
                    let detail = object([
                        ("run", id.as_str().into()),
                        ("candidate", candidate.label.as_str().into()),
                        ("candidate_revision", candidate.revision.as_str().into()),
                        ("candidate_tree", candidate.tree.as_str().into()),
                        (
                            "candidate_path",
                            candidate.path.display().to_string().into(),
                        ),
                        ("candidate_sha256", hash(&candidate.path)?.into()),
                        ("workload", cell.workload.as_str().into()),
                        ("columns", cell.columns.into()),
                        ("output_width", cell.width.into()),
                        ("output_height", cell.height.into()),
                        ("measured", (round >= settings.warmups).into()),
                        ("load_start", load.into()),
                        ("load_end", load_end.into()),
                        (
                            "result",
                            if outcome.is_ok() { "pass" } else { "fail" }.into(),
                        ),
                        ("error", error_message.clone().into()),
                        ("observation", outcome.ok().unwrap_or(Value::Null)),
                    ]);
                    fs::write(dir.join("sample.json"), detail.to_pretty()).map_err(error)?;
                    records.push(object([
                        ("run", id.into()),
                        ("candidate", candidate.label.as_str().into()),
                        ("cell", cell.id().into()),
                        ("measured", (round >= settings.warmups).into()),
                        (
                            "result",
                            if error_message.is_none() {
                                "pass"
                            } else {
                                "fail"
                            }
                            .into(),
                        ),
                        ("sample_sha256", hash(&dir.join("sample.json"))?.into()),
                    ]));
                    if let Some(e) = error_message {
                        return Err(format!("{e}; see {}", dir.display()));
                    }
                }
            }
        }
        Ok(())
    })();
    let mut cells = Vec::new();
    for cell in &options.cells {
        for candidate in &options.candidates {
            let runs = records
                .iter()
                .filter(|r| {
                    r.get("candidate").and_then(Value::as_str) == Some(candidate.label.as_str())
                        && r.get("cell").and_then(Value::as_str) == Some(cell.id().as_str())
                        && r.get("measured").and_then(Value::as_bool) == Some(true)
                })
                .cloned()
                .collect::<Vec<_>>();
            let complete = runs.len() as u64 == settings.samples
                && runs
                    .iter()
                    .all(|r| r.get("result").and_then(Value::as_str) == Some("pass"));
            let statistics = if complete && options.mode != "check" {
                let stats = (|| -> Result<Value> {
                    let mut windows = Vec::new();
                    let mut cpu = Vec::new();
                    for run in &runs {
                        let dir = settings.out.join(
                            run.get("run")
                                .and_then(Value::as_str)
                                .ok_or("missing run path")?,
                        );
                        let sample = read(&dir.join("sample.json"))?;
                        let observed =
                            sample.get("observation").ok_or("missing run observation")?;
                        cpu.push(
                            observed
                                .get("cpu_percent_one_core")
                                .and_then(Value::as_f64)
                                .ok_or("missing process CPU statistic")?,
                        );
                        if options.mode != "control" {
                            windows.push(observation_window(&dir, observed)?);
                        }
                    }
                    let summary = Summary::of(&cpu).ok_or("missing CPU samples")?;
                    Ok(object([
                        (
                            "frame",
                            if options.mode == "control" {
                                Value::Null
                            } else {
                                diagnostics::pooled(&windows)?
                            },
                        ),
                        (
                            "process_cpu",
                            object([
                                ("unit", "percent of one core".into()),
                                ("n", (summary.n as u64).into()),
                                ("p50", summary.median.into()),
                                ("p95", summary.p95.into()),
                                ("min", summary.min.into()),
                                ("max", summary.max.into()),
                            ]),
                        ),
                    ]))
                })();
                match stats {
                    Ok(stats) => stats,
                    Err(e) => {
                        execution = Err(e);
                        Value::Null
                    }
                }
            } else {
                Value::Null
            };
            cells.push(object([
                ("cell", cell.id().into()),
                ("candidate", candidate.label.as_str().into()),
                ("complete", complete.into()),
                ("expected_samples", settings.samples.into()),
                ("runs", Value::Array(runs)),
                ("statistics", statistics),
            ]));
        }
    }
    if let Value::Object(fields) = &mut result {
        fields.extend([
            (
                "finished_at".into(),
                utc_timestamp(SystemTime::now()).into(),
            ),
            (
                "result".into(),
                if execution.is_ok() { "pass" } else { "fail" }.into(),
            ),
            ("error".into(), execution.as_ref().err().cloned().into()),
            ("cells".into(), Value::Array(cells)),
            ("runs".into(), Value::Array(records)),
        ]);
    }
    fs::write(settings.out.join("result.json"), result.to_pretty()).map_err(error)?;
    write_manifest(&settings.out)?;
    execution
}

fn write_manifest(dir: &Path) -> Result {
    fn visit(root: &Path, dir: &Path, lines: &mut Vec<String>) -> Result {
        for entry in fs::read_dir(dir).map_err(error)? {
            let entry = entry.map_err(error)?;
            if entry.file_type().map_err(error)?.is_dir() {
                visit(root, &entry.path(), lines)?;
            } else if entry.file_type().map_err(error)?.is_file()
                && entry.file_name() != "SHA256SUMS"
            {
                lines.push(format!(
                    "{}  {}",
                    hash(&entry.path())?,
                    entry.path().strip_prefix(root).map_err(error)?.display()
                ));
            }
        }
        Ok(())
    }
    let mut lines = Vec::new();
    visit(dir, dir, &mut lines)?;
    lines.sort();
    fs::write(dir.join("SHA256SUMS"), lines.join("\n") + "\n").map_err(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observation_window_retains_required_nullable_completion_revision() {
        let observed=Value::parse(r#"{"venus_pid":3,"setup_revision":7,"completion_revision":9,"begin_ns":100,"input_stop_ns":200,"end_ns":300}"#).unwrap();
        assert_eq!(
            observation_window(Path::new("/sample"), &observed)
                .unwrap()
                .completion_revision,
            Some(9)
        );
        let observed=Value::parse(r#"{"venus_pid":3,"setup_revision":7,"completion_revision":null,"begin_ns":100,"input_stop_ns":200,"end_ns":300}"#).unwrap();
        assert_eq!(
            observation_window(Path::new("/sample"), &observed)
                .unwrap()
                .completion_revision,
            None
        );
        for text in [
            r#"{"venus_pid":3,"setup_revision":7,"begin_ns":100,"input_stop_ns":200,"end_ns":300}"#,
            r#"{"venus_pid":3,"setup_revision":7,"completion_revision":"9","begin_ns":100,"input_stop_ns":200,"end_ns":300}"#,
        ] {
            assert!(
                observation_window(Path::new("/sample"), &Value::parse(text).unwrap()).is_err()
            );
        }
    }
    #[test]
    fn timed_sample_rejects_a_busy_end_even_when_admitted_quietly() {
        assert!(load_guard("measure", 8.0, 2.0, 8.1).is_err());
        assert!(load_guard("control", 8.0, 2.0, 8.1).is_err());
        assert!(load_guard("measure", 8.0, 8.1, 2.0).is_err());
        assert!(load_guard("measure", 8.0, 8.0, 8.0).is_ok());
        assert!(load_guard("check", 8.0, 12.0, 12.0).is_ok());
    }
    #[test]
    fn workspace_columns_follow_flat_cli_panes_not_the_wire_shape() {
        let tab=Value::parse(r#"{"selected_pane":"p1","selected_popup":null,"panes":[{"id":"p1","column":1,"expanded":true}]}"#).unwrap();
        assert!(verify_columns(&tab, &["p1".into()]).is_ok());
        let panes = (1..=4).map(|n| format!("p{n}")).collect::<Vec<_>>();
        let tab = object([
            ("selected_pane", "p4".into()),
            ("selected_popup", Value::Null),
            (
                "panes",
                Value::Array(
                    (1..=4)
                        .map(|n| {
                            object([
                                ("id", format!("p{n}").into()),
                                ("column", (n as u64).into()),
                                ("expanded", true.into()),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]);
        assert!(verify_columns(&tab, &panes).is_ok());
        assert!(verify_columns(&tab, &panes[..3]).is_err());
        let duplicate=Value::parse(r#"{"selected_pane":"p2","selected_popup":null,"panes":[{"id":"p1","column":1,"expanded":true},{"id":"p2","column":1,"expanded":true}]}"#).unwrap();
        assert!(verify_columns(&duplicate, &["p1".into(), "p2".into()]).is_err());
    }
    #[test]
    fn cpu_accounting_detects_pid_reuse_and_reports_one_core_percent() {
        let before = Cpu {
            start: 1,
            user: 10,
            system: 20,
        };
        assert_eq!(
            cpu_percent(
                before,
                Cpu {
                    start: 1,
                    user: 15,
                    system: 25
                },
                1_000_000_000,
                100
            )
            .unwrap(),
            10.0
        );
        assert!(
            cpu_percent(
                before,
                Cpu {
                    start: 2,
                    user: 15,
                    system: 25
                },
                1,
                100
            )
            .is_err()
        );
        assert!(
            cpu_percent(
                before,
                Cpu {
                    start: 1,
                    user: 0,
                    system: 25
                },
                1,
                100
            )
            .is_err()
        );
    }
    #[test]
    fn fixture_is_identical_and_history_must_exceed_the_full_scroll_bound() {
        assert_eq!(scroll_bound(150), 706);
        let history = object([
            ("screen", "primary".into()),
            ("rows", 1u64.into()),
            ("cut_bytes", 0u64.into()),
            ("text", "000000 line\n000001 line\n".into()),
        ]);
        assert_eq!(retained_lines(&history).unwrap(), 1);
    }
}
