#![forbid(unsafe_code)]
#![cfg(target_os = "linux")]
//! Project profiles end to end (EON-C40 clause 6): caps counted by the admitted process,
//! role guards, executable-keyed overlays, selection by `--project` or the working directory's
//! repository, and the `{name}`/`{add_dirs}` launch placeholders.
//!
//! Fixture agents are `bin/codex` and `bin/claude`: each writes its argv (NUL-separated) to the
//! path in its first argument and then `exec sleep`s, so the PID the test kills is the admitted
//! process itself. A path ending in `-exit` exits at once instead. Every backend's quota reader
//! appends to `reads`, so a test can tell that a hold read no quota.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const AGENT: &str = "#!/bin/sh\nout=$1\nshift\nprintf '%s\\0' \"$@\" > \"$out.tmp\" && mv \"$out.tmp\" \"$out\"\n\
                     case \"$out\" in *-exit) exit 0 ;; esac\nexec sleep 600\n";

struct Root {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

/// A running agent, killed and reaped on drop.
struct Agent {
    child: Child,
}

impl Agent {
    fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn root() -> Root {
    let dir = tempfile::tempdir().unwrap();
    let path = fs::canonicalize(dir.path()).unwrap();
    for sub in [
        "bin",
        "out",
        "repo/.git/worktrees/w",
        "repo/sub",
        "w",
        "elsewhere",
        "fake-bin",
    ] {
        fs::create_dir_all(path.join(sub)).unwrap();
    }
    // Every fixture executable is written before anything is spawned (ETXTBSY).
    for name in ["codex", "claude"] {
        executable(&path.join("bin").join(name), AGENT);
    }
    executable(
        &path.join("guard"),
        &format!(
            "#!/bin/sh\npwd > '{}'\necho >&2; echo machine busy >&2\nexit 1\n",
            path.join("guard-cwd").display()
        ),
    );
    // A stand-in systemd-run: the probe passes and the run's command runs directly.
    executable(
        &path.join("fake-bin/systemd-run"),
        "#!/bin/sh\ncase \"$*\" in *--unit=*) ;; *) exit 0 ;; esac\n\
         while [ \"$1\" != -- ]; do shift; done\nshift\nexec \"$@\"\n",
    );
    fs::write(path.join("repo/.git/worktrees/w/commondir"), "../..\n").unwrap();
    fs::write(path.join("w/.git"), "gitdir: ../repo/.git/worktrees/w\n").unwrap();
    // `../prompt` from every launch folder, `repo/sub` included.
    fs::write(path.join("prompt"), "do the task\n").unwrap();
    fs::write(path.join("repo/prompt"), "do the task\n").unwrap();
    Root { _dir: dir, path }
}

/// One backend. Its level commands run `bin/EXE` with `out/{name}`, `{overlay}` and the prompt.
fn backend(name: &str, exe: &str, roles: &str, extra: &str) -> String {
    format!(
        "\n[[backends]]\nname = '{name}'\nroles = {roles}\nguard = ['true']\n{extra}\n\
         [backends.commands]\nmedium = ['<ROOT>bin/{exe}', '<ROOT>out/{{name}}', '{{overlay}}', '{{prompt}}']\n\
         high = ['<ROOT>bin/{exe}', '<ROOT>out/{{name}}', '{{overlay}}', '{{prompt}}']\n\
         [backends.quota]\ncommand = ['sh', '-c', 'echo x >> \"$0\"; printf \"{{\\\"week\\\":90}}\"', '<ROOT>reads']\n\
         parser = 'remaining_json'\n"
    )
}

fn project(name: &str, body: &str) -> String {
    format!(
        "\n[projects.{name}]\nrepo = '<ROOT>repo'\nagents_root = '<ROOT>agents'\n{}\n",
        body.replace("[.", &format!("[projects.{name}."))
    )
}

impl Root {
    fn p(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }

    fn config(&self, body: &str) {
        let text = format!(
            "state_file = '<ROOT>state/anchor'\nlog_file = '<ROOT>log/decisions.jsonl'\n{body}"
        );
        let root = format!("{}/", self.path.display());
        fs::write(self.p("config.toml"), text.replace("<ROOT>", &root)).unwrap();
    }

