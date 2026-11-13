use super::*;
use std::fs as disk;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::os::unix::process::ExitStatusExt;

fn status(uid: u32, state: &str) -> String {
    format!("Uid:\t{uid}\t{uid}\t{uid}\t{uid}\nState:\t{state}\n")
}

struct Fixture {
    temp: tempfile::TempDir,
    config: Config,
    options: Options,
    proc: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path();
        disk::create_dir(base.join("agents")).unwrap();
        disk::create_dir(base.join("proc")).unwrap();
        let config = Self::config(base);
        let proc = base.join("proc");
        Self {
            temp,
            config,
            options: Options {
                name: "a".into(),
                bead: Some("task-1".into()),
                ..Options::default()
            },
            proc,
        }
    }
    fn config(base: &Path) -> Config {
        let mut config = Config::parse(include_str!("../../config.example.toml")).unwrap();
        config.state_file = base.join("state/last");
        config.log_file = base.join("state/log");
        config.projects.clear();
        config.projects.insert(
            "alpha".into(),
            Project {
                repo: base.into(),
                agents_root: base.join("agents"),
                done: Some(Done {
                    command: vec![
                        "printf".into(),
                        "{\"status\":\"closed\",\"bead\":\"{bead}\"}".into(),
                    ],
                    pointers: vec!["/status".into()],
                    equals: "closed".into(),
                }),
            },
        );
        config.cleanup.store_prefixes = vec!["/fictional/store".into()];
        config
    }
    fn root(&self) -> PathBuf {
        self.temp.path().join("agents")
    }
    fn target(&self, relative: &str) -> PathBuf {
        let path = self.root().join(relative);
        disk::create_dir_all(path.join("debug/deps")).unwrap();
        disk::write(path.join(TAG), SIGNATURE).unwrap();
        disk::write(path.join("debug/deps/output"), b"build output").unwrap();
        path
    }
    fn process(&self, pid: u32, cwd: &Path, exe: &Path, env: &[u8]) {
        let path = self.proc.join(pid.to_string());
        disk::create_dir(&path).unwrap();
        disk::write(path.join("status"), status(unsafe { libc::geteuid() }, "S")).unwrap();
        symlink(cwd, path.join("cwd")).unwrap();
        symlink(exe, path.join("exe")).unwrap();
        disk::write(path.join("environ"), env).unwrap();
        disk::create_dir(path.join("fd")).unwrap();
        disk::write(path.join("maps"), b"").unwrap();
    }
    fn run(&self) -> (io::Result<u8>, Report) {
        let project = &self.config.projects["alpha"];
        let mut report = Report {
            name: self.options.name.clone(),
            project: "alpha".into(),
            bead: self.options.bead.clone(),
            dry_run: self.options.dry_run,
            target_only: self.options.target_only,
            done: "not checked".into(),
            error: None,
            freed_bytes: 0,
            paths: Vec::new(),
        };
        let result = execute(
            &self.config,
            project,
            &self.options,
            &self.proc,
            &mut report,
        );
        (result, report)
    }
}

fn hook(f: impl FnMut(&str) -> io::Result<()> + 'static) {
    fs::HOOK.with_borrow_mut(|h| *h = Some(Box::new(f)));
}
fn clear_hook() {
    fs::HOOK.with_borrow_mut(|h| *h = None);
}

fn age(path: &Path, seconds: i64) {
    if path.is_dir() && !path.is_symlink() {
        for entry in disk::read_dir(path).unwrap() {
            age(&entry.unwrap().path(), seconds);
        }
    }
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds as u64);
    disk::File::open(path)
        .unwrap()
        .set_times(disk::FileTimes::new().set_modified(time))
        .unwrap();
}

#[test]
fn evidence_and_tagged_targets() {
    let f = Fixture::new();
    let target = f.target("a-target");
    f.target("a-work/target");
    let work = f.root().join("a-work");
    disk::create_dir(work.join("nested")).unwrap();
    let large = vec![42; 2_000_000];
    disk::write(work.join("large"), &large).unwrap();
    disk::write(work.join("nested/evidence"), b"proof").unwrap();
    disk::set_permissions(
        work.join("nested/evidence"),
        disk::Permissions::from_mode(0o444),
    )
    .unwrap();
    disk::hard_link(work.join("nested/evidence"), work.join("hardlink")).unwrap();
    symlink("/fictional/store/build", work.join("store")).unwrap();
    symlink("/fictional/store2/build", work.join("other")).unwrap();
    symlink("/fictional/store/../evidence", work.join("traversal")).unwrap();
    disk::create_dir(f.root().join("a")).unwrap();
    disk::write(f.root().join("a/untouched"), b"source").unwrap();
    for name in [
        "a-prompt.md",
        "a-launch.sh",
        "a-prompt-notes.md",
        "a-launchpad-launch.sh",
    ] {
        disk::write(f.root().join(name), b"keep exact names").unwrap();
    }
    disk::set_permissions(
        target.join("debug/deps"),
        disk::Permissions::from_mode(0o555),
    )
    .unwrap();
    let (result, report) = f.run();
    assert_eq!(result.unwrap(), 0);
    assert!(!target.exists());
    assert!(!work.join("target").exists());
    assert_eq!(disk::read(work.join("large")).unwrap(), large);
    assert_eq!(disk::read(work.join("nested/evidence")).unwrap(), b"proof");
    assert_eq!(disk::read(work.join("hardlink")).unwrap(), b"proof");
    assert_eq!(disk::read(f.root().join("a/untouched")).unwrap(), b"source");
    assert!(!work.join("store").is_symlink());
    assert!(work.join("other").is_symlink());
    assert!(work.join("traversal").is_symlink());
    assert!(!f.root().join("a-prompt.md").exists());
    assert!(!f.root().join("a-launch.sh").exists());
    assert!(f.root().join("a-prompt-notes.md").exists());
    assert!(f.root().join("a-launchpad-launch.sh").exists());
    assert_eq!(
        report.freed_bytes,
        report
            .paths
            .iter()
            .filter(|p| p.outcome == "removed")
            .map(|p| p.bytes)
            .sum::<u64>()
    );
    assert!(report.freed_bytes > 0);
}

