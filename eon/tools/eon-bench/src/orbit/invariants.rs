//! Timing-free, paced output checks; uses the real Orbit and its opt-in trace.
use super::*;
use std::{
    fs,
    os::unix::net::UnixDatagram,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
};

type Result<T = ()> = std::result::Result<T, String>;
const FRAME_BOUND: u64 = 128 * 1024;
// Budget data + query turns per 2 KiB chunk, with 4x slack for PTY fragmentation.
const FRAMES_PER_MIB: u64 = 4096;

/// Child of the existing `child` handshake. A reply completes each chunk;
/// the harness drains its frames before acknowledging the next chunk.
#[doc(hidden)]
pub fn writer(dir: &Path) -> Result {
    for index in 0.. {
        let file = dir.join(format!("chunk-{index}.vt"));
        if !file.exists() {
            break;
        }
        crate::writer::run(&file, &dir.join("report.json"))?;
        publish(&dir.join(format!("done-{index}")), "")?;
        let start = Instant::now();
        while !dir.join(format!("ack-{index}")).exists() {
            if start.elapsed() > SETUP_WAIT {
                return Err("frame drain acknowledgment timed out".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Ok(())
}

struct Trace {
    receiver: Receiver<std::io::Result<Vec<u8>>>,
    events: Vec<(String, u64)>,
    frames: Vec<u64>,
    replies: usize,
    ended: bool,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}
impl Trace {
    fn start(path: &Path) -> Result<Self> {
        let socket = UnixDatagram::bind(path).map_err(|e| e.to_string())?;
        socket
            .set_read_timeout(Some(Duration::from_millis(20)))
            .map_err(|e| e.to_string())?;
        let (sender, receiver) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while !stopping.load(Ordering::Relaxed) {
                let packet = match socket.recv(&mut buffer) {
                    Ok(n) => Ok(buffer[..n].to_vec()),
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(e) => Err(e),
                };
                let failed = packet.is_err();
                if sender.send(packet).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            receiver,
            events: Vec::new(),
            frames: Vec::new(),
            replies: 0,
            ended: false,
            stop,
            handle: Some(handle),
        })
    }
    fn drain(&mut self, pid: u32) -> Result {
        for packet in self.receiver.try_iter() {
            let packet = packet.map_err(|e| e.to_string())?;
            let line = std::str::from_utf8(&packet).map_err(|e| e.to_string())?;
            let f = line.splitn(7, '\t').collect::<Vec<_>>();
            if f.len() != 7
                || f[0] != "orbit"
                || f[1].parse::<u32>() != Ok(pid)
                || f[2].parse::<usize>() != Ok(self.events.len())
                || f[4] != "0"
            {
                return Err(format!("incomplete/lost trace: {line}"));
            }
            let detail = if matches!(f[5], "frame" | "pty_read" | "pty_write") {
                let value = f[6]
                    .parse::<u64>()
                    .map_err(|_| format!("invalid trace count: {line}"))?;
                if f[5] != "frame" && value == 0 {
                    return Err(format!("empty PTY operation: {line}"));
                }
                value
            } else {
                0
            };
            match f[5] {
                "frame" => self.frames.push(detail),
                "pty_write" => self.replies += 1,
                "end" => self.ended = true,
                _ => (),
            }
            self.events.push((f[5].to_owned(), detail));
        }
        Ok(())
    }
    fn complete(&mut self, pid: u32) -> Result<Vec<(String, u64)>> {
        self.drain(pid)?;
        if self.events.first().map(|e| e.0.as_str()) != Some("start")
            || self.events.last().map(|e| e.0.as_str()) != Some("end")
        {
            return Err("trace start/end missing".into());
        }
        Ok(std::mem::take(&mut self.events))
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Run one whole-unit quick prefix per corpus. The expected text is a fixed
/// frozen regression capture from the accepted Orbit, not a VT reference model.
#[doc(hidden)]
pub fn check(exe: &Path, orbit: &Path, expected: &[(&str, &str)]) -> Result {
    for (session, corpus) in corpus::CORPORA.iter().enumerate() {
        let dir = run_dir(session as u64)?;
        let root = &dir.0;
        let chunks = corpus.quick_chunks();
        for (i, chunk) in chunks.iter().enumerate() {
            fs::write(root.join(format!("chunk-{i}.vt")), chunk).map_err(|e| e.to_string())?;
        }
        let trace_path = root.join("trace");
        let mut trace = Trace::start(&trace_path)?;
        let log = root.join("orbit.log");
        let log_file = fs::File::create(&log).map_err(|e| e.to_string())?;
        let mut command = Command::new(orbit);
        command
            .arg("serve")
            .arg(root.join("s"))
            .arg("--")
            .arg(exe)
            .arg("child")
            .arg(root)
            .args(["--rows", "24", "--cols", "80", "--"])
            .arg(exe)
            .arg("gate-writer")
            .arg(root)
            .env("EON_BENCH_TRACE", &trace_path)
            .env("EON_BENCH_TRACE_BLOCKING", "1")
            .stdin(Stdio::null())
            .stdout(log_file.try_clone().map_err(|e| e.to_string())?)
            .stderr(log_file);
        let child = environment::die_with_parent(&mut command)
            .spawn()
            .map_err(|e| e.to_string())?;
        let pid = child.id();
        let mut server = Server { child, log };
        server.wait_for(SETUP_WAIT, "socket", || root.join("s").exists())?;
        let mut client =
            Client::new(UnixStream::connect(root.join("s")).map_err(|e| e.to_string())?)?;
        // A different first grid prevents the initial attachment frame from
        // masquerading as the resize completion when Orbit starts at 80x24.
        client.attach(25, 81)?;
        client.send(&ClientMessage::Resize(SurfaceSize {
            cols: 80,
            rows: 24,
            screen_width: 800,
            screen_height: 480,
            cell_width: 10,
            cell_height: 20,
            padding_top: 0,
            padding_bottom: 0,
            padding_left: 0,
            padding_right: 0,
        }))?;
        let resize_wait = Instant::now();
        let initial = loop {
            let message = next_message(&mut client.reader, &mut || {
                if resize_wait.elapsed() > SETUP_WAIT {
                    return Err("no workload grid".into());
                }
                server.alive()
            })?
            .ok_or("Orbit closed before workload grid")?;
            if let (ServerMessage::Frame(frame), _) = message
                && (frame.dimensions.rows, frame.dimensions.cols) == (24, 80)
            {
                break frame;
            }
        };
        let first = initial.revision;
        // Own the messages in this thread: retaining the final frame costs no clone.
        let delivered = Arc::new(Mutex::new(Vec::<(u64, u64)>::new()));
        let shared = delivered.clone();
        let stream = client.stream.try_clone().map_err(|e| e.to_string())?;
        let reader = std::thread::spawn(move || -> Result<Box<orbit_protocol::Frame>> {
            let mut last = initial;
            while let Some((message, bytes)) = next_message(&mut client.reader, &mut || Ok(()))? {
                match message {
                    ServerMessage::Frame(frame) => {
                        if (frame.dimensions.rows, frame.dimensions.cols) != (24, 80) {
                            return Err("workload frame changed grid".into());
                        }
                        shared.lock().unwrap().push((frame.revision, bytes));
                        last = frame;
                    }
                    ServerMessage::Exited { code } if code != 0 => {
                        return Err(format!("Orbit child exited {code}"));
                    }
                    _ => (),
                }
            }
            Ok(last)
        });
        // Ensure an early error also ends the reader instead of leaving a blocked thread.
        struct Close(UnixStream);
        impl Drop for Close {
            fn drop(&mut self) {
                let _ = self.0.shutdown(Shutdown::Both);
            }
        }
        let close = Close(stream);
        server.wait_for(SETUP_WAIT, "ready", || root.join("ready").exists())?;
        publish(&root.join("go"), "")?;
        for index in 0..chunks.len() {
            server.wait_for(Duration::from_secs(60), "chunk reply", || {
                root.join(format!("done-{index}")).exists()
            })?;
            // The host traces the final frame before writing the status reply.
            // Wait for that trace and the matching wire revision, not a sleep.
            let wait = Instant::now();
            loop {
                trace.drain(pid)?;
                if trace.replies > index
                    && trace.frames.last().is_some_and(|last| {
                        delivered.lock().unwrap().last().map_or(first, |f| f.0) == *last
                    })
                {
                    break;
                }
                server.alive()?;
                if wait.elapsed() > SETUP_WAIT {
                    return Err("trace and frame drain timed out".into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            publish(&root.join(format!("ack-{index}")), "")?;
        }
        server.wait_for(SETUP_WAIT, "writer completion", || {
            root.join("done").exists()
        })?;
        if read_number::<u64>(&root.join("done"))? != 0 {
            return Err(format!("{} writer failed{}", corpus.id, server.log_tail()));
        }
        publish(&root.join("release"), "")?;
        let end_wait = Instant::now();
        loop {
            trace.drain(pid)?;
            if trace.ended {
                break;
            }
            if end_wait.elapsed() > SETUP_WAIT {
                return Err("trace end missing".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let status = server.child.wait().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("Orbit exited {status}"));
        }
        let frame = reader.join().map_err(|_| "frame reader panicked")??;
        drop(close);
        let events = trace.complete(pid)?;
        let emitted = trace
            .frames
            .iter()
            .copied()
            .filter(|r| *r > first)
            .collect::<Vec<_>>();
        let received = delivered.lock().unwrap();
        let revisions = received.iter().map(|f| f.0).collect::<Vec<_>>();
        if emitted != revisions
            || emitted.is_empty()
            || emitted[0] != first + 1
            || emitted.windows(2).any(|p| p[1] != p[0] + 1)
        {
            return Err(format!(
                "{}: emitted/delivered revisions differ or repeat: {emitted:?} / {revisions:?}",
                corpus.id
            ));
        }
        let reads = events.iter().filter(|e| e.0 == "pty_read").count() as u64;
        let frames = received.len() as u64;
        let bytes = chunks.iter().map(Vec::len).sum::<usize>() as u64;
        if frames > reads {
            return Err(format!(
                "{}: {frames} frames exceed {reads} PTY turns",
                corpus.id
            ));
        }
        if received.iter().any(|f| f.1 > FRAME_BOUND) {
            return Err(format!(
                "{}: a frame exceeds {FRAME_BOUND} bytes",
                corpus.id
            ));
        }
        if frames * (1 << 20) > bytes * FRAMES_PER_MIB + 4 * (1 << 20) {
            return Err(format!(
                "{}: {frames} frames / {bytes} bytes exceed {FRAMES_PER_MIB} frames/MiB + 4",
                corpus.id
            ));
        }
        let screen = frame
            .rows
            .iter()
            .map(|row| {
                row.cells
                    .iter()
                    .filter(|c| {
                        !matches!(
                            c.width,
                            orbit_protocol::CellWidth::SpacerTail
                                | orbit_protocol::CellWidth::SpacerHead
                        )
                    })
                    .map(|c| {
                        if c.text.is_empty() {
                            " "
                        } else {
                            c.text.as_str()
                        }
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end_matches('\n')
            .to_owned()
            + "\n";
        let oracle = expected
            .iter()
            .find(|e| e.0 == corpus.id)
            .ok_or("missing screen oracle")?
            .1;
        if screen != oracle {
            return Err(format!(
                "{}: screen differs from oracle\nACTUAL-BEGIN\n{screen}ACTUAL-END",
                corpus.id
            ));
        }
        eprintln!(
            "{}: {bytes} bytes, {reads} PTY turns, {frames} frames, max {} bytes/frame, oracle matched",
            corpus.id,
            received.iter().map(|f| f.1).max().unwrap_or(0)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_burst_survives_a_stalled_consumer() {
        let dir = run_dir(900).unwrap();
        let path = dir.0.join("burst");
        let mut trace = Trace::start(&path).unwrap();
        let producer = std::thread::spawn(move || {
            let socket = UnixDatagram::unbound().unwrap();
            socket.connect(path).unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(60)))
                .unwrap();
            for sequence in 0..=257 {
                let stage = match sequence {
                    0 => "start",
                    257 => "end",
                    _ => "pty_read",
                };
                socket
                    .send(format!("orbit\t42\t{sequence}\t0\t0\t{stage}\t1").as_bytes())
                    .unwrap();
            }
        });
        // Consume nothing until the producer has sent far beyond the namespace's queue of 10.
        producer.join().unwrap();
        let wait = Instant::now();
        loop {
            trace.drain(42).unwrap();
            if trace.ended {
                break;
            }
            assert!(
                wait.elapsed() < SETUP_WAIT,
                "trace collector did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(trace.complete(42).unwrap().len(), 258);
    }
}
