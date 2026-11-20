use super::*;
use std::os::unix::fs::symlink;

const EXAMPLE: &str = include_str!("../../config.example.toml");

fn parse(extra: &str) -> Result<Config, String> {
    Config::parse(&format!("{EXAMPLE}\n{extra}"), "rota")
}

fn project(name: &str, repo: &Path, extra: &str) -> String {
    format!(
        "[projects.{name}]\nrepo = {:?}\nagents_root = \"/PATH/TO/agents\"\n{extra}\n",
        repo.to_str().unwrap()
    )
}

#[test]
fn validation_names_each_bad_key() {
    for (extra, error) in [
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\nmax = 0",
            "projects.p.max must be at least 1",
        ),
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.backends.nope]\nmax = 1",
            "projects.p.backends.nope is not a configured backend",
        ),
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.backends.code]\nmax = 0",
            "projects.p.backends.code.max must be at least 1",
        ),
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.lead]\nmax = 1",
            "projects.p.roles.lead.max is invalid: leads are not capped by projects",
        ),
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.review]\nmax = 0",
            "projects.p.roles.review.max must be at least 1",
        ),
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.review]\nguard = []",
            "projects.p.roles.review.guard needs a nonempty command without NUL",
        ),
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.review.overlay]\n'..' = []",
            "projects.p.roles.review.overlay...: overlay keys are executable base names",
        ),
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.review.overlay]\n'bin/x' = []",
            "overlay keys are executable base names",
        ),
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.review.overlay]\nagy = ['{prompt}']",
            "overlay arguments take no NUL",
        ),
        // The example's agy and claude level commands have no {overlay}.
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.review.overlay]\nagy = ['-x']",
            "projects.p.roles.review.overlay.agy: backend alpha's medium command has no {overlay} argument",
        ),
        // The fallback (`main`, no roles) is reached by implement.
        (
            "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.implement.overlay]\nclaude = []",
            "backend secondary's medium command has no {overlay} argument",
        ),
    ] {
        let error_text = parse(extra).unwrap_err();
        assert!(error_text.contains(error), "{extra}\n=> {error_text}");
    }
}

#[test]
fn overlay_coverage_reaches_the_fallback_and_run_only_commands() {
    // Give every claude level command {overlay} except the fallback's.
    let text = EXAMPLE.replacen(
        "medium = [\"claude\", \"{prompt}\"]\nhigh = [\"claude\", \"{prompt}\"]",
        "medium = [\"claude\", \"{overlay}\", \"{prompt}\"]\nhigh = [\"claude\", \"{overlay}\", \"{prompt}\"]",
        1,
    );
    let profile = "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.implement.overlay]\nclaude = []";
    let error = Config::parse(&format!("{text}\n{profile}"), "rota").unwrap_err();
    assert!(
        error.contains("backend main's medium command has no {overlay} argument"),
        "{error}"
    );
    // `code` serves review only through run.commands.review: still reached.
    let profile = "[projects.p]\nrepo = '/r'\nagents_root = '/a'\n[projects.p.roles.review.overlay]\ncodex = []";
    let error = parse(profile).unwrap_err();
    assert!(
        error.contains("backend code's run review command has no {overlay} argument"),
        "{error}"
    );
}

