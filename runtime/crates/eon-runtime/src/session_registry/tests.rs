use super::*;
use std::fs;
use std::os::unix::{
    ffi::OsStrExt,
    fs::{PermissionsExt, symlink},
};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        Self(crate::supervisor::temporary_directory())
    }

    fn registry(&self) -> Registry {
        Registry {
            root: self.0.join("state/eon/sessions"),
            fault: None,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

// Catches name-as-identity, volatile allocation, and read-side initialization.
#[test]
fn c38_identity_and_optional_names_survive_reopen() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let missing = "000000000000000000000000000000000000000000000000";
    assert!(registry.list().unwrap().is_empty());
    assert_eq!(registry.lookup(missing).unwrap(), None);
    assert!(!registry.root.exists());

    let first = registry.create(Some("same")).unwrap();
    let second = fixture.registry().create(Some("same")).unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.id.len(), 48);
    assert!(
        first
            .id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_eq!(&first.id[..32], &second.id[..32]);
    assert_eq!(&first.id[32..], "0000000000000001");
    assert_eq!(&second.id[32..], "0000000000000002");
    let renamed = fixture
        .registry()
        .rename(&first.id, Some("renamed"))
        .unwrap()
        .unwrap();
    assert_eq!(renamed.id, first.id);
    assert_eq!(renamed.name.as_deref(), Some("renamed"));
    let cleared = registry.rename(&first.id, None).unwrap().unwrap();
    assert_eq!(cleared.id, first.id);
    assert_eq!(cleared.name, None);
    assert_eq!(
        fixture.registry().lookup(&first.id).unwrap(),
        Some(cleared.clone())
    );
    assert_eq!(fixture.registry().list().unwrap(), vec![cleared, second]);
    assert_eq!(registry.rename(missing, None).unwrap(), None);
    assert!(
        registry
            .lookup("same")
            .unwrap_err()
            .detail
            .contains("identity")
    );

    let default = Registry::new().unwrap();
    assert_eq!(
        default.root,
        crate::diagnostics::state_directory()
            .unwrap()
            .join("sessions")
    );
}

fn no_temporaries(registry: &Registry) {
    assert!(fs::read_dir(&registry.root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tmp-")
    }));
}

// Catches premature ID disclosure, allocator rollback and false write acknowledgements.
#[test]
fn c38_creation_distinguishes_reservation_and_publication_failures() {
    for (fault, publication, reserved, visible, next) in [
        (
            Fault::AllocatorBeforePublish,
            Publication::Definitive,
            false,
            false,
            "0000000000000002",
        ),
        (
            Fault::AllocatorDirectorySync,
            Publication::Uncertain,
            false,
            false,
            "0000000000000003",
        ),
        (
            Fault::RecordBeforePublish,
            Publication::Definitive,
            true,
            false,
            "0000000000000003",
        ),
        (
            Fault::RecordDirectorySync,
            Publication::Uncertain,
            true,
            true,
            "0000000000000003",
        ),
    ] {
        let fixture = Fixture::new();
        let mut registry = fixture.registry();
        let original = registry.create(Some("original")).unwrap();
        let allocator_path = registry.root.join("allocator.json");
        let original_allocator = fs::read(&allocator_path).unwrap();
        registry.fault = Some(fault);
        let error = registry.create(Some("failed")).unwrap_err();
        assert_eq!(error.publication, publication);
        let burned = format!("{}0000000000000002", &original.id[..32]);
        assert_eq!(
            error.reserved_id.as_deref(),
            reserved.then_some(burned.as_str())
        );
        let reopened = fixture.registry();
        assert_eq!(reopened.lookup(&burned).unwrap().is_some(), visible);
        assert_eq!(reopened.lookup(&original.id).unwrap(), Some(original));
        if fault == Fault::AllocatorBeforePublish {
            assert_eq!(fs::read(&allocator_path).unwrap(), original_allocator);
        }
        no_temporaries(&reopened);
        assert!(reopened.create(None).unwrap().id.ends_with(next));
        if reserved && !visible {
            assert_eq!(reopened.lookup(&burned).unwrap(), None);
        }
    }
}

