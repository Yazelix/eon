use super::{tests::script, *};
use eon_workspace_protocol::v50::{PopupGeometry, Sidebar};
use serde_json::json;
use std::os::unix::fs::{PermissionsExt, symlink};

fn root() -> PathBuf {
    crate::supervisor::temporary_directory()
}

fn report() -> Value {
    report_from(include_bytes!("../../testdata/agy-usage-1.4.1.json"))
}

/// A fixture without resets, which would expire against the wall-clock read completion.
fn report_from(fixture: &[u8]) -> Value {
    let mut value: Value = serde_json::from_slice(fixture).unwrap();
    for group in value["command"]["data"]["groups"].as_array_mut().unwrap() {
        for bucket in group["buckets"].as_array_mut().unwrap() {
            bucket.as_object_mut().unwrap().remove("reset_time");
        }
    }
    value
}

/// An agy that answers only the usage command: collection runs nothing else.
fn backend(root: &Path, name: &str, body: &str) -> PathBuf {
    let path = root.join(name);
    script(
        &path,
        &format!("[ \"$*\" = '-p /usage --output-format json' ] || exit 8\n{body}"),
    );
    path
}

fn good(root: &Path) -> PathBuf {
    backend(root, "agy", &format!("printf '%s' '{}'", report()))
}

fn wait(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(condition(), "condition did not become true");
}

fn widgets() -> Vec<String> {
    vec!["google_quota:Unreported group:7d".into()]
}

fn start_google(program: &Path, unit: Duration, store: Option<PathBuf>) -> Quotas {
    Quotas::start_with_google(
        Path::new("/absent/codexbar"),
        program,
        unit,
        store,
        &widgets(),
    )
}

fn snapshot(quotas: &Quotas) -> Snapshot {
    let mut snapshot = Snapshot {
        focus: None,
        input_lock: crate::fixtures::input_lock(),
        input_locked: false,
        released_shortcuts: Default::default(),
        generation: "g1-00000000000000000000000000000001".into(),
        created_build: "g1-00000000000000000000000000000001".into(),
        active_build: "g1-00000000000000000000000000000001".into(),
        workspace: String::new(),
        active_workspace: String::new(),
        workspaces: vec![],
        active_tab: String::new(),
        geometry: PopupGeometry {
            side_margin: 0.0,
            vertical_margin: 0.0,
        },
        entries: vec![],
        tabs: vec![],
        codex_quota: None,
        claude_quota: None,
        google_quota: None,
        google_quota_failure: None,
        codex_quota_failure: None,
        claude_quota_failure: None,
        running: None,
        busy: None,
        sidebar: Sidebar::Off,
        attention_queue: vec![],
    };
    quotas.apply(&mut snapshot);
    snapshot
}