#[test]
fn selection_by_flag_repository_and_worktree() {
    let root = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(root.path()).unwrap();
    let repo = root.join("repo");
    fs::create_dir_all(repo.join(".git/worktrees/w")).unwrap();
    fs::create_dir_all(repo.join("src/deep")).unwrap();
    fs::write(repo.join(".git/worktrees/w/commondir"), "../..\n").unwrap();
    let worktree = root.join("w");
    fs::create_dir_all(worktree.join("sub")).unwrap();
    fs::write(worktree.join(".git"), "gitdir: ../repo/.git/worktrees/w\n").unwrap();
    let link = root.join("repo-link");
    symlink(&repo, &link).unwrap();
    // `repo` is compared after canonicalizing, so a symlinked spelling matches.
    let config = parse(&project("tide", &link, "")).unwrap();
    for dir in [
        repo.clone(),
        repo.join("src/deep"),
        worktree.clone(),
        worktree.join("sub"),
    ] {
        let selected = select(&config, None, &dir).unwrap().unwrap();
        assert_eq!(selected.name, "tide", "{}", dir.display());
        assert!(!selected.from_flag);
        assert_eq!(
            selected.line(),
            "project: tide (from the working directory's repository)"
        );
    }
    let other = root.join("other");
    fs::create_dir_all(other.join(".git")).unwrap();
    assert!(select(&config, None, &other).unwrap().is_none());
    let flagged = select(&config, Some("alpha"), &other).unwrap().unwrap();
    assert_eq!(flagged.line(), "project: alpha (from --project)");
    assert_eq!(
        select(&config, Some("nope"), &other).err().unwrap(),
        (2, "unknown project nope".into())
    );
    // Two projects on one repository: legal for retire, ambiguous for selection.
    let config = parse(&format!(
        "{}{}",
        project("tide", &repo, ""),
        project("tide2", &link, "")
    ))
    .unwrap();
    let (code, message) = select(&config, None, &repo).err().unwrap();
    assert_eq!(code, 2);
    assert!(
        message.starts_with("projects tide and tide2 both hold repository")
            && message.ends_with("; pass --project"),
        "{message}"
    );
    assert_eq!(
        select(&config, Some("tide2"), &repo).unwrap().unwrap().name,
        "tide2"
    );
}

#[test]
fn repository_of_follows_each_kind_of_git_file() {
    let root = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(root.path()).unwrap();
    let repo = root.join("repo");
    fs::create_dir_all(repo.join(".git/worktrees/abs")).unwrap();
    fs::create_dir_all(repo.join(".git/modules/sub")).unwrap();
    fs::write(repo.join(".git/worktrees/abs/commondir"), "../..").unwrap();
    // Absolute gitdir.
    let abs = root.join("abs");
    fs::create_dir_all(&abs).unwrap();
    fs::write(
        abs.join(".git"),
        format!("gitdir: {}\n", repo.join(".git/worktrees/abs").display()),
    )
    .unwrap();
    assert_eq!(repository_of(&abs), Some(repo.clone()));
    // A submodule: a .git file without commondir is its own checkout.
    let sub = repo.join("sub");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join(".git"), "gitdir: ../.git/modules/sub\n").unwrap();
    assert_eq!(repository_of(&sub), Some(sub.clone()));
    // A worktree of a bare repository: the common dir is the repository.
    let bare = root.join("bare.git");
    fs::create_dir_all(bare.join("worktrees/b")).unwrap();
    fs::write(bare.join("worktrees/b/commondir"), "../..\n").unwrap();
    let wb = root.join("wb");
    fs::create_dir_all(&wb).unwrap();
    fs::write(wb.join(".git"), "gitdir: ../bare.git/worktrees/b").unwrap();
    assert_eq!(repository_of(&wb), Some(bare));
    // A .git file that is not a gitdir pointer, or points nowhere: the folder itself.
    let odd = root.join("odd");
    fs::create_dir_all(&odd).unwrap();
    fs::write(odd.join(".git"), "nonsense").unwrap();
    assert_eq!(repository_of(&odd), Some(odd.clone()));
    fs::write(odd.join(".git"), "gitdir: missing").unwrap();
    assert_eq!(repository_of(&odd), Some(odd.clone()));
    // No .git in the folder: whatever holds the tempdir decides, as for its parent.
    let plain = root.join("plain");
    fs::create_dir_all(&plain).unwrap();
    assert_eq!(repository_of(&plain), repository_of(&root));
}

const EUID: u32 = 4242;
const NS: u64 = 7;

struct FakeProc {
    _dir: tempfile::TempDir,
    proc: PathBuf,
    records: PathBuf,
}