// Catches an uncertain rename being reported as success or rolled back on disk.
#[test]
fn c38_rename_preserves_old_or_published_state_without_claiming_rollback() {
    for (fault, publication, name) in [
        (
            Fault::RecordBeforePublish,
            Publication::Definitive,
            Some("old"),
        ),
        (Fault::RecordDirectorySync, Publication::Uncertain, None),
    ] {
        let fixture = Fixture::new();
        let mut registry = fixture.registry();
        let record = registry.create(Some("old")).unwrap();
        let allocator = fs::read(registry.root.join("allocator.json")).unwrap();
        registry.fault = Some(fault);
        let error = registry.rename(&record.id, None).unwrap_err();
        assert_eq!(error.publication, publication);
        assert_eq!(error.reserved_id, None);
        let current = fixture.registry().lookup(&record.id).unwrap().unwrap();
        assert_eq!(current.id, record.id);
        assert_eq!(current.name.as_deref(), name);
        assert_eq!(
            fs::read(registry.root.join("allocator.json")).unwrap(),
            allocator
        );
        no_temporaries(&registry);
    }
}

// Catches replacement of a lost lock inode, which would split concurrent writers.
#[test]
fn c38_missing_allocator_or_lock_is_not_repaired() {
    for missing in ["allocator.json", "registry.lock"] {
        let fixture = Fixture::new();
        let registry = fixture.registry();
        let record = registry.create(Some("kept")).unwrap();
        fs::remove_file(registry.root.join(missing)).unwrap();
        let original = fs::read(registry.record_path(&record.id)).unwrap();
        assert!(registry.list().is_err());
        assert!(registry.lookup(&record.id).is_err());
        assert!(registry.rename(&record.id, None).is_err());
        assert!(registry.create(None).is_err());
        assert!(!registry.root.join(missing).exists());
        assert_eq!(
            fs::read(registry.record_path(&record.id)).unwrap(),
            original
        );
    }
}

