use super::*;
use crate::supervisor::{prepare_runtime, temporary_directory};
use std::os::unix::{
    ffi::OsStringExt,
    fs::{PermissionsExt, symlink},
};

const G: &str = "g1-11111111111111111111111111111111";
const C: &str = "g1-22222222222222222222222222222222";
const A: &str = "g1-33333333333333333333333333333333";

fn fixture(path: Option<&[u8]>) -> Vec<u8> {
    let mut bytes = b"EONI\x01\x00".to_vec();
    for id in [G, C, A] {
        bytes.extend_from_slice(id.as_bytes());
    }
    bytes.push(u8::from(path.is_some()));
    if let Some(path) = path {
        bytes.extend_from_slice(&(path.len() as u16).to_le_bytes());
        bytes.extend_from_slice(path);
    }
    bytes
}

fn runtime() -> (PathBuf, PathBuf) {
    let root = temporary_directory();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    prepare_runtime(&root).unwrap();
    let runtime = generation_directory(&root, G);
    prepare_runtime(&runtime).unwrap();
    (root, runtime)
}

fn write_identity(runtime: &Path, bytes: &[u8]) {
    if runtime.join("identity").exists() {
        fs::set_permissions(runtime.join("identity"), fs::Permissions::from_mode(0o600)).unwrap();
    }
    fs::write(runtime.join("identity"), bytes).unwrap();
    fs::set_permissions(runtime.join("identity"), fs::Permissions::from_mode(0o600)).unwrap();
}