#[test]
fn unsafe_targets_and_symlinked_tags_are_kept() {
    for kind in [
        "missing",
        "wrong",
        "symlink",
        "directory",
        "target_link",
        "target_file",
        "work_link",
    ] {
        let f = Fixture::new();
        let target = f.target("a-target");
        let outside = f.temp.path().join("outside");
        disk::create_dir(&outside).unwrap();
        disk::write(outside.join(TAG), SIGNATURE).unwrap();
        disk::write(outside.join("evidence"), b"keep").unwrap();
        match kind {
            "missing" => disk::remove_file(target.join(TAG)).unwrap(),
            "wrong" => disk::write(target.join(TAG), b"wrong").unwrap(),
            "symlink" => {
                disk::remove_file(target.join(TAG)).unwrap();
                symlink(outside.join(TAG), target.join(TAG)).unwrap();
            }
            "directory" => {
                disk::remove_file(target.join(TAG)).unwrap();
                disk::create_dir(target.join(TAG)).unwrap();
            }
            "target_link" => {
                disk::remove_dir_all(&target).unwrap();
                symlink(&outside, &target).unwrap();
            }
            "target_file" => {
                disk::remove_dir_all(&target).unwrap();
                disk::write(&target, b"evidence").unwrap();
            }
            "work_link" => {
                symlink(&outside, f.root().join("a-work")).unwrap();
            }
            _ => unreachable!(),
        }
        assert_eq!(f.run().0.unwrap(), 3, "{kind}");
        assert_eq!(disk::read(outside.join("evidence")).unwrap(), b"keep");
    }
    let f = Fixture::new();
    f.target("a-target");
    let alias = f.temp.path().join("alias");
    symlink(f.root(), &alias).unwrap();
    assert!(Dir::absolute(&alias).is_err());
}

#[test]
fn live_session_in_both_modes_and_all_locations() {
    for target_only in [false, true] {
        for location in [
            "worktree",
            "work",
            "target",
            "exe",
            "absolute",
            "relative",
            "alias",
            "alias_missing",
            "cwd_alias",
        ] {
            let mut f = Fixture::new();
            f.options.target_only = target_only;
            let target = f.target("a-target");
            let work = f.root().join("a-work");
            disk::create_dir(&work).unwrap();
            let wt = f.root().join("a");
            disk::create_dir(&wt).unwrap();
            let mut cwd = f.temp.path().to_path_buf();
            let mut exe = PathBuf::from("/fictional/bin/agent");
            let mut env = Vec::new();
            match location {
                "worktree" => cwd = wt,
                "work" => cwd = work,
                "target" => cwd = target.clone(),
                "exe" => exe = target.join("binary"),
                "absolute" => env = format!("CARGO_TARGET_DIR={}\0", target.display()).into_bytes(),
                "relative" => {
                    cwd = f.root();
                    env = b"CARGO_TARGET_DIR=a-target\0".to_vec();
                }
                "alias" | "alias_missing" => {
                    let alias = f.temp.path().join("alias");
                    symlink(&target, &alias).unwrap();
                    let path = if location == "alias_missing" {
                        alias.join("not-created-yet")
                    } else {
                        alias
                    };
                    env = format!("CARGO_TARGET_DIR={}\0", path.display()).into_bytes();
                }
                "cwd_alias" => {
                    let alias = f.temp.path().join("alias");
                    symlink(&target, &alias).unwrap();
                    cwd = alias.join("debug/deps");
                }
                _ => unreachable!(),
            }
            f.process(4242, &cwd, &exe, &env);
            assert_eq!(f.run().0.unwrap(), 3, "{target_only} {location}");
            assert!(target.join(TAG).exists());
            assert!(target.join("debug/deps/output").exists());
        }
    }
}

#[test]
fn own_traversal_descriptors_do_not_block_cleanup_or_dry_run() {
    use std::os::fd::AsRawFd;
    for target_only in [false, true] {
        for dry_run in [false, true] {
            let mut f = Fixture::new();
            f.options.target_only = target_only;
            f.options.dry_run = dry_run;
            let target = f.target("a-work/target");
            let work = f.root().join("a-work");
            symlink("/fictional/store/output", work.join("result")).unwrap();
            let held = disk::File::open(&work).unwrap();
            assert_eq!(
                disk::read_link(format!("/proc/self/fd/{}", held.as_raw_fd())).unwrap(),
                work
            );
            symlink(
                format!("/proc/{}", std::process::id()),
                f.proc.join(std::process::id().to_string()),
            )
            .unwrap();
            let (result, report) = f.run();
            assert_eq!(
                result.unwrap(),
                0,
                "target_only={target_only} dry_run={dry_run}"
            );
            assert_eq!(target.exists(), dry_run);
            assert!(report.paths.iter().any(|p| p.kind == "target"
                && p.outcome == if dry_run { "would_remove" } else { "removed" }));
            assert!(held.metadata().unwrap().is_dir());
        }
    }
}