fn reject_without_changing_files(registry: &Registry, id: &str) {
    let before: Vec<_> = fs::read_dir(&registry.root)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    for result in [
        registry.list().map(|_| ()),
        registry.lookup(id).map(|_| ()),
        registry.create(None).map(|_| ()),
        registry.rename(id, Some("changed")).map(|_| ()),
    ] {
        let error = result.unwrap_err();
        assert_eq!(error.publication, Publication::Definitive);
        assert_eq!(error.reserved_id, None);
    }
    assert_eq!(fs::read_dir(&registry.root).unwrap().count(), before.len());
    for (path, bytes) in before {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

// Catches reuse of an absent burned ID when a later registration exposes allocator rollback.
#[test]
fn c38_stale_allocator_with_a_burned_gap_is_preserved() {
    let fixture = Fixture::new();
    let mut registry = fixture.registry();
    let first = registry.create(Some("first")).unwrap();
    let path = registry.root.join("allocator.json");
    let stale = fs::read(&path).unwrap();
    registry.fault = Some(Fault::RecordBeforePublish);
    let burned = registry.create(None).unwrap_err().reserved_id.unwrap();
    assert!(burned.ends_with("0000000000000002"));
    registry.fault = None;
    let third = registry.create(Some("third")).unwrap();
    assert!(third.id.ends_with("0000000000000003"));
    assert_eq!(registry.lookup(&burned).unwrap(), None);
    fs::write(path, stale).unwrap();
    reject_without_changing_files(&fixture.registry(), &first.id);
}

// Catches partial list results and writes that silently repair unsupported durable data.
#[test]
fn c38_malformed_records_and_allocators_are_preserved() {
    for allocator in [false, true] {
        for defect in [
            "truncated",
            "version",
            "identity",
            "missing-field",
            "extra-field",
            "oversized",
        ] {
            let fixture = Fixture::new();
            let registry = fixture.registry();
            let good = registry.create(Some("good")).unwrap();
            let other = registry.create(Some("other")).unwrap();
            let path = if allocator {
                registry.root.join("allocator.json")
            } else {
                registry.record_path(&other.id)
            };
            let mut value: serde_json::Value =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            match defect {
                "version" => value["format"] = 99.into(),
                "identity" if allocator => value["namespace"] = "A".repeat(32).into(),
                "identity" => value["id"] = good.id.clone().into(),
                "missing-field" => {
                    value.as_object_mut().unwrap().remove("format");
                }
                "extra-field" => value["directory"] = "/old/prototype".into(),
                _ => {}
            }
            let bytes = match defect {
                "truncated" => b"{\"format\":".to_vec(),
                "oversized" => vec![b' '; 8193],
                _ => serde_json::to_vec(&value).unwrap(),
            };
            fs::write(&path, bytes).unwrap();
            reject_without_changing_files(&registry, &good.id);
        }
    }
}

// Catches invalid filename/ID agreement and conflating no name with an invalid stored name.
#[test]
fn c38_invalid_ids_names_and_filenames_are_preserved() {
    for defect in [
        "uppercase-file",
        "short-file",
        "non-utf8-file",
        "empty-name",
        "control-name",
        "long-name",
        "foreign-namespace",
        "invalid-id",
    ] {
        let fixture = Fixture::new();
        let registry = fixture.registry();
        let record = registry.create(Some("kept")).unwrap();
        let path = registry.record_path(&record.id);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match defect {
            "uppercase-file" => {
                fs::rename(
                    &path,
                    registry.root.join(format!("{}.json", "A".repeat(48))),
                )
                .unwrap();
            }
            "short-file" => {
                fs::rename(&path, registry.root.join("123.json")).unwrap();
            }
            "non-utf8-file" => {
                fs::rename(
                    &path,
                    registry
                        .root
                        .join(std::ffi::OsStr::from_bytes(b"\xff.json")),
                )
                .unwrap();
            }
            "foreign-namespace" => {
                value["id"] = "f".repeat(48).into();
                fs::remove_file(&path).unwrap();
                let foreign = registry.record_path(&"f".repeat(48));
                fs::write(&foreign, serde_json::to_vec(&value).unwrap()).unwrap();
                fs::set_permissions(foreign, fs::Permissions::from_mode(0o600)).unwrap();
            }
            _ => {
                match defect {
                    "empty-name" => value["name"] = "".into(),
                    "control-name" => value["name"] = "bad\nname".into(),
                    "long-name" => value["name"] = "🦀".repeat(65).into(),
                    "invalid-id" => value["id"] = "../escape".into(),
                    _ => unreachable!(),
                }
                fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
            }
        }
        reject_without_changing_files(&registry, &record.id);
    }
}

// Catches a wrapping allocator and overwriting a registration after allocator rollback.
#[test]
fn c38_sequence_exhaustion_and_collision_refuse_without_mutation() {
    for (sequence, diagnostic) in [(u64::MAX, "exhausted"), (0, "stale")] {
        let fixture = Fixture::new();
        let registry = fixture.registry();
        let record = registry.create(Some("kept")).unwrap();
        let path = registry.root.join("allocator.json");
        let mut allocator: Allocator = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        allocator.sequence = sequence;
        fs::write(&path, serde_json::to_vec(&allocator).unwrap()).unwrap();
        let original = fs::read(&path).unwrap();
        let original_record = fs::read(registry.record_path(&record.id)).unwrap();
        let error = registry.create(None).unwrap_err();
        assert!(error.detail.contains(diagnostic), "{}", error.detail);
        assert_eq!(error.publication, Publication::Definitive);
        assert_eq!(error.reserved_id, None);
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(
            fs::read(registry.record_path(&record.id)).unwrap(),
            original_record
        );
    }
}

// Catches the prototype's byte-based name limit and an off-by-one record bound.
#[test]
fn c38_current_name_rules_and_exact_record_bound() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    for name in [String::new(), "bad\0name".into(), "x".repeat(65)] {
        assert!(registry.create(Some(&name)).is_err());
        assert!(!registry.root.exists());
    }
    let unicode = "🦀".repeat(64);
    let record = registry.create(Some(&unicode)).unwrap();
    assert_eq!(record.name.as_deref(), Some(unicode.as_str()));
    assert!(registry.rename(&record.id, Some("")).is_err());
    assert_eq!(registry.lookup(&record.id).unwrap(), Some(record.clone()));
    for path in [
        registry.root.join("allocator.json"),
        registry.record_path(&record.id),
    ] {
        let mut bytes = fs::read(&path).unwrap();
        bytes.resize(8192, b' ');
        fs::write(&path, bytes).unwrap();
    }
    assert_eq!(registry.list().unwrap(), vec![record.clone()]);
    assert_eq!(
        registry.rename(&record.id, None).unwrap().unwrap().name,
        None
    );
    let json: serde_json::Value =
        serde_json::from_slice(&fs::read(registry.record_path(&record.id)).unwrap()).unwrap();
    assert!(json.get("name").is_none());
}

// Catches following links, blocking on FIFOs, or chmod-repair of unsafe durable files.
#[test]
fn c38_unsafe_files_fail_without_repair_or_following_links() {
    for role in ["record", "allocator.json", "registry.lock"] {
        for kind in [
            "permissions",
            "special-bits",
            "directory",
            "symlink",
            "dangling",
            "fifo",
        ] {
            let fixture = Fixture::new();
            let registry = fixture.registry();
            let record = registry.create(Some("kept")).unwrap();
            let path = if role == "record" {
                registry.record_path(&record.id)
            } else {
                registry.root.join(role)
            };
            let outside = fixture.0.join("outside");
            fs::write(&outside, b"untouched").unwrap();
            match kind {
                "permissions" => {
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap()
                }
                "special-bits" => {
                    // Sticky bit tests exact-mode rejection without setid privileges.
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o1600)).unwrap();
                    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o7777, 0o1600);
                }
                _ => {
                    fs::remove_file(&path).unwrap();
                    match kind {
                        "directory" => fs::create_dir(&path).unwrap(),
                        "symlink" => symlink(&outside, &path).unwrap(),
                        "dangling" => symlink(fixture.0.join("missing"), &path).unwrap(),
                        "fifo" => {
                            let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
                            // SAFETY: path is a live NUL-terminated filename owned by this fixture.
                            assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                        }
                        _ => unreachable!(),
                    }
                }
            }
            let before = fs::symlink_metadata(&path).unwrap();
            for result in [
                registry.list().map(|_| ()),
                registry.lookup(&record.id).map(|_| ()),
                registry.create(None).map(|_| ()),
                registry.rename(&record.id, None).map(|_| ()),
            ] {
                let error = result.unwrap_err();
                assert_eq!(error.publication, Publication::Definitive, "{role}: {kind}");
                assert_eq!(error.reserved_id, None, "{role}: {kind}");
                assert!(
                    error.detail.contains(&path.display().to_string()),
                    "{role}: {kind}: {error}"
                );
            }
            let after = fs::symlink_metadata(&path).unwrap();
            assert_eq!(
                (after.ino(), after.mode(), after.uid()),
                (before.ino(), before.mode(), before.uid())
            );
            assert_eq!(fs::read(&outside).unwrap(), b"untouched");
        }
    }
}