impl FakeProc {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let proc = dir.path().join("proc");
        let records = dir.path().join("agents");
        fs::create_dir_all(&proc).unwrap();
        fs::create_dir_all(&records).unwrap();
        Self {
            _dir: dir,
            proc,
            records,
        }
    }

    fn scanner(&self) -> Scanner {
        Scanner {
            proc: self.proc.clone(),
            euid: EUID,
            pid_ns: NS,
        }
    }

    /// A process with a command name holding spaces and parentheses.
    fn process(&self, pid: u32, state: char, start: u64, euid: u32) {
        let dir = self.proc.join(pid.to_string());
        fs::create_dir_all(&dir).unwrap();
        let fields = "0 ".repeat(18);
        fs::write(
            dir.join("stat"),
            format!("{pid} (a (b) c) {state} {fields}{start} 0 0\n"),
        )
        .unwrap();
        fs::write(
            dir.join("status"),
            format!("Name:\tx\nUid:\t1000\t{euid}\t{euid}\t{euid}\n"),
        )
        .unwrap();
    }

    fn record(&self, id: &str, role: Role, backend: &str, ns: u64, processes: &[(u32, u64)]) {
        let record = Record {
            schema: SCHEMA,
            id: id.into(),
            project: "tide".into(),
            role,
            backend: backend.into(),
            pid_ns: ns,
            processes: processes
                .iter()
                .map(|&(pid, start_ticks)| Process { pid, start_ticks })
                .collect(),
            started: processes[0].0.into(),
        };
        fs::write(
            self.records.join(format!("{id}.json")),
            serde_json::to_vec(&record).unwrap(),
        )
        .unwrap();
    }

    fn ids(&self, prune: bool) -> Vec<String> {
        scan(&self.records, &self.scanner(), prune)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    }

    fn exists(&self, id: &str) -> bool {
        self.records.join(format!("{id}.json")).exists()
    }
}

#[test]
fn live_is_the_admitted_process() {
    let fake = FakeProc::new();
    // Live: matching start ticks, running, our effective uid. The test's /proc directories are
    // owned by the test user, not EUID, standing in for a non-dumpable agent's root-owned one.
    fake.process(10, 'S', 500, EUID);
    fake.record("live", Role::Implement, "code", NS, &[(10, 500)]);
    // Gone: a zombie, another start time (PID reuse), another user, no such PID.
    fake.process(11, 'Z', 501, EUID);
    fake.record("zombie", Role::Implement, "code", NS, &[(11, 501)]);
    fake.process(12, 'S', 999, EUID);
    fake.record("reused", Role::Implement, "code", NS, &[(12, 502)]);
    fake.process(13, 'R', 503, EUID + 1);
    fake.record("other-user", Role::Review, "code", NS, &[(13, 503)]);
    fake.record("exited", Role::Review, "code", NS, &[(14, 504)]);
    // A run whose supervisor died while its leader runs on.
    fake.process(16, 'S', 506, EUID);
    fake.record("leader", Role::Review, "code", NS, &[(15, 505), (16, 506)]);
    // Another PID namespace: unjudgeable here, so it counts and is never pruned.
    fake.record("jailed", Role::Review, "code", NS + 1, &[(17, 507)]);
    // Temporaries and other files are not records.
    fs::write(fake.records.join(".tmpXYZ"), "{").unwrap();
    fs::write(fake.records.join("notes.txt"), "{").unwrap();
    let mut live = fake.ids(false);
    live.sort();
    assert_eq!(live, ["jailed", "leader", "live"]);
    // Without pruning (dry run) nothing is removed.
    assert!(fake.exists("zombie") && fake.exists("exited"));
    let mut live = fake.ids(true);
    live.sort();
    assert_eq!(live, ["jailed", "leader", "live"]);
    // A record keeps only its live processes, so a hold names the leader, not the dead
    // supervisor; a record from another namespace keeps all of its own.
    let records = scan(&fake.records, &fake.scanner(), false).unwrap();
    let processes = |id: &str| {
        let record = records.iter().find(|r| r.id == id).unwrap();
        record.processes.iter().map(|p| p.pid).collect::<Vec<_>>()
    };
    assert_eq!(processes("leader"), [16]);
    assert_eq!(processes("jailed"), [17]);
    for gone in ["zombie", "reused", "other-user", "exited"] {
        assert!(!fake.exists(gone), "{gone}");
    }
    assert!(fake.exists("jailed") && fake.records.join(".tmpXYZ").exists());
}