#[test]
fn empty_targets_finish_but_nonempty_untagged_and_live_targets_stay() {
    for target_only in [false, true] {
        for relative in ["a-target", "a-work/target"] {
            let mut f = Fixture::new();
            f.options.target_only = target_only;
            if target_only {
                f.options.bead = None;
            }
            let target = f.root().join(relative);
            disk::create_dir_all(&target).unwrap();
            assert_eq!(f.run().0.unwrap(), 0);
            assert!(!target.exists());
            disk::create_dir_all(&target).unwrap();
            disk::write(target.join("evidence"), b"keep").unwrap();
            assert_eq!(f.run().0.unwrap(), 3);
            assert_eq!(disk::read(target.join("evidence")).unwrap(), b"keep");
            disk::remove_file(target.join("evidence")).unwrap();
            f.process(4242, &target, Path::new("/fictional/bin/agent"), b"");
            assert_eq!(f.run().0.unwrap(), 3);
            assert!(target.exists());
        }
    }
}

#[test]
fn empty_target_completion_cannot_remove_a_late_file() {
    let f = Fixture::new();
    let target = f.root().join("a-target");
    disk::create_dir(&target).unwrap();
    let late = target.join("evidence");
    let write = late.clone();
    hook(move |at| {
        if at == "rmdir" {
            disk::write(&write, b"late evidence")?;
        }
        Ok(())
    });
    let result = f.run().0;
    clear_hook();
    assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::ENOTEMPTY));
    assert_eq!(disk::read(late).unwrap(), b"late evidence");
    assert_eq!(f.run().0.unwrap(), 3);
}

#[test]
fn registry_live_outside_folders_and_errors_keep_all() {
    let f = Fixture::new();
    let target = f.target("a-target");
    f.process(4242, f.temp.path(), Path::new("/fictional/bin/agent"), b"");
    let records = f.temp.path().join("state/sessions");
    disk::create_dir_all(&records).unwrap();
    let record = serde_json::json!({"schema":1,"id":"a-1","name":"a","kind":"run","backend":"fake",
        "role":"review","cwd":"/fictional/elsewhere","brief":"/fictional/brief","pid":4242,"rota_pid":1,"started":1});
    disk::write(records.join("a-1.json"), record.to_string()).unwrap();
    assert_eq!(f.run().0.unwrap(), 3);
    disk::write(
        f.proc.join("4242/status"),
        status(unsafe { libc::geteuid() }.wrapping_add(1), "S"),
    )
    .unwrap();
    assert_eq!(f.run().0.unwrap(), 3);
    disk::write(records.join("a-1.json"), b"broken").unwrap();
    assert!(f.run().0.is_err());
    assert!(target.join("debug/deps/output").exists());
}

#[test]
fn process_inspection_failures_keep_everything() {
    for field in ["status", "cwd", "exe", "environ", "fd", "maps"] {
        let f = Fixture::new();
        let target = f.target("a-target");
        f.process(4242, f.temp.path(), Path::new("/fictional/bin/agent"), b"");
        let detail = f.proc.join("4242").join(field);
        if field == "fd" {
            disk::remove_dir(detail).unwrap();
        } else {
            disk::remove_file(detail).unwrap();
        }
        let error = f.run().0.unwrap_err();
        assert!(
            error.to_string().contains("cannot inspect process 4242"),
            "{field}: {error}"
        );
        assert!(target.join("debug/deps/output").exists());
    }
    let mut f = Fixture::new();
    let target = f.target("a-target");
    f.proc = f.temp.path().join("absent-proc");
    assert!(f.run().0.is_err());
    assert!(target.join(TAG).exists());
}

#[test]
fn unreadable_process_details_and_vanishing_processes() {
    if unsafe { libc::geteuid() } != 0 {
        for field in ["status", "environ", "fd", "maps"] {
            let f = Fixture::new();
            let target = f.target("a-target");
            f.process(4242, f.temp.path(), Path::new("/fictional/bin/agent"), b"");
            let path = f.proc.join("4242").join(field);
            disk::set_permissions(&path, disk::Permissions::from_mode(0o0)).unwrap();
            assert!(
                f.run()
                    .0
                    .unwrap_err()
                    .to_string()
                    .contains("cannot inspect process 4242")
            );
            assert!(target.join(TAG).exists());
            disk::set_permissions(
                path,
                disk::Permissions::from_mode(if field == "fd" { 0o700 } else { 0o600 }),
            )
            .unwrap();
        }
        let f = Fixture::new();
        f.target("a-target");
        disk::set_permissions(&f.proc, disk::Permissions::from_mode(0o0)).unwrap();
        assert!(f.run().0.is_err());
        disk::set_permissions(&f.proc, disk::Permissions::from_mode(0o700)).unwrap();
    }
    let f = Fixture::new();
    f.target("a-target");
    f.process(4242, f.temp.path(), Path::new("/fictional/bin/agent"), b"");
    let path = f.proc.join("4242");
    hook(move |at| {
        if at == "process_owned" {
            disk::remove_dir_all(&path)?;
        }
        Ok(())
    });
    let result = f.run().0;
    clear_hook();
    assert_eq!(result.unwrap(), 0);
}