#[test]
fn c38_unsafe_registry_directories_are_preserved() {
    for kind in ["permissions", "symlink", "file"] {
        let fixture = Fixture::new();
        let registry = fixture.registry();
        registry.create(None).unwrap();
        if kind == "permissions" {
            fs::set_permissions(&registry.root, fs::Permissions::from_mode(0o755)).unwrap();
        } else {
            let saved = fixture.0.join("saved");
            fs::rename(&registry.root, &saved).unwrap();
            if kind == "symlink" {
                symlink(saved, &registry.root).unwrap();
            } else {
                fs::write(&registry.root, b"untouched").unwrap();
            }
        }
        let before = fs::symlink_metadata(&registry.root).unwrap();
        for result in [
            registry.list().map(|_| ()),
            registry.create(None).map(|_| ()),
        ] {
            let error = result.unwrap_err();
            assert_eq!(error.publication, Publication::Definitive);
            assert_eq!(error.reserved_id, None);
            assert!(
                error.detail.contains(&registry.root.display().to_string()),
                "{error}"
            );
        }
        let after = fs::symlink_metadata(&registry.root).unwrap();
        assert_eq!(
            (after.ino(), after.mode(), after.uid()),
            (before.ino(), before.mode(), before.uid())
        );
    }
}

// First initialization works below an existing state directory with an unreadable ancestor.
#[test]
fn c38_first_creation_below_an_unreadable_ancestor() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(fixture.0.join("state"))
        .unwrap();
    assert!(!registry.root.exists());
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o111)).unwrap();
    let ancestor_read = File::open(&fixture.0);
    let created = registry.create(Some("first"));
    let listed = registry.list();
    let second = registry.create(None);
    let ancestor_mode = fs::symlink_metadata(&fixture.0).unwrap().mode() & 0o7777;
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        ancestor_read.unwrap_err().kind(),
        std::io::ErrorKind::PermissionDenied
    );
    assert_eq!(ancestor_mode, 0o111);
    let first = created.unwrap();
    assert!(first.id.ends_with("0000000000000001"));
    assert_eq!(first.name.as_deref(), Some("first"));
    assert_eq!(listed.unwrap(), vec![first]);
    assert!(second.unwrap().id.ends_with("0000000000000002"));
}