    fn command(&self, cwd: &str, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rota"));
        let path = format!(
            "{}:{}",
            self.p("fake-bin").display(),
            std::env::var("PATH").unwrap()
        );
        command
            .env("ROTA_CONFIG", self.p("config.toml"))
            .env("PATH", path)
            .current_dir(self.p(cwd))
            .args(args)
            .stdin(Stdio::null());
        command
    }

    fn rota(&self, cwd: &str, args: &[&str]) -> Output {
        self.command(cwd, args).output().unwrap()
    }

    /// Starts `rota ARGS` and waits until the fixture agent named `name` has written its argv.
    fn start(&self, cwd: &str, args: &[&str], name: &str) -> Agent {
        let child = self
            .command(cwd, args)
            .stdout(Stdio::null())
            .stderr(fs::File::create(self.p(&format!("stderr-{name}"))).unwrap())
            .spawn()
            .unwrap();
        let agent = Agent { child };
        self.argv(name);
        agent
    }

    fn stderr(&self, name: &str) -> String {
        fs::read_to_string(self.p(&format!("stderr-{name}"))).unwrap()
    }

    /// The argv a fixture agent wrote, waiting up to 20 s for it.
    fn argv(&self, name: &str) -> Vec<String> {
        let path = self.p(&format!("out/{name}"));
        let until = Instant::now() + Duration::from_secs(20);
        while !path.exists() {
            assert!(
                Instant::now() < until,
                "agent {name} never wrote its argv; stderr: {}",
                fs::read_to_string(self.p(&format!("stderr-{name}"))).unwrap_or_default()
            );
            thread::sleep(Duration::from_millis(20));
        }
        let bytes = fs::read(path).unwrap();
        bytes
            .split(|b| *b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8(s.to_vec()).unwrap())
            .collect()
    }

    fn reads(&self) -> usize {
        fs::read_to_string(self.p("reads")).map_or(0, |t| t.lines().count())
    }

    fn records(&self) -> Vec<serde_json::Value> {
        let Ok(entries) = fs::read_dir(self.p("state/agents")) else {
            return Vec::new();
        };
        entries
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .map(|p| serde_json::from_slice(&fs::read(p).unwrap()).unwrap())
            .collect()
    }

