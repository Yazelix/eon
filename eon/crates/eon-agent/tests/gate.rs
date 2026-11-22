#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]
//! The admission gate end to end (EON-C40, 2026-11-17 amendment clauses 2-5): configured order,
//! one admission per quota observation across `launch` and `run`, fresh readings, declared-absent
//! windows and lead launches. Readers are `printf`/`sh` fixtures, so no real account is touched.
//! Backends and accounts carry different names (`wrapper-a` on `acct-1`), so a refusal shows
//! which one it names.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Root {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

fn root() -> Root {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap();
    fs::write(path.join("brief"), "brief text\n").unwrap();
    fs::write(path.join("run-brief.md"), "run brief\n").unwrap();
    Root { _dir: dir, path }
}

impl Root {
    fn p(&self, name: &str) -> String {
        self.path.join(name).to_str().unwrap().to_string()
    }

    /// Writes the configuration: the state anchor in `state/`, the decision log in `log/`.
    fn config(&self, body: &str) {
        let text = format!(
            "state_file = '{root}/state/anchor'\nlog_file = '{root}/log/decisions.jsonl'\n{body}",
            root = self.path.display()
        );
        fs::write(
            self.path.join("config.toml"),
            text.replace("<ROOT>", &self.p("")),
        )
        .unwrap();
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rota"));
        command
            .env("ROTA_CONFIG", self.path.join("config.toml"))
            .env("PATH", self.path_with_fakes())
            .current_dir(&self.path)
            .args(args)
            .stdin(Stdio::null());
        command
    }

    fn rota(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// `PATH` with a stand-in `systemd-run` first: the start-up probe passes and the run's
    /// command runs directly, so `run` admits without a systemd user manager. The run then
    /// fails its scope check, after its admission is recorded.
    fn path_with_fakes(&self) -> String {
        let bin = self.path.join("fake-bin");
        if !bin.exists() {
            fs::create_dir_all(&bin).unwrap();
            let script = bin.join("systemd-run");
            fs::write(
                &script,
                "#!/bin/sh\ncase \"$*\" in *--unit=*) ;; *) exit 0 ;; esac\n\
                 while [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"\n",
            )
            .unwrap();
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap())
    }

    fn ledger(&self) -> Option<serde_json::Value> {
        let text = fs::read_to_string(self.path.join("state/admissions.json")).ok()?;
        Some(serde_json::from_str(&text).unwrap())
    }

    fn spent(&self, account: &str) -> Option<u64> {
        self.ledger()?["accounts"][account]["observed_at"].as_u64()
    }

    fn exists(&self, name: &str) -> bool {
        self.path.join(name).exists()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// A minute ago, in Unix milliseconds: a fixed observation well inside the default 600 s bound.
fn recent() -> u64 {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    now.as_millis() as u64 - 60_000
}

/// One backend. `reader` is the quota reader's argv (TOML array) and `limits` its limit tables.
fn backend(name: &str, roles: &str, reader: &str, extra: &str, limits: &str) -> String {
    format!(
        "\n[[backends]]\nname = '{name}'\nroles = {roles}\nguard = ['true']\n{extra}\n\
         [backends.commands]\nmedium = ['sh', '-c', 'touch \"$0\"', '<ROOT>launched-{name}', '{{prompt}}']\n\
         high = ['sh', '-c', 'touch \"$0\"', '<ROOT>launched-{name}', '{{prompt}}']\n\
         [backends.quota]\ncommand = {reader}\nparser = 'remaining_json'\n{limits}"
    )
}

/// A reader printing one fixed observation.
fn fixed(week: u32, at: u64) -> String {
    format!("['printf', '{{\"week\":{week},\"observed_at\":{at}}}']")
}

/// A reader stamping its observation `age_ms` before the moment it runs.
fn aged(week: u32, age_ms: u64) -> String {
    format!(
        "['sh', '-c', 'printf \"{{\\\"week\\\":{week},\\\"observed_at\\\":%s}}\" \
         $(( $(date +%s%3N) - {age_ms} ))']"
    )
}

/// A reader whose `observed_at` advances from `base` on every call, counting its calls in
/// `counter`.
fn counting(week: u32, counter: &str, base: u64) -> String {
    format!(
        "['sh', '-c', 'n=$(cat \"$0\" 2>/dev/null || echo 0); n=$((n+1)); echo $n > \"$0\"; \
         printf \"{{\\\"week\\\":{week},\\\"observed_at\\\":%s}}\" $(({base}+n))', '{counter}']"
    )
}

/// A reader that creates `marker` once it runs, then waits for `go` (at most 30 s, failing
/// loudly) before printing a fixed observation. Its backend needs a reader timeout above 30 s.
fn gated(at: u64, marker: &str, go: &str) -> String {
    format!(
        "['sh', '-c', 'touch \"$0\"; i=0; while [ ! -e \"$1\" ]; do i=$((i+1)); \
         if [ $i -gt 3000 ]; then echo gate never opened >&2; exit 1; fi; sleep 0.01; done; \
         printf \"{{\\\"week\\\":80,\\\"observed_at\\\":{at}}}\"', '{marker}', '{go}']"
    )
}

const IMPLEMENT_LIMIT: &str = "[backends.limits.implement.min_remaining]\nweek = 10\n";

fn run_table(name: &str) -> String {
    format!(
        "[backends.run]\nstdin = 'brief'\nefforts = ['high']\ndefault_effort = 'high'\n\
         [backends.run.commands]\nimplement = ['touch', '<ROOT>ran-{name}']\n"
    )
}

fn run_args(only: &str) -> Vec<String> {
    [
        "run",
        "--role",
        "implement",
        "--only",
        only,
        "--brief",
        "run-brief.md",
        "--name",
        "t",
        "--timeout",
        "30",
    ]
    .map(String::from)
    .to_vec()
}

fn rota_run(root: &Root, only: &str) -> Output {
    let args = run_args(only);
    root.rota(&args.iter().map(String::as_str).collect::<Vec<_>>())
}

fn spawn_run(root: &Root, only: &str) -> std::process::Child {
    let args = run_args(only);
    root.command(&args.iter().map(String::as_str).collect::<Vec<_>>())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn repeat(at: u64, account: &str) -> String {
    format!(
        "reading observed at {at} already admitted a launch on account {account}; \
         hold for a fresh reading"
    )
}

/// Waits up to 30 s for `done`, failing the test with `what` after that.
fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "waited 30 s for {what}");
        thread::sleep(Duration::from_millis(10));
    }
}

/// Whether process `pid` is blocked waiting for a `flock` (a `->` line in `/proc/locks`).
fn waits_for_a_lock(pid: u32) -> bool {
    let pid = pid.to_string();
    fs::read_to_string("/proc/locks")
        .unwrap()
        .lines()
        .any(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            fields.get(1) == Some(&"->")
                && fields.get(2) == Some(&"FLOCK")
                && fields.get(5) == Some(&pid.as_str())
        })
}

