//! Native workspace client delta, using the shared display launcher and estimator.
use super::*;
use crate::launcher::{BusPolicy, Headless, WindowMode};
use std::os::{
    fd::AsRawFd,
    unix::{ffi::OsStrExt, net::UnixDatagram},
};

struct Trace {
    receiver: std::sync::mpsc::Receiver<Result<String>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    ignored: Vec<u32>,
    gpu: Option<(u32, String)>,
    presented: Option<u32>,
}
impl Trace {
    fn new(socket: UnixDatagram, log: File) -> Result<Self> {
        // Keep one-shot GPU records through workspace startup bursts.
        let buffer: libc::c_int = 1024 * 1024;
        // SAFETY: this live socket and initialized scalar have the stated size.
        if unsafe {
            libc::setsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                (&buffer as *const libc::c_int).cast(),
                std::mem::size_of_val(&buffer) as libc::socklen_t,
            )
        } != 0
        {
            return Err(error(std::io::Error::last_os_error()));
        }
        socket
            .set_read_timeout(Some(Duration::from_millis(100)))
            .map_err(error)?;
        let (sender, receiver) = std::sync::mpsc::channel();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let done = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut log = std::io::BufWriter::new(log);
            let result = (|| -> Result {
                let mut bytes = [0; 4097];
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    let n = match socket.recv(&mut bytes) {
                        Ok(n) => n,
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue;
                        }
                        Err(e) => return Err(error(e)),
                    };
                    let line = std::str::from_utf8(&bytes[..n]).map_err(error)?;
                    writeln!(log, "{line}").map_err(error)?;
                    let fields: Vec<_> = line.splitn(7, '\t').collect();
                    if fields.len() == 7
                        && fields[0] == "venus"
                        && matches!(fields[5], "gpu" | "present_revision")
                    {
                        sender.send(Ok(line.to_owned())).map_err(error)?;
                    }
                }
                log.flush().map_err(error)
            })();
            if let Err(e) = result {
                let _ = sender.send(Err(e));
            }
        });
        Ok(Self {
            receiver,
            stop,
            thread: Some(thread),
            ignored: Vec::new(),
            gpu: None,
            presented: None,
        })
    }
    fn finish(mut self) -> Result {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| "trace collector panicked")?;
        }
        self.pump()
    }
    fn reset(&mut self) -> Result {
        self.pump()?;
        if let Some((pid, _)) = self.gpu.take() {
            self.ignored.push(pid);
        }
        self.presented = None;
        Ok(())
    }
    fn pump(&mut self) -> Result {
        for line in self.receiver.try_iter() {
            let line = line?;
            let fields: Vec<_> = line.splitn(7, '\t').collect();
            let pid = fields[1].parse().map_err(error)?;
            if self.ignored.contains(&pid) {
                continue;
            }
            if fields[5] == "gpu" {
                self.gpu = Some((pid, fields[6].into()));
            }
            if fields[5] == "present_revision" && !fields[6].ends_with(" None") {
                self.presented = Some(pid);
            }
        }
        Ok(())
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn validate_client_delta(before: &[Process], after: &[Process], client: &Process) -> Result {
    validate_set(before)?;
    validate_set(after)?;
    if after.len() != before.len() + 1
        || before.iter().any(|p| p.pid == client.pid)
        || !after
            .iter()
            .any(|p| p.pid == client.pid && p.start == client.start)
        || before
            .iter()
            .any(|p| !after.iter().any(|a| a.pid == p.pid && a.start == p.start))
    {
        return Err("native after set must be the unchanged before set plus exactly Venus".into());
    }
    Ok(())
}

fn connections_match(counts: &[usize], attached: bool) -> bool {
    counts.len() == COUNT
        && counts
            .iter()
            .enumerate()
            .all(|(i, &n)| n == 1 + usize::from(attached && i == COUNT - 1))
}

fn adapter(gpu: &str, software: bool) -> Result {
    if !gpu.starts_with(if software {
        "software=true "
    } else {
        "software=false "
    }) || !gpu.contains("backend: Vulkan")
    {
        return Err(format!("requested software={software}, observed {gpu}"));
    }
    Ok(())
}

fn viewport(value: &Value) -> Result {
    if value.get("rows").and_then(Value::as_u64) != Some(24)
        || value.get("columns").and_then(Value::as_u64) != Some(80)
    {
        return Err("workspace pane is not 80x24".into());
    }
    verify_screen(
        value
            .get("text")
            .and_then(Value::as_str)
            .ok_or("missing pane text")?,
    )
}

impl Session<'_> {
    fn display_call(&mut self, launch: &Headless, args: &[&str]) -> Result<String> {
        let mut command = launch.command("timeout");
        command.arg("10s").arg(&launch.eon).args(args);
        self.output(command)
    }
    fn display_wait(&self, trace: &mut Trace, mut ready: impl FnMut(&Trace) -> bool) -> Result {
        loop {
            self.tick()?;
            trace.pump()?;
            if ready(trace) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn workspace_engine(&mut self) -> Result {
        let mut found = None;
        self.wait("workspace workload registration", || {
            found = fs::read_dir(&self.dir)
                .ok()
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .find(|p| p.join("ready").is_file() && !self.terminals.contains(p));
            found.is_some()
        })?;
        let dir = found.ok_or("no workspace workload")?;
        self.wait("workspace child 80x24", || dir.join("ready").exists())?;
        let workload = Process::at(read(&dir.join("ready"))?.parse().map_err(error)?)?;
        let host = Process::at(workload.parent)?;
        let children = read(&PathBuf::from(format!(
            "/proc/{}/task/{}/children",
            host.pid, host.pid
        )))?
        .split_whitespace()
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(error)?;
        let worker = choose_worker(host.pid, workload.pid, &children)?;
        let pair = vec![host.clone(), Process::at(worker)?];
        self.required.push(workload);
        self.required.extend(pair.clone());
        self.worker_sets.push(pair);
        let args = fs::read(format!("/proc/{}/cmdline", host.pid)).map_err(error)?;
        let socket = args
            .split(|b| *b == 0)
            .nth(2)
            .ok_or("missing Orbit endpoint")?;
        std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(socket), dir.join("s"))
            .map_err(error)?;
        publish(&dir.join("go"), "")?;
        self.wait("workspace completion", || dir.join("done").exists())?;
        if read(&dir.join("done"))? != "0" {
            return Err("workspace completion failed".into());
        }
        let report = Value::parse(&read(&dir.join("report.json"))?)?;
        if report.get("bytes").and_then(Value::as_u64) != Some(payload().len() as u64) {
            return Err("workspace completion byte mismatch".into());
        }
        fs::copy(
            dir.join("report.json"),
            self.evidence
                .join(format!("terminal-{}-completion.json", self.terminals.len())),
        )
        .map_err(error)?;
        fs::copy(
            dir.join("identity"),
            self.evidence
                .join(format!("terminal-{}-identity.txt", self.terminals.len())),
        )
        .map_err(error)?;
        self.terminals.push(dir);
        Ok(())
    }
    fn workspace_connections(&self, attached: bool) -> Result {
        self.tick()?;
        if self.terminals.len() != COUNT || self.worker_sets.len() != COUNT {
            return Err("workspace must keep exactly 50 filled terminals".into());
        }
        let mut counts = Vec::new();
        for dir in &self.terminals {
            let socket = fs::canonicalize(dir.join("s")).map_err(error)?;
            counts.push(
                crate::orbit::procfs::connections(&socket)
                    .ok_or("cannot inspect workspace attachments")?,
            );
        }
        if !connections_match(&counts, attached) {
            return Err(format!("workspace attachment set differs: {counts:?}"));
        }
        if attached && !self.client.as_ref().is_some_and(Process::alive) {
            return Err("Venus exited before/after sampling".into());
        }
        fs::write(
            self.evidence.join(if attached {
                "attached-connections.txt"
            } else {
                "detached-connections.txt"
            }),
            format!("{counts:?}"),
        )
        .map_err(error)
    }
    pub(super) fn venus(&mut self) -> Result<(Memory, Memory, String)> {
        let display = self.evidence.join("display");
        fs::create_dir(&display).map_err(error)?;
        let mut launch = Headless::new(&display, &self.options.eon, WindowMode::Floating)?;
        self.options.renderer.apply(&mut launch)?;
        launch.env.insert(
            "EON_BENCH_MEMORY_ROOT".into(),
            self.dir.display().to_string(),
        );
        let trace_path = PathBuf::from(&launch.env["XDG_RUNTIME_DIR"]).join("trace");
        let socket = UnixDatagram::bind(&trace_path).map_err(error)?;
        let mut trace = Trace::new(
            socket,
            File::create(display.join("trace.tsv")).map_err(error)?,
        )?;
        launch
            .env
            .insert("EON_BENCH_TRACE".into(), trace_path.display().to_string());
        let link = self.dir.join("memory-workload");
        std::os::unix::fs::symlink(&self.exe, &link).map_err(error)?;
        let argv = Value::Array(vec![
            link.display().to_string().into(),
            self.dir.display().to_string().into(),
            self.options.deadline.as_secs().to_string().into(),
        ])
        .to_pretty();
        fs::write(display.join("config/config.toml"), format!("[shell]\ncommand = {}\n[anima]\nenabled = false\n[terminal]\ncolumns = 80\nrows = 24\nbackground_opacity = 1.0\n", argv.trim())).map_err(error)?;
        let versions = self.display_call(&launch, &["versions"])?;
        fs::write(display.join("versions.txt"), versions).map_err(error)?;
        launch.start_compositor(BusPolicy::Private)?;
        launch.start_eon()?;
        let pid = launch
            .processes
            .children
            .last()
            .ok_or("no supervisor")?
            .id();
        self.server = Some(Process::at(pid)?);
        self.display_wait(&mut trace, |t| t.gpu.is_some())?;
        let snapshot = loop {
            let text = self.display_call(&launch, &["session", "--json"])?;
            let snapshot = Value::parse(&text)?;
            if snapshot
                .get("tabs")
                .and_then(Value::as_array)
                .is_some_and(|t| !t.is_empty())
            {
                break snapshot;
            }
            self.tick()?;
            trace.pump()?;
            std::thread::sleep(Duration::from_millis(10));
        };
        let pending = snapshot
            .get("tabs")
            .and_then(Value::as_array)
            .and_then(|t| t.first())
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .ok_or("no startup tab")?;
        let first = Value::parse(
            &self.display_call(&launch, &["tab", "create", "--json", "--directory", "/tmp"])?,
        )?;
        let pane = first
            .get("created")
            .and_then(|c| c.get("pane"))
            .and_then(Value::as_str)
            .ok_or("no first pane")?
            .to_owned();
        self.display_call(&launch, &["tab", "close", pending, "--yes", "--json"])?;
        // Re-admit geometry for a pane snapshot rather than the startup Files popup.
        let initial = Process::at(trace.gpu.as_ref().ok_or("no setup GPU")?.0)?;
        if initial.parent != pid {
            return Err("initial Venus not owned by supervisor".into());
        }
        initial.signal(libc::SIGTERM);
        self.wait("startup presentation exited", || !initial.alive())?;
        trace.reset()?;
        self.display_call(&launch, &["window", "attach"])?;
        self.display_wait(&mut trace, |t| {
            t.gpu
                .as_ref()
                .is_some_and(|(pid, _)| Some(*pid) == t.presented)
        })?;
        self.workspace_engine()?;
        let mut panes = vec![pane];
        for _ in 1..COUNT {
            trace.pump()?;
            let reply = Value::parse(
                &self.display_call(&launch, &["tab", "create", "--json", "--directory", "/tmp"])?,
            )?;
            panes.push(
                reply
                    .get("created")
                    .and_then(|c| c.get("pane"))
                    .and_then(Value::as_str)
                    .ok_or("no created pane")?
                    .to_owned(),
            );
            self.workspace_engine()?;
        }
        self.display_wait(&mut trace, |t| t.presented.is_some())?;
        let setup = Process::at(trace.gpu.as_ref().ok_or("no setup GPU")?.0)?;
        if setup.parent != pid {
            return Err("setup Venus not owned by supervisor".into());
        }
        setup.signal(libc::SIGTERM);
        self.wait("setup Venus detached", || !setup.alive())?;
        self.settle()?;
        let host_exe =
            fs::read_link(format!("/proc/{}/exe", self.worker_sets[0][0].pid)).map_err(error)?;
        let hosts = self
            .owned()
            .into_iter()
            .filter(|p| {
                fs::read_link(format!("/proc/{}/exe", p.pid)).ok().as_ref() == Some(&host_exe)
            })
            .count();
        if hosts != COUNT {
            return Err(format!("expected 50 workspace engines, found {hosts}"));
        }
        for pane in &panes {
            let screen = self.display_call(&launch, &["pane", "read", pane, "--json"])?;
            viewport(&Value::parse(&screen)?)?;
            fs::write(self.evidence.join(format!("{pane}-screen.json")), screen).map_err(error)?;
        }
        self.workspace_connections(false)?;
        let set = self.terminal_set()?;
        let before = self.measure("native-before", &set)?;
        self.workspace_connections(false)?;
        trace.reset()?;
        self.display_call(&launch, &["window", "attach"])?;
        self.display_wait(&mut trace, |t| {
            t.gpu
                .as_ref()
                .is_some_and(|(pid, _)| Some(*pid) == t.presented)
        })?;
        let (pid, gpu) = trace.gpu.as_ref().ok_or("no Venus GPU")?.clone();
        adapter(&gpu, self.options.renderer.0 == "lavapipe")?;
        let client = Process::at(pid)?;
        if Some(client.parent) != self.server.as_ref().map(|p| p.pid) {
            return Err("Venus not owned by supervisor".into());
        }
        self.client = Some(client.clone());
        self.required.push(client.clone());
        self.settle()?;
        self.workspace_connections(true)?;
        let screen = self.display_call(
            &launch,
            &["pane", "read", panes.last().ok_or("no panes")?, "--json"],
        )?;
        viewport(&Value::parse(&screen)?)?;
        fs::write(self.evidence.join("attached-screen.json"), screen).map_err(error)?;
        let mut attached = self.terminal_set()?;
        attached.push(client.clone());
        validate_client_delta(&set, &attached, &client)?;
        let after = self.measure("native-after", &attached)?;
        self.workspace_connections(true)?;
        fs::write(
            display.join("adapter.json"),
            object(vec![
                ("requested", self.options.renderer.0.clone().into()),
                ("venus_gpu", gpu.clone().into()),
                (
                    "vulkan_icd",
                    launch.env.get("VK_DRIVER_FILES").cloned().into(),
                ),
                (
                    "compositor_renderer",
                    launch.env["WLR_RENDERER"].clone().into(),
                ),
                (
                    "env",
                    Value::Object(
                        launch
                            .env
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone().into()))
                            .collect(),
                    ),
                ),
            ])
            .to_pretty(),
        )
        .map_err(error)?;
        trace.pump()?;
        launch.stop_eon()?;
        trace.finish()?;
        Ok((before, after, gpu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_delta_keeps_every_server_identity_and_adds_only_venus() {
        let process = |pid| Process {
            pid,
            start: 1,
            parent: 0,
        };
        let server = process(1);
        let engines = (0..COUNT)
            .map(|i| vec![process(10 + i as u32 * 2), process(11 + i as u32 * 2)])
            .collect::<Vec<_>>();
        let before = engine_set(&server, &engines);
        let venus = process(1000);
        let mut after = before.clone();
        after.push(venus.clone());
        assert!(validate_client_delta(&before, &after, &venus).is_ok());
        let mut extra = after.clone();
        extra.push(process(1001));
        assert!(validate_client_delta(&before, &extra, &venus).is_err());
        let mut missing = after.clone();
        missing.remove(1);
        assert!(validate_client_delta(&before, &missing, &venus).is_err());
        let mut replaced = after.clone();
        replaced[1] = process(1001);
        assert!(validate_client_delta(&before, &replaced, &venus).is_err());
        let mut reused = after.clone();
        reused[1].start += 1;
        assert!(validate_client_delta(&before, &reused, &venus).is_err());
        assert!(validate_client_delta(&before, &before, &venus).is_err());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn a_final_trace_flush_error_prevents_publishing_a_sample() {
        let (receiver, sender) = UnixDatagram::pair().unwrap();
        let mut trace = Trace::new(
            receiver,
            File::options().write(true).open("/dev/full").unwrap(),
        )
        .unwrap();
        sender
            .send(b"venus\t7\t0\t1\t0\tgpu\tsoftware=true backend: Vulkan")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while trace.gpu.is_none() {
            trace.pump().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(trace.finish().is_err());
    }
    #[test]
    fn native_connections_include_runtime_metadata_observers_and_one_visible_client() {
        let mut counts = vec![1; COUNT];
        assert!(connections_match(&counts, false));
        assert!(!connections_match(&counts, true));
        counts[COUNT - 1] = 2;
        assert!(connections_match(&counts, true));
        assert!(!connections_match(&counts, false));
        counts[0] = 2;
        assert!(!connections_match(&counts, true));
        counts[0] = 0;
        assert!(!connections_match(&counts, true));
    }
    #[test]
    fn viewport_rejects_a_compositor_override_of_the_requested_grid() {
        let text = String::from_utf8(payload())
            .unwrap()
            .split("\r\n")
            .skip(9976)
            .collect::<Vec<_>>()
            .join("\n");
        let frame = |rows, cols| {
            object(vec![
                ("rows", Value::from(rows)),
                ("columns", Value::from(cols)),
                ("text", text.clone().into()),
            ])
        };
        assert!(viewport(&frame(24u64, 80u64)).is_ok());
        assert!(viewport(&frame(24u64, 92u64)).is_err());
        assert!(viewport(&frame(37u64, 80u64)).is_err());
    }
    #[test]
    fn software_fallback_and_non_vulkan_adapters_are_not_publishable_hardware() {
        assert!(adapter("software=true AdapterInfo { backend: Vulkan }", true).is_ok());
        assert!(adapter("software=false AdapterInfo { backend: Vulkan }", false).is_ok());
        assert!(adapter("software=true AdapterInfo { backend: Vulkan }", false).is_err());
        assert!(adapter("software=false AdapterInfo { backend: Metal }", false).is_err());
        assert!(adapter("", true).is_err());
    }
}
