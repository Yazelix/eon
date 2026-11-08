//! The orbit suite end to end against the workspace's Orbit (`ORBIT_BIN`,
//! which eon-test sets): a vtebench-style script generated at the grid, its
//! untimed setup, the handshake, detach, the child's hold, the writer and the
//! wire counters.

use std::process::Command;

/// A one-line benchmark that prints the grid its terminal reports, repeated
/// to 64 KiB so every turn scrolls and changes the screen, after a setup,
/// completes in both modes; only the attached cell has frames, and its
/// revisions reach the client.
#[test]
fn orbit_runs_a_vtebench_sample_detached_and_attached() {
    let root = std::env::temp_dir().join(format!("eon-bench-orbit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let benchmark = root.join("benchmarks/light_cells");
    std::fs::create_dir_all(&benchmark).unwrap();
    std::fs::write(
        benchmark.join("benchmark"),
        "printf '%s\\r\\n' \"$(stty size)\"\n",
    )
    .unwrap();
    std::fs::write(benchmark.join("setup"), "printf 'setup\\r\\n'\n").unwrap();
    let out = root.join("out");
    let run = Command::new(env!("CARGO_BIN_EXE_eon-bench"))
        .arg("orbit")
        .arg("--orbit")
        .arg(std::env::var_os("ORBIT_BIN").expect("ORBIT_BIN"))
        .arg("--vtebench")
        .arg(root.join("benchmarks"))
        .args(["--only", "vtebench-light_cells", "--grid", "30x100"])
        .args(["--vtebench-bytes", "65536"])
        .args(["--samples", "1", "--warmups", "0", "--max-load", "1000"])
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{}\n{stdout}{}",
        run.status,
        String::from_utf8_lossy(&run.stderr)
    );
    let setup = std::fs::read(out.join("vtebench-30x100/light_cells.setup.vt")).unwrap();
    assert_eq!(setup, b"\x1bcsetup\r\n");
    let sample = std::fs::read(out.join("vtebench-30x100/light_cells.vt")).unwrap();
    assert!(
        sample.starts_with(b"30 100\r\n"),
        "{:?}",
        &sample[..sample.len().min(64)]
    );
    assert!(sample.len() >= 65536);
    let result = std::fs::read_to_string(out.join("result.json")).unwrap();
    for layer in ["orbit-detached", "orbit-attached"] {
        assert!(
            result.contains(&format!("\"layer\": \"{layer}\"")),
            "{result}"
        );
    }
    // Only the attached sample's counters hold frames. This small fixture's
    // client keeps up, so every revision after the go arrives once, in
    // order, and at most the query's own frame follows the reply; the
    // setup's frames are read but not counted.
    let attached = &result[result.find("\"layer\": \"orbit-attached\"").unwrap()..];
    let detached = &result[..result.find("\"layer\": \"orbit-attached\"").unwrap()];
    assert!(!detached.contains("\"frames\""), "{result}");
    let counter = |name: &str| -> u64 {
        let at = attached.find(&format!("\"{name}\": ")).unwrap() + name.len() + 4;
        let digits: String = attached[at..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().unwrap()
    };
    let frames = counter("frames");
    assert!(frames > 1, "{result}");
    assert_eq!(counter("revision_gaps"), 0, "{result}");
    assert_eq!(counter("revision_repeats"), 0, "{result}");
    assert_eq!(
        counter("last_revision") - counter("first_revision") + 1,
        frames,
        "{result}"
    );
    assert!(counter("frames_after_reply") <= 1, "{result}");
    assert!(counter("setup_frames") >= 1, "{result}");
    assert!(
        counter("first_revision") > counter("setup_last_revision"),
        "{result}"
    );
    if cfg!(target_os = "linux") {
        assert!(attached.contains("\"host_cpu_us\": "), "{result}");
    }
    assert!(stdout.contains("\nvtebench-light_cells "), "{stdout}");
    std::fs::remove_dir_all(&root).unwrap();
}

/// Stopping the harness by a signal (Linux, where Orbit gets a parent-death
/// signal).
#[cfg(target_os = "linux")]
mod stopped {
    use std::{
        os::unix::{fs::PermissionsExt, process::ExitStatusExt},
        path::PathBuf,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    /// A harness stopped by a signal in the middle of a sample leaves nothing
    /// running: after SIGTERM no Orbit host, no `eon-bench child`, no
    /// workload and no session directory survive, no result is written, and
    /// the harness ends by SIGTERM; after SIGKILL, Orbit's parent-death
    /// signal still takes the host, the child and the workload with it.
    /// Before the first sample, SIGTERM ends the harness at once, even while
    /// it waits on a preparation step (here kitten's version), and the step
    /// dies with it.
    #[test]
    fn a_stopped_harness_leaves_no_orbit_running() {
        let root = std::env::temp_dir().join(format!("eon-bench-stop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let sh = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|dir| dir.join("sh"))
            .find(|path| path.is_file())
            .expect("sh on PATH");
        let rounds = [
            (libc::SIGTERM, false),
            (libc::SIGKILL, false),
            (libc::SIGTERM, true),
        ];
        for (round, (signal, in_preparation)) in rounds.into_iter().enumerate() {
            // A kitten that records its PID and hangs, in its benchmark or,
            // in preparation, already when asked its version.
            let mark = root.join(format!("mark-{round}"));
            let kitten = root.join(format!("kitten-{round}"));
            let hang = format!(
                "echo $$ > {mark}.partial && mv {mark}.partial {mark}; exec sleep 600",
                mark = mark.display()
            );
            let version = if in_preparation {
                hang.clone()
            } else {
                "echo 'kitten 0'".to_owned()
            };
            std::fs::write(
                &kitten,
                format!(
                    "#!{}\ncase \"$1\" in --version) {version} ;; *) {hang} ;; esac\n",
                    sh.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&kitten, std::fs::Permissions::from_mode(0o755)).unwrap();
            let mut harness = Command::new(env!("CARGO_BIN_EXE_eon-bench"))
                .arg("orbit")
                .arg("--orbit")
                .arg(std::env::var_os("ORBIT_BIN").expect("ORBIT_BIN"))
                .arg("--kitten")
                .arg(&kitten)
                .args(["--only", "kitten-ascii", "--samples", "1", "--warmups", "0"])
                .args(["--max-load", "1000"])
                .arg("--out")
                .arg(root.join(format!("out-{round}")))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let pid = harness.id();
            let workload: u32 = wait_until(Duration::from_secs(60), || {
                std::fs::read_to_string(&mark).ok()?.trim().parse().ok()
            })
            .expect("the hanging kitten started");
            if in_preparation {
                // SAFETY: kill has no memory preconditions; pid is our own child.
                assert_eq!(unsafe { libc::kill(pid as i32, signal) }, 0);
                let status = wait_until(Duration::from_secs(5), || harness.try_wait().unwrap())
                    .expect("the harness ended");
                assert_eq!(status.signal(), Some(signal), "{status}");
                assert_eq!(sessions(pid), Vec::<PathBuf>::new());
                // The version query dies with the harness.
                assert!(
                    wait_until(Duration::from_secs(10), || (!alive(workload)).then_some(()))
                        .is_some(),
                    "the version query {workload} survived the harness"
                );
                continue;
            }
            let (_, child) = stat(workload).expect("the workload's parent");
            let command = std::fs::read(format!("/proc/{child}/cmdline")).unwrap();
            assert!(
                command
                    .split(|&byte| byte == 0)
                    .any(|word| word == b"child"),
                "the workload's parent is eon-bench child"
            );
            let host = children(pid);
            assert_eq!(host.len(), 1, "the harness's one child is Orbit: {host:?}");
            assert!(!sessions(pid).is_empty());
            // SAFETY: kill has no memory preconditions; pid is our own child.
            assert_eq!(unsafe { libc::kill(pid as i32, signal) }, 0);
            let status = wait_until(Duration::from_secs(20), || harness.try_wait().unwrap())
                .expect("the harness ended");
            assert_eq!(status.signal(), Some(signal), "{status}");
            for gone in [host[0], child, workload] {
                assert!(
                    wait_until(Duration::from_secs(10), || (!alive(gone)).then_some(())).is_some(),
                    "process {gone} survived signal {signal}"
                );
            }
            if signal == libc::SIGTERM {
                assert_eq!(sessions(pid), Vec::<PathBuf>::new());
                assert!(!root.join(format!("out-{round}/result.json")).exists());
            }
            for dir in sessions(pid) {
                std::fs::remove_dir_all(dir).unwrap();
            }
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn wait_until<T>(limit: Duration, mut ready: impl FnMut() -> Option<T>) -> Option<T> {
        let start = Instant::now();
        loop {
            if let Some(value) = ready() {
                return Some(value);
            }
            if start.elapsed() >= limit {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The PIDs whose parent is `pid`.
    fn children(pid: u32) -> Vec<u32> {
        std::fs::read_dir("/proc")
            .unwrap()
            .flatten()
            .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
            .filter(|&child| stat(child).is_some_and(|(_, parent)| parent == pid))
            .collect()
    }

    /// A process that exists and is not a zombie.
    fn alive(pid: u32) -> bool {
        stat(pid).is_some_and(|(state, _)| state != 'Z' && state != 'X')
    }

    /// A process's state and parent from `/proc/PID/stat`.
    fn stat(pid: u32) -> Option<(char, u32)> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let mut fields = stat.get(stat.rfind(')')? + 2..)?.split(' ');
        let state = fields.next()?.chars().next()?;
        Some((state, fields.next()?.parse().ok()?))
    }

    /// The harness's session directories, `eb<PID>-<N>` in the temp directory.
    fn sessions(pid: u32) -> Vec<PathBuf> {
        let prefix = format!("eb{pid}-");
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&prefix))
            })
            .collect()
    }
}
