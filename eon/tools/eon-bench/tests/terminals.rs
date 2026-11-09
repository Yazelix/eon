//! A failed display startup must leave an honest failed comparison, not a zero timing.
#![cfg(target_os = "linux")]
mod support;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::current_dir()
            .unwrap()
            .join(format!(".terminals-{name}-{}-{nonce}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn executable(name: &str) -> PathBuf {
    let path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join(name))
        .find(|p| p.is_file())
        .unwrap();
    assert!(
        path.starts_with("/nix/store"),
        "requires eon-test's Nix tools"
    );
    path
}

fn comparison(root: &Path, tools: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_eon-bench"));
    support::bind_fixture(&mut command, root);
    command
        .args([
            "terminals",
            "--source-tree",
            "0000000000000000000000000000000000000000",
            "--subject",
            "foot",
            "--only",
            "cat64",
            "--samples",
            "1",
            "--warmups",
            "0",
            "--max-load",
            "10000",
            "--eon",
        ])
        .arg(executable("true"))
        .arg("--out")
        .arg(root.join("out"))
        .env(
            "PATH",
            format!("{}:{}", tools.display(), std::env::var("PATH").unwrap()),
        )
        .env("EON_BENCH_FOOT", executable("true"))
        .env("EON_BENCH_KITTEN", executable("true"))
        .env("EON_BENCH_FONT", executable("true"))
        .env("EON_BENCH_FONTCONFIG", executable("true"))
        .env("EON_BENCH_PRODUCT_FONTCONFIG", executable("true"))
        .env("EON_BENCH_LAVAPIPE", executable("true"))
        .env("EON_BENCH_EGL", executable("true"))
        .env("EON_BENCH_DRI", tools)
        .env("EON_BENCH_GBM", tools);
    command
}

#[test]
fn failed_subject_is_retained_without_a_timing_and_out_is_never_reused() {
    let fixture = Fixture::new("failed");
    let root = &fixture.0;
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    for (name, body) in [("dbus-daemon", "exit 18"), ("sway", "echo fixture-sway")] {
        let path = tools.join(name);
        fs::write(&path, format!("#!{}\n{body}\n", executable("sh").display())).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let out = root.join("out");
    let mut command = comparison(root, &tools);
    let output = command.output().unwrap();
    let result = fs::read_to_string(out.join("result.json"))
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stderr)));
    assert!(!output.status.success());
    assert!(result.contains("D-Bus exited"), "{result}");
    assert!(result.contains("\"result\": \"fail\""), "{result}");
    assert!(result.contains("\"samples\": []"), "{result}");
    let repeated = Command::new(env!("CARGO_BIN_EXE_eon-bench"))
        .args([
            "terminals",
            "--subject",
            "foot",
            "--source-tree",
            "0000000000000000000000000000000000000000",
            "--eon",
        ])
        .arg(executable("true"))
        .arg("--out")
        .arg(&out)
        .output()
        .unwrap();
    assert!(!repeated.status.success());
    assert_eq!(fs::read_to_string(out.join("result.json")).unwrap(), result);
}

#[test]
fn compositor_waits_for_ipc_replies_after_its_socket_files_appear() {
    let fixture = Fixture::new("ipc-ready");
    let root = &fixture.0;
    let tools = root.join("tools");
    fs::create_dir(&tools).unwrap();
    for (name, body) in [
        (
            "sway",
            r#"touch "$XDG_RUNTIME_DIR/wayland-fixture" "$XDG_RUNTIME_DIR/sway-ipc.fixture.sock"
sleep 30"#,
        ),
        (
            "swaymsg",
            r#"n=0
if test -f "$HOME/ipc-attempts"; then read -r n < "$HOME/ipc-attempts"; fi
n=$((n + 1))
echo "$n" > "$HOME/ipc-attempts"
if test "$n" -lt 3; then exit 7; fi
echo '[]'"#,
        ),
    ] {
        let path = tools.join(name);
        fs::write(&path, format!("#!{}\n{body}\n", executable("sh").display())).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let output = comparison(root, &tools).output().unwrap();
    let result = fs::read_to_string(root.join("out/result.json"))
        .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&output.stderr)));
    assert!(!output.status.success());
    assert!(
        result.contains("private compositor is not one 1280x800"),
        "{result}"
    );
    let attempts: u32 = fs::read_to_string(root.join("out/00-cat64-foot/home/ipc-attempts"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        attempts >= 4,
        "metadata query must follow successful IPC readiness"
    );
}

#[test]
fn a_grid_change_is_reported_as_failure_without_waiting_for_the_sample_deadline() {
    let fixture = Fixture::new("resize");
    let root = &fixture.0;
    fs::write(root.join("go"), "").unwrap();
    let (_master, slave) = eon_bench::pty::open(24, 80).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_eon-bench"))
        .arg("child")
        .arg(root)
        .args(["--rows", "24", "--cols", "80", "--grid-report"])
        .arg(root.join("grid"))
        .args(["--release-wait", "0", "--"])
        .arg(executable("stty"))
        .args(["rows", "25", "cols", "80"])
        .stdin(slave)
        .output()
        .unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(
        fs::read_to_string(root.join("grid.go")).unwrap(),
        "Some((24, 80))"
    );
    assert_eq!(
        fs::read_to_string(root.join("grid.after")).unwrap(),
        "Some((25, 80))"
    );
    let done = fs::read_to_string(root.join("done"));
    assert_eq!(done.unwrap(), "1");
}