#[test]
fn google_source_resolves_absolute_executables() {
    let root = root().canonicalize().unwrap();
    let absolute = root.join("bin");
    fs::create_dir(&absolute).unwrap();
    let program = good(&absolute);
    let path = std::env::join_paths([
        Path::new(""),
        Path::new("."),
        Path::new("planted"),
        &absolute,
    ])
    .unwrap();
    assert_eq!(google::resolve(Some(&path)), Ok(program.clone()));
    for path in [None, Some(std::ffi::OsStr::new(":.:relative"))] {
        assert_eq!(google::resolve(path), Err(Failure::Start));
    }
    fs::set_permissions(&program, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        google::resolve(Some(absolute.as_os_str())),
        Err(Failure::Start)
    );
    let (_stop, stopped) = mpsc::channel();
    assert_eq!(
        google::fetch(&root.join("missing"), Duration::from_secs(1), &stopped),
        Err(Failure::Start)
    );
    let link = root.join("link");
    symlink(good(&root), &link).unwrap();
    assert_eq!(
        google::fetch(&link, Duration::from_secs(2), &stopped).unwrap(),
        Some(report().to_string().into_bytes())
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_commands_use_private_cwd_and_sanitized_environment() {
    let root = root();
    let seen = root.join("directories");
    let check = format!(
        "[ -z \"$(ls -A)\" ] && [ \"$(stat -c %a .)\" = 700 ] && [ -z \"${{EON_TERMINAL-}}${{EON_TAB-}}${{CLAUDE_CODE_MESSAGING_TOKEN-}}\" ] && ! read line || exit 8\npwd >> '{}'",
        seen.display()
    );
    let program = backend(
        &root,
        "agy",
        &format!("{check}\nprintf '%s' '{}'", report()),
    );
    let (_stop, stopped) = mpsc::channel();
    assert!(
        google::fetch(&program, Duration::from_secs(2), &stopped)
            .unwrap()
            .is_some()
    );
    let directories = fs::read_to_string(&seen).unwrap();
    let paths = directories.lines().collect::<Vec<_>>();
    assert_eq!(paths.len(), 1);
    assert_ne!(Path::new(paths[0]), root);
    assert!(!Path::new(paths[0]).exists());
    for args in [
        &["--version"][..],
        &["-p", "/usage", "--output-format", "json"][..],
    ] {
        let command = google::command(&program, &root, args);
        for name in [
            "EON_TERMINAL",
            "EON_TAB",
            "CLAUDE_CODE_MESSAGING_TOKEN",
            "CODEX_SESSION_ID",
        ] {
            assert!(
                command
                    .get_envs()
                    .any(|(key, value)| key == name && value.is_none()),
                "{name}"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_worker_classifies_object_reports_and_nonzero_exits() {
    let root = root();
    let (_stop, stopped) = mpsc::channel();
    // The successful object is the red sentinel; every error case is independently checked.
    let program = good(&root);
    assert!(
        google::fetch(&program, Duration::from_secs(2), &stopped)
            .unwrap()
            .is_some()
    );
    for (index, (body, exit, expected)) in [
        (
            json!({"status":"ERROR"}).to_string(),
            3,
            Failure::ProviderError,
        ),
        (report().to_string(), 3, Failure::ExitStatus),
        ("[]".into(), 3, Failure::ExitStatus),
        ("not json".into(), 3, Failure::ExitStatus),
        (
            json!({"status":"ERROR"}).to_string(),
            0,
            Failure::ProviderError,
        ),
        ("[]".into(), 0, Failure::Malformed),
        ("not json".into(), 0, Failure::Malformed),
        (
            {
                let mut r = report();
                r["command"]["name"] = json!("inference");
                r.to_string()
            },
            0,
            Failure::ProviderError,
        ),
        (
            {
                let mut r = report();
                r["num_turns"] = json!(1);
                r.to_string()
            },
            0,
            Failure::ProviderError,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let program = backend(
            &root,
            &format!("case-{index}"),
            &format!("printf '%s' '{body}'; exit {exit}"),
        );
        let fetched = google::fetch(&program, Duration::from_secs(2), &stopped).and_then(|bytes| {
            google::normalize(&bytes.unwrap(), epoch_seconds().unwrap()).map(|_| ())
        });
        assert_eq!(fetched, Err(expected), "case {index}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_read_bounds_cover_the_usage_command() {
    let root = root();
    let (_stop, stopped) = mpsc::channel();
    // Real processes can reach either bound first; the fetch unit pins the byte limit.
    let bounded_errors = &[Failure::Deadline, Failure::OutputLimit][..];
    for (name, body, expected) in [
        ("slow-usage", "sleep 2", &[Failure::Deadline][..]),
        (
            "flood-usage",
            "while :; do printf '%01024d' 0; done",
            bounded_errors,
        ),
    ] {
        let program = backend(&root, name, body);
        let fetched = google::fetch(&program, Duration::from_millis(250), &stopped)
            .map(|bytes| bytes.map(|bytes| bytes.len()));
        assert!(
            fetched
                .as_ref()
                .is_err_and(|failure| expected.contains(failure)),
            "{name}: expected {expected:?}, got {fetched:?}"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn agy_reports_reach_the_session_whatever_the_version() {
    let root = root();
    let asked = root.join("asked-version");
    let broken = root.join("broken");
    let v1_5_17 = report_from(include_bytes!("../../testdata/agy-usage-1.5.17.json"));
    let mut missing = v1_5_17.clone();
    missing["usage"]
        .as_object_mut()
        .unwrap()
        .remove("total_tokens");
    // Each agy would print its version if asked; collection must not ask.
    for (version, report, windows) in [
        ("1.4.1", report(), 4),
        (
            "1.5.6",
            report_from(include_bytes!("../../testdata/agy-usage-1.5.6.json")),
            5,
        ),
        ("1.5.17", v1_5_17.clone(), 4),
        ("2.0.0", v1_5_17.clone(), 4),
        ("nightly", v1_5_17.clone(), 4),
    ] {
        let program = root.join(version);
        script(
            &program,
            &format!(
                "if [ \"$1\" = --version ]; then : > '{}'; printf '{version}\\n'; exit; fi\n[ \"$*\" = '-p /usage --output-format json' ] || exit 8\nif [ -e '{}' ]; then printf '%s' '{missing}'; else printf '%s' '{report}'; fi",
                asked.display(),
                broken.display(),
            ),
        );
        let store = root.join(format!("{version}-quota"));
        let quotas = start_google(&program, Duration::from_millis(1), Some(store.clone()));
        wait(|| snapshot(&quotas).google_quota.is_some());
        let read = snapshot(&quotas);
        assert_eq!(read.google_quota_failure, None, "{version}");
        assert_eq!(
            read.google_quota.unwrap().windows.len(),
            windows,
            "{version}"
        );
        if version == "nightly" {
            // A report missing a required field fails; the last good reading stays.
            let retained = fs::read(store.join("google.json")).unwrap();
            fs::write(&broken, "").unwrap();
            wait(|| snapshot(&quotas).google_quota_failure.is_some());
            let failed = snapshot(&quotas);
            assert_eq!(
                failed.google_quota_failure.unwrap().reason,
                Failure::Malformed
            );
            assert!(failed.google_quota.is_some());
            assert_eq!(fs::read(store.join("google.json")).unwrap(), retained);
        }
        drop(quotas);
    }
    assert!(!asked.exists(), "collection ran agy --version");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_collection_projects_all_groups_independently_of_selection() {
    let root = root();
    let program = good(&root);
    let quotas = start_google(&program, Duration::from_secs(1), None);
    wait(|| snapshot(&quotas).google_quota.is_some());
    let snapshot = snapshot(&quotas);
    assert!(snapshot.codex_quota.is_none() && snapshot.claude_quota.is_none());
    assert!(quotas.codex.worker.is_none() && quotas.claude.worker.is_none());
    assert_eq!(snapshot.google_quota.as_ref().unwrap().windows.len(), 4);
    let json: Value = serde_json::from_str(&crate::workspace::json(&snapshot)).unwrap();
    assert_eq!(json["google_quota"]["windows"].as_array().unwrap().len(), 4);
    assert_eq!(
        json["google_quota"]["observed_at"],
        snapshot.google_quota.unwrap().observed_at * 1000
    );
    drop(quotas);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_success_missing_and_failure_keep_distinct_state() {
    let root = root();
    let mode = root.join("mode");
    let program = backend(
        &root,
        "agy",
        &format!(
            "case $(cat '{}') in good) printf '%s' '{}';; empty) printf '%s' '{}';; *) printf '%s' '{{\"status\":\"ERROR\"}}'; exit 3;; esac",
            mode.display(),
            report(),
            {
                let mut r = report();
                r["command"]["data"]["groups"] = json!([]);
                r
            }
        ),
    );
    let store = root.join("quota");
    fs::write(&mode, "good").unwrap();
    let quotas = start_google(&program, Duration::from_millis(1), Some(store.clone()));
    wait(|| snapshot(&quotas).google_quota.is_some());
    let retained = fs::read(store.join("google.json")).unwrap();
    fs::write(&mode, "error").unwrap();
    wait(|| snapshot(&quotas).google_quota_failure.is_some());
    assert_eq!(
        snapshot(&quotas).google_quota_failure.unwrap().reason,
        Failure::ProviderError
    );
    assert!(snapshot(&quotas).google_quota.is_some());
    assert_eq!(fs::read(store.join("google.json")).unwrap(), retained);
    fs::write(&mode, "empty").unwrap();
    wait(|| snapshot(&quotas).google_quota_failure.is_none());
    assert!(snapshot(&quotas).google_quota.is_none());
    assert!(!store.join("google.json").exists());
    drop(quotas);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_cache_restart_ages_and_validates_each_window() {
    let root = root();
    let store = root.join("quota");
    let program = good(&root);
    let quotas = start_google(&program, Duration::from_secs(1), Some(store.clone()));
    wait(|| snapshot(&quotas).google_quota.is_some());
    drop(quotas);
    let path = store.join("google.json");
    let original: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let quotas = start_google(
        &root.join("absent"),
        Duration::from_secs(1),
        Some(store.clone()),
    );
    wait(|| snapshot(&quotas).google_quota.is_some());
    drop(quotas);
    let now = epoch_seconds().unwrap();
    let stored = load(&path, "google", now).unwrap();
    assert_eq!(
        present_at(stored.clone(), now + STALE_AFTER + 1)
            .unwrap()
            .state,
        CodexQuotaState::Stale
    );
    assert!(present_at(stored.clone(), now + EXPIRE_AFTER + 1).is_none());
    let valid_at = stored.observed_at;
    assert_eq!(
        present_at(stored.clone(), valid_at + STALE_AFTER)
            .unwrap()
            .state,
        CodexQuotaState::Fresh
    );
    assert!(present_at(stored.clone(), valid_at + EXPIRE_AFTER).is_some());
    let mut resets = stored.clone();
    resets.windows.truncate(1);
    resets.windows[0].resets_at = Some(valid_at + 20);
    assert!(present_at(resets.clone(), valid_at + 19).is_some());
    assert!(present_at(resets, valid_at + 20).is_none());
    let mut bads = vec![];
    for (key, value) in [
        ("version", json!(2)),
        ("observed_at", json!(now + 1)),
        ("observed_at", json!(0)),
        ("windows", json!([])),
        ("identity", json!("forbidden")),
    ] {
        let mut bad = original.clone();
        bad[key] = value;
        bads.push(bad);
    }
    for (key, value) in [
        ("scope", Value::Null),
        ("scope", json!("")),
        ("scope", json!("a\nb")),
        ("scope", json!("x".repeat(33))),
        ("duration_minutes", json!(60)),
        ("remaining_percent", json!(101)),
        ("resets_at", json!(valid_at)),
    ] {
        let mut bad = original.clone();
        bad["windows"][0][key] = value;
        bads.push(bad);
    }
    let mut duplicate = original.clone();
    duplicate["windows"] = json!([original["windows"][0], original["windows"][0]]);
    bads.push(duplicate);
    let mut excessive = original.clone();
    excessive["windows"] = json!(
        (0..7)
            .map(|i| {
                let mut w = original["windows"][0].clone();
                w["scope"] = json!(format!("group-{i}"));
                w
            })
            .collect::<Vec<_>>()
    );
    bads.push(excessive);
    for bad in bads {
        fs::write(&path, bad.to_string()).unwrap();
        assert!(load(&path, "google", now).is_none(), "{bad}");
    }
    for bytes in [b"not json".to_vec(), vec![b' '; MAX_BYTES + 1]] {
        fs::write(&path, bytes).unwrap();
        assert!(load(&path, "google", now).is_none());
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_cache_replacement_is_private_and_atomic() {
    let root = root();
    let path = root.join("quota/google.json");
    let reading = google::normalize(report().to_string().as_bytes(), epoch_seconds().unwrap())
        .unwrap()
        .unwrap();
    let reading = CodexQuota {
        state: CodexQuotaState::Fresh,
        observed_at: reading.observed_at,
        windows: reading.windows,
    };
    save(&path, Some(&reading));
    assert!(load(&path, "google", epoch_seconds().unwrap()).is_some());
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let original = fs::read(&path).unwrap();
    assert!(!String::from_utf8_lossy(&original).contains("account"));
    save(&path, Some(&reading));
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o500)).unwrap();
    let mut replacement = reading.clone();
    replacement.windows[0].remaining_percent = 1;
    save(&path, Some(&replacement));
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::set_permissions(path.parent().unwrap(), fs::Permissions::from_mode(0o700)).unwrap();
    save(&path, None);
    assert!(!path.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_unlisting_cancels_without_erasing_last_good() {
    let root = root();
    let store = root.join("quota");
    let program = good(&root);
    let mut quotas = start_google(&program, Duration::from_secs(1), Some(store.clone()));
    wait(|| snapshot(&quotas).google_quota.is_some());
    let retained = fs::read(store.join("google.json")).unwrap();
    quotas.show(&[]);
    assert!(snapshot(&quotas).google_quota.is_none());
    assert!(snapshot(&quotas).google_quota_failure.is_none());
    assert_eq!(fs::read(store.join("google.json")).unwrap(), retained);
    for widget in ["google_quota", "google_quota::5h", "google_quota:bad:1h"] {
        quotas.show(&[widget.into()]);
        assert!(snapshot(&quotas).google_quota.is_none());
    }
    quotas.show(&widgets());
    wait(|| snapshot(&quotas).google_quota.is_some());
    drop(quotas);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_supervisors_share_schedule_and_backoff() {
    let root = root();
    let log = root.join("reads");
    let mode = root.join("mode");
    fs::write(&mode, "good").unwrap();
    let program = backend(
        &root,
        "agy",
        &format!(
            "mode=$(cat '{mode}'); : > '{root}'/read-$$; printf '%s %s\\n' \"$mode\" \"$$\" >> '{log}'; if [ \"$mode\" = error ]; then printf '%s' '{{\"status\":\"ERROR\"}}'; exit 3; fi; printf '%s' '{report}'",
            mode = mode.display(),
            root = root.display(),
            log = log.display(),
            report = report()
        ),
    );
    let store = root.join("quota");
    let supervisors = (0..2)
        .map(|_| start_google(&program, Duration::from_millis(2), Some(store.clone())))
        .collect::<Vec<_>>();
    let calls = || {
        fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    wait(|| {
        supervisors
            .iter()
            .all(|q| snapshot(q).google_quota.is_some())
    });
    assert_eq!(calls().len(), 1);
    wait(|| calls().len() >= 2);
    let first = calls();
    let time = |line: &str| {
        fs::metadata(root.join(format!("read-{}", line.split_whitespace().nth(1).unwrap())))
            .unwrap()
            .modified()
            .unwrap()
    };
    assert!(time(&first[1]).duration_since(time(&first[0])).unwrap() >= Duration::from_millis(600));
    let path = store.join("google.schedule");
    let maximum = Duration::from_secs(20);
    let schedule = || {
        update_schedule(
            &Mutex::default(),
            Some(&path),
            maximum,
            &Clock::System,
            true,
            |_, _, _| Some(false),
        )
        .0
    };
    let force_due = || {
        update_schedule(
            &Mutex::default(),
            Some(&path),
            maximum,
            &Clock::System,
            true,
            |schedule, _, _| {
                schedule.next_at = 0;
                Some(true)
            },
        );
    };
    fs::write(&mode, "error").unwrap();
    // The first provider error retries after one 600 ms interval plus up to a fifth; later ones
    // back off to 6, 12 and 24 intervals. Each check allows 300 ms for a loaded machine.
    for (failures, shortest, delay) in [
        (1, 600, 720),
        (2, 3600, 3600),
        (3, 7200, 7200),
        (4, 14400, 14400),
    ] {
        if failures > 1 {
            force_due();
        }
        wait(|| schedule().failures == failures);
        let now = milliseconds(SystemTime::now().duration_since(UNIX_EPOCH).unwrap());
        let scheduled = schedule();
        assert_eq!(scheduled.failure.unwrap().reason, Failure::ProviderError);
        assert!(
            (shortest - 300..=delay).contains(&scheduled.next_at.saturating_sub(now)),
            "failure {failures}: delay {}",
            scheduled.next_at.saturating_sub(now)
        );
    }
    wait(|| {
        supervisors
            .iter()
            .all(|q| snapshot(q).google_quota_failure.is_some())
    });
    assert!(
        supervisors
            .iter()
            .all(|q| snapshot(q).google_quota.is_some())
    );
    fs::write(&mode, "good").unwrap();
    force_due();
    wait(|| schedule().failures == 0 && schedule().failure.is_none());
    wait(|| {
        supervisors
            .iter()
            .all(|q| snapshot(q).google_quota_failure.is_none())
    });
    assert!(!store.join("claude.schedule").exists());
    drop(supervisors);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_schedule_fallback_preserves_local_spacing() {
    let root = root();
    let log = root.join("reads");
    let blocked = root.join("blocked");
    fs::write(&blocked, "file").unwrap();
    let program = backend(
        &root,
        "agy",
        &format!(
            "echo x >> '{}'; printf '%s' '{{\"status\":\"ERROR\"}}'; exit 3",
            log.display()
        ),
    );
    let mut quotas = start_google(&program, Duration::from_millis(1), Some(blocked));
    wait(|| snapshot(&quotas).google_quota_failure.is_some());
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 1);
    // Shown again, the in-memory spacing stands: one retry after one 300 ms interval plus
    // jitter, then six intervals before the next.
    quotas.show(&[]);
    quotas.show(&widgets());
    thread::sleep(Duration::from_millis(1000));
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 2);
    drop(quotas);
    let program = good(&root);
    for (index, invalid) in [
        "not json".to_owned(),
        json!({"version":2,"next_at":0,"failures":0,"failure":null}).to_string(),
        json!({"version":1,"next_at":u64::MAX,"failures":0,"failure":null}).to_string(),
    ]
    .into_iter()
    .enumerate()
    {
        let store = root.join(format!("invalid-{index}"));
        fs::create_dir(&store).unwrap();
        fs::write(store.join("google.schedule"), invalid).unwrap();
        let quotas = start_google(&program, Duration::from_millis(10), Some(store));
        wait(|| snapshot(&quotas).google_quota.is_some());
        drop(quotas);
    }
    let store = root.join("locked");
    let file = open_schedule(&store.join("google.schedule"), true).unwrap();
    let quotas = start_google(&program, Duration::from_millis(10), Some(store));
    wait(|| snapshot(&quotas).google_quota.is_some());
    drop(file);
    drop(quotas);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn google_unlisting_cancels_the_usage_command() {
    let root = root();
    let started = root.join("started");
    let directory = root.join("directory");
    let pid = root.join("pid");
    let slow = format!(
        "pwd > '{}'; sleep 30 & echo $! > '{}'; touch '{}'; wait",
        directory.display(),
        pid.display(),
        started.display()
    );
    let program = backend(&root, "agy", &slow);
    let mut quotas = start_google(&program, Duration::from_secs(1), None);
    wait(|| started.exists());
    let cwd = fs::read_to_string(&directory).unwrap();
    let descendant: i32 = fs::read_to_string(&pid).unwrap().trim().parse().unwrap();
    let began = Instant::now();
    quotas.show(&[]);
    assert!(began.elapsed() < Duration::from_secs(2));
    assert!(!Path::new(cwd.trim()).exists());
    // A killed descendant can briefly remain a zombie under the sandbox's init.
    wait(|| {
        let stat = fs::read_to_string(format!("/proc/{descendant}/stat")).unwrap_or_default();
        stat.is_empty() || stat.split_whitespace().nth(2) == Some("Z")
    });
    assert!(snapshot(&quotas).google_quota.is_none());
    assert!(snapshot(&quotas).google_quota_failure.is_none());
    fs::remove_dir_all(root).unwrap();
}