#[test]
fn open_descriptor_under_target_keeps_both_modes() {
    use std::os::fd::AsRawFd;
    for target_only in [false, true] {
        let mut f = Fixture::new();
        f.options.target_only = target_only;
        let target = f.target("a-target");
        let file = disk::File::open(target.join("debug/deps/output")).unwrap();
        let reference = disk::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).unwrap();
        f.process(4242, f.temp.path(), Path::new("/fictional/bin/agent"), b"");
        symlink(reference, f.proc.join("4242/fd/7")).unwrap();
        let (result, report) = f.run();
        assert_eq!(result.unwrap(), 3);
        assert_eq!(report.freed_bytes, 0);
        assert_eq!(
            disk::read(target.join("debug/deps/output")).unwrap(),
            b"build output"
        );
    }
}

#[test]
fn mapped_file_under_target_keeps_both_modes() {
    for target_only in [false, true] {
        for name in ["a-target", "a-work/target"] {
            let mut f = Fixture::new();
            f.options.target_only = target_only;
            let target = f.target(name);
            let file = target.join("debug/deps/mapped file.so");
            disk::write(&file, b"mapped evidence").unwrap();
            f.process(4242, f.temp.path(), Path::new("/fictional/bin/agent"), b"");
            let maps = format!(
                "1000-2000 r--p 00000000 08:02 42     {} (deleted)\n",
                file.display()
            );
            disk::write(f.proc.join("4242/maps"), maps).unwrap();
            let (result, report) = f.run();
            assert_eq!(result.unwrap(), 3, "{target_only} {name}");
            assert_eq!(report.freed_bytes, 0);
            assert_eq!(disk::read(file).unwrap(), b"mapped evidence");
        }
    }
}

#[test]
fn maps_parser_preserves_spaces_and_both_newline_interpretations() {
    let paths = mapped_paths(
        b"1000-2000 r-xp 0 00:01 42 /fixture/mapped file.so (deleted)\n\
        2000-3000 rw-p 0 00:00 0 [heap]\n\
        3000-4000 rw-p 0 00:00 0\n\
        4000-5000 r--p 0 00:01 99 /fixture/line\\012break.so\n",
    )
    .unwrap();
    assert_eq!(
        paths,
        vec![
            PathBuf::from("/fixture/mapped file.so (deleted)"),
            PathBuf::from(r"/fixture/line\012break.so"),
            PathBuf::from("/fixture/line\nbreak.so")
        ]
    );
    assert!(mapped_paths(b"incomplete entry\n").is_err());
}

#[test]
fn unreadable_fd_links_and_malformed_maps_keep_everything() {
    for detail in ["fd", "maps"] {
        let f = Fixture::new();
        let target = f.target("a-target");
        f.process(4242, f.temp.path(), Path::new("/fictional/bin/agent"), b"");
        if detail == "fd" {
            disk::write(f.proc.join("4242/fd/7"), b"not a descriptor link").unwrap();
        } else {
            disk::write(f.proc.join("4242/maps"), b"incomplete entry").unwrap();
        }
        assert!(
            f.run()
                .0
                .unwrap_err()
                .to_string()
                .contains("cannot inspect process 4242")
        );
        assert_eq!(
            disk::read(target.join("debug/deps/output")).unwrap(),
            b"build output"
        );
    }
}

#[test]
fn unrelated_and_kernel_only_references_allow_cleanup() {
    let f = Fixture::new();
    let target = f.target("a-target");
    let outside = f.temp.path().join("unrelated");
    disk::write(&outside, b"outside").unwrap();
    f.process(4242, f.temp.path(), Path::new("/fictional/bin/agent"), b"");
    for (number, reference) in [
        (1, "socket:[123]"),
        (2, "pipe:[456]"),
        (3, "anon_inode:[eventpoll]"),
    ] {
        symlink(reference, f.proc.join(format!("4242/fd/{number}"))).unwrap();
    }
    symlink(&outside, f.proc.join("4242/fd/4")).unwrap();
    disk::write(
        f.proc.join("4242/maps"),
        format!(
            "1000-2000 r--p 0 00:01 42 {}\n2000-3000 rw-p 0 00:00 0 [heap]\n",
            outside.display()
        ),
    )
    .unwrap();
    assert_eq!(f.run().0.unwrap(), 0);
    assert!(!target.exists());
    assert_eq!(disk::read(outside).unwrap(), b"outside");
}

#[test]
fn zombie_and_other_owner_processes_are_skipped() {
    let f = Fixture::new();
    f.target("a-target");
    disk::create_dir(f.proc.join("4242")).unwrap();
    let uid = unsafe { libc::geteuid() };
    disk::write(f.proc.join("4242/status"), status(uid, "Z")).unwrap();
    assert_eq!(f.run().0.unwrap(), 0);
    f.target("a-target");
    // The directory is ours, but status reports a different effective owner.
    disk::write(f.proc.join("4242/status"), status(uid.wrapping_add(1), "S")).unwrap();
    assert_eq!(f.run().0.unwrap(), 0);
}

#[test]
fn idle_future_and_deep_modification_guards() {
    for case in ["fresh", "old", "deep", "future", "future_child", "tag"] {
        let mut f = Fixture::new();
        f.options.target_only = true;
        f.options.bead = None;
        let target = f.target("a-target");
        let now = crate::run::now() as i64;
        if case != "fresh" {
            age(&target, now - 10_800);
        }
        match case {
            "deep" => age(&target.join("debug/deps/output"), now),
            "future" => {
                let time =
                    std::time::UNIX_EPOCH + std::time::Duration::from_secs((now + 3600) as u64);
                disk::File::open(&target)
                    .unwrap()
                    .set_times(disk::FileTimes::new().set_modified(time))
                    .unwrap();
            }
            "future_child" => age(&target.join("debug/deps/output"), now + 3600),
            "tag" => age(&target.join(TAG), now),
            _ => (),
        }
        assert_eq!(
            f.run().0.unwrap(),
            if case == "old" { 0 } else { 3 },
            "{case}"
        );
        if case != "old" {
            f.options.bead = Some("task-1".into());
            assert_eq!(f.run().0.unwrap(), 0);
        }
    }
}