#[test]
fn records_that_cannot_be_judged_hold_and_stay() {
    for (setup, error) in [
        ("corrupt", "unreadable live-agent record"),
        ("schema", "unknown schema Some(2)"),
        ("stat-dir", "cannot scan /proc for project caps"),
        (
            "stat-garbled",
            "cannot scan /proc for project caps: cannot parse",
        ),
        (
            "status-garbled",
            "cannot scan /proc for project caps: cannot parse",
        ),
    ] {
        let fake = FakeProc::new();
        fake.process(10, 'S', 500, EUID);
        fake.record("r", Role::Implement, "code", NS, &[(10, 500)]);
        // Gone records beside the bad one stay too: nothing is pruned until every record is
        // judged, whatever order the directory lists them in.
        for gone in ["a", "m", "z"] {
            fake.record(gone, Role::Implement, "code", NS, &[(20, 1)]);
        }
        let path = fake.records.join("r.json");
        match setup {
            "corrupt" => fs::write(&path, "{not json").unwrap(),
            "schema" => fs::write(&path, r#"{"schema":2}"#).unwrap(),
            "stat-dir" => {
                fs::remove_file(fake.proc.join("10/stat")).unwrap();
                fs::create_dir(fake.proc.join("10/stat")).unwrap();
            }
            "stat-garbled" => fs::write(fake.proc.join("10/stat"), "10 (x) S 1").unwrap(),
            _ => fs::write(fake.proc.join("10/status"), "Uid:\tnope\n").unwrap(),
        }
        let found = scan(&fake.records, &fake.scanner(), true).unwrap_err();
        assert!(found.contains(error), "{setup}: {found}");
        assert!(path.exists(), "{setup}: kept");
        for gone in ["a", "m", "z"] {
            assert!(fake.exists(gone), "{setup}: {gone} kept");
        }
    }
    // No records directory yet: nothing is live.
    let scanner = FakeProc::new().scanner();
    assert!(
        scan(Path::new("/nonexistent/agents"), &scanner, true)
            .unwrap()
            .is_empty()
    );
}

fn selected<'a>(config: &'a Config, name: &'a str) -> Selected<'a> {
    Selected {
        name,
        project: &config.projects[name],
        from_flag: true,
    }
}

fn records(spec: &[(Role, &str, u32)]) -> Vec<Record> {
    spec.iter()
        .map(|&(role, backend, pid)| Record {
            schema: SCHEMA,
            id: pid.to_string(),
            project: "tide".into(),
            role,
            backend: backend.into(),
            pid_ns: NS,
            processes: vec![Process {
                pid,
                start_ticks: 1,
            }],
            started: pid.into(),
        })
        .collect()
}

#[test]
fn holds_name_the_cap_and_the_live_pids() {
    let config = parse(&project(
        "tide",
        Path::new("/r"),
        "max = 2\n[tide.backends.code]\nmax = 1\n[tide.roles.implement]\nmax = 1\n"
            .replace("[tide.", "[projects.tide.")
            .as_str(),
    ))
    .unwrap();
    let tide = selected(&config, "tide");
    let one = records(&[(Role::Implement, "code", 41)]);
    assert_eq!(
        hold(&tide, Role::Implement, &one).unwrap(),
        "project tide: 1 live implement agent (roles.implement.max 1; pids 41)"
    );
    assert_eq!(hold(&tide, Role::Review, &one), None);
    assert_eq!(
        backend_hold(&tide, Role::Review, "code", &one).unwrap(),
        "project tide: 1 live agent on code (backends.code.max 1; pids 41)"
    );
    assert_eq!(backend_hold(&tide, Role::Review, "secondary", &one), None);
    let two = records(&[
        (Role::Implement, "code", 41),
        (Role::Review, "secondary", 42),
    ]);
    assert_eq!(
        hold(&tide, Role::Review, &two).unwrap(),
        "project tide: 2 live agents (max 2; pids 41, 42)"
    );
    // Other projects' agents do not count; leads are never held.
    let mut others = records(&[(Role::Implement, "code", 43), (Role::Review, "code", 44)]);
    for record in &mut others {
        record.project = "strand".into();
    }
    assert_eq!(hold(&tide, Role::Implement, &others), None);
    assert_eq!(hold(&tide, Role::Lead, &two), None);
    assert_eq!(backend_hold(&tide, Role::Lead, "code", &two), None);
}

#[test]
fn overlays_by_executable() {
    let text = EXAMPLE
        .replace("\"--sandbox\"]", "\"--sandbox\", \"{overlay}\"]")
        .replace(
            "[\"claude\", \"{prompt}\"]",
            "[\"claude\", \"{overlay}\", \"{prompt}\"]",
        );
    let profile = "[projects.tide]\nrepo = '/r'\nagents_root = '/a'\n\
                   [projects.tide.roles.review.overlay]\nclaude = ['--settings', '{cwd}/s.json']\nagy = []\n";
    let config = Config::parse(&format!("{text}\n{profile}"), "rota").unwrap();
    let tide = selected(&config, "tide");
    let claude: Vec<String> = ["claude", "{overlay}", "{prompt}"].map(String::from).into();
    assert_eq!(
        overlay(Some(&tide), Role::Review, &claude).unwrap(),
        ["--settings", "{cwd}/s.json"]
    );
    assert!(
        overlay(Some(&tide), Role::Review, &["agy".to_string()])
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        overlay(Some(&tide), Role::Review, &["codex".to_string()]).unwrap_err(),
        "project tide role review has no overlay for codex"
    );
    // No overlays for the role, or no project: nothing, never a refusal.
    assert!(
        overlay(Some(&tide), Role::Implement, &["codex".to_string()])
            .unwrap()
            .is_empty()
    );
    assert!(
        overlay(None, Role::Review, &["codex".to_string()])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn an_admission_that_never_runs_leaves_no_record() {
    let fake = FakeProc::new();
    let me = std::process::id();
    fake.process(me, 'S', 7, EUID);
    fake.process(me + 1, 'S', 8, EUID);
    symlink(me.to_string(), fake.proc.join("self")).unwrap();
    let anchor = fake.records.parent().unwrap().join("anchor");
    let scanner = fake.scanner();
    // A failed exec or spawn returns early and drops the record.
    let admitted = Admitted::write(&anchor, "tide", Role::Implement, "code", &scanner).unwrap();
    let path = admitted.path.clone();
    assert!(path.exists());
    // The project name is in the record, never in its path.
    assert_eq!(path.parent().unwrap(), fake.records);
    assert!(!path.file_name().unwrap().to_str().unwrap().contains("tide"));
    drop(admitted);
    assert!(!path.exists());
    // A run adds its leader and keeps the record once started.
    let mut admitted = Admitted::write(&anchor, "tide", Role::Review, "code", &scanner).unwrap();
    let path = admitted.path.clone();
    admitted.add(scanner.identity(me + 1).unwrap()).unwrap();
    admitted.keep();
    let record: Record = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        record.processes,
        [
            Process {
                pid: me,
                start_ticks: 7
            },
            Process {
                pid: me + 1,
                start_ticks: 8
            }
        ]
    );
    assert_eq!(fake.ids(false).len(), 1);
}