// Catches creating a child before its parent can be opened for the durability barrier.
#[test]
fn c38_missing_state_home_requires_a_sync_capable_parent() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o333)).unwrap();
    let created = registry.create(None);
    fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o700)).unwrap();
    let error = created.unwrap_err();
    assert_eq!(error.publication, Publication::Definitive);
    assert_eq!(error.reserved_id, None);
    assert!(!fixture.0.join("state").exists());
}

// A retry must sync its first existing parent before creating more directories.
#[test]
fn c38_initialization_retry_syncs_the_interrupted_creation_chain() {
    let fixture = Fixture::new();
    let mut workers = Workers(vec![
        worker_command(&fixture)
            .env("EON_C38_TEST_EXIT_AFTER_MKDIR", "1")
            .spawn()
            .unwrap(),
    ]);
    assert_eq!(workers.outputs()[0].status.code(), Some(74));
    let mut registry = fixture.registry();
    let state = fixture.0.join("state");
    let inode = fs::metadata(&state).unwrap().ino();
    assert!(!state.join("eon").exists());
    registry.fault = Some(Fault::InitializationParentSync);
    let error = registry.create(None).unwrap_err();
    assert_eq!(error.publication, Publication::Definitive);
    assert_eq!(error.reserved_id, None);
    assert_eq!(
        error.detail,
        format!(
            "session registry {}: injected initialization parent sync failure",
            state.display()
        )
    );
    assert_eq!(fs::metadata(&state).unwrap().ino(), inode);
    assert!(!state.join("eon").exists());
    assert!(!registry.root.join("registry.lock").exists());
    assert!(!registry.root.join("allocator.json").exists());
    registry.fault = None;
    let first = registry.create(Some("first")).unwrap();
    assert!(first.id.ends_with("0000000000000001"));
    assert_eq!(fixture.registry().list().unwrap(), vec![first]);
}

// State-home paths can have existing linked ancestors; the registry itself must be private.
#[test]
fn c38_linked_state_home_and_abandoned_temporaries() {
    let fixture = Fixture::new();
    let mut registry = fixture.registry();
    let actual = fixture.0.join("actual-state");
    fs::create_dir(&actual).unwrap();
    symlink(actual, fixture.0.join("state")).unwrap();
    let record = registry.create(None).unwrap();
    let stale = registry.root.join(".tmp-abandoned");
    fs::write(&stale, b"diagnostic artifact").unwrap();
    registry.fault = Some(Fault::RecordBeforePublish);
    assert!(registry.rename(&record.id, Some("failed")).is_err());
    assert_eq!(fs::read(&stale).unwrap(), b"diagnostic artifact");
    assert_eq!(registry.list().unwrap(), vec![record]);
}

// Open readers retain complete old objects when publication replaces their paths.
#[test]
fn c38_publication_replaces_files_without_truncating_open_readers() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let record = registry.create(Some("old")).unwrap();
    let path = registry.record_path(&record.id);
    let mut old_record = File::open(&path).unwrap();
    let mut old_allocator = File::open(registry.root.join("allocator.json")).unwrap();
    registry.rename(&record.id, None).unwrap();
    registry.create(None).unwrap();
    let mut bytes = Vec::new();
    old_record.read_to_end(&mut bytes).unwrap();
    assert_eq!(
        serde_json::from_slice::<Registration>(&bytes).unwrap(),
        record
    );
    assert_ne!(
        old_record.metadata().unwrap().ino(),
        fs::metadata(path).unwrap().ino()
    );
    bytes.clear();
    old_allocator.read_to_end(&mut bytes).unwrap();
    assert_eq!(
        serde_json::from_slice::<Allocator>(&bytes)
            .unwrap()
            .sequence,
        1
    );
    let current = registry.lookup(&record.id).unwrap().unwrap();
    assert_eq!(current.name, None);
    assert_eq!(registry.list().unwrap().len(), 2);
}

use std::process::{Child, Command, Output, Stdio};

struct Workers(Vec<Child>);

impl Workers {
    fn outputs(&mut self) -> Vec<Output> {
        let deadline = Instant::now() + Duration::from_secs(15);
        while self
            .0
            .iter_mut()
            .any(|child| child.try_wait().unwrap().is_none())
        {
            assert!(Instant::now() < deadline, "registry child did not exit");
            thread::sleep(Duration::from_millis(10));
        }
        self.0
            .drain(..)
            .map(|child| child.wait_with_output().unwrap())
            .collect()
    }
}