#[test]
fn real_launches_take_the_first_available_account_every_time() {
    let root = root();
    let base = recent();
    let (a, b) = (root.p("counter-a"), root.p("counter-b"));
    let reader_a = |week| counting(week, &a, base);
    let reader_b = |week| counting(week, &b, base);
    let body = |week_a, week_b| {
        backend(
            "wrapper-a",
            "['implement']",
            &reader_a(week_a),
            "account = 'acct-1'",
            IMPLEMENT_LIMIT,
        ) + &backend(
            "wrapper-b",
            "['implement']",
            &reader_b(week_b),
            "account = 'acct-2'",
            IMPLEMENT_LIMIT,
        )
    };
    root.config(&body(80, 80));
    for launch in 1..=5 {
        let output = root.rota(&["launch", "brief"]);
        assert!(output.status.success(), "launch {launch}: {output:?}");
        assert_eq!(text(&output.stdout), "pick: wrapper-a\n", "launch {launch}");
        let stderr = text(&output.stderr);
        assert!(stderr.contains("rota: wrapper-b [not checked]"), "{stderr}");
    }
    // Each launch read wrapper-a once, never wrapper-b, and recorded acct-1's newest observation.
    assert_eq!(fs::read_to_string(&a).unwrap().trim(), "5");
    assert!(!Path::new(&b).exists(), "wrapper-b's reader ran");
    assert_eq!(root.spent("acct-1"), Some(base + 5));
    assert_eq!(root.spent("acct-2"), None);
    // Nothing is written at state_file: there is no cursor.
    assert!(!root.exists("state/anchor"));

    // wrapper-a held: wrapper-b is next. The hold names the account, rule and threshold.
    let held_a = "rota: wrapper-a [quota limit on account acct-1: week left 5% \
                  (min_remaining >= 10%)]: week_left=5%\n";
    let held_b = "rota: wrapper-b [quota limit on account acct-2: week left 4% \
                  (min_remaining >= 10%)]: week_left=4%\n";
    root.config(&body(5, 80));
    let output = root.rota(&["launch", "brief"]);
    assert_eq!(text(&output.stdout), "pick: wrapper-b\n", "{output:?}");
    assert!(text(&output.stderr).contains(held_a), "{output:?}");
    // Both held: refusal naming both reasons.
    root.config(&body(5, 4));
    let output = root.rota(&["launch", "brief"]);
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    assert_eq!(
        text(&output.stderr),
        format!("{held_a}{held_b}rota: no backend available, refusing\n")
    );
    // Both held with a fallback configured: the fallback.
    let fallback = "\n[[backends]]\nname = 'main'\nroles = []\nguard = ['true']\n\
                    allow_unknown_quota = true\n[backends.commands]\n\
                    medium = ['true', '{prompt}']\nhigh = ['true', '{prompt}']\n";
    root.config(&format!("fallback = 'main'\n{}{fallback}", body(5, 4)));
    let output = root.rota(&["launch", "brief"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(text(&output.stdout), "pick: main\n");
    assert!(text(&output.stderr).contains("FALLBACK main"));
}

#[test]
fn an_observation_admits_once_and_a_refresh_brings_a_later_one() {
    let root = root();
    let at = recent();
    let reading = root.p("reading.json");
    fs::write(&reading, format!("{{\"week\":80,\"observed_at\":{at}}}")).unwrap();
    let body = backend(
        "wrapper-a",
        "['implement']",
        &format!("['cat', '{reading}']"),
        "account = 'acct-1'",
        IMPLEMENT_LIMIT,
    );
    root.config(&body);
    let first = root.rota(&["launch", "brief"]);
    assert!(first.status.success(), "{first:?}");
    assert!(root.exists("launched-wrapper-a"));
    assert_eq!(
        root.ledger().unwrap(),
        serde_json::json!({"version": 1, "accounts": {"acct-1": {"observed_at": at}}})
    );
    fs::remove_file(root.path.join("launched-wrapper-a")).unwrap();
    let held = format!("rota: wrapper-a [{}]", repeat(at, "acct-1"));
    let second = root.rota(&["launch", "brief"]);
    assert_eq!(second.status.code(), Some(75), "{second:?}");
    assert!(text(&second.stderr).contains(&held), "{second:?}");
    assert!(!root.exists("launched-wrapper-a"));
    // A dry run shows the hold and writes nothing.
    let before = fs::read(root.path.join("state/admissions.json")).unwrap();
    let dry = root.rota(&["launch", "--dry-run", "brief"]);
    assert_eq!(dry.status.code(), Some(75), "{dry:?}");
    assert!(text(&dry.stderr).contains(&held));
    let status = root.rota(&["status", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(json[0]["implement"], repeat(at, "acct-1"));
    assert_eq!(json[0]["reading"]["source"]["account"], "acct-1");
    assert_eq!(json[0]["reading"]["source"]["observed_at"], at);
    assert_eq!(
        fs::read(root.path.join("state/admissions.json")).unwrap(),
        before
    );
    // A configured refresh that brings a later observation admits.
    let later = at + 1000;
    let refresh = format!(
        "refresh = ['sh', '-c', 'printf \"{{\\\"week\\\":80,\\\"observed_at\\\":{later}}}\" > \"$0\"', '{reading}']\n"
    );
    root.config(&body.replace(
        "parser = 'remaining_json'\n",
        &format!("parser = 'remaining_json'\n{refresh}"),
    ));
    let dry = root.rota(&["launch", "--dry-run", "brief"]);
    assert_eq!(
        dry.status.code(),
        Some(75),
        "a dry run never refreshes: {dry:?}"
    );
    let third = root.rota(&["launch", "brief"]);
    assert!(third.status.success(), "{third:?}");
    assert_eq!(root.spent("acct-1"), Some(later));
}

#[test]
fn backends_on_one_account_share_its_observations() {
    let root = root();
    let at = recent();
    let body = backend(
        "wrapper-a",
        "['implement']",
        &fixed(80, at),
        "account = 'acct-1'",
        IMPLEMENT_LIMIT,
    ) + &backend(
        "wrapper-b",
        "['implement']",
        &fixed(80, at),
        "account = 'acct-1'",
        IMPLEMENT_LIMIT,
    );
    root.config(&body);
    let first = root.rota(&["launch", "--only", "wrapper-a", "brief"]);
    assert!(first.status.success(), "{first:?}");
    let second = root.rota(&["launch", "--only", "wrapper-b", "brief"]);
    assert_eq!(second.status.code(), Some(75), "{second:?}");
    assert!(
        text(&second.stderr).contains(&format!("rota: wrapper-b [{}]", repeat(at, "acct-1"))),
        "{second:?}"
    );
    assert!(!root.exists("launched-wrapper-b"));
}

#[test]
fn a_limited_backend_needs_a_present_and_recent_reading_time() {
    let root = root();
    let untimed = "['printf', '{\"week\":80}']";
    let future = fixed(80, 4_102_444_800_000);
    let limited = |name: &str, reader: &str, account: &str, limits: &str| {
        backend(
            name,
            "['implement']",
            reader,
            &format!("account = '{account}'"),
            limits,
        )
    };
    let wide = format!("max_age_seconds = 2000\n{IMPLEMENT_LIMIT}");
    let body = limited("untimed", untimed, "acct-1", IMPLEMENT_LIMIT)
        + &limited("future", &future, "acct-2", IMPLEMENT_LIMIT)
        + &limited("stale", &aged(80, 900_000), "acct-3", IMPLEMENT_LIMIT)
        + &limited("old-wide", &aged(80, 1_805_000), "acct-4", &wide)
        + &limited("recent", &aged(80, 300_000), "acct-5", IMPLEMENT_LIMIT)
        + &backend("unlimited", "['implement']", untimed, "", "");
    root.config(&body);
    let status = root.rota(&["status", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(
        json[0]["implement"],
        "quota reading on account acct-1 has no observed_at"
    );
    assert_eq!(
        json[1]["implement"],
        "quota reading on account acct-2 observed in the future (observed_at 4102444800000)"
    );
    assert_eq!(
        json[2]["implement"],
        "quota reading on account acct-3 is 900 s old (max_age_seconds 600)"
    );
    assert_eq!(json[3]["implement"], "available", "{json}");
    assert_eq!(json[4]["implement"], "available", "{json}");
    assert_eq!(json[5]["implement"], "available");
    // The default bound holds the 1805 s reading that a 2000 s bound admits.
    root.config(&body.replacen(&wide, IMPLEMENT_LIMIT, 1));
    let output = root.rota(&["launch", "--dry-run", "--only", "old-wide", "brief"]);
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    assert!(
        text(&output.stderr).contains(
            "rota: old-wide [quota reading on account acct-4 is 1805 s old (max_age_seconds 600)]"
        ),
        "{output:?}"
    );
    root.config(&body);
    let output = root.rota(&["launch", "brief"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(text(&output.stdout), "pick: old-wide\n");
    // Backends without limits consume nothing and have no age bound.
    let output = root.rota(&["launch", "--only", "unlimited", "brief"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        root.ledger().unwrap()["accounts"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["acct-4"]
    );
}

#[test]
fn an_unreadable_ledger_holds_limited_backends_with_its_path() {
    let root = root();
    root.config(&backend(
        "wrapper-a",
        "['implement']",
        &fixed(80, recent()),
        "account = 'acct-1'",
        IMPLEMENT_LIMIT,
    ));
    fs::create_dir_all(root.path.join("state")).unwrap();
    let ledger = root.path.join("state/admissions.json");
    for content in [
        "not json",
        "{\"version\":2,\"accounts\":{}}",
        "{\"version\":1,\"accounts\":{\"acct-1\":{\"observed_at\":\"soon\"}}}",
    ] {
        fs::write(&ledger, content).unwrap();
        let output = root.rota(&["launch", "brief"]);
        assert_eq!(output.status.code(), Some(75), "{content}: {output:?}");
        let stderr = text(&output.stderr);
        assert!(
            stderr.contains(&format!(
                "rota: wrapper-a [quota hold on account acct-1: admission ledger unreadable: {}",
                ledger.display()
            )),
            "{stderr}"
        );
        assert_eq!(fs::read_to_string(&ledger).unwrap(), content);
        assert!(!root.exists("launched-wrapper-a"));
    }
}

/// The state directory holds only the admission lock and refuses new files.
fn read_only_state(root: &Root) {
    let state = root.path.join("state");
    fs::create_dir_all(&state).unwrap();
    fs::write(state.join("anchor.lock"), "").unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o500)).unwrap();
}

fn writable_state(root: &Root) {
    fs::set_permissions(root.path.join("state"), fs::Permissions::from_mode(0o700)).unwrap();
}

fn assert_started(run: &Output) {
    let stdout = text(&run.stdout);
    let receipt: serde_json::Value =
        serde_json::from_str(stdout.lines().next().expect("started receipt")).unwrap();
    assert_eq!(receipt["event"], "started");
    assert_eq!(receipt["backend"], "wrapper-a");
}

#[test]
fn a_failed_ledger_write_refuses_launch_and_run_before_their_command() {
    let root = root();
    let at = recent();
    let body = backend(
        "wrapper-a",
        "['implement']",
        &fixed(80, at),
        "account = 'acct-1'",
        IMPLEMENT_LIMIT,
    ) + &run_table("wrapper-a");
    root.config(&body);
    read_only_state(&root);
    let launch = root.rota(&["launch", "brief"]);
    let run = rota_run(&root, "wrapper-a");
    writable_state(&root);
    assert_eq!(launch.status.code(), Some(1), "{launch:?}");
    assert!(
        text(&launch.stderr).contains("cannot record the admission"),
        "{launch:?}"
    );
    assert!(!root.exists("launched-wrapper-a"));
    assert_eq!(run.status.code(), Some(1), "{run:?}");
    assert!(
        text(&run.stderr).contains("cannot record the admission"),
        "{run:?}"
    );
    assert!(!root.exists("ran-wrapper-a"));
    assert!(run.stdout.is_empty(), "{run:?}");
    assert!(root.ledger().is_none());
    // With the directory writable, the same run starts its command and records the admission.
    let run = rota_run(&root, "wrapper-a");
    assert_started(&run);
    assert!(root.exists("ran-wrapper-a"), "{run:?}");
    assert_eq!(root.spent("acct-1"), Some(at));
}

#[test]
fn an_observation_spent_by_one_command_holds_the_other() {
    let at = recent();
    let body = backend(
        "wrapper-a",
        "['implement']",
        &fixed(80, at),
        "account = 'acct-1'",
        IMPLEMENT_LIMIT,
    ) + &run_table("wrapper-a");
    let held = format!("rota: wrapper-a [{}]", repeat(at, "acct-1"));
    // launch, then run.
    let root_a = root();
    root_a.config(&body);
    let launch = root_a.rota(&["launch", "brief"]);
    assert!(launch.status.success(), "{launch:?}");
    let run = rota_run(&root_a, "wrapper-a");
    assert_eq!(run.status.code(), Some(75), "{run:?}");
    assert!(text(&run.stderr).contains(&held), "{run:?}");
    assert!(!root_a.exists("ran-wrapper-a"));
    // run, then launch.
    let root_b = root();
    root_b.config(&body);
    let run = rota_run(&root_b, "wrapper-a");
    assert_started(&run);
    assert_eq!(root_b.spent("acct-1"), Some(at));
    let launch = root_b.rota(&["launch", "brief"]);
    assert_eq!(launch.status.code(), Some(75), "{launch:?}");
    assert!(text(&launch.stderr).contains(&held), "{launch:?}");
    assert!(!root_b.exists("launched-wrapper-a"));
}

/// Two backends on `acct-1` over one observation: `gated_name` reads through the gated reader,
/// `fixed_name` through a fixed one, and both can launch and run.
fn one_account_pair(root: &Root, gated_name: &str, fixed_name: &str, at: u64) {
    let (marker, go) = (root.p("gated-reading"), root.p("go"));
    root.config(
        &(backend(
            gated_name,
            "['implement']",
            &gated(at, &marker, &go),
            "account = 'acct-1'",
            &format!("timeout_seconds = 60\n{IMPLEMENT_LIMIT}"),
        ) + &run_table(gated_name)
            + &backend(
                fixed_name,
                "['implement']",
                &fixed(80, at),
                "account = 'acct-1'",
                IMPLEMENT_LIMIT,
            )
            + &run_table(fixed_name)),
    );
}

#[test]
fn a_launch_while_a_run_holds_the_admission_lock_is_refused() {
    let root = root();
    let at = recent();
    one_account_pair(&root, "wrapper-a", "wrapper-b", at);
    let run = spawn_run(&root, "wrapper-a");
    // The run holds the admission lock while its reader waits for `go`.
    wait_until("the run's quota reader", || root.exists("gated-reading"));
    let launch = root.rota(&["launch", "--only", "wrapper-b", "brief"]);
    fs::write(root.path.join("go"), "").unwrap();
    let run = run.wait_with_output().unwrap();
    assert_started(&run);
    assert!(launch.stdout.is_empty(), "{launch:?}");
    assert_eq!(launch.status.code(), Some(75), "{launch:?}");
    assert_eq!(
        text(&launch.stderr),
        "rota: another rota command is selecting a backend\n"
    );
    assert!(root.exists("ran-wrapper-a"), "{run:?}");
    assert!(!root.exists("launched-wrapper-b"));
    assert_eq!(root.spent("acct-1"), Some(at));
}

#[test]
fn a_post_prepare_cap_hold_names_the_capped_backend() {
    for args in [
        &["launch", "--json", "brief"][..],
        &[
            "run",
            "--brief",
            "run-brief.md",
            "--name",
            "cap",
            "--timeout",
            "30",
            "--json",
        ],
    ] {
        let root = root();
        let reader = fixed(80, recent());
        let chosen = backend(
            "chosen",
            "['implement']",
            &reader,
            "prepare = [['sh', '-c', 'touch prepared; until test -e go; do sleep .05; done']]",
            "[backends.cap]\nmax = 1\nexecutable = 'sleep'\n[backends.cap.env]\nROTA_TEST_CAP = '<ROOT>cap'\n",
        ) + &run_table("chosen");
        let unchecked =
            backend("unchecked", "['implement']", &reader, "", "") + &run_table("unchecked");
        root.config(&(chosen + &unchecked));
        let command = root
            .command(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        wait_until("chosen backend's prepare hook", || root.exists("prepared"));
        let mut occupant = Command::new("sleep")
            .arg("30")
            .env("ROTA_TEST_CAP", root.path.join("cap"))
            .spawn()
            .unwrap();
        fs::write(root.path.join("go"), "").unwrap();
        let result = command.wait_with_output();
        occupant.kill().unwrap();
        occupant.wait().unwrap();
        let result = result.unwrap();
        assert_eq!(result.status.code(), Some(75), "{result:?}");
        let error: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(
            error["error"]["reasons"],
            serde_json::json!([{"backend": "chosen", "detail": "busy (1 running)"}])
        );
    }
}

#[test]
fn a_run_waiting_for_the_admission_lock_sees_the_admission_it_waited_for() {
    let root = root();
    let at = recent();
    one_account_pair(&root, "wrapper-a", "wrapper-b", at);
    let launch = root
        .command(&["launch", "--only", "wrapper-a", "brief"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The launch holds the admission lock while its reader waits for `go`.
    wait_until("the launch's quota reader", || root.exists("gated-reading"));
    let mut run = spawn_run(&root, "wrapper-b");
    // The run blocks on the lock (or, were nothing serialized, finishes on its own).
    let pid = run.id();
    wait_until("the run to wait for the admission lock", || {
        waits_for_a_lock(pid) || run.try_wait().unwrap().is_some()
    });
    fs::write(root.path.join("go"), "").unwrap();
    let launch = launch.wait_with_output().unwrap();
    let run = run.wait_with_output().unwrap();
    assert!(
        text(&launch.stdout).starts_with("pick: wrapper-a\n"),
        "{launch:?}"
    );
    assert!(run.stdout.is_empty(), "{run:?}");
    assert_eq!(run.status.code(), Some(75), "{run:?}");
    assert!(
        text(&run.stderr).contains(&format!("rota: wrapper-b [{}]", repeat(at, "acct-1"))),
        "{run:?}"
    );
    assert!(root.exists("launched-wrapper-a"));
    assert!(!root.exists("ran-wrapper-b"));
    assert_eq!(root.spent("acct-1"), Some(at));
}

#[test]
fn lead_launches_skip_the_quota_gate_but_keep_access_checks() {
    let root = root();
    let marker = root.p("reader-ran");
    let reader = format!(
        "['sh', '-c', 'touch \"$0\"; printf \"{{\\\"week\\\":5,\\\"observed_at\\\":{}}}\"', '{marker}']",
        recent()
    );
    let body = backend(
        "wrapper-a",
        "['implement', 'lead']",
        &reader,
        "account = 'acct-1'",
        "[backends.limits.implement.min_remaining]\nweek = 50\n",
    );
    root.config(&body);
    let lead = root.rota(&["launch", "--role", "lead", "brief"]);
    assert!(lead.status.success(), "{lead:?}");
    assert_eq!(text(&lead.stdout), "pick: wrapper-a\n");
    assert!(root.exists("launched-wrapper-a"));
    assert!(
        !Path::new(&marker).exists(),
        "a lead launch ran the quota reader"
    );
    assert!(root.ledger().is_none());
    let decision = fs::read_to_string(root.path.join("log/decisions.jsonl")).unwrap();
    let decision: serde_json::Value =
        serde_json::from_str(decision.lines().last().unwrap()).unwrap();
    assert_eq!(decision["role"], "lead");
    assert_eq!(decision["decision"], "pick");
    // The same account over its limits holds an implement launch.
    let implement = root.rota(&["launch", "brief"]);
    assert_eq!(implement.status.code(), Some(75), "{implement:?}");
    // Status shows the lead column from access checks alone.
    let status = root.rota(&["status", "--json"]);
    let json: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(json[0]["lead"], "available");
    assert_eq!(
        json[0]["implement"],
        "quota limit on account acct-1: week left 5% (min_remaining >= 50%)"
    );
    // run never takes the lead role.
    let run = root.rota(&[
        "run",
        "--role",
        "lead",
        "--brief",
        "run-brief.md",
        "--name",
        "t",
        "--timeout",
        "30",
    ]);
    assert_eq!(run.status.code(), Some(2), "{run:?}");
    // A guard-failing lead backend still holds; a disabled one refuses (69), since waiting
    // cannot enable it.
    for (change, code) in [
        (("guard = ['true']", "guard = ['false']"), 75),
        (
            ("name = 'wrapper-a'", "name = 'wrapper-a'\nenabled = false"),
            69,
        ),
    ] {
        root.config(&body.replace(change.0, change.1));
        let lead = root.rota(&["launch", "--role", "lead", "brief"]);
        assert_eq!(lead.status.code(), Some(code), "{change:?}: {lead:?}");
    }
    // A backend that does not opt in to the lead role is not a lead candidate: no backend
    // serves the role.
    root.config(&body.replace("['implement', 'lead']", "['implement']"));
    let lead = root.rota(&["launch", "--role", "lead", "brief"]);
    assert_eq!(lead.status.code(), Some(69), "{lead:?}");
    assert!(
        text(&lead.stderr).contains("no backend serves role lead"),
        "{lead:?}"
    );
}

#[test]
fn an_idle_claude_account_needs_its_five_hour_window_declared_absent() {
    let root = root();
    let body = |absent: &str| {
        format!(
            "\n[[backends]]\nname = 'wrapper-a'\naccount = 'acct-1'\nroles = ['implement']\n\
             guard = ['true']\n[backends.commands]\nmedium = ['true', '{{prompt}}']\n\
             high = ['true', '{{prompt}}']\n[backends.quota]\n\
             command = ['printf', 'None 20 observed_at={}']\nparser = 'claude_text'\n{absent}\n\
             [backends.limits.implement.max_used]\nfive_hour = 30\nseven_day = 40\n",
            recent()
        )
    };
    root.config(&body("absent_windows = ['five_hour']"));
    let output = root.rota(&["launch", "--dry-run", "brief"]);
    assert!(output.status.success(), "{output:?}");
    root.config(&body(""));
    let output = root.rota(&["launch", "--dry-run", "brief"]);
    assert_eq!(output.status.code(), Some(75), "{output:?}");
    assert!(
        text(&output.stderr)
            .contains("rota: wrapper-a [missing quota window five_hour on account acct-1]"),
        "{output:?}"
    );
}

#[test]
fn codexbar_quota_uses_the_shared_gate() {
    use serde_json::{Value, json};

    let root = root();
    let observed_at = recent();
    let date = |at: u64| {
        let output = Command::new("date")
            .args([
                "-u",
                "-d",
                &format!("@{}.{:03}", at / 1000, at % 1000),
                "+%Y-%m-%dT%H:%M:%S.%3NZ",
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        text(&output.stdout).trim().to_owned()
    };
    let updated = date(observed_at);
    let reset = date(observed_at + 86_400_000);
    let fixture = |content: &str| -> Value {
        serde_json::from_str(
            &content
                .replace("2026-10-14T00:00:00Z", &updated)
                .replace("2026-10-19T20:59:59Z", &reset),
        )
        .unwrap()
    };
    let codex = fixture(include_str!("fixtures/codexbar/codex.json"));
    let claude = fixture(include_str!("fixtures/codexbar/claude.json"));
    let mut weekly = fixture(include_str!("fixtures/codexbar/claude-weekly-only.json"));
    let error = fixture(include_str!("fixtures/codexbar/error.json"));

    let write = |name: &str, report: &Value| {
        fs::write(root.path.join(name), report.to_string()).unwrap();
    };
    let limits = "[backends.limits.implement.max_used]\nfive_hour = 30\n\
                  [backends.limits.review.max_used]\nfive_hour = 50\nseven_day = 40\n";
    let backend = |name: &str, command: &str, rules: &str| {
        backend(
            name,
            "['implement', 'review']",
            command,
            "account = 'configured-subscription'",
            rules,
        )
        .replacen("parser = 'remaining_json'", "parser = 'codexbar'", 1)
    };
    let codex_backend = backend("codex", "['cat', '<ROOT>codex.json']", limits);
    let claude_backend = backend(
        "claude",
        "['cat', '<ROOT>claude.json']",
        &format!("{limits}seven_day_fable = 50\n"),
    );
    let status = || -> Value {
        let output = root.rota(&["status", "--json"]);
        assert!(output.status.success(), "{output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    };
    let dry = |name: &str, role: &str, hold: Option<&str>| {
        let output = root.rota(&[
            "launch",
            "--dry-run",
            "--only",
            name,
            "--role",
            role,
            "brief",
        ]);
        if let Some(reason) = hold {
            assert_eq!(output.status.code(), Some(75), "{name}/{role}: {output:?}");
            assert!(
                text(&output.stderr).contains(reason),
                "{reason}: {output:?}"
            );
        } else {
            assert!(output.status.success(), "{name}/{role}: {output:?}");
            assert!(text(&output.stdout).starts_with(&format!("pick: {name}\n")));
        }
    };
    write("codex.json", &codex);
    write("claude.json", &claude);
    root.config(&format!("{codex_backend}{claude_backend}"));
    let report = status();
    let codex_reading = &report[0]["reading"];
    assert_eq!(
        codex_reading["remaining"],
        json!({"five_hour":76.5,"seven_day":98.0})
    );
    assert_eq!(codex_reading["resets"], json!({"seven_day":reset}));
    assert_eq!(
        codex_reading["source"],
        json!({
            "account": "configured-subscription",
            "observed_at": observed_at,
            "identity": {"account_id":"fixture-workspace","account_email":"discard@example.invalid"}
        })
    );
    let claude_reading = &report[1]["reading"];
    assert_eq!(
        claude_reading["remaining"],
        json!({"five_hour":76.5,"seven_day":98.0,"seven_day_fable":60.0})
    );
    assert_eq!(
        claude_reading["source"],
        json!({"account":"configured-subscription","observed_at":observed_at})
    );
    let mut mirrored = codex.clone();
    let email = mirrored[0]["usage"]["identity"]
        .as_object_mut()
        .unwrap()
        .remove("accountEmail")
        .unwrap();
    mirrored[0]["usage"]["accountEmail"] = email;
    write("codex.json", &mirrored);
    assert_eq!(
        status()[0]["reading"]["source"]["identity"],
        codex_reading["source"]["identity"]
    );

    for (provider, baseline) in [("codex", &codex), ("claude", &claude)] {
        let file = format!("{provider}.json");
        for (role, slot, just_below, limit, reason) in [
            ("review", "primary", 49, 50, "five_hour used 50%"),
            ("review", "secondary", 39, 40, "seven_day used 40%"),
            ("implement", "primary", 29, 30, "five_hour used 30%"),
        ] {
            let mut payload = baseline.clone();
            payload[0]["usage"][slot]["usedPercent"] = json!(just_below);
            write(&file, &payload);
            dry(provider, role, None);
            payload[0]["usage"][slot]["usedPercent"] = json!(limit);
            write(&file, &payload);
            dry(provider, role, Some(reason));
        }
    }
    for confidence in ["exact", "percentOnly", "estimated"] {
        let mut payload = codex.clone();
        payload[0]["usage"]["dataConfidence"] = json!(confidence);
        write("codex.json", &payload);
        dry(
            "codex",
            "review",
            (confidence == "estimated").then_some("CodexBar usage confidence"),
        );
    }

    let mut scoped = claude.clone();
    scoped[0]["usage"]["extraRateWindows"][0]["window"]["usedPercent"] = json!(49);
    write("claude.json", &scoped);
    dry("claude", "review", None);
    scoped[0]["usage"]["extraRateWindows"][0]["window"]["usedPercent"] = json!(50);
    write("claude.json", &scoped);
    dry("claude", "review", Some("seven_day_fable used 50%"));
    scoped[0]["usage"]["extraRateWindows"][0]["window"]["usedPercent"] = json!(102);
    write("claude.json", &scoped);
    dry(
        "claude",
        "review",
        Some("missing quota window seven_day_fable"),
    );

    write("claude.json", &weekly);
    dry("claude", "review", Some("missing quota window five_hour"));
    let weekly_backend = claude_backend.replace(
        "parser = 'codexbar'",
        "parser = 'codexbar'\nabsent_windows = ['five_hour']",
    );
    root.config(&weekly_backend);
    dry("claude", "review", None);
    for slot in ["primary", "secondary"] {
        weekly[0]["usage"][slot]["usedPercent"] = json!(40);
    }
    write("claude.json", &weekly);
    dry("claude", "review", Some("seven_day used 40%"));
    weekly[0]["usage"]["secondary"] = Value::Null;
    write("claude.json", &weekly);
    dry("claude", "review", Some("missing quota window seven_day"));
    write("claude.json", &error);
    dry("claude", "review", Some("CodexBar reported an error"));

    let wildcard = backend(
        "codex",
        "['cat', '<ROOT>codex.json']",
        "[backends.limits.review.max_used]\n'*' = 20\n",
    );
    let mut clean = codex.clone();
    clean[0]["usage"]["primary"]["usedPercent"] = json!(5);
    root.config(&wildcard);
    write("codex.json", &clean);
    dry("codex", "review", None);
    for (slot, key) in [("primary", "five_hour"), ("secondary", "seven_day")] {
        let mut partial = clean.clone();
        partial[0]["usage"][slot] = Value::Null;
        write("codex.json", &partial);
        root.config(&wildcard);
        dry(
            "codex",
            "review",
            Some(&format!("missing quota window {key}")),
        );
        root.config(&wildcard.replace(
            "parser = 'codexbar'",
            &format!("parser = 'codexbar'\nabsent_windows = ['{key}']"),
        ));
        dry("codex", "review", None);
    }
    root.config(&wildcard);
    for slot in ["primary", "secondary"] {
        let mut payload = clean.clone();
        payload[0]["usage"][slot]["usedPercent"] = json!(102);
        payload[0]["usage"][slot]["resetsAt"] = json!(updated);
        write("codex.json", &payload);
        dry("codex", "review", Some("invalid percentage"));
    }
    let mut tertiary = clean.clone();
    tertiary[0]["usage"]["tertiary"] = clean[0]["usage"]["secondary"].clone();
    write("codex.json", &tertiary);
    dry("codex", "review", Some("CodexBar Codex tertiary"));
    let mut placeholder = clean.clone();
    placeholder[0]["usage"]["primary"]["isSyntheticPlaceholder"] = json!(true);
    write("codex.json", &placeholder);
    dry(
        "codex",
        "review",
        Some("invalid CodexBar window placeholder"),
    );
    let mut unsupported = clean.clone();
    unsupported[0]["provider"] = json!("claude");
    unsupported[0]["source"] = json!("web");
    write("codex.json", &unsupported);
    dry(
        "codex",
        "review",
        Some("unsupported CodexBar provider or source"),
    );
    write("codex.json", &json!([clean[0], clean[0]]));
    dry(
        "codex",
        "review",
        Some("expected one CodexBar usage result"),
    );
    let mut untimed = clean;
    untimed[0]["usage"]
        .as_object_mut()
        .unwrap()
        .remove("updatedAt");
    write("codex.json", &untimed);
    dry("codex", "review", Some("invalid CodexBar observation time"));

    root.config(&codex_backend);
    for (at, problem) in [
        (observed_at - 900_000, "s old"),
        (observed_at + 86_400_000, "observed in the future"),
    ] {
        let mut payload = codex.clone();
        payload[0]["usage"]["updatedAt"] = json!(date(at));
        write("codex.json", &payload);
        dry("codex", "review", Some(problem));
    }
    let mut expired = codex.clone();
    expired[0]["usage"]["secondary"]["resetsAt"] = json!(updated);
    write("codex.json", &expired);
    dry("codex", "review", Some("missing quota window seven_day"));
    let mut stale = codex.clone();
    stale[0]["usage"]["updatedAt"] = json!(date(observed_at - 900_000));
    root.config(&backend("codex", "['cat', '<ROOT>codex.json']", ""));
    write("codex.json", &stale);
    dry("codex", "review", Some("s old"));

    write("codex.json", &codex);
    root.config(&backend(
        "codex",
        "['sh', '-c', 'cat \"$1\"; exit 7', 'fixture', '<ROOT>codex.json']",
        limits,
    ));
    dry("codex", "review", Some("exit status: 7"));

    let mut label_only = codex.clone();
    label_only[0]["usage"]["identity"] = Value::Null;
    label_only[0]["usage"]["accountEmail"] = Value::Null;
    label_only[0]["account"] = json!("fixture-profile");
    write("claude.json", &claude);
    root.config(&format!("{codex_backend}{claude_backend}"));
    write("codex.json", &label_only);
    assert_eq!(
        status()[0]["reading"]["source"]["identity"],
        json!({"account_label":"fixture-profile"})
    );
    write("codex.json", &codex);
    let launched = root.rota(&["launch", "--only", "codex", "brief"]);
    assert!(launched.status.success(), "{launched:?}");
    assert!(root.exists("launched-codex"));
    assert_eq!(root.spent("configured-subscription"), Some(observed_at));

    let log = fs::read_to_string(root.path.join("log/decisions.jsonl")).unwrap();
    let decision: Value = serde_json::from_str(log.lines().last().unwrap()).unwrap();
    assert_eq!(
        decision["readings"][0]["reading"]["source"],
        json!({"account":"configured-subscription","observed_at":observed_at})
    );
    let held = root.rota(&["launch", "--only", "claude", "brief"]);
    assert_eq!(held.status.code(), Some(75), "{held:?}");
    assert!(text(&held.stderr).contains(&repeat(observed_at, "configured-subscription")));
    assert!(!root.exists("launched-claude"));
}
