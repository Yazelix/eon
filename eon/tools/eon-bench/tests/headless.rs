//! Startup evidence and ambient-session isolation need no real display.
#![cfg(target_os = "linux")]
mod support;
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::{Command, Output},
};

struct Fixture(Vec<PathBuf>);
impl Fixture {
    fn new(name: &str) -> Self {
        let id = format!("headless-{name}-{}", std::process::id());
        let mut result = Self(Vec::new());
        for path in [
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(".{id}")),
            std::env::temp_dir().join(&id),
            Path::new("/run").join(&id),
        ] {
            fs::create_dir(&path).unwrap();
            result.0.push(path);
        }
        fs::create_dir(result.0[0].join("tools")).unwrap();
        result
    }
    fn tool(&self, name: &str, body: &str) {
        let path = self.0[0].join("tools").join(name);
        fs::write(&path, format!("#!{}\n{body}\n", executable("sh").display())).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fn command(&self, suite: &str, out: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_eon-bench"));
        support::bind_fixture(&mut command, &self.0[0]);
        command
            .arg(suite)
            .arg("--out")
            .arg(out)
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    self.0[0].join("tools").display(),
                    std::env::var("PATH").unwrap()
                ),
            )
            .env("EON_BENCH_LAVAPIPE", executable("sh"));
        command.env(
            "EON_BENCH_DBUS_SESSION_CONFIG",
            fs::canonicalize(executable("sh")).unwrap(),
        );
        command
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = fs::remove_dir_all(path);
        }
    }
}
fn executable(name: &str) -> PathBuf {
    let path = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
        .unwrap();
    assert!(
        path.starts_with("/nix/store"),
        "test requires the Nix harness"
    );
    path
}
fn evidence(output: &Output, out: &Path) -> String {
    assert!(!output.status.success());
    fs::read_to_string(out.join("result.json")).unwrap_or_else(|e| {
        panic!(
            "missing result: {e}; {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn candidate_outlinks_and_run_evidence_survive_namespace_setup() {
    let fixture = Fixture::new("paths");
    fixture.tool("sway", "echo fixture-sway; exit 0");
    fixture.tool("dbus-daemon", "exit 18");
    fixture.tool("versions", "exit 0");
    let candidate = fs::canonicalize(executable("sh")).unwrap();
    symlink(&candidate, fixture.0[0].join("candidate")).unwrap();
    symlink(&candidate, fixture.0[1].join("candidate")).unwrap();
    for (name, argument, out) in [
        (
            "relative",
            PathBuf::from("candidate"),
            fixture.0[1].join("out-relative"),
        ),
        (
            "tmp",
            fixture.0[1].join("candidate"),
            fixture.0[1].join("out-tmp"),
        ),
        ("run", candidate.clone(), fixture.0[2].join("out-run")),
    ] {
        let output = fixture
            .command("headless", &out)
            .arg("--eon")
            .arg(argument)
            .current_dir(&fixture.0[0])
            .output()
            .unwrap();
        let json = evidence(&output, &out);
        assert!(
            json.contains(&format!("\"eon\": \"{}\"", candidate.display())),
            "{name}: {json}"
        );
        assert!(json.contains("D-Bus exited"), "{name}: {json}");
    }
}

#[test]
fn failed_version_probe_still_writes_the_startup_result() {
    let fixture = Fixture::new("metadata");
    fixture.tool("sway", "exit 17");
    fixture.tool("dbus-daemon", "exit 18");
    fixture.tool("versions", "exit 0");
    let out = fixture.0[1].join("out");
    let output = fixture
        .command("headless", &out)
        .arg("--eon")
        .arg(fs::canonicalize(executable("sh")).unwrap())
        .output()
        .unwrap();
    let json = evidence(&output, &out);
    assert!(json.contains("\"result\": \"fail\""), "{json}");
    assert!(json.contains("sway --version"), "{json}");
    assert!(json.contains("D-Bus exited"), "{json}");
}

#[test]
fn response_commands_clear_inherited_display_and_bus_values() {
    let fixture = Fixture::new("environment");
    fixture.tool(
        "dbus-daemon",
        r#"printf '%s\n' "$@" > "${0%/*}/bus.args"
exit 18"#,
    );
    fixture.tool("versions", "exit 0");
    fixture.tool(
        "sway",
        r#"i=0
while [ ! -f "${0%/*}/bus.args" ] && [ "$i" -lt 100 ]; do i=$((i+1)); sleep 0.01; done
exit 17"#,
    );
    fixture.tool("timeout", &format!(r#"printf 'DISPLAY=%s\nWAYLAND_DISPLAY=%s\nBUS=%s\n' "${{DISPLAY-unset}}" "${{WAYLAND_DISPLAY-unset}}" "${{DBUS_SESSION_BUS_ADDRESS-unset}}" >> "${{0%/*}}/environment.log"
exec {} "$@""#, executable("timeout").display()));
    // eon-test owns /run; geteuid has no side effects and the host runtime is never involved.
    // SAFETY: geteuid has no arguments or side effects.
    fs::create_dir_all(format!("/run/user/{}", unsafe { libc::geteuid() })).unwrap();
    let out = fixture.0[1].join("out");
    let output = fixture
        .command("response", &out)
        .arg("--eon")
        .arg(fs::canonicalize(executable("sh")).unwrap())
        .env("DISPLAY", ":parent-display")
        .env("WAYLAND_DISPLAY", "parent-wayland")
        .env("DBUS_SESSION_BUS_ADDRESS", "unix:path=/parent-bus")
        .output()
        .unwrap();
    assert!(!output.status.success());
    let captured = fs::read_to_string(fixture.0[0].join("tools/environment.log")).unwrap();
    assert!(
        captured.contains("DISPLAY=unset\nWAYLAND_DISPLAY=unset\nBUS=unix:path="),
        "{captured}"
    );
    assert!(!captured.contains("parent-"), "{captured}");
    let bus_args = fs::read_to_string(fixture.0[0].join("tools/bus.args")).unwrap();
    assert!(
        bus_args.starts_with(&format!(
            "--config-file={}\n--nofork\n--address=unix:path=",
            fs::canonicalize(executable("sh")).unwrap().display()
        )),
        "{bus_args}"
    );
    assert!(!out.join("dbus.conf").exists());
}
