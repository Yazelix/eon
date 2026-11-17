#![cfg(target_os = "linux")]

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

#[test]
fn shipped_example_commands_preserve_backend_permissions_and_tui_isolation() {
    use rota::config::{Config, Level, Role};

    let example = include_str!("../config.example.toml");
    let original = Config::parse(example, "rota").unwrap();
    let root = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(root.path()).unwrap();
    fs::write(root.join("brief"), "example task").unwrap();
    let mut fixture: toml::Value = example.parse().unwrap();
    let mut backends = fixture["backends"].as_array().unwrap().clone();
    for backend in &mut backends {
        let backend = backend.as_table_mut().unwrap();
        // Keep the shipped work argv, replacing only admission and environment fixtures.
        backend.retain(|key, _| matches!(key, "name" | "commands" | "popup_command" | "run"));
        backend.insert("roles".into(), vec!["implement"].into());
        backend.insert("guard".into(), vec!["true"].into());
        backend.insert("allow_unknown_quota".into(), true.into());
    }
    fixture = toml::Value::Table(toml::map::Map::from_iter([
        (
            "state_file".into(),
            root.join("state").to_str().unwrap().into(),
        ),
        ("log_file".into(), root.join("log").to_str().unwrap().into()),
        ("backends".into(), backends.into()),
    ]));
    fs::write(root.join("config.toml"), toml::to_string(&fixture).unwrap()).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rota"))
            .env_clear()
            .env("PATH", env::var_os("PATH").unwrap())
            .env("HOME", &root)
            .env("ROTA_CONFIG", root.join("config.toml"))
            .current_dir(&root)
            .args(args)
            .output()
            .unwrap()
    };
    let mut failures = Vec::new();
    for backend in &original.backends {
        let mut commands: Vec<_> = backend.commands.values().cloned().collect();
        commands.extend(backend.popup_command());
        if let Some(run) = &backend.run {
            commands.extend(run.commands.values().cloned());
            for (role, argv) in &run.commands {
                let sandbox = if *role == Role::Implement {
                    "workspace-write"
                } else {
                    "read-only"
                };
                assert!(argv.windows(2).any(|pair| pair == ["-s", sandbox]));
                assert!(!argv.iter().any(|arg| arg == "--no-daemon"));
            }
        }
        for argv in &commands {
            let bypass = argv.iter().any(|arg| {
                arg.starts_with("--dangerously")
                    || matches!(
                        arg.as_str(),
                        "--yolo"
                            | "danger-full-access"
                            | "--sandbox=danger-full-access"
                            | "--full-auto"
                            | "--ask-for-approval=never"
                            | "--permission-mode=bypassPermissions"
                    )
            }) || argv.windows(2).any(|pair| {
                matches!(pair[0].as_str(), "-a" | "--ask-for-approval") && pair[1] == "never"
                    || pair[0] == "--permission-mode" && pair[1] == "bypassPermissions"
            });
            if bypass {
                failures.push(format!("{} bypass: {argv:?}", backend.name));
            }
        }
        for (level, key) in [(Level::Medium, "medium"), (Level::High, "high")] {
            let argv = &backend.commands[&level];
            if argv[0] == "codex" {
                if !argv.iter().any(|arg| arg == "--no-daemon") {
                    failures.push(format!("{} {key} missing --no-daemon", backend.name));
                }
                for pair in [["-s", "workspace-write"], ["-a", "on-request"]] {
                    if !argv.windows(2).any(|args| args == pair) {
                        failures.push(format!("{} {key} missing {pair:?}", backend.name));
                    }
                }
            }
            if argv[0] == "agy" && !argv.iter().any(|arg| arg == "--sandbox") {
                failures.push(format!("{} {key} missing --sandbox", backend.name));
            }
            let output = run(&[
                "launch",
                "--dry-run",
                "--only",
                &backend.name,
                "--level",
                key,
                "brief",
            ]);
            assert!(output.status.success(), "{}: {output:?}", backend.name);
            let stdout = String::from_utf8(output.stdout).unwrap();
            assert!(stdout.starts_with(&format!("pick: {}\ncommand: ", backend.name)));
            for arg in argv
                .iter()
                .filter(|arg| !matches!(arg.as_str(), "{prompt}" | "{cwd}"))
            {
                assert!(stdout.contains(arg), "{} {key}: {stdout}", backend.name);
            }
        }
        if backend.popup_command().is_some() {
            let output = run(&["launch", "--popup", "--dry-run", "--only", &backend.name]);
            assert!(
                output.status.success(),
                "{} popup: {output:?}",
                backend.name
            );
            assert!(
                String::from_utf8(output.stdout)
                    .unwrap()
                    .starts_with(&format!("pick: {}\ncommand: ", backend.name))
            );
        }
    }
    assert!(!root.join("state").exists());
    assert!(!root.join("state.lock").exists());
    assert!(!root.join("log").exists());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn tool(name: &str) -> PathBuf {
    // Renamed fixtures need standalone tools; multicall aliases depend on argv[0].
    env::split_paths(&env::var_os("PATH").unwrap())
        .filter_map(|directory| fs::canonicalize(directory.join(name)).ok())
        .find(|path| path.is_file() && path.file_name() == Some(std::ffi::OsStr::new(name)))
        .unwrap_or_else(|| panic!("{name} missing from test PATH"))
}