impl Drop for Workers {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn worker_command(fixture: &Fixture) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "session_registry::tests::c38_process_writer",
            "--nocapture",
        ])
        .env("EON_C38_TEST_DIRECTORY", &fixture.0)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

pub(super) fn initialization_rendezvous() {
    let directory = PathBuf::from(std::env::var_os("EON_C38_TEST_DIRECTORY").unwrap());
    let worker = std::env::var("EON_C38_TEST_WORKER").unwrap();
    fs::write(directory.join(format!("ready-{worker}")), b"ready").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !directory.join("initialize").exists() {
        assert!(Instant::now() < deadline, "initialization was not released");
        thread::sleep(Duration::from_millis(10));
    }
}

// Invoked as a separate test process by the process-boundary tests.
#[test]
fn c38_process_writer() {
    let Some(directory) = std::env::var_os("EON_C38_TEST_DIRECTORY") else {
        return;
    };
    let directory = PathBuf::from(directory);
    let mut registry = Registry {
        root: directory.join("state/eon/sessions"),
        fault: None,
    };
    if std::env::var_os("EON_C38_TEST_EXIT_AFTER_MKDIR").is_some() {
        registry.fault = Some(Fault::ExitAfterDirectoryCreate);
        registry.create(None).unwrap();
        panic!("initialization fault must exit before parent sync");
    }
    if std::env::var_os("EON_C38_TEST_EXIT").is_some() {
        registry.fault = Some(Fault::ExitAfterReservation);
        registry.create(None).unwrap();
        panic!("reservation fault must exit before registration");
    }
    let worker = std::env::var("EON_C38_TEST_WORKER").unwrap();
    if std::env::var_os("EON_C38_TEST_INITIALIZE").is_some() {
        registry.fault = Some(Fault::WaitBeforeDirectoryCreate);
    } else {
        fs::write(directory.join(format!("ready-{worker}")), b"ready").unwrap();
    }
    for sequence in 0..10 {
        let record = retry_busy(|| registry.create(Some("same")));
        retry_busy(|| registry.rename(&record.id, Some(&format!("worker-{worker}-{sequence}"))))
            .unwrap();
    }
}