#[test]
fn done_check_failures_and_pointer_precedence() {
    for target_only in [false, true] {
        for argv in [
            vec!["false", "{bead}"],
            vec!["missing-test-command", "{bead}"],
            vec!["printf", "not JSON {bead}"],
            vec!["printf", "{}", "{bead}"],
            vec!["sh", "-c", "head -c 1048577 /dev/zero", "{bead}"],
        ] {
            let mut f = Fixture::new();
            f.options.target_only = target_only;
            let target = f.target("a-target");
            f.config
                .projects
                .get_mut("alpha")
                .unwrap()
                .done
                .as_mut()
                .unwrap()
                .command = argv.into_iter().map(String::from).collect();
            assert!(f.run().0.is_err());
            assert!(target.join("debug/deps/output").exists());
        }
    }
    let mut f = Fixture::new();
    let target = f.target("a-target");
    f.config.projects.get_mut("alpha").unwrap().done = None;
    assert_eq!(f.run().0.unwrap(), 3);
    assert!(target.exists());
    let mut check = Done {
        command: vec![
            "printf".into(),
            "{\"status\":\"closed\",\"other\":\"open\"}".into(),
            "{bead}".into(),
        ],
        pointers: vec!["/other".into(), "/status".into()],
        equals: "closed".into(),
    };
    f.config.projects.get_mut("alpha").unwrap().done = Some(check);
    assert_eq!(f.run().0.unwrap(), 3);
    check = f
        .config
        .projects
        .get_mut("alpha")
        .unwrap()
        .done
        .take()
        .unwrap();
    check.pointers.reverse();
    f.config.projects.get_mut("alpha").unwrap().done = Some(check);
    assert_eq!(f.run().0.unwrap(), 0);
}

#[test]
fn dry_run_json_preserves_paths_bytes_links_and_modes() {
    let mut f = Fixture::new();
    f.target("a-target");
    f.target("a-work/target");
    disk::write(f.root().join("a-prompt.md"), b"prompt").unwrap();
    disk::write(f.root().join("a-launch.sh"), b"launcher").unwrap();
    symlink("/fictional/store/build", f.root().join("a-work/link")).unwrap();
    let dir = Dir::absolute(&f.root()).unwrap();
    let before: Vec<_> = dir
        .names()
        .unwrap()
        .iter()
        .map(|n| scan(&dir, n).unwrap())
        .collect();
    f.options.dry_run = true;
    let (result, report) = f.run();
    assert_eq!(result.unwrap(), 0);
    let after: Vec<_> = dir
        .names()
        .unwrap()
        .iter()
        .map(|n| scan(&dir, n).unwrap())
        .collect();
    assert_eq!(before, after);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["freed_bytes"], 0);
    assert_eq!(json["dry_run"], true);
    for kind in ["target", "store_link", "prompt", "launcher", "work"] {
        assert!(report.paths.iter().any(|p| p.kind == kind));
    }
    assert!(
        report
            .paths
            .iter()
            .filter(|p| p.kind != "work")
            .all(|p| p.outcome == "would_remove")
    );
    f.options.target_only = true;
    f.options.dry_run = false;
    assert_eq!(f.run().0.unwrap(), 0);
    assert!(f.root().join("a-prompt.md").exists());
    assert!(f.root().join("a-work/link").is_symlink());
}

#[test]
fn symlink_replaced_by_evidence_is_kept() {
    let f = Fixture::new();
    disk::create_dir(f.root().join("a-work")).unwrap();
    let link = f.root().join("a-work/link");
    symlink("/fictional/store/build", &link).unwrap();
    let replaced = link.clone();
    hook(move |at| {
        if at == "before_remove" {
            disk::remove_file(&replaced)?;
            disk::write(&replaced, b"replacement evidence")?;
        }
        Ok(())
    });
    let result = f.run().0;
    clear_hook();
    assert!(result.is_err());
    assert_eq!(disk::read(link).unwrap(), b"replacement evidence");
}