/// The file `Command::new(name)` runs, as /proc reports it.
fn on_path(name: &str) -> PathBuf {
    env::split_paths(&env::var_os("PATH").unwrap())
        .filter_map(|directory| fs::canonicalize(directory.join(name)).ok())
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("{name} missing from test PATH"))
}

/// Waits until `child` runs `executable` with its arguments and environment
/// in place. `spawn` returns as soon as the kernel releases the parent during
/// exec, before the new image's argv and environment are set up; until then
/// /proc shows an empty cmdline and environ, so a cap scan cannot match it.
fn wait_for_exec(child: &mut Child, executable: &Path) {
    let process = PathBuf::from(format!("/proc/{}", child.id()));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let exe = fs::read_link(process.join("exe")).ok();
        let cmdline = fs::read(process.join("cmdline")).unwrap_or_default();
        let environ = fs::read(process.join("environ")).unwrap_or_default();
        if exec_finished(exe.as_deref(), executable, &cmdline, &environ) {
            return;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "child {} did not finish exec into {} within 10s: exe {exe:?}, cmdline {:?}, environ {} bytes",
                child.id(),
                executable.display(),
                String::from_utf8_lossy(&cmdline),
                environ.len(),
            );
        }
        thread::sleep(Duration::from_millis(1));
    }
}

/// The kernel publishes argv and the environment separately, so exec is finished only once
/// both are visible. Every caller's child has an environment, so an empty one means not yet.
fn exec_finished(exe: Option<&Path>, executable: &Path, cmdline: &[u8], environ: &[u8]) -> bool {
    exe == Some(executable) && !cmdline.is_empty() && !environ.is_empty()
}

#[test]
fn exec_wait_needs_the_executable_argv_and_environment() {
    let sleep = Path::new("/bin/sleep");
    assert!(exec_finished(Some(sleep), sleep, b"sleep\0", b"A=1\0"));
    // Mid-exec states: argv published but not yet the environment, and the reverse.
    assert!(!exec_finished(Some(sleep), sleep, b"sleep\0", b""));
    assert!(!exec_finished(Some(sleep), sleep, b"", b"A=1\0"));
    assert!(!exec_finished(
        Some(Path::new("/bin/sh")),
        sleep,
        b"sleep\0",
        b"A=1\0"
    ));
    assert!(!exec_finished(None, sleep, b"sleep\0", b"A=1\0"));
}

