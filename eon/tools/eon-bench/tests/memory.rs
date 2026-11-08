//! A daemon-like grandchild must not outlive an interrupted memory suite.
#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::ExitStatusExt},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn namespace_present(namespace: &str) -> bool {
    fs::read_dir("/proc").unwrap().flatten().any(|entry| {
        fs::read_link(entry.path().join("ns/pid"))
            .is_ok_and(|path| path.to_string_lossy() == namespace)
    })
}

#[test]
fn signals_and_deadlines_destroy_the_private_namespace_and_leave_no_result() {
    let root = std::env::temp_dir().join(format!("memory-stop-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let sh = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("sh"))
        .find(|p| p.is_file())
        .unwrap();
    let fake = root.join("tmux");
    // The grandchild has no parent-death guard. Namespace teardown must take it.
    fs::write(
        &fake,
        format!(
            "#!{}\nreadlink /proc/self/ns/pid\nsleep 600 &\nwait\n",
            sh.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
    for (round, signal) in [Some(libc::SIGTERM), Some(libc::SIGKILL), None]
        .into_iter()
        .enumerate()
    {
        let out = root.join(round.to_string());
        fs::create_dir(&out).unwrap();
        fs::write(out.join("result.json"), "stale").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_eon-bench"))
            .args(["memory", "--subject", "tmux", "--tmux"])
            .arg(&fake)
            .args([
                "--samples",
                "1",
                "--warmups",
                "0",
                "--max-load",
                "1000",
                "--deadline",
                if signal.is_some() { "30" } else { "1" },
                "--out",
            ])
            .arg(&out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let log = out.join("evidence/001-tmux/process-1.log");
        let start = Instant::now();
        let namespace = loop {
            if let Ok(text) = fs::read_to_string(&log)
                && text.trim().starts_with("pid:[")
            {
                break text.trim().to_owned();
            }
            if let Some(status) = child.try_wait().unwrap() {
                let output = child.wait_with_output().unwrap();
                panic!(
                    "memory exited before fixture: {status}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "fixture never started"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(namespace_present(&namespace));
        if let Some(signal) = signal {
            // SAFETY: this exact child is still unreaped, so its PID cannot be reused.
            assert_eq!(unsafe { libc::kill(child.id() as i32, signal) }, 0);
        }
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "memory did not stop"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        if let Some(signal) = signal {
            assert_eq!(status.signal(), Some(signal));
        } else {
            assert!(!status.success());
        }
        let start = Instant::now();
        while namespace_present(&namespace) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "namespace survived the harness"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!out.join("result.json").exists());
    }
    fs::remove_dir_all(root).unwrap();
}