#[test]
fn target_work_parent_and_tag_races_do_not_escape() {
    for case in ["target", "parent", "tag", "child", "link_text"] {
        let f = Fixture::new();
        let target = if case == "parent" {
            f.target("a-work/target")
        } else {
            f.target("a-target")
        };
        let outside = f.temp.path().join("outside");
        disk::create_dir(&outside).unwrap();
        disk::write(outside.join(TAG), SIGNATURE).unwrap();
        disk::write(outside.join("evidence"), b"outside proof").unwrap();
        disk::set_permissions(&outside, disk::Permissions::from_mode(0o555)).unwrap();
        let path = target.clone();
        let dest = outside.clone();
        let root = f.root();
        let mut once = false;
        if case == "link_text" {
            disk::remove_dir_all(&target).unwrap();
            disk::create_dir(f.root().join("a-work")).unwrap();
            symlink("/fictional/store/build", f.root().join("a-work/link")).unwrap();
        }
        hook(move |at| {
            if at == "before_remove" && !once {
                once = true;
                match case {
                    "target" => {
                        disk::rename(&path, root.join("saved"))?;
                        symlink(&dest, &path)?;
                    }
                    "parent" => {
                        disk::rename(root.join("a-work"), root.join("saved"))?;
                        symlink(&dest, root.join("a-work"))?;
                    }
                    "tag" => {
                        disk::remove_file(path.join(TAG))?;
                        symlink(dest.join(TAG), path.join(TAG))?;
                    }
                    "child" => {
                        disk::remove_file(path.join("debug/deps/output"))?;
                        symlink(dest.join("evidence"), path.join("debug/deps/output"))?;
                    }
                    "link_text" => {
                        disk::remove_file(root.join("a-work/link"))?;
                        symlink("/fictional/other", root.join("a-work/link"))?;
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        });
        let result = f.run().0;
        clear_hook();
        if case == "parent" {
            assert_eq!(result.unwrap(), 0);
        } else {
            assert!(result.is_err(), "{case}");
        }
        assert_eq!(
            disk::read(outside.join("evidence")).unwrap(),
            b"outside proof"
        );
        assert_eq!(
            disk::metadata(&outside).unwrap().permissions().mode() & 0o777,
            0o555
        );
        disk::set_permissions(&outside, disk::Permissions::from_mode(0o755)).unwrap();
    }
}

#[test]
fn removal_errors_retain_tag_stop_later_entries_and_allow_retry() {
    for fail in ["unlink", "before_tag_unlink"] {
        let f = Fixture::new();
        let target = f.target("a-target");
        disk::write(f.root().join("a-prompt.md"), b"prompt").unwrap();
        let mut once = false;
        hook(move |at| {
            if at == fail && !once {
                once = true;
                Err(invalid("injected removal failure"))
            } else {
                Ok(())
            }
        });
        let (result, report) = f.run();
        clear_hook();
        assert!(result.is_err(), "{fail}");
        assert!(disk::read(target.join(TAG)).unwrap().starts_with(SIGNATURE));
        assert!(f.root().join("a-prompt.md").exists());
        assert_eq!(report.freed_bytes, 0);
        assert!(report.paths.iter().any(|p| p.outcome == "failed"));
        assert_eq!(f.run().0.unwrap(), 0);
    }
}

#[test]
fn read_only_top_targets_are_removed_in_place() {
    for name in ["a-target", "a-work/target"] {
        let f = Fixture::new();
        let target = f.target(name);
        disk::set_permissions(&target, disk::Permissions::from_mode(0o555)).unwrap();
        disk::set_permissions(
            target.join("debug/deps"),
            disk::Permissions::from_mode(0o555),
        )
        .unwrap();
        assert_eq!(f.run().0.unwrap(), 0);
        assert!(!target.exists());
    }
}

#[test]
fn final_directory_failure_keeps_the_original_error_and_retry_finishes() {
    let f = Fixture::new();
    let target = f.target("a-target");
    hook(|at| {
        if at == "final_rmdir" {
            Err(io::Error::from_raw_os_error(libc::EIO))
        } else {
            Ok(())
        }
    });
    let (result, report) = f.run();
    clear_hook();
    assert_eq!(result.unwrap_err().raw_os_error(), Some(libc::EIO));
    assert!(target.is_dir());
    assert!(disk::read_dir(&target).unwrap().next().is_none());
    assert_eq!(report.freed_bytes, 0);
    assert!(report.paths.iter().any(|p| p.outcome == "failed"));
    assert_eq!(f.run().0.unwrap(), 0);
    assert!(!target.exists());
}

#[test]
fn effective_uid_is_read_from_status() {
    assert_eq!(
        process_uid("Name:\tfake\nUid:\t10\t20\t30\t40\n").unwrap(),
        20
    );
    assert!(process_uid("State:\tS\n").is_err());
    assert!(process_uid("Uid:\t10\tunreadable\t30\t40\n").is_err());
}

#[test]
#[ignore = "subprocess helper for non-dumpable process coverage"]
fn nondumpable_child() {
    use std::io::{Read, Write};
    let zero: libc::c_ulong = 0;
    assert_eq!(
        unsafe { libc::prctl(libc::PR_SET_DUMPABLE, zero, zero, zero, zero) },
        0
    );
    assert_eq!(
        unsafe { libc::prctl(libc::PR_GET_DUMPABLE, zero, zero, zero, zero) },
        0
    );
    println!("NON-DUMPABLE READY");
    io::stdout().flush().unwrap();
    let mut stop = [0];
    let _ = io::stdin().read(&mut stop).unwrap();
}

#[test]
fn nondumpable_owned_process_cannot_be_skipped() {
    use std::io::BufRead;
    use std::process::Stdio;
    let uid = unsafe { libc::geteuid() };
    if uid == 0 {
        eprintln!("non-dumpable ownership test requires a non-root user");
        return;
    }
    let f = Fixture::new();
    let target = f.target("a-target");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cleanup::tests::nondumpable_child",
            "--ignored",
            "--nocapture",
        ])
        .current_dir(&target)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut ready = false;
    let mut line = String::new();
    while output.read_line(&mut line).unwrap() > 0 {
        if line.contains("NON-DUMPABLE READY") {
            ready = true;
            break;
        }
        line.clear();
    }
    let actual = PathBuf::from(format!("/proc/{}", child.id()));
    symlink(&actual, f.proc.join(child.id().to_string())).unwrap();
    let owner = disk::metadata(&actual).unwrap().uid();
    let status_uid = process_uid(&disk::read_to_string(actual.join("status")).unwrap()).unwrap();
    let result = f.run().0;
    drop(child.stdin.take());
    let exit = child.wait().unwrap();
    assert!(ready && exit.success());
    assert_eq!(status_uid, uid);
    match result {
        Ok(code) => assert_eq!(code, 3),
        Err(e) => assert!(e.to_string().contains(&format!("process {}", child.id()))),
    }
    assert_eq!(
        disk::read(target.join("debug/deps/output")).unwrap(),
        b"build output"
    );
    eprintln!(
        "non-dumpable guard exercised: proc directory owner {owner}, status effective UID {status_uid}"
    );
}

#[test]
#[ignore = "subprocess helper for in-place interruption coverage"]
fn interrupted_removal_child() {
    let base = PathBuf::from(std::env::var_os("ROTA_TEST_INTERRUPT_ROOT").unwrap());
    let config = Fixture::config(&base);
    let options = Options {
        name: std::env::var("ROTA_TEST_INTERRUPT_NAME").unwrap(),
        bead: Some("task-1".into()),
        ..Options::default()
    };
    let mut report = Report {
        name: options.name.clone(),
        project: "alpha".into(),
        bead: options.bead.clone(),
        dry_run: false,
        target_only: false,
        done: "not checked".into(),
        error: None,
        freed_bytes: 0,
        paths: Vec::new(),
    };
    let phase = std::env::var("ROTA_TEST_INTERRUPT_PHASE").unwrap();
    hook(move |at| {
        if at == phase {
            unsafe { libc::kill(libc::getpid(), libc::SIGKILL) };
        }
        Ok(())
    });
    execute(
        &config,
        &config.projects["alpha"],
        &options,
        &base.join("proc"),
        &mut report,
    )
    .unwrap();
    panic!("interruption point not reached");
}

#[test]
fn interrupted_removal_retries_before_and_after_tag_unlink() {
    for (name, relative, phase) in [
        ("a", "a-target", "before_tag_unlink"),
        ("build-", "build--target", "after_tag_unlink"),
        ("a", "a-work/target", "after_tag_unlink"),
    ] {
        let mut f = Fixture::new();
        f.options.name = name.into();
        let target = f.target(relative);
        let exit = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cleanup::tests::interrupted_removal_child",
                "--ignored",
                "--nocapture",
            ])
            .env("ROTA_TEST_INTERRUPT_ROOT", f.temp.path())
            .env("ROTA_TEST_INTERRUPT_NAME", name)
            .env("ROTA_TEST_INTERRUPT_PHASE", phase)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert_eq!(exit.signal(), Some(libc::SIGKILL));
        assert_eq!(target.join(TAG).exists(), phase == "before_tag_unlink");
        assert_eq!(f.run().0.unwrap(), 0);
        assert!(!target.exists());
    }
}