#[test]
fn dry_run_rotates_renders_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(dir.path()).unwrap();
    let config_path = dir.join("config.toml");
    let state = dir.join("state");
    let log = dir.join("log");
    let trusted = dir.join("trust.json");
    let prompt = dir.join("task's prompt");
    let marker = dir.join("prepared");
    fs::write(&prompt, "SECRET PROMPT CONTENT").unwrap();
    fs::write(&state, "first\n").unwrap();
    let mut config = format!(
        "state_file = '{}'\nlog_file = '{}'\n",
        state.display(),
        log.display()
    );
    for name in ["first", "second"] {
        config.push_str(&format!(
            r#"
[[backends]]
name = '{name}'
roles = ['implement', 'review']
guard = ['true']
allow_unknown_quota = true
trusted_settings = '{}'
prepare = [['touch', '{}']]
[backends.commands]
medium = ['never-start-this', '--model', 'medium', '{{prompt}}']
high = ['never-start-this', '--model', 'high', '{{prompt}}']
"#,
            trusted.display(),
            marker.display()
        ));
    }
    fs::write(&config_path, &config).unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_rota"))
            .env("ROTA_CONFIG", &config_path)
            .current_dir(&dir)
            .args(args)
            .arg(&prompt)
            .output()
            .unwrap()
    };
    let output = run(&["launch", "--dry-run"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("pick: second"));
    assert!(stdout.contains("--model high"));
    assert!(stdout.contains("$(cat "));
    assert!(!stdout.contains("SECRET"));
    assert_eq!(fs::read_to_string(&state).unwrap(), "first\n");
    assert!(!log.exists());
    assert!(!trusted.exists());
    assert!(!marker.exists());
    assert!(!dir.join("state.lock").exists());
    let output = run(&[
        "launch",
        "--dry-run",
        "--level",
        "medium",
        "--only",
        "first",
    ]);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("pick: first\ncommand: never-start-this --model medium")
    );
    let output = run(&["launch", "--dry-run", "--only", "absent"]);
    assert_eq!(output.status.code(), Some(2));
    fs::write(
        &config_path,
        config.replace("guard = ['true']", "guard = ['false']"),
    )
    .unwrap();
    assert_eq!(
        run(&["launch", "--dry-run", "--role", "review"])
            .status
            .code(),
        Some(3)
    );
    fs::remove_file(&prompt).unwrap();
    assert_eq!(run(&["launch", "--dry-run"]).status.code(), Some(2));
}

#[test]
fn process_cap_follows_account_directory_and_executable_symlinks() {
    use rota::{config::Cap, readings::running};
    use std::{collections::BTreeMap, os::unix::fs::symlink};
    let dir = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(dir.path()).unwrap();
    let account = dir.join("account");
    let alias = dir.join("alias");
    fs::create_dir(&account).unwrap();
    symlink(&account, &alias).unwrap();
    let cap = Cap {
        max: 1,
        executable: "sleep".into(),
        runtime_executables: vec![],
        script: None,
        env: BTreeMap::from([("ROTA_TEST_ACCOUNT".into(), account.to_str().unwrap().into())]),
        exclude_args: vec![],
    };
    let executable_alias = dir.join("worker-alias");
    symlink(tool("sleep"), &executable_alias).unwrap();
    let mut child = Command::new(executable_alias)
        .arg("30")
        .current_dir(&dir)
        .env("ROTA_TEST_ACCOUNT", "alias")
        .spawn()
        .unwrap();
    wait_for_exec(&mut child, &tool("sleep"));
    let count = running(&cap);
    let mut config =
        rota::config::Config::parse(include_str!("../config.example.toml"), "rota").unwrap();
    config.backends[0].cap = Some(cap);
    let mut reading = rota::decision::Reading::default();
    let rechecked = rota::readings::recheck_cap(&config.backends[0], &mut reading);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(count.unwrap(), 1);
    assert!(rechecked.unwrap_err().contains("busy"));
    assert_eq!(reading.running, 1);
}