// Catches per-instance caches, lost allocations/renames, lock replacement and a host count cap.
#[test]
fn c38_processes_serialize_allocations_and_renames() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    registry.prepare_directory().unwrap();
    let lock = registry.lock(true, false).unwrap().unwrap();
    let inode = lock.metadata().unwrap().ino();
    let mut workers = Workers(Vec::new());
    for worker in 0..4 {
        workers.0.push(
            worker_command(&fixture)
                .env("EON_C38_TEST_WORKER", worker.to_string())
                .spawn()
                .unwrap(),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !(0..4).all(|worker| fixture.0.join(format!("ready-{worker}")).exists()) {
        assert!(
            Instant::now() < deadline,
            "registry children did not reach the held lock"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        workers
            .0
            .iter_mut()
            .all(|child| child.try_wait().unwrap().is_none())
    );
    drop(lock);
    for output in workers.outputs() {
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let records = fixture.registry().list().unwrap();
    assert_eq!(records.len(), 40);
    assert!(records.windows(2).all(|pair| pair[0].id < pair[1].id));
    assert!(records.first().unwrap().id.ends_with("0000000000000001"));
    assert!(records.last().unwrap().id.ends_with("0000000000000028"));
    for worker in 0..4 {
        for sequence in 0..10 {
            let name = format!("worker-{worker}-{sequence}");
            assert_eq!(
                records
                    .iter()
                    .filter(|record| record.name.as_deref() == Some(&name))
                    .count(),
                1
            );
        }
    }
    assert_eq!(
        fs::metadata(registry.root.join("registry.lock"))
            .unwrap()
            .ino(),
        inode
    );
    assert_eq!(fs::metadata(&registry.root).unwrap().mode() & 0o7777, 0o700);
    for entry in fs::read_dir(&registry.root).unwrap() {
        assert_eq!(entry.unwrap().metadata().unwrap().mode() & 0o7777, 0o600);
    }
    no_temporaries(&registry);
}

// All children open the parent and observe the missing chain before any mkdir can win.
#[test]
fn c38_processes_initialize_an_absent_registry() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let mut workers = Workers(Vec::new());
    for worker in 0..4 {
        workers.0.push(
            worker_command(&fixture)
                .env("EON_C38_TEST_WORKER", worker.to_string())
                .env("EON_C38_TEST_INITIALIZE", "1")
                .spawn()
                .unwrap(),
        );
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !(0..4).all(|worker| fixture.0.join(format!("ready-{worker}")).exists()) {
        assert!(Instant::now() < deadline, "initializers did not rendezvous");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!fixture.0.join("state").exists());
    fs::write(fixture.0.join("initialize"), b"go").unwrap();
    for output in workers.outputs() {
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let records = registry.list().unwrap();
    assert_eq!(records.len(), 40);
    assert!(records.windows(2).all(|pair| pair[0].id < pair[1].id));
    assert!(records.first().unwrap().id.ends_with("0000000000000001"));
    assert!(records.last().unwrap().id.ends_with("0000000000000028"));
    assert!(
        records
            .iter()
            .all(|record| record.name.as_deref().unwrap().starts_with("worker-"))
    );
    no_temporaries(&registry);
}

// A process can die with an allocated ID and no registration; a later process must skip it.
#[test]
fn c38_process_exit_after_reservation_burns_the_sequence() {
    let fixture = Fixture::new();
    let mut workers = Workers(vec![
        worker_command(&fixture)
            .env("EON_C38_TEST_EXIT", "1")
            .spawn()
            .unwrap(),
    ]);
    assert_eq!(workers.outputs()[0].status.code(), Some(73));
    let registry = fixture.registry();
    assert!(registry.list().unwrap().is_empty());
    let record = registry.create(None).unwrap();
    assert!(record.id.ends_with("0000000000000002"));
    let burned = format!("{}0000000000000001", &record.id[..32]);
    assert_eq!(registry.lookup(&burned).unwrap(), None);
    no_temporaries(&registry);
}

#[test]
fn c38_lock_contention_is_immediately_busy_without_mutating_state() {
    let fixture = Fixture::new();
    let registry = fixture.registry();
    let record = registry.create(None).unwrap();
    let lock = registry.lock(false, false).unwrap().unwrap();
    let allocator = fs::read(registry.root.join("allocator.json")).unwrap();
    let start = Instant::now();
    let error = registry.create(None).unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(error.busy);
    assert_eq!(
        registry
            .rename(&record.id, Some("busy"))
            .unwrap_err()
            .into_control()
            .code,
        "registry-busy"
    );
    assert_eq!(error.publication, Publication::Definitive);
    assert_eq!(error.reserved_id, None);
    assert_eq!(
        fs::read(registry.root.join("allocator.json")).unwrap(),
        allocator
    );
    drop(lock);
    assert_eq!(registry.list().unwrap(), vec![record]);
}

fn retry_busy<T>(mut operation: impl FnMut() -> Result<T, Failure>) -> T {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match operation() {
            Ok(value) => return value,
            Err(error) if error.busy && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(error) => panic!("{error}"),
        }
    }
}

#[test]
fn c38_readers_share_the_lock_and_initial_registration_retries_busy() {
    let fixture = Fixture::new();
    let mut registry = fixture.registry();
    let first = registry.create(None).unwrap();
    let reader = registry.lock(false, true).unwrap().unwrap();
    assert_eq!(registry.list().unwrap(), vec![first.clone()]);
    assert_eq!(registry.lookup(&first.id).unwrap(), Some(first));
    registry.inject(Some(Fault::ObserveBusyRetry));
    let mut worker = Worker::start(registry.clone()).unwrap();
    worker.submit(Operation::Register).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !registry.root.join(".tmp-test-retrying").exists() {
        assert!(Instant::now() < deadline, "worker did not observe busy");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(
        worker.poll().is_none(),
        "transient busy must not finish registration"
    );
    assert_eq!(
        registry.create(None).unwrap_err().into_control().code,
        "registry-busy"
    );
    drop(reader);
    let deadline = Instant::now() + Duration::from_secs(5);
    let registered = loop {
        if let Some(result) = worker.poll() {
            break result.unwrap().unwrap();
        }
        assert!(
            Instant::now() < deadline,
            "registration did not retry after the reader released"
        );
        thread::sleep(Duration::from_millis(5));
    };
    assert!(registered.id.ends_with("0000000000000002"));
    assert_eq!(registry.list().unwrap().len(), 2);
}