#[test]
fn unreadable_directory_and_unwritable_parent_fail_without_content_loss() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let f = Fixture::new();
    let target = f.target("a-target");
    disk::set_permissions(target.join("debug"), disk::Permissions::from_mode(0o0)).unwrap();
    assert!(f.run().0.is_err());
    assert!(target.join(TAG).exists());
    disk::set_permissions(target.join("debug"), disk::Permissions::from_mode(0o755)).unwrap();
    f.target("a-work/target");
    disk::set_permissions(f.root().join("a-work"), disk::Permissions::from_mode(0o555)).unwrap();
    // Refuse the unwritable parent before changing the nested target.
    assert!(f.run().0.is_err());
    assert!(f.root().join("a-work/target/debug/deps/output").exists());
    assert!(f.root().join("a-work/target/CACHEDIR.TAG").exists());
    disk::set_permissions(f.root().join("a-work"), disk::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(f.run().0.unwrap(), 0);
}

#[test]
fn hardlink_disk_blocks_are_counted_once() {
    let f = Fixture::new();
    let target = f.target("a-target");
    let dir = Dir::absolute(&f.root()).unwrap();
    let first = scan(&dir, OsStr::new("a-target")).unwrap();
    disk::hard_link(target.join("debug/deps/output"), target.join("copy")).unwrap();
    let second = scan(&dir, OsStr::new("a-target")).unwrap();
    assert_eq!(
        size(&first, &mut BTreeSet::new()),
        size(&second, &mut BTreeSet::new())
    );
}

#[test]
fn unreportable_link_names_fail_before_any_deletion() {
    use std::os::unix::ffi::OsStringExt;
    let f = Fixture::new();
    let target = f.target("a-target");
    disk::create_dir(f.root().join("a-work")).unwrap();
    let link = f.root().join("a-work").join(OsString::from_vec(vec![0xff]));
    symlink("/fictional/store/build", &link).unwrap();
    let (result, report) = f.run();
    assert!(result.unwrap_err().to_string().contains("non-UTF-8"));
    assert!(target.join("debug/deps/output").exists());
    assert!(link.is_symlink());
    serde_json::to_string(&report).unwrap();
}