#[test]
fn refresh_rechecks_cap_after_a_slow_failed_probe() {
    let dir = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(dir.path()).unwrap();
    let started = dir.join("probe-started");
    let child_ready = dir.join("child-ready");
    let refreshed = dir.join("refreshed");
    let mut config =
        rota::config::Config::parse(include_str!("../config.example.toml"), "rota").unwrap();
    let mut backend = config.backends.remove(1);
    backend.guard = vec!["true".into()];
    backend.required_files.clear();
    backend
        .env
        .insert("ROTA_TEST_REFRESH".into(), dir.to_str().unwrap().into());
    backend.cap = Some(rota::config::Cap {
        max: 1,
        executable: "sleep".into(),
        runtime_executables: vec![],
        script: None,
        env: [("ROTA_TEST_REFRESH".into(), dir.to_str().unwrap().into())].into(),
        exclude_args: vec![],
    });
    let reader = backend.quota.as_mut().unwrap();
    reader.command = Some(vec![
        "sh".into(),
        "-c".into(),
        // The slow probe ends only once the capped child is running, bounded at 10s.
        r#"touch "$1"; i=0; while [ ! -e "$2" ] && [ $i -lt 1000 ]; do sleep 0.01; i=$((i+1)); done; printf failed"#.into(),
        "probe".into(),
        started.to_str().unwrap().into(),
        child_ready.to_str().unwrap().into(),
    ]);
    reader.refresh = Some(vec!["touch".into(), refreshed.to_str().unwrap().into()]);
    let probe = thread::spawn(move || {
        rota::readings::read(&backend, std::path::Path::new("."), true, &|_| {
            panic!("custom reader called Runtime")
        })
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while !started.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(started.exists());
    let mut child = Command::new("sleep")
        .arg("30")
        .env("ROTA_TEST_REFRESH", &dir)
        .spawn()
        .unwrap();
    wait_for_exec(&mut child, &on_path("sleep"));
    fs::write(&child_ready, "").unwrap();
    let reading = probe.join();
    child.kill().unwrap();
    child.wait().unwrap();
    let reading = reading.unwrap();
    assert_eq!(reading.running, 1);
    assert!(reading.problem.unwrap().contains("busy"));
    assert!(!refreshed.exists());
}

#[test]
fn invalid_filename_bytes_are_an_input_error() {
    use std::os::unix::ffi::OsStringExt;
    let output = Command::new(env!("CARGO_BIN_EXE_rota"))
        .args(["launch", "--dry-run"])
        .arg(std::ffi::OsString::from_vec(b"missing-\xff".to_vec()))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("arguments must be UTF-8")
    );
}

#[test]
fn wrapped_alias_uses_configured_runtime_identity() {
    use rota::{config::Cap, readings::running};
    use std::{
        collections::BTreeMap,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let dir = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(dir.path()).unwrap();
    // A child process writes both executables. A write descriptor opened here
    // can leak into a child another test thread is forking, and exec of the
    // file then fails with "Text file busy" until that child execs.
    let copy = |from: &Path, to: &Path| {
        let status = Command::new("cp").arg(from).arg(to).status().unwrap();
        assert!(status.success(), "cp {} {}", from.display(), to.display());
    };
    let native = dir.join(".worker-wrapped");
    copy(&tool("sleep"), &native);
    let wrapper = dir.join("worker");
    let source = dir.join("worker.source");
    fs::write(
        &source,
        format!(
            "#!{}\nexec -a \"$0\" \"${{0%/*}}/.worker-wrapped\" \"$@\"\n",
            tool("bash").display()
        ),
    )
    .unwrap();
    copy(&source, &wrapper);
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let alias = dir.join("different-alias");
    symlink(&wrapper, &alias).unwrap();
    let cap = Cap {
        max: 1,
        executable: "worker".into(),
        runtime_executables: vec![native.clone()],
        script: None,
        env: BTreeMap::from([("ROTA_TEST_WRAPPED".into(), dir.to_str().unwrap().into())]),
        exclude_args: vec![],
    };
    let mut child = Command::new(alias)
        .arg("30")
        .env("ROTA_TEST_WRAPPED", &dir)
        .spawn()
        .unwrap();
    // The wrapper execs its native runtime, so wait for that second exec.
    wait_for_exec(&mut child, &native);
    let count = running(&cap);
    child.kill().unwrap();
    child.wait().unwrap();
    assert_eq!(count.unwrap(), 1);
}

#[test]
fn interpreted_cli_cap_distinguishes_other_scripts() {
    use rota::{config::Cap, readings::running};
    use std::{collections::BTreeMap, process::Stdio};
    let dir = tempfile::tempdir().unwrap();
    let dir = fs::canonicalize(dir.path()).unwrap();
    let script = dir.join("cli.sh");
    let other = dir.join("other.sh");
    fs::write(&script, "read ignored\n").unwrap();
    fs::write(&other, "read ignored\n").unwrap();
    let cap = Cap {
        max: 1,
        executable: "sh".into(),
        runtime_executables: vec![],
        script: Some(script.clone()),
        env: BTreeMap::from([("ROTA_TEST_SCRIPT".into(), dir.to_str().unwrap().into())]),
        exclude_args: vec![],
    };
    let spawn = |path: &str| {
        Command::new("sh")
            .current_dir(&dir)
            .arg(path)
            .env("ROTA_TEST_SCRIPT", &dir)
            .stdin(Stdio::piped())
            .spawn()
            .unwrap()
    };
    let mut cli = spawn("cli.sh");
    let mut unrelated = spawn("other.sh");
    wait_for_exec(&mut cli, &on_path("sh"));
    wait_for_exec(&mut unrelated, &on_path("sh"));
    let count = running(&cap);
    cli.kill().unwrap();
    cli.wait().unwrap();
    unrelated.kill().unwrap();
    unrelated.wait().unwrap();
    assert_eq!(count.unwrap(), 1);
}
