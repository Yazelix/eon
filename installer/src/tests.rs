use super::*;
use std::os::unix::fs::symlink;

fn archive(entries: &[(&str, tar::EntryType, &str)]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (path, kind, link) in entries {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(*kind);
        header.set_mode(0o755);
        header.set_size(0);
        // Deliberately bypass the writer's path validation to exercise the reader.
        header.as_mut_bytes()[..path.len()].copy_from_slice(path.as_bytes());
        if !link.is_empty() {
            header.set_link_name(link).unwrap();
        }
        header.set_cksum();
        builder.append(&header, io::empty()).unwrap();
    }
    builder.into_inner().unwrap()
}

#[test]
fn extraction_rejects_escapes_and_special_files() {
    use tar::EntryType as T;
    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), "user data").unwrap();
    let cases = [
        vec![("../outside/keep", T::Regular, "")],
        vec![("/absolute", T::Regular, "")],
        vec![("other/entry", T::Regular, "")],
        vec![("root/device", T::Char, "")],
        vec![("root/pipe", T::Fifo, "")],
        vec![("root/link", T::Link, "../outside/keep")],
        vec![("root/link", T::Symlink, "../../outside")],
        vec![("root/nix/store/a/link", T::Symlink, "/nix/store/b/file")],
        vec![("root", T::Symlink, "file")],
        vec![
            ("root/duplicate", T::Regular, ""),
            ("root/duplicate", T::Regular, ""),
        ],
    ];
    for entries in cases {
        let destination = tempfile::tempdir_in(temp.path()).unwrap();
        assert!(
            extract(archive(&entries).as_slice(), destination.path(), "root").is_err(),
            "accepted {entries:?}"
        );
        assert_eq!(
            fs::read_to_string(outside.join("keep")).unwrap(),
            "user data"
        );
    }
    let destination = tempfile::tempdir_in(temp.path()).unwrap();
    fs::create_dir(destination.path().join("root")).unwrap();
    symlink(&outside, destination.path().join("root/link")).unwrap();
    assert!(
        extract(
            archive(&[("root/link/keep", T::Regular, "")]).as_slice(),
            destination.path(),
            "root"
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(outside.join("keep")).unwrap(),
        "user data"
    );
}

fn fixture_version(prefix: &Path, hash: &str) -> PathBuf {
    let root = prefix.join("versions").join(hash);
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    fs::create_dir(root.join("nix")).unwrap();
    fs::copy(env::current_exe().unwrap(), root.join("nix/eon")).unwrap();
    symlink("nix/eon", root.join("eon")).unwrap();
    fs::write(
        root.join(".eon-source"),
        format!("{MARKER}{hash}\nhttps://proof.invalid/exact\n"),
    )
    .unwrap();
    root
}

#[test]
fn selection_and_removal_preserve_payloads_and_unrelated_files() {
    let temp = tempfile::tempdir().unwrap();
    let prefix = temp.path().join("space and apostrophe's prefix");
    let _lock = prefix_lock(&prefix, true).unwrap();
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    let old = fixture_version(&prefix, &a);
    fixture_version(&prefix, &b);
    let inode = fs::metadata(&old).unwrap().ino();
    fs::write(temp.path().join("user-config"), "keep").unwrap();
    select(&prefix, &a).unwrap();
    select(&prefix, &b).unwrap();
    assert_eq!(selected(&prefix).unwrap().as_deref(), Some(b.as_str()));
    select(&prefix, &a).unwrap();
    assert_eq!(fs::metadata(&old).unwrap().ino(), inode);
    fs::remove_file(prefix.join("bin/eon")).unwrap();
    symlink("/unrelated/eon", prefix.join("bin/eon")).unwrap();
    remove(&prefix).unwrap();
    assert_eq!(
        fs::read_link(prefix.join("bin/eon")).unwrap(),
        Path::new("/unrelated/eon")
    );
    assert!(selected(&prefix).unwrap().is_none());
    assert!(version(&prefix, &a).is_ok() && version(&prefix, &b).is_ok());
    assert_eq!(
        fs::read_to_string(temp.path().join("user-config")).unwrap(),
        "keep"
    );
}

