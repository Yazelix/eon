//! Namespace containment shared by memory and display suites.
use crate::{environment, suite};
use std::{
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

pub(crate) fn command(out: &Path, tools: &[&Path]) -> Command {
    let mut command =
        Command::new(std::env::var_os("EON_BENCH_BWRAP").unwrap_or_else(|| "bwrap".into()));
    command.args([
        "--die-with-parent",
        "--unshare-pid",
        "--unshare-net",
        "--ro-bind",
        "/",
        "/",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--tmpfs",
        "/run",
        "--unsetenv",
        "DISPLAY",
        "--unsetenv",
        "WAYLAND_DISPLAY",
        "--unsetenv",
        "DBUS_SESSION_BUS_ADDRESS",
        "--unsetenv",
        "SWAYSOCK",
        "--chdir",
        "/",
    ]);
    command.arg("--bind").arg(out).arg(out);
    for tool in tools {
        if !tool.as_os_str().is_empty() {
            command.arg("--ro-bind").arg(tool).arg(tool);
        }
    }
    command
}

pub(crate) fn display(command: &mut Command, render_node: Option<&Path>) {
    command
        .arg("--dir")
        // SAFETY: geteuid has no arguments or side effects.
        .arg(format!("/run/user/{}", unsafe { libc::geteuid() }));
    if let Some(node) = render_node {
        command.arg("--dev-bind").arg(node).arg(node);
    }
}

pub(crate) fn run(command: &mut Command) -> Result<(), String> {
    command.stdin(Stdio::null());
    suite::interrupt::install()?;
    let mut child = environment::die_with_parent(command)
        .spawn()
        .map_err(|e| e.to_string())?;
    loop {
        if suite::interrupt::check().is_err() {
            let _ = child.kill();
            let _ = child.wait();
            break Err("isolated run interrupted".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break if status.success() {
                Ok(())
            } else {
                Err(format!("isolated run: {status}"))
            };
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn display_namespace_hides_runtime_and_tmp_and_keeps_host_files_read_only() {
        let root = std::env::current_dir()
            .unwrap()
            .join(format!(".isolation-test-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let runtime = Path::new("/run").join(format!("isolation-test-{}", std::process::id()));
        fs::create_dir(&runtime).unwrap();
        let hidden = runtime.join("ambient.sock");
        let listener = std::os::unix::net::UnixListener::bind(&hidden).unwrap();
        let tmp = std::env::temp_dir().join(format!("isolation-test-{}", std::process::id()));
        fs::create_dir(&tmp).unwrap();
        let hidden_tmp = tmp.join("ambient.sock");
        let tmp_listener = std::os::unix::net::UnixListener::bind(&hidden_tmp).unwrap();
        let out = runtime.join("out");
        fs::create_dir(&out).unwrap();
        let victim = root.join("host-file");
        fs::write(&victim, "unchanged").unwrap();
        let sh = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|dir| dir.join("sh"))
            .find(|p| p.is_file())
            .unwrap();
        let mut isolated = command(&out, &[]);
        display(&mut isolated, None);
        let output = isolated
            .arg("--")
            .arg(sh)
            .args([
                "-c",
                r#"
            set -eu
            test ! -e "$1"
            test ! -e "$4"
            test -f "$2"
            if (echo changed > "$2") 2>/dev/null; then exit 1; fi
            test -z "${DISPLAY+x}${WAYLAND_DISPLAY+x}${DBUS_SESSION_BUS_ADDRESS+x}"
            readlink /proc/self/ns/pid > "$3/pid-namespace"
            readlink /proc/self/ns/net > "$3/net-namespace"
        "#,
                "sh",
            ])
            .arg(&hidden)
            .arg(&victim)
            .arg(&out)
            .arg(&hidden_tmp)
            .env("DISPLAY", ":parent-display")
            .env("WAYLAND_DISPLAY", "parent-wayland")
            .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/parent-bus")
            .output()
            .unwrap();
        let contents = fs::read_to_string(&victim).unwrap();
        let pid = fs::read_to_string(out.join("pid-namespace"));
        let net = fs::read_to_string(out.join("net-namespace"));
        drop(listener);
        drop(tmp_listener);
        fs::remove_dir_all(tmp).unwrap();
        fs::remove_dir_all(runtime).unwrap();
        fs::remove_dir_all(root).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(contents, "unchanged");
        for (observed, namespace) in [(pid, "pid"), (net, "net")] {
            assert_ne!(
                observed.unwrap().trim(),
                fs::read_link(format!("/proc/self/ns/{namespace}"))
                    .unwrap()
                    .to_str()
                    .unwrap()
            );
        }
    }
}
