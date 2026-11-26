use super::*;
use crate::{
    checkpoint::{Session, tests::fixture},
    supervisor::{prepare_runtime, temporary_directory},
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(10);

struct Runtime {
    root: PathBuf,
    path: PathBuf,
    original: Checkpoint,
}

impl Runtime {
    fn new() -> Self {
        let root = temporary_directory();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let mut original = fixture();
        original.header.namespace = root.to_str().unwrap().into();
        let path = path(&original.header);
        prepare_runtime(path.parent().unwrap()).unwrap();
        Self {
            root,
            path,
            original,
        }
    }

    fn seed(&self) -> Vec<u8> {
        let bytes = self.original.encode().unwrap();
        private_file::publish(&self.path, &bytes).unwrap();
        bytes
    }

    fn worker(&self) -> Worker {
        Worker::start(self.original.header.clone()).unwrap()
    }

    fn next_core(&self) -> Core {
        let mut core = self.original.core.clone();
        core.revision += 1;
        core
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

fn completion(worker: &mut Worker) -> Completion {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(completion) = worker.poll().unwrap() {
            return completion;
        }
        assert!(
            Instant::now() < deadline,
            "checkpoint worker did not finish"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

fn submit_and_wait(worker: &mut Worker, core: Core) -> Result<(), Error> {
    let revision = core.revision;
    worker.submit(core).unwrap();
    let (completed, result) = completion(worker);
    assert_eq!(completed, revision);
    result
}

fn hold(runtime: &Runtime, at: Stage) -> (Worker, Receiver<()>, mpsc::Sender<()>) {
    let (arrived, waiting) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let mut gate = Some((arrived, resume));
    let worker = Worker::start_with(runtime.original.header.clone(), move |stage| {
        if stage == at
            && let Some((arrived, resume)) = gate.take()
        {
            arrived.send(()).unwrap();
            resume.recv_timeout(WAIT).unwrap();
        }
        Ok(())
    })
    .unwrap();
    (worker, waiting, release)
}

#[test]
fn real_file_failures_distinguish_failed_and_uncertain_publication() {
    let runtime = Runtime::new();
    let mut expected = runtime.original.clone();
    expected.core = runtime.next_core();
    let updated = expected.encode().unwrap();
    for fault in [
        Stage::Write,
        Stage::FileSync,
        Stage::Rename,
        Stage::DirectorySync,
    ] {
        let old = runtime.seed();
        let mut worker = Worker::start_with(runtime.original.header.clone(), move |stage| {
            if stage == fault {
                return Err(format!("injected {stage:?} failure").into());
            }
            Ok(())
        })
        .unwrap();
        let result = submit_and_wait(&mut worker, expected.core.clone());
        let bytes = if fault == Stage::DirectorySync {
            assert!(matches!(
                result,
                Err(Error::Publish(PublishError::Uncertain(_)))
            ));
            assert_eq!(
                worker.submit(expected.core.clone()),
                Err(Error::StaleRevision)
            );
            &updated
        } else {
            assert_eq!(
                result,
                Err(Error::Publish(PublishError::Failed(format!(
                    "injected {fault:?} failure"
                ))))
            );
            &old
        };
        assert_eq!(&fs::read(&runtime.path).unwrap(), bytes, "{fault:?}");
        assert!(
            !runtime
                .path
                .with_file_name(".recovery.checkpoint.tmp")
                .exists()
        );
    }
}

#[test]
fn retries_only_the_latest_collected_definite_failure() {
    let runtime = Runtime::new();
    let old = runtime.seed();
    let mut failures = 2;
    let mut worker = Worker::start_with(runtime.original.header.clone(), move |stage| {
        if stage == Stage::Rename && failures != 0 {
            failures -= 1;
            return Err("injected rename failure".to_owned().into());
        }
        Ok(())
    })
    .unwrap();
    let first = runtime.next_core();
    let mut latest = first.clone();
    latest.revision += 1;
    worker.submit(first.clone()).unwrap();
    worker.submit(latest.clone()).unwrap();

    let failed = |revision| {
        (
            revision,
            Err(Error::Publish(PublishError::Failed(
                "injected rename failure".into(),
            ))),
        )
    };
    assert_eq!(completion(&mut worker), failed(first.revision));
    assert_eq!(worker.submit(first.clone()), Err(Error::StaleRevision));
    assert_eq!(worker.submit(latest.clone()), Err(Error::StaleRevision));
    assert_eq!(completion(&mut worker), failed(latest.revision));
    assert_eq!(worker.submit(first), Err(Error::StaleRevision));
    assert_eq!(fs::read(&runtime.path).unwrap(), old);
    submit_and_wait(&mut worker, latest.clone()).unwrap();
    assert_eq!(worker.submit(latest.clone()), Err(Error::StaleRevision));
    let expected = Checkpoint {
        core: latest,
        ..runtime.original.clone()
    };
    assert_eq!(fs::read(&runtime.path).unwrap(), expected.encode().unwrap());
}

#[test]
fn held_write_queues_a_new_revision_and_preserves_opaque_sections() {
    let runtime = Runtime::new();
    let old = runtime.seed();
    let (mut worker, waiting, release) = hold(&runtime, Stage::Rename);
    let first = runtime.next_core();
    let mut next = first.clone();
    next.revision += 1;
    next.sessions.reverse();
    next.terminal_id_high_water += 1;
    worker.submit(first.clone()).unwrap();
    waiting.recv_timeout(WAIT).unwrap();
    assert_eq!(worker.poll(), Ok(None));
    assert_eq!(fs::read(&runtime.path).unwrap(), old);

    worker.submit(next.clone()).unwrap();
    assert_eq!(worker.submit(first.clone()), Err(Error::StaleRevision));
    assert_eq!(
        worker.submit(Core {
            revision: next.revision + 1,
            ..next.clone()
        }),
        Err(Error::Busy)
    );
    release.send(()).unwrap();
    assert_eq!(completion(&mut worker), (first.revision, Ok(())));
    assert_eq!(completion(&mut worker), (next.revision, Ok(())));

    drop(worker);
    assert_eq!(
        submit_and_wait(&mut runtime.worker(), first),
        Err(Error::StaleRevision)
    );
    let expected = Checkpoint {
        core: next,
        ..runtime.original.clone()
    };
    assert_eq!(fs::read(&runtime.path).unwrap(), expected.encode().unwrap());
    assert_eq!(
        read(&runtime.path, &expected.header).unwrap(),
        Some(expected)
    );
}

#[test]
fn cancellation_fences_held_and_queued_revisions() {
    let runtime = Runtime::new();
    let old = runtime.seed();
    let (mut worker, waiting, release) = hold(&runtime, Stage::Rename);
    let mut core = runtime.next_core();
    worker.submit(core.clone()).unwrap();
    waiting.recv_timeout(WAIT).unwrap();
    core.revision += 1;
    worker.submit(core.clone()).unwrap();

    worker.cancel();
    assert!(!worker.is_fenced());
    assert_eq!(worker.poll(), Ok(None));
    release.send(()).unwrap();
    for revision in [core.revision - 1, core.revision] {
        assert_eq!(
            completion(&mut worker),
            (revision, Err(Error::Publish(PublishError::Cancelled)))
        );
    }
    assert!(worker.is_fenced());
    assert_eq!(fs::read(&runtime.path).unwrap(), old);
    assert!(
        !runtime
            .path
            .with_file_name(".recovery.checkpoint.tmp")
            .exists()
    );
    fs::remove_file(&runtime.path).unwrap();
    core.revision += 1;
    assert_eq!(worker.submit(core), Err(Error::Closed));
    assert_eq!(worker.poll(), Ok(None));
    assert!(!runtime.path.exists());
}

#[test]
fn cancellation_drains_a_rename_already_published_before_acknowledging() {
    let runtime = Runtime::new();
    let (mut worker, waiting, release) = hold(&runtime, Stage::DirectorySync);
    let core = runtime.next_core();
    worker.submit(core.clone()).unwrap();
    waiting.recv_timeout(WAIT).unwrap();
    let saved = read(&runtime.path, &runtime.original.header)
        .unwrap()
        .unwrap();
    assert_eq!(saved.core, core);
    assert!(saved.sections.is_empty());
    worker.cancel();
    assert_eq!(worker.poll(), Ok(None));
    assert!(!worker.is_fenced());
    release.send(()).unwrap();
    assert_eq!(completion(&mut worker), (core.revision, Ok(())));
    assert!(worker.is_fenced());
}

#[test]
fn refuses_a_decrease_in_terminal_high_water_without_replacing_the_checkpoint() {
    let runtime = Runtime::new();
    let old = runtime.seed();
    let mut core = runtime.next_core();
    core.terminal_id_high_water -= 1;
    Checkpoint {
        core: core.clone(),
        ..runtime.original.clone()
    }
    .encode()
    .unwrap();
    assert_eq!(
        submit_and_wait(&mut runtime.worker(), core),
        Err(Error::TerminalHighWaterDecreased)
    );
    assert_eq!(fs::read(&runtime.path).unwrap(), old);
}

#[test]
fn namespace_spellings_preserve_scope_bytes_and_refuse_linked_entries() {
    let mut runtime = Runtime::new();
    let link = runtime.root.join("linked-namespace");
    symlink(&runtime.root, &link).unwrap();

    for suffix in ["", "///", "/.", "/.//./"] {
        runtime.original.header.namespace = format!("{}{suffix}", runtime.root.display());
        runtime.seed();
        let core = runtime.next_core();
        submit_and_wait(&mut runtime.worker(), core.clone()).unwrap();
        let expected = Checkpoint {
            core,
            ..runtime.original.clone()
        }
        .encode()
        .unwrap();
        assert_eq!(fs::read(&runtime.path).unwrap(), expected);

        let mut header = runtime.original.header.clone();
        header.namespace = format!("{}{suffix}", link.display());
        let mut worker = Worker::start(header).unwrap();
        assert!(matches!(
            submit_and_wait(&mut worker, runtime.next_core()),
            Err(Error::Io(detail)) if detail.contains("must be an owned private directory")
        ));
        assert_eq!(fs::read(&runtime.path).unwrap(), expected);
    }
    assert_eq!(fs::read_link(&link).unwrap(), runtime.root);
}

#[test]
fn refuses_a_linked_generations_directory_without_touching_its_target() {
    let runtime = Runtime::new();
    let old = runtime.seed();
    let generations = runtime.root.join("generations");
    let target = runtime.root.join("moved-generations");
    fs::rename(&generations, &target).unwrap();
    symlink(&target, &generations).unwrap();

    assert!(matches!(
        submit_and_wait(&mut runtime.worker(), runtime.next_core()),
        Err(Error::Io(detail)) if detail.contains("must be an owned private directory")
    ));
    assert_eq!(fs::read_link(&generations).unwrap(), target);
    let relative_path = runtime.path.strip_prefix(&generations).unwrap();
    assert_eq!(fs::read(target.join(relative_path)).unwrap(), old);
}

#[test]
fn refuses_nonprivate_files_and_symlinks_without_replacing_them() {
    let runtime = Runtime::new();
    let old = runtime.seed();
    let target = runtime.path.with_extension("target");
    fs::rename(&runtime.path, &target).unwrap();
    symlink(&target, &runtime.path).unwrap();
    assert!(matches!(
        submit_and_wait(&mut runtime.worker(), runtime.next_core()),
        Err(Error::Io(_))
    ));
    assert_eq!(fs::read_link(&runtime.path).unwrap(), target);
    assert_eq!(fs::read(&target).unwrap(), old);

    fs::remove_file(&runtime.path).unwrap();
    fs::rename(target, &runtime.path).unwrap();
    fs::set_permissions(&runtime.path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        submit_and_wait(&mut runtime.worker(), runtime.next_core()),
        Err(Error::Io(_))
    ));
    assert_eq!(fs::read(&runtime.path).unwrap(), old);
    assert_eq!(
        fs::metadata(&runtime.path).unwrap().permissions().mode() & 0o7777,
        0o644
    );

    fs::remove_file(&runtime.path).unwrap();
    prepare_runtime(&runtime.path).unwrap();
    assert!(matches!(
        submit_and_wait(&mut runtime.worker(), runtime.next_core()),
        Err(Error::Io(_))
    ));
    assert!(runtime.path.is_dir());
}

#[test]
fn refuses_required_sections_unknown_versions_and_foreign_scope_without_rewriting() {
    let runtime = Runtime::new();
    let mut unknown = runtime.original.clone();
    unknown.sections[0].required = true;
    let mut wrong_version = runtime.original.encode().unwrap();
    wrong_version[4] = 2;
    let mut wrong_scope = runtime.original.clone();
    wrong_scope.header.generation = "g1-44444444444444444444444444444444".into();
    for (bytes, error) in [
        (
            unknown.encode().unwrap(),
            super::super::Error::UnknownRequiredSection(unknown.sections[0].name.clone()),
        ),
        (wrong_version, super::super::Error::UnsupportedVersion(2)),
        (
            wrong_scope.encode().unwrap(),
            super::super::Error::ScopeMismatch("generation"),
        ),
    ] {
        private_file::publish(&runtime.path, &bytes).unwrap();
        assert_eq!(
            submit_and_wait(&mut runtime.worker(), runtime.next_core()),
            Err(Error::Codec(error))
        );
        assert_eq!(fs::read(&runtime.path).unwrap(), bytes);
    }
}

#[test]
fn file_and_rewrite_share_the_encoded_size_limit() {
    let runtime = Runtime::new();
    let mut checkpoint = runtime.original.clone();
    let length = checkpoint.encode().unwrap().len();
    let payload = &mut checkpoint.sections[0].payload;
    payload.resize(payload.len() + MAX_BYTES - length, 0xa5);
    let bytes = checkpoint.encode().unwrap();
    assert_eq!(bytes.len(), MAX_BYTES);
    private_file::publish(&runtime.path, &bytes).unwrap();
    drop(bytes);

    checkpoint.core = runtime.next_core();
    let mut worker = runtime.worker();
    submit_and_wait(&mut worker, checkpoint.core.clone()).unwrap();
    let saved = checkpoint.encode().unwrap();
    assert_eq!(fs::read(&runtime.path).unwrap(), saved);

    let mut larger = checkpoint.core;
    larger.revision += 1;
    larger.sessions.push(Session {
        id: "s4".into(),
        persistent_id: None,
        tabs: Vec::new(),
        active_tab: None,
    });
    assert_eq!(
        submit_and_wait(&mut worker, larger.clone()),
        Err(Error::Codec(super::super::Error::Invalid("size")))
    );
    assert_eq!(fs::read(&runtime.path).unwrap(), saved);

    let oversized = (MAX_BYTES + 1) as u64;
    fs::OpenOptions::new()
        .write(true)
        .open(&runtime.path)
        .unwrap()
        .set_len(oversized)
        .unwrap();
    larger.revision += 1;
    assert!(matches!(
        submit_and_wait(&mut worker, larger),
        Err(Error::Io(detail)) if detail.contains(&format!("exceeds {MAX_BYTES} bytes"))
    ));
    assert_eq!(fs::metadata(&runtime.path).unwrap().len(), oversized);
}