    fn last_decision(&self) -> serde_json::Value {
        let text = fs::read_to_string(self.p("log/decisions.jsonl")).unwrap();
        serde_json::from_str(text.lines().last().unwrap()).unwrap()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn launch<'a>(name: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec!["launch", "--name", name];
    args.extend_from_slice(extra);
    args.push("../prompt");
    args
}

/// Kills `pid` (not our child) and waits until it is gone or a zombie.
fn kill_pid(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", &pid.to_string()])
        .status();
    let until = Instant::now() + Duration::from_secs(10);
    while fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !stat
            .rsplit(')')
            .next()
            .unwrap()
            .trim_start()
            .starts_with('Z')
    }) {
        assert!(Instant::now() < until, "{pid} survived SIGKILL");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn tide_total_and_backend_caps() {
    let root = root();
    root.config(&format!(
        "{}{}{}",
        backend("cx", "codex", "['implement', 'review']", ""),
        backend("cl", "claude", "['implement', 'review']", ""),
        project("tide", "max = 2\n[.backends.cx]\nmax = 1\n"),
    ));
    let first = root.start("repo", &launch("a", &[]), "a");
    assert!(
        root.stderr("a")
            .contains("project: tide (from the working directory's repository)")
    );
    let records = root.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["backend"], "cx");
    assert_eq!(records[0]["processes"][0]["pid"], first.pid());
    // The second skips cx at its project cap, naming the live PID, and takes cl.
    let second = root.start("repo/sub", &launch("b", &[]), "b");
    assert!(
        root.stderr("b").contains(&format!(
            "cx [project tide: 1 live agent on cx (backends.cx.max 1; pids {})]",
            first.pid()
        )),
        "{}",
        root.stderr("b")
    );
    assert_eq!(root.records().len(), 2);
    // The third is held by the total before any quota is read.
    let reads = root.reads();
    let held = root.rota("repo", &launch("c", &[]));
    assert_eq!(held.status.code(), Some(75), "{}", text(&held.stderr));
    let mut pids = [first.pid(), second.pid()];
    pids.sort();
    assert!(
        text(&held.stderr).contains(&format!(
            "project tide: 2 live agents (max 2; pids {}, {})",
            pids[0], pids[1]
        )),
        "{}",
        text(&held.stderr)
    );
    assert_eq!(root.reads(), reads);
    let logged = root.last_decision();
    assert_eq!(logged["project"], "tide");
    assert_eq!(logged["decision"], "REFUSAL");
    assert!(
        logged["hold"]
            .as_str()
            .unwrap()
            .starts_with("project tide: 2 live agents")
    );
    // Once the first agent exits, its slot frees and its record is pruned.
    drop(first);
    let third = root.start("repo", &launch("d", &[]), "d");
    assert_eq!(root.records().len(), 2);
    drop((second, third));
}

#[test]
fn back_to_back_launches_count_from_the_record() {
    let root = root();
    root.config(&format!(
        "{}{}",
        backend("cl", "claude", "['implement']", ""),
        project("tide", "max = 1\n"),
    ));
    let child = root
        .command("repo", &launch("a", &[]))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let first = Agent { child };
    // Start the second as soon as the first's record exists, before it has necessarily exec'd.
    let until = Instant::now() + Duration::from_secs(20);
    while root.records().is_empty() {
        assert!(Instant::now() < until);
        thread::sleep(Duration::from_millis(1));
    }
    let held = loop {
        let out = root.rota("repo", &launch("b", &[]));
        if !text(&out.stderr).contains("is selecting a backend") {
            break out;
        }
    };
    assert_eq!(held.status.code(), Some(75), "{}", text(&held.stderr));
    assert!(
        text(&held.stderr).contains(&format!("1 live agent (max 1; pids {})", first.pid())),
        "{}",
        text(&held.stderr)
    );
    root.argv("a");
}

#[test]
fn strand_role_caps_and_an_implement_only_guard() {
    let root = root();
    root.config(&format!(
        "{}{}",
        backend("cl", "claude", "['implement', 'review']", ""),
        project(
            "strand",
            "[.roles.implement]\nmax = 2\n[.roles.review]\nmax = 2\n"
        ),
    ));
    let a = root.start("repo", &launch("a", &[]), "a");
    let b = root.start("repo", &launch("b", &[]), "b");
    let reads = root.reads();
    let held = root.rota("repo", &launch("c", &[]));
    assert_eq!(held.status.code(), Some(75));
    let mut pids = [a.pid(), b.pid()];
    pids.sort();
    assert!(
        text(&held.stderr).contains(&format!(
            "project strand: 2 live implement agents (roles.implement.max 2; pids {}, {})",
            pids[0], pids[1]
        )),
        "{}",
        text(&held.stderr)
    );
    assert_eq!(root.reads(), reads);
    let review = root.start("repo", &launch("r", &["--role", "review"]), "r");
    drop((a, b, review));

    // A failing implement guard holds builders with its reason, in the caller's folder, and
    // reads no quota; reviews never run it.
    let root = self::root();
    root.config(&format!(
        "{}{}",
        backend("cl", "claude", "['implement', 'review']", ""),
        project("strand", "[.roles.implement]\nguard = ['<ROOT>guard']\n"),
    ));
    let held = root.rota("repo/sub", &launch("g", &[]));
    assert_eq!(held.status.code(), Some(75), "{}", text(&held.stderr));
    assert!(
        text(&held.stderr)
            .contains("project strand: implement guard failed (exit 1): machine busy"),
        "{}",
        text(&held.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.p("guard-cwd")).unwrap().trim_end(),
        root.p("repo/sub").to_str().unwrap()
    );
    assert_eq!(root.reads(), 0);
    assert_eq!(
        root.last_decision()["hold"],
        "project strand: implement guard failed (exit 1): machine busy"
    );
    fs::remove_file(root.p("guard-cwd")).unwrap();
    let review = root.start("repo", &launch("r", &["--role", "review"]), "r");
    assert!(!root.p("guard-cwd").exists());
    drop(review);
}

#[test]
fn prompt_name_and_add_dirs_reach_argv_exactly() {
    let root = root();
    root.config(&format!(
        "{}{}",
        backend(
            "cl",
            "claude",
            "['implement']",
            "add_dir = ['--add-dir={dir}']"
        )
        .replace(
            "'{overlay}', '{prompt}'",
            "'-n', '{name}', '--session=x-{name}', '{overlay}', '{add_dirs}', '--', '{prompt}'"
        ),
        backend(
            "pair",
            "codex",
            "['review']",
            "add_dir = ['--add-dir', '{dir}']"
        )
        .replace("'{overlay}', '{prompt}'", "'{add_dirs}', '{prompt}'"),
    ));
    // A 6 KB prompt with shell metacharacters, placeholder text and newlines.
    let mut prompt = String::new();
    while prompt.len() < 6144 {
        prompt.push_str("`date` $(id) \"double\" 'single' {name} {add_dirs} {overlay} \\ $HOME\n");
    }
    fs::write(root.p("big-prompt"), &prompt).unwrap();
    let a = root.start(
        "elsewhere",
        &[
            "launch",
            "--name",
            "big",
            "--add-dir",
            "../repo",
            "--add-dir",
            "../w",
            "../big-prompt",
        ],
        "big",
    );
    let argv = root.argv("big");
    let repo = root.p("repo").display().to_string();
    let w = root.p("w").display().to_string();
    assert_eq!(
        argv,
        [
            "-n".to_string(),
            "big".into(),
            "--session=x-big".into(),
            format!("--add-dir={repo}"),
            format!("--add-dir={w}"),
            "--".into(),
            prompt.clone(),
        ]
    );
    // A two-part template repeats both parts; no --add-dir adds nothing.
    let b = root.start(
        "elsewhere",
        &[
            "launch",
            "--role",
            "review",
            "--name",
            "two",
            "--add-dir",
            "../repo",
            "--add-dir",
            "../w",
            "../prompt",
        ],
        "two",
    );
    assert_eq!(
        root.argv("two"),
        ["--add-dir", &repo, "--add-dir", &w, "do the task\n"]
    );
    let c = root.start("elsewhere", &launch("none", &["--role", "review"]), "none");
    assert_eq!(root.argv("none"), ["do the task\n"]);
    // An unprojected launch writes no record.
    assert!(root.records().is_empty());
    // Without --name the backend cannot take the launch at all: refused, not held.
    let refused = root.rota("elsewhere", &["launch", "../prompt"]);
    assert_eq!(refused.status.code(), Some(69));
    assert!(
        text(&refused.stderr)
            .contains("no backend can take this launch: cl: the high command needs --name"),
        "{}",
        text(&refused.stderr)
    );
    // --add-dir on a command without {add_dirs} is refused the same way.
    root.config(&backend("cl", "claude", "['implement']", ""));
    let refused = root.rota("elsewhere", &launch("x", &["--add-dir", "../w"]));
    assert_eq!(refused.status.code(), Some(69));
    assert!(text(&refused.stderr).contains("cl: the high command takes no --add-dir"));
    drop((a, b, c));
}

#[test]
fn overlays_follow_the_executable_never_the_account() {
    let root = root();
    // Two claude accounts and one codex; implement overlays name claude only.
    root.config(&format!(
        "{}{}{}{}",
        backend("cx", "codex", "['implement']", ""),
        backend("s", "claude", "['implement']", ""),
        backend("u", "claude", "['implement']", ""),
        project(
            "tide",
            "[.backends.s]\nmax = 1\n[.roles.implement.overlay]\nclaude = ['--settings={name}.json']\n"
        ),
    ));
    let first = root.start("repo", &launch("a", &[]), "a");
    // cx is skipped for its missing overlay, with the reason, while s admits with the overlay.
    assert!(
        root.stderr("a")
            .contains("cx [project tide role implement has no overlay for codex]"),
        "{}",
        root.stderr("a")
    );
    assert_eq!(root.argv("a"), ["--settings=a.json", "do the task\n"]);
    // s is at its cap: u, the other claude account, gets the same overlay.
    let second = root.start("repo", &launch("b", &[]), "b");
    assert!(
        root.stderr("b")
            .contains("s [project tide: 1 live agent on s")
    );
    assert_eq!(root.argv("b"), ["--settings=b.json", "do the task\n"]);
    assert_eq!(root.records().len(), 2);
    // Without --name nothing can take the launch.
    let refused = root.rota("repo", &["launch", "../prompt"]);
    assert_eq!(refused.status.code(), Some(69));
    assert!(text(&refused.stderr).contains("s: the high command needs --name"));
    drop((first, second));

    // `codex = []` admits codex unchanged; the implement fallback gets the overlay too.
    let root = self::root();
    root.config(&format!(
        "fallback = 'fb'\n{}{}{}",
        backend("cx", "codex", "['implement']", "enabled = false"),
        backend("fb", "claude", "[]", ""),
        project(
            "tide",
            "[.roles.implement.overlay]\nclaude = ['--fb']\ncodex = []\n"
        ),
    ));
    let fallback = root.start("repo", &launch("f", &[]), "f");
    assert_eq!(root.argv("f"), ["--fb", "do the task\n"]);
    drop(fallback);
    root.config(&format!(
        "{}{}",
        backend("cx", "codex", "['implement']", ""),
        project(
            "tide",
            "[.roles.implement.overlay]\nclaude = ['--fb']\ncodex = []\n"
        ),
    ));
    let plain = root.start("repo", &launch("p", &[]), "p");
    assert_eq!(root.argv("p"), ["do the task\n"]);
    drop(plain);
}

#[test]
fn leads_and_the_popup_are_never_capped() {
    let root = root();
    let config = |guard: &str| {
        format!(
            "{}{}",
            backend(
                "cl",
                "claude",
                "['implement', 'lead']",
                "popup_command = ['<ROOT>bin/claude', '<ROOT>out/popup']"
            ),
            project(
                "tide",
                &format!(
                    "max = 1\n[.backends.cl]\nmax = 1\n[.roles.implement]\nmax = 1\n{guard}\n\
                     [.roles.implement.overlay]\nclaude = ['--overlaid']\n"
                )
            ),
        )
    };
    // Seed one live implement agent before the guard exists, then fill every cap and guard.
    root.config(&config(""));
    let seeded = root.start("repo", &launch("a", &[]), "a");
    assert_eq!(root.argv("a"), ["--overlaid", "do the task\n"]);
    root.config(&config("guard = ['<ROOT>guard']"));
    let held = root.rota("repo", &launch("b", &[]));
    assert_eq!(held.status.code(), Some(75));
    // A lead launch in the full project admits, takes its role's (absent) overlay, and adds
    // no record.
    let lead = root.start("repo", &launch("lead", &["--role", "lead"]), "lead");
    assert_eq!(root.argv("lead"), ["do the task\n"]);
    assert_eq!(root.records().len(), 1);
    // The person's popup in the same repository selects no project: no line, no overlay,
    // no guard, no record.
    let popup = root.start("repo", &["launch", "--popup"], "popup");
    assert!(root.argv("popup").is_empty());
    assert!(!root.stderr("popup").contains("project:"));
    assert_eq!(root.records().len(), 1);
    let refused = root.rota("repo", &["launch", "--popup", "--project", "tide"]);
    assert_eq!(refused.status.code(), Some(2));
    drop((seeded, lead, popup));
}

#[test]
fn selection_by_flag_folder_and_worktree() {
    let root = root();
    root.config(&format!(
        "{}{}",
        backend("cl", "claude", "['implement']", ""),
        project("tide", "[.roles.implement.overlay]\nclaude = ['--tide']\n"),
    ));
    // A linked worktree of repo selects tide.
    let wt = root.start("w", &launch("wt", &[]), "wt");
    assert!(
        root.stderr("wt")
            .contains("project: tide (from the working directory's repository)")
    );
    assert_eq!(root.argv("wt"), ["--tide", "do the task\n"]);
    // A folder in no project: no project line, no overlay, no record for it.
    let out = root.start("elsewhere", &launch("out", &[]), "out");
    assert!(!root.stderr("out").contains("project:"));
    assert_eq!(root.argv("out"), ["do the task\n"]);
    assert_eq!(root.records().len(), 1);
    // --project selects from anywhere; an unknown one is refused.
    let flagged = root.start("elsewhere", &launch("fl", &["--project", "tide"]), "fl");
    assert!(root.stderr("fl").contains("project: tide (from --project)"));
    let unknown = root.rota("elsewhere", &launch("x", &["--project", "nope"]));
    assert_eq!(unknown.status.code(), Some(69));
    assert!(text(&unknown.stderr).contains("unknown project nope"));
    drop((wt, out, flagged));
}

/// A backend whose run command runs `bin/codex` with `out/{name}` and `{overlay}`.
fn run_backend(name: &str) -> String {
    format!(
        "{}[backends.run]\nstdin = 'brief'\nefforts = ['high']\ndefault_effort = 'high'\n\
         [backends.run.commands]\nimplement = ['<ROOT>bin/codex', '<ROOT>out/{{name}}', '{{overlay}}']\n",
        backend(name, "codex", "['implement']", "")
    )
}

#[test]
fn runs_count_by_supervisor_and_leader() {
    let root = root();
    root.config(&format!(
        "{}{}",
        run_backend("cx"),
        project(
            "tide",
            "max = 1\n[.roles.implement.overlay]\ncodex = ['--ran']\n"
        ),
    ));
    // Selected by --cwd in a worktree; the overlay and {name} reach the run's argv.
    let quick = root.rota(
        "elsewhere",
        &[
            "run",
            "--brief",
            "../prompt",
            "--name",
            "q-exit",
            "--timeout",
            "60",
            "--cwd",
            "../w",
        ],
    );
    assert!(
        text(&quick.stderr).contains("project: tide (from the working directory's repository)"),
        "{}",
        text(&quick.stderr)
    );
    assert_eq!(root.argv("q-exit"), ["--ran"]);
    // A live run holds the cap through its supervisor, and through its leader once the
    // supervisor is gone.
    let supervisor = root.start(
        "repo",
        &[
            "run",
            "--brief",
            "../prompt",
            "--name",
            "long",
            "--timeout",
            "300",
        ],
        "long",
    );
    // The leader joins the record after spawn, which may land after the leader writes argv.
    let until = Instant::now() + Duration::from_secs(20);
    let record = loop {
        let records = root.records();
        if let Some(record) = records
            .iter()
            .find(|r| r["processes"].as_array().unwrap().len() == 2)
        {
            break record.clone();
        }
        assert!(
            Instant::now() < until,
            "no record with a leader: {records:?}"
        );
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(record["processes"][0]["pid"], supervisor.pid());
    let leader = record["processes"][1]["pid"].as_u64().unwrap() as u32;
    let held = root.rota("repo", &launch("b", &[]));
    assert_eq!(held.status.code(), Some(75), "{}", text(&held.stderr));
    assert!(text(&held.stderr).contains(&format!("pids {})", supervisor.pid())));
    // A run is held at the cap itself, before any quota read, and logs why.
    let reads = root.reads();
    let run = root.rota(
        "repo",
        &[
            "run",
            "--brief",
            "../prompt",
            "--name",
            "r2",
            "--timeout",
            "60",
        ],
    );
    assert_eq!(run.status.code(), Some(75), "{}", text(&run.stderr));
    assert!(
        text(&run.stderr).contains("project tide: 1 live agent (max 1; pids"),
        "{}",
        text(&run.stderr)
    );
    assert_eq!(root.reads(), reads);
    let logged = root.last_decision();
    assert_eq!(logged["project"], "tide");
    assert!(
        logged["hold"]
            .as_str()
            .unwrap()
            .starts_with("project tide: 1 live agent")
    );
    // With the supervisor gone, the leader alone holds the cap, and the hold names it.
    drop(supervisor);
    let held = root.rota("repo", &launch("b", &[]));
    assert_eq!(held.status.code(), Some(75), "{}", text(&held.stderr));
    assert!(
        text(&held.stderr).contains(&format!("(max 1; pids {leader})")),
        "{}",
        text(&held.stderr)
    );
    kill_pid(leader);
    let after = root.start("repo", &launch("c", &[]), "c");
    drop(after);
}

#[test]
fn the_admitted_project_holds_when_prepare_moves_discovery() {
    // `repo/sub` is tide's until a `.git` appears in it, which makes it project inner's. The
    // backend's prepare hook creates that `.git` after admission: the launch and the run keep
    // tide's overlay and record, the project whose guards and caps admitted them.
    for run in [false, true] {
        let root = root();
        root.config(&format!(
            "{}[backends.run]\nstdin = 'brief'\nefforts = ['high']\ndefault_effort = 'high'\n\
             [backends.run.commands]\nimplement = ['<ROOT>bin/codex', '<ROOT>out/{{name}}', '{{overlay}}']\n\
             {}\n[projects.inner]\nrepo = '<ROOT>repo/sub'\nagents_root = '<ROOT>agents'\n\
             [projects.inner.roles.implement.overlay]\ncodex = ['--inner']\n",
            backend(
                "cx",
                "codex",
                "['implement']",
                "prepare = [['mkdir', '-p', '.git']]"
            ),
            project("tide", "[.roles.implement.overlay]\ncodex = ['--tide']\n"),
        ));
        if run {
            let output = root.rota(
                "repo/sub",
                &[
                    "run",
                    "--brief",
                    "../prompt",
                    "--name",
                    "r-exit",
                    "--timeout",
                    "60",
                ],
            );
            assert!(
                text(&output.stderr).contains("project: tide"),
                "{}",
                text(&output.stderr)
            );
            assert_eq!(root.argv("r-exit"), ["--tide"]);
        } else {
            let agent = root.start("repo/sub", &launch("a", &[]), "a");
            assert!(
                root.stderr("a").contains("project: tide"),
                "{}",
                root.stderr("a")
            );
            assert_eq!(root.argv("a"), ["--tide", "do the task\n"]);
            let records = root.records();
            assert_eq!(records.len(), 1, "{records:?}");
            assert_eq!(records[0]["project"], "tide");
            drop(agent);
        }
        assert!(root.p("repo/sub/.git").is_dir(), "the prepare hook ran");
    }
}

#[test]
fn run_takes_project_and_role_guard() {
    let root = root();
    root.config(&format!(
        "{}{}",
        run_backend("cx"),
        project("tide", "[.roles.implement.overlay]\ncodex = ['--ran']\n"),
    ));
    // --project selects from a folder in no repository.
    let flagged = root.rota(
        "elsewhere",
        &[
            "run",
            "--project",
            "tide",
            "--brief",
            "../prompt",
            "--name",
            "f-exit",
            "--timeout",
            "60",
        ],
    );
    assert!(
        text(&flagged.stderr).contains("project: tide (from --project)"),
        "{}",
        text(&flagged.stderr)
    );
    assert_eq!(root.argv("f-exit"), ["--ran"]);
    // A role guard holds a run too, with its reason and no quota read.
    root.config(&format!(
        "{}{}",
        run_backend("cx"),
        project(
            "tide",
            "[.roles.implement]\nguard = ['<ROOT>guard']\n[.roles.implement.overlay]\ncodex = []\n"
        ),
    ));
    let reads = root.reads();
    let held = root.rota(
        "repo",
        &[
            "run",
            "--brief",
            "../prompt",
            "--name",
            "g",
            "--timeout",
            "60",
        ],
    );
    assert_eq!(held.status.code(), Some(75), "{}", text(&held.stderr));
    assert!(
        text(&held.stderr).contains("project tide: implement guard failed (exit 1): machine busy"),
        "{}",
        text(&held.stderr)
    );
    assert_eq!(root.reads(), reads);
}