#[test]
fn failed_selection_and_locking_leave_current_usable() {
    let temp = tempfile::tempdir().unwrap();
    let prefix = temp.path().join("owned");
    assert!(prefix_lock(&prefix, false).is_err());
    assert!(!prefix.exists());
    let _lock = prefix_lock(&prefix, true).unwrap();
    assert!(prefix_lock(&prefix, false).is_err());
    let a = "a".repeat(64);
    let b = "b".repeat(64);
    fixture_version(&prefix, &a);
    let bad = fixture_version(&prefix, &b);
    select(&prefix, &a).unwrap();
    fs::write(bad.join("eon"), "invalid executable").unwrap();
    assert!(select(&prefix, &b).is_err());
    // A working executable is not an entrypoint unless it is the link into the tree.
    fs::remove_file(bad.join("eon")).unwrap();
    fs::copy(env::current_exe().unwrap(), bad.join("eon")).unwrap();
    assert!(select(&prefix, &b).is_err());
    assert_eq!(selected(&prefix).unwrap().as_deref(), Some(a.as_str()));
    fs::remove_file(prefix.join("bin/eon")).unwrap();
    fs::write(prefix.join("bin/eon"), "user command").unwrap();
    assert!(select(&prefix, &a).is_err());
    assert_eq!(selected(&prefix).unwrap().as_deref(), Some(a.as_str()));
    assert_eq!(
        fs::read_to_string(prefix.join("bin/eon")).unwrap(),
        "user command"
    );
    let unrelated = temp.path().join("unrelated");
    fs::create_dir(&unrelated).unwrap();
    fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        prefix_lock(&unrelated, true).unwrap_err().to_string(),
        "prefix is not owned by eon-install"
    );
    assert!(unrelated.read_dir().unwrap().next().is_none());
}

#[test]
fn checksums_reject_ambiguous_bindings_and_arguments_reject_invalid_requests() {
    let a = "a".repeat(64);
    assert_eq!(checksum(&format!("{a}  exact\n"), "exact").unwrap(), a);
    for text in [
        format!("{a}  other\n"),
        format!("{a}  exact\n{a}  exact\n"),
        format!("wrong  exact\n{a}  exact\n"),
    ] {
        assert!(checksum(&text, "exact").is_err());
    }
    for args in [
        vec!["remove", "unexpected"],
        vec!["rollback", "invalid", "--dry-run"],
        vec!["remove", "--prefix", "relative"],
        vec!["install", "http://host/a.tar.zst", "--signers", "keys"],
    ] {
        assert!(options(args.into_iter().map(Into::into)).is_err());
    }
}

#[test]
fn extraction_preserves_relative_links_and_hardlinks() {
    use tar::EntryType as T;
    let temp = tempfile::tempdir().unwrap();
    let entries = [
        ("root", T::Directory, "ignored-link-name"),
        ("root/file", T::Regular, "../../ignored-link-name"),
        ("root/hard", T::Link, "root/file"),
        ("root/eon", T::Symlink, "nix/store/component/bin/eon"),
        ("root/nix/store/component/link", T::Symlink, "../other/file"),
    ];
    extract(archive(&entries).as_slice(), temp.path(), "root").unwrap();
    assert!(temp.path().join("root/file").is_file());
    assert!(temp.path().join("root/hard").is_file());
    assert_eq!(
        fs::read_link(temp.path().join("root/eon")).unwrap(),
        Path::new("nix/store/component/bin/eon")
    );
    assert_eq!(
        fs::read_link(temp.path().join("root/nix/store/component/link")).unwrap(),
        Path::new("../other/file")
    );
}

#[test]
fn smoke_timeout_stops_wrapper_and_child() {
    use std::time::{Duration, Instant};

    let root = tempfile::tempdir().unwrap();
    let shell = Command::new("sh")
        .args(["-c", "command -v sh"])
        .output()
        .unwrap();
    assert!(shell.status.success());
    let shell = String::from_utf8(shell.stdout).unwrap();
    let program = root.path().join("eon");
    fs::write(
        &program,
        format!(
            "#!{}\nsleep 300 &\necho $! > \"$0.pid\"\nwait\n",
            shell.trim()
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o700)).unwrap();
    std::thread::scope(|scope| {
        let check = scope.spawn(|| smoke(root.path()));
        let receipt = root.path().join("eon.pid");
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::read_to_string(&receipt)
            .unwrap_or_default()
            .trim()
            .is_empty()
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(10));
        }
        let pid = fs::read_to_string(receipt).unwrap();
        let process = PathBuf::from("/proc").join(pid.trim());
        let start = fs::read_to_string(process.join("stat")).unwrap();
        let start = start
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap();
        assert!(check.join().unwrap().is_err());
        let same_child = || {
            fs::read_to_string(process.join("stat")).is_ok_and(|stat| {
                stat.rsplit_once(')').unwrap().1.split_whitespace().nth(19) == Some(start)
            }) && fs::read_link(process.join("exe")).is_ok()
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        while same_child() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let leaked = same_child();
        if leaked {
            Command::new("sh")
                .args(["-c", "kill -KILL \"$1\"", "--", pid.trim()])
                .status()
                .unwrap();
        }
        assert!(!leaked, "timed-out preflight left its child running");
    });
}