#[test]
#[ignore = "subprocess helper for the mount-boundary test"]
fn mount_boundary_child() {
    use std::ffi::CString;
    let base = PathBuf::from(std::env::var_os("ROTA_TEST_MOUNT_ROOT").unwrap());
    let live_alias = std::env::var_os("ROTA_TEST_MOUNT_LIVE").is_some();
    let source = CString::new(
        base.join(if live_alias {
            "agents/a-target"
        } else {
            "outside"
        })
        .as_os_str()
        .as_bytes(),
    )
    .unwrap();
    let dest = CString::new(
        base.join(if live_alias {
            "alias"
        } else {
            "agents/a-target/debug/deps"
        })
        .as_os_str()
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(
        unsafe {
            libc::mount(
                source.as_ptr(),
                dest.as_ptr(),
                std::ptr::null(),
                libc::MS_BIND,
                std::ptr::null(),
            )
        },
        0,
        "bind mount: {}",
        io::Error::last_os_error()
    );
    println!("MOUNT FIXTURE ACTIVE");
    use std::io::Write;
    io::stdout().flush().unwrap();
    let config = Fixture::config(&base);
    let mut options = Options {
        name: "a".into(),
        bead: Some("task-1".into()),
        ..Options::default()
    };
    let mut report = Report {
        name: "a".into(),
        project: "alpha".into(),
        bead: options.bead.clone(),
        dry_run: false,
        target_only: false,
        done: "not checked".into(),
        error: None,
        freed_bytes: 0,
        paths: Vec::new(),
    };
    if live_alias {
        disk::write(
            base.join("proc/4242/status"),
            status(unsafe { libc::geteuid() }, "S"),
        )
        .unwrap();
        for target_only in [false, true] {
            options.target_only = target_only;
            assert_eq!(
                execute(
                    &config,
                    &config.projects["alpha"],
                    &options,
                    &base.join("proc"),
                    &mut report
                )
                .unwrap(),
                3
            );
            assert_eq!(report.freed_bytes, 0);
            assert_eq!(
                disk::read(base.join("agents/a-target/debug/deps/output")).unwrap(),
                b"build output"
            );
        }
    } else {
        assert_eq!(
            execute(
                &config,
                &config.projects["alpha"],
                &options,
                &base.join("proc"),
                &mut report
            )
            .unwrap_err()
            .raw_os_error(),
            Some(libc::EXDEV)
        );
        assert_eq!(
            disk::read(base.join("outside/evidence")).unwrap(),
            b"outside proof"
        );
    }
    assert!(base.join("agents/a-target/CACHEDIR.TAG").exists());
}

#[test]
fn directory_identity_matches_ancestors() {
    let f = Fixture::new();
    let target = f.target("a-target");
    let stat = disk::metadata(&target).unwrap();
    let protected = BTreeSet::from([(stat.dev(), stat.ino())]);
    assert!(fs::ancestors_match(&target.join("debug/deps"), &protected).unwrap());
    assert!(!fs::ancestors_match(f.temp.path(), &protected).unwrap());
    assert!(
        !fs::ancestors_match(&target, &BTreeSet::from([(stat.dev() ^ 1, stat.ino())])).unwrap()
    );
    assert!(!fs::ancestors_match(&target, &BTreeSet::from([(stat.dev(), u64::MAX)])).unwrap());
}

#[test]
fn bind_mount_aliases_keep_live_sessions() {
    let f = Fixture::new();
    let target = f.target("a-target");
    disk::create_dir(f.temp.path().join("alias")).unwrap();
    f.process(
        4242,
        &f.temp.path().join("alias/debug/deps"),
        Path::new("/fictional/bin/agent"),
        b"",
    );
    let result = Command::new("unshare")
        .args(["--user", "--map-root-user", "--mount", "--"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cleanup::tests::mount_boundary_child",
            "--ignored",
            "--nocapture",
        ])
        .env("ROTA_TEST_MOUNT_ROOT", f.temp.path())
        .env("ROTA_TEST_MOUNT_LIVE", "1")
        .output();
    match result {
        Ok(output) if String::from_utf8_lossy(&output.stdout).contains("MOUNT FIXTURE ACTIVE") => {
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            eprintln!("bind-mount cwd alias refusal exercised in both modes");
        }
        result => {
            eprintln!("bind-mount alias test skipped: private namespaces unavailable: {result:?}")
        }
    }
    assert_eq!(
        disk::read(target.join("debug/deps/output")).unwrap(),
        b"build output"
    );
}

#[test]
fn mounted_descendants_are_not_traversed() {
    let f = Fixture::new();
    f.target("a-target");
    disk::create_dir(f.temp.path().join("outside")).unwrap();
    disk::write(f.temp.path().join("outside/evidence"), b"outside proof").unwrap();
    let result = Command::new("unshare")
        .args(["--user", "--map-root-user", "--mount", "--"])
        .arg(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "cleanup::tests::mount_boundary_child",
            "--ignored",
            "--nocapture",
        ])
        .env("ROTA_TEST_MOUNT_ROOT", f.temp.path())
        .output();
    match result {
        Ok(output) if String::from_utf8_lossy(&output.stdout).contains("MOUNT FIXTURE ACTIVE") => {
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        result => eprintln!(
            "mount-boundary test skipped: private user/mount namespace unavailable: {result:?}"
        ),
    }
    assert_eq!(
        disk::read(f.temp.path().join("outside/evidence")).unwrap(),
        b"outside proof"
    );
}

#[test]
fn parsing_and_config_validation() {
    for name in ["", ".", "..", "../x", "a/b", ".hidden"] {
        assert_eq!(parse(&[name.into()]).unwrap_err().0, 2);
    }
    let mut f = Fixture::new();
    assert_eq!(f.run().0.unwrap(), 0);
    for idle in ["0", "bad", "999999999999999999999h"] {
        f.config.cleanup.min_idle = idle.into();
        assert!(validate(&f.config).is_err());
    }
    f.config.cleanup.min_idle = "2h".into();
    for prefix in ["/", "relative", "/fictional/../other"] {
        f.config.cleanup.store_prefixes = vec![prefix.into()];
        assert!(validate(&f.config).is_err());
    }
    f.config.cleanup.store_prefixes.clear();
    for pointer in ["relative", "/status~", "/status~2"] {
        f.config
            .projects
            .get_mut("alpha")
            .unwrap()
            .done
            .as_mut()
            .unwrap()
            .pointers = vec![pointer.into()];
        assert!(validate(&f.config).is_err());
    }
}