#[test]
fn identity_record_build_validation_uses_active_identity() {
    let (root, runtime) = runtime();
    write_identity(&runtime, &fixture(None));
    let identity = Identity::read(&runtime).unwrap().unwrap();
    assert!(
        identity.require_build(A).is_ok(),
        "lifetime is not the active build"
    );
    assert!(identity.require_build(G).is_err());
    assert!(identity.require_build(C).is_err());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn namespace_launch_recovers_only_its_active_build_and_refuses_ambiguity() {
    let (root, runtime) = runtime();
    write_identity(&runtime, &fixture(None));
    let lock = NamespaceLock::acquire(&root).unwrap();
    assert_eq!(lock.select(A).unwrap().as_deref(), Some(G));
    assert_eq!(lock.select(C).unwrap(), None);
    // Equal roles from the existing build-derived baseline remain valid.
    let older = generation_directory(&root, C);
    prepare_runtime(&older).unwrap();
    Identity::initial(C, C, None).publish(&older).unwrap();
    assert_eq!(lock.select(C).unwrap().as_deref(), Some(C));
    let second = generation_directory(&root, "g1-44444444444444444444444444444444");
    prepare_runtime(&second).unwrap();
    let mut duplicate = Identity::read(&runtime).unwrap().unwrap();
    duplicate.generation = second.file_name().unwrap().to_str().unwrap().into();
    duplicate.publish(&second).unwrap();
    assert!(lock.select(A).unwrap_err().contains("ambiguous"));
    let mut malformed = fixture(None);
    malformed[4] = 2;
    write_identity(&runtime, &malformed);
    assert!(
        lock.select(C).is_err(),
        "unknown metadata cannot be classified as unrelated"
    );
    drop(lock);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn namespace_launch_allocation_is_independent_and_exclusive() {
    let root = temporary_directory();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    prepare_runtime(&root).unwrap();
    let inputs = crate::fixtures::inputs();
    let build = current_generation(&inputs).unwrap();
    let lock = NamespaceLock::acquire(&root).unwrap();
    let first = lock.allocate(&inputs).unwrap();
    let second = lock.allocate(&inputs).unwrap();
    assert!(valid_generation(&first));
    assert!(valid_generation(&second));
    assert_ne!(first, second);
    assert_ne!(first, build);
    assert_ne!(second, build);
    let identity = record_launcher(&inputs, &generation_directory(&root, &first)).unwrap();
    assert_eq!(identity.generation, first);
    assert_eq!(identity.created_build, build);
    assert_eq!(identity.active_build, build);
    assert_eq!(
        lock.select(&build).unwrap().as_deref(),
        Some(first.as_str())
    );
    drop(lock);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn namespace_overflow_sweeps_host_free_entries_before_launch_selection() {
    let (root, runtime) = runtime();
    fs::remove_dir(&runtime).unwrap();
    for index in 0..=MAX_GENERATIONS {
        prepare_runtime(&generation_directory(&root, &format!("g1-{index:032x}"))).unwrap();
    }
    assert_eq!(generation_inventory(&root).unwrap().count, 257);
    assert!(generation_candidates(&root).is_err());
    assert_eq!(
        fs::read_dir(root.join("generations")).unwrap().count(),
        257,
        "read-only discovery must not sweep"
    );
    let lock = NamespaceLock::acquire(&root).unwrap();
    assert_eq!(lock.select(A).unwrap(), None);
    assert_eq!(generation_inventory(&root).unwrap().count, 0);
    drop(lock);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn namespace_overflow_retains_held_lifetimes_and_host_residue() {
    let (root, runtime) = runtime();
    fs::remove_dir(&runtime).unwrap();
    let mut held = None;
    for index in 0..=MAX_GENERATIONS {
        let id = format!("g1-{index:032x}");
        let runtime = generation_directory(&root, &id);
        prepare_runtime(&runtime).unwrap();
        fs::write(runtime.join("orbit.sock.record"), b"retained host record").unwrap();
        if index == 0 {
            held = Some(
                crate::supervisor::lock_supervisor_lifecycle(&supervisor_lock_path(&root, &id))
                    .unwrap(),
            );
        }
    }
    let lock = NamespaceLock::acquire(&root).unwrap();
    let error = lock.select(A).unwrap_err();
    for text in [
        "257",
        "256-entry inspection limit",
        "eon window stop <full-id>",
        "eon window ls",
    ] {
        assert!(error.contains(text), "{error}");
    }
    assert!(error.contains(&root.join("generations").display().to_string()));
    assert_eq!(generation_inventory(&root).unwrap().count, 257);
    drop(held);
    drop(lock);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn active_publication_and_launch_serialize_in_both_orders_without_merging_lifetimes() {
    for publish_first in [false, true] {
        let (root, runtime) = runtime();
        write_identity(&runtime, &fixture(None));
        let inputs = crate::fixtures::inputs();
        let build = current_generation(&inputs).unwrap();
        assert_ne!(build, A);
        let operate = |lock: NamespaceLock, publish: bool| {
            if publish {
                let lifecycle =
                    crate::supervisor::lock_supervisor_lifecycle(&supervisor_lock_path(&root, G))
                        .unwrap();
                let expected = Identity::read(&runtime).unwrap().unwrap();
                let mut identity = expected.clone();
                identity.active_build = build.clone();
                identity.active_launcher = Some("/target-build/eon".into());
                let published = lock
                    .publish_active(&expected, &identity, &lifecycle)
                    .is_ok();
                (G.to_owned(), published)
            } else {
                let lifetime = lock
                    .select(&build)
                    .unwrap()
                    .unwrap_or_else(|| lock.allocate(&inputs).unwrap());
                let _lifecycle = crate::supervisor::lock_supervisor_lifecycle(
                    &supervisor_lock_path(&root, &lifetime),
                )
                .unwrap();
                record_launcher(&inputs, &generation_directory(&root, &lifetime)).unwrap();
                (lifetime, true)
            }
        };
        let first = NamespaceLock::acquire(&root).unwrap();
        let (started, ready) = std::sync::mpsc::channel();
        let (first_result, second_result) = std::thread::scope(|scope| {
            let second = scope.spawn(|| {
                started.send(()).unwrap();
                let lock = NamespaceLock::acquire(&root).unwrap();
                operate(lock, !publish_first)
            });
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            let result = operate(first, publish_first);
            (result, second.join().unwrap())
        });
        let (publication, launch) = if publish_first {
            (first_result, second_result)
        } else {
            (second_result, first_result)
        };
        assert_eq!(publication.1, publish_first);
        assert_eq!(launch.0 == G, publish_first);
        let record = Identity::read(&runtime).unwrap().unwrap();
        assert_eq!(record.generation, G);
        assert_eq!(record.created_build, C);
        assert_eq!(
            record.active_build,
            if publish_first {
                build.clone()
            } else {
                A.into()
            }
        );
        let active = generation_candidates(&root)
            .unwrap()
            .into_iter()
            .filter(|(_, runtime)| Identity::read(runtime).unwrap().unwrap().active_build == build)
            .count();
        assert_eq!(active, 1, "two lifetimes acquired the same active build");
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn active_publication_refuses_lifetime_creation_and_stale_record_changes() {
    let (root, runtime) = runtime();
    write_identity(&runtime, &fixture(None));
    let namespace = NamespaceLock::acquire(&root).unwrap();
    let lifecycle =
        crate::supervisor::lock_supervisor_lifecycle(&supervisor_lock_path(&root, G)).unwrap();
    let expected = Identity::read(&runtime).unwrap().unwrap();
    let mut next = expected.clone();
    next.active_build = C.into();
    next.active_launcher = Some("/target-build/eon".into());
    for field in ["lifetime", "creation", "stale"] {
        let mut original = expected.clone();
        let mut changed = next.clone();
        match field {
            "lifetime" => changed.generation = C.into(),
            "creation" => changed.created_build = G.into(),
            "stale" => original.active_build = G.into(),
            _ => unreachable!(),
        }
        assert!(
            namespace
                .publish_active(&original, &changed, &lifecycle)
                .is_err()
        );
        assert_eq!(Identity::read(&runtime).unwrap().unwrap(), expected);
    }
    namespace
        .publish_active(&expected, &next, &lifecycle)
        .unwrap();
    assert_eq!(Identity::read(&runtime).unwrap().unwrap(), next);
    drop((lifecycle, namespace));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn identity_record_is_launcher_authority() {
    let (root, runtime) = runtime();
    symlink("/decoy", runtime.join("eon")).unwrap();
    let mut failures = Vec::new();
    for path in [
        None,
        Some(b"/active".as_slice()),
        Some(b"/non-utf8-\xff".as_slice()),
    ] {
        write_identity(&runtime, &fixture(path));
        let expected = path.map(|bytes| PathBuf::from(OsString::from_vec(bytes.to_vec())));
        if recorded_launcher(&runtime) != expected {
            failures.push(format!("path {path:?} did not win over link"));
        }
    }
    fs::remove_file(runtime.join("eon")).unwrap();
    write_identity(&runtime, &fixture(Some(b"/active")));
    assert_eq!(
        recorded_launcher(&runtime),
        Some("/active".into()),
        "missing projection"
    );
    assert!(failures.is_empty(), "{failures:?}");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn identity_record_rejections_never_fall_through_to_link() {
    let (root, runtime) = runtime();
    symlink("/decoy", runtime.join("eon")).unwrap();
    let good = fixture(Some(b"/active"));
    let mut cases = Vec::new();
    for length in 0..good.len() {
        cases.push((format!("truncated-{length}"), good[..length].to_vec()));
    }
    for (name, offset, byte) in [
        ("magic", 0, b'X'),
        ("version", 4, 2),
        ("version-high", 5, 1),
        ("generation-prefix", 6, b'x'),
        ("generation-uppercase", 9, b'A'),
        ("created-prefix", 41, b'x'),
        ("created-nonhex", 44, b'z'),
        ("active-prefix", 76, b'x'),
        ("active-uppercase", 79, b'A'),
        ("presence", 111, 2),
    ] {
        let mut bytes = good.clone();
        bytes[offset] = byte;
        cases.push((name.into(), bytes));
    }
    for offset in [6, 41, 76] {
        for (name, index, byte) in [
            ("prefix", 1, b'2'),
            ("nonhex", 3, b'z'),
            ("uppercase", 3, b'A'),
            ("nonascii", 3, 0xff),
        ] {
            let mut bytes = good.clone();
            bytes[offset + index] = byte;
            cases.push((format!("{offset}-{name}"), bytes));
        }
    }
    for (name, path) in [
        ("relative", b"relative".as_slice()),
        ("nul", b"/x\0y"),
        ("empty", b""),
    ] {
        cases.push((name.into(), fixture(Some(path))));
    }
    cases.push(("over-bound-path".into(), fixture(Some(&vec![b'/'; 4097]))));
    let mut mismatch = good.clone();
    mismatch[9] = b'4';
    cases.push(("directory-mismatch".into(), mismatch));
    for length in [0u16, 1, 4097, u16::MAX] {
        let mut bytes = good.clone();
        bytes[112..114].copy_from_slice(&length.to_le_bytes());
        cases.push((format!("forged-length-{length}"), bytes));
    }
    for (name, mut bytes) in [
        ("trailing-launcher", good),
        ("trailing-none", fixture(None)),
    ] {
        bytes.push(0);
        cases.push((name.into(), bytes));
    }
    let mut accepted = Vec::new();
    for (name, bytes) in cases {
        write_identity(&runtime, &bytes);
        if Identity::read(&runtime).is_ok() {
            accepted.push(format!("{name}: decoder accepted"));
        }
        if recorded_launcher(&runtime).is_some() {
            accepted.push(format!("{name}: routed"));
        }
    }
    assert!(
        accepted.is_empty(),
        "invalid record accepted or routed: {accepted:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn identity_record_unsafe_files_never_fall_through_to_link() {
    let (root, runtime) = runtime();
    symlink("/decoy", runtime.join("eon")).unwrap();
    let path = runtime.join("identity");
    let mut accepted = Vec::new();
    let mut check_rejection = |case: &str| {
        if Identity::read(&runtime).is_ok() {
            accepted.push(format!("{case}: decoder accepted"));
        }
        if recorded_launcher(&runtime).is_some() {
            accepted.push(format!("{case}: routed"));
        }
    };
    for mode in [0o644, 0o640, 0o400, 0o1600] {
        write_identity(&runtime, &fixture(Some(b"/active")));
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        check_rejection(&format!("mode-{mode:o}"));
    }
    fs::remove_file(&path).unwrap();
    symlink("/missing", &path).unwrap();
    check_rejection("symlink");
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    check_rejection("directory");
    fs::remove_dir(&path).unwrap();
    let fifo = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: the CString is a live NUL-terminated path in this isolated root.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    check_rejection("fifo");
    fs::remove_file(&path).unwrap();
    let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
    check_rejection("socket");
    drop(listener);
    fs::remove_file(&path).unwrap();
    write_identity(&runtime, &fixture(Some(b"/active")));
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
    check_rejection("directory-mode");
    fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        accepted.is_empty(),
        "unsafe record accepted or routed: {accepted:?}"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn identity_record_fresh_publication_exists_even_without_launcher() {
    let (root, runtime) = runtime();
    let mut inputs = crate::fixtures::inputs();
    inputs.managed.shell_bin = None;
    // Fresh publication uses the directory's build-derived identity.
    let runtime = runtime.with_file_name(current_generation(&inputs).unwrap());
    prepare_runtime(&runtime).unwrap();
    super::record_launcher(&inputs, &runtime).unwrap();
    let bytes = fs::read(runtime.join("identity")).expect("fresh launch must publish identity");
    assert_eq!(&bytes[..6], b"EONI\x01\x00");
    assert_eq!(bytes.len(), 112);
    for offset in [6, 41, 76] {
        assert_eq!(
            &bytes[offset..offset + 35],
            runtime.file_name().unwrap().as_bytes()
        );
    }
    assert_eq!(bytes[111], 0);
    assert_eq!(
        fs::metadata(runtime.join("identity")).unwrap().mode() & 0o7777,
        0o600
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn identity_record_cleanup_preserves_every_residue() {
    let mut failures = Vec::new();
    for residue in [
        "orbit.sock",
        "terminal-2.sock",
        "terminal-2.sock.management",
        "terminal-2.sock.record",
        "terminal-2.sock.pending",
        ".identity.tmp",
        "unknown",
        "snapshots",
    ] {
        for link in [false, true] {
            let (root, runtime) = runtime();
            write_identity(&runtime, &fixture(None));
            if link {
                symlink("/active", runtime.join("eon")).unwrap();
            }
            if residue == "snapshots" {
                fs::create_dir(runtime.join(residue)).unwrap();
            } else {
                fs::write(runtime.join(residue), b"residue").unwrap();
            }
            super::inspect_generation(
                G,
                "previous",
                runtime.clone(),
                &supervisor_lock_path(&root, G),
                "test",
                true,
                true,
                None,
            );
            if !runtime.join("identity").is_file()
                || (link && fs::read_link(runtime.join("eon")).is_err())
            {
                failures.push(format!("{residue}, link={link}"));
            }
            fs::remove_dir_all(root).unwrap();
        }
    }
    assert!(
        failures.is_empty(),
        "metadata lost with residue: {failures:?}"
    );
}

#[test]
fn identity_record_cleanup_removes_hostless_identity_with_or_without_link() {
    let mut leaks = Vec::new();
    for identity in [false, true] {
        for link in [false, true] {
            let (root, runtime) = runtime();
            if identity {
                write_identity(&runtime, &fixture(None));
            }
            if link {
                symlink("/active", runtime.join("eon")).unwrap();
            }
            super::inspect_generation(
                G,
                "previous",
                runtime.clone(),
                &supervisor_lock_path(&root, G),
                "test",
                true,
                true,
                None,
            );
            if runtime.exists() {
                leaks.push((identity, link));
            }
            fs::remove_dir_all(root).unwrap();
        }
    }
    assert!(leaks.is_empty(), "hostless metadata leaked: {leaks:?}");
}

#[test]
fn identity_record_cleanup_trust_lock_and_read_only_guards() {
    let mut failures = Vec::new();
    for case in [
        "identity-mode",
        "identity-symlink",
        "identity-directory",
        "launcher-file",
        "directory-mode",
        "lock",
        "read-only",
    ] {
        let (root, runtime) = runtime();
        write_identity(&runtime, &fixture(None));
        symlink("/active", runtime.join("eon")).unwrap();
        match case {
            "identity-mode" => {
                fs::set_permissions(runtime.join("identity"), fs::Permissions::from_mode(0o644))
                    .unwrap()
            }
            "identity-symlink" => {
                fs::remove_file(runtime.join("identity")).unwrap();
                symlink("/missing", runtime.join("identity")).unwrap();
            }
            "identity-directory" => {
                fs::remove_file(runtime.join("identity")).unwrap();
                fs::create_dir(runtime.join("identity")).unwrap();
            }
            "launcher-file" => {
                fs::remove_file(runtime.join("eon")).unwrap();
                fs::write(runtime.join("eon"), b"keep").unwrap();
            }
            "directory-mode" => {
                fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap()
            }
            _ => {}
        }
        let lock_path = supervisor_lock_path(&root, G);
        let held = (case == "lock")
            .then(|| crate::supervisor::lock_supervisor_lifecycle(&lock_path).unwrap());
        super::inspect_generation(
            G,
            "previous",
            runtime.clone(),
            &lock_path,
            "test",
            case != "read-only",
            case != "read-only",
            None,
        );
        if fs::symlink_metadata(runtime.join("identity")).is_err()
            || fs::symlink_metadata(runtime.join("eon")).is_err()
        {
            failures.push(case);
        }
        if let Some(held) = held {
            held.unlock().unwrap();
        }
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    assert!(failures.is_empty(), "unsafe cleanup: {failures:?}");
}

#[test]
fn identity_record_whole_replacement_and_projection_failure_keep_committed_facts() {
    let (root, runtime) = runtime();
    for path in [
        None,
        Some(b"/".as_slice()),
        Some(b"/raw-\xff".as_slice()),
        Some(vec![b'/'; 4096].as_slice()),
    ] {
        let bytes = fixture(path);
        write_identity(&runtime, &bytes);
        let record = Identity::read(&runtime).unwrap().unwrap();
        record.publish(&runtime).unwrap();
        assert_eq!(fs::read(runtime.join("identity")).unwrap(), bytes);
        assert_eq!(record.generation, G);
        assert_eq!(record.created_build, C);
        assert_eq!(record.active_build, A);
    }
    let mut record = Identity::read(&runtime).unwrap().unwrap();
    record.active_build = C.into();
    record.active_launcher = Some("/new-build/eon".into());
    record.publish(&runtime).unwrap();
    assert_eq!(Identity::read(&runtime).unwrap().unwrap(), record);
    let committed = fs::read(runtime.join("identity")).unwrap();
    fs::write(runtime.join("eon"), b"unexpected object").unwrap();
    assert!(record.project_launcher(&runtime).is_err());
    assert_eq!(fs::read(runtime.join("identity")).unwrap(), committed);
    fs::remove_file(runtime.join("eon")).unwrap();
    record.project_launcher(&runtime).unwrap();
    assert_eq!(
        fs::read_link(runtime.join("eon")).unwrap(),
        record.active_launcher.unwrap()
    );
    fs::remove_dir_all(root).unwrap();
}
