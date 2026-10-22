use crate::support::*;
use orbit_protocol::management::{
    self, LiveIdentity, ProcessOutcome, Record, TerminationReason, Tombstone,
};
use std::{
    ffi::OsStr,
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
        net::UnixListener,
    },
    path::{Path, PathBuf},
};

const MANAGED: [&str; 4] = ["--management-v2", "session-1", "run-1", "component-1"];

/// The record at a path, decoded canonically.
pub fn record(path: &Path) -> Record {
    management::decode_record(&fs::read(path).expect("record")).expect("a canonical record")
}

pub fn live(path: &Path) -> LiveIdentity {
    match record(path) {
        Record::Live(identity) => identity,
        other => panic!("expected a live record, found {other:?}"),
    }
}

pub fn tombstone(path: &Path) -> Tombstone {
    match record(path) {
        Record::Tombstone(tombstone) => tombstone,
        other => panic!("expected a tombstone, found {other:?}"),
    }
}

pub fn suffixed(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

/// A process's start time in clock ticks since boot, from /proc on Linux.
fn start_time(pid: i32) -> u64 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).expect("stat");
    let (_, fields) = stat.rsplit_once(')').expect("stat fields");
    fields.split_whitespace().nth(19).expect("starttime").parse().expect("a number")
}

fn private_file(path: &Path, socket: bool) {
    let metadata = fs::symlink_metadata(path).expect("lstat");
    let kind = if socket {
        metadata.file_type().is_socket()
    } else {
        metadata.file_type().is_file()
    };
    // SAFETY: geteuid has no preconditions.
    let euid = unsafe { libc::geteuid() };
    assert!(
        kind && metadata.mode() & 0o7777 == 0o600 && metadata.uid() == euid,
        "{} is not a private {}",
        path.display(),
        if socket { "socket" } else { "file" }
    );
}

fn child_script(directory: &Path, text: &str) -> PathBuf {
    executable(directory, "child.sh", text)
}

pub fn run() {
    // SAFETY: geteuid has no preconditions.
    let euid = unsafe { libc::geteuid() };

    // A managed run publishes its live record at Ready and a tombstone with
    // the child's exact code when the child exits.
    {
        let directory = TempDir::new();
        let gate = Gate::new(&directory, "exit-gate");
        let shell = child_script(&directory, "#!/bin/sh\nread -r _ < \"$ORBIT_GATE\"\nexit 7\n");
        let mut server = Server::launch(
            &directory,
            &[shell.as_os_str()],
            &[("ORBIT_GATE", gate.path())],
            &[],
            &MANAGED,
        );
        let record_path = suffixed(&server.path, ".record");
        let management_path = suffixed(&server.path, ".management");
        wait_for(&record_path);
        let live = live(&record_path);
        assert_eq!(live.process_id, server.pid() as u32, "{live:?}");
        if cfg!(target_os = "linux") {
            assert_eq!(live.process_start, start_time(server.pid()), "{live:?}");
        }
        assert_eq!(live.uid, euid, "{live:?}");
        assert_eq!(
            (
                live.session_id.as_str(),
                live.run_id.as_str(),
                live.component_generation.as_str(),
            ),
            ("session-1", "run-1", "component-1"),
            "{live:?}"
        );
        assert_eq!(live.presentation.path, server.path.as_os_str().as_bytes());
        assert_eq!(live.management.path, management_path.as_os_str().as_bytes());
        private_file(&record_path, false);
        private_file(&management_path, true);
        // The child inherits neither listener.
        if cfg!(target_os = "linux") {
            let descriptors = format!("/proc/{}/fd", server.child);
            for entry in fs::read_dir(&descriptors).expect("descriptors") {
                let target = fs::read_link(entry.expect("descriptor").path()).expect("readlink");
                assert!(
                    !target.to_string_lossy().starts_with("socket:"),
                    "{}",
                    target.display()
                );
            }
        }

        gate.release();
        server.passed(7, 10.0);
        let tombstone = tombstone(&record_path);
        assert_eq!(
            (tombstone.reason, tombstone.outcome),
            (TerminationReason::NaturalExit, ProcessOutcome::ExitCode(7)),
            "{tombstone:?}"
        );
        assert_eq!(tombstone.identity.process_id, live.process_id);
        assert_eq!(tombstone.identity.process_start, live.process_start);
        assert!(!management_path.exists());
    }

    // A presentation pathname replaced by another endpoint is left in place,
    // and the run still exits with the child's code, removes its management
    // socket and records the natural exit.
    {
        let directory = TempDir::new();
        let gate = Gate::new(&directory, "exit-gate");
        let shell = child_script(&directory, "#!/bin/sh\nread -r _ < \"$ORBIT_GATE\"\nexit 5\n");
        let mut server = Server::launch(
            &directory,
            &[shell.as_os_str()],
            &[("ORBIT_GATE", gate.path())],
            &[],
            &MANAGED,
        );
        let record_path = suffixed(&server.path, ".record");
        let management_path = suffixed(&server.path, ".management");
        wait_for(&record_path);
        live(&record_path);
        fs::remove_file(&server.path).expect("remove the socket pathname");
        let _replacement = UnixListener::bind(&server.path).expect("bind a replacement");
        let replaced = fs::symlink_metadata(&server.path).expect("lstat").ino();

        gate.release();
        server.communicate(10.0).assert_silent(5);
        assert!(!alive(server.child), "the child outlived the server");
        assert_eq!(
            fs::symlink_metadata(&server.path).expect("the replacement").ino(),
            replaced
        );
        let tombstone = tombstone(&record_path);
        assert_eq!(
            (tombstone.reason, tombstone.outcome),
            (TerminationReason::NaturalExit, ProcessOutcome::ExitCode(5)),
            "{tombstone:?}"
        );
        assert!(!management_path.exists());
    }

    // A termination signal stops the run: the tombstone records an explicit
    // stop and the signal that ended the child.
    {
        let directory = TempDir::new();
        let shell = child_script(&directory, "#!/bin/sh\nwhile :; do sleep 1; done\n");
        let mut server =
            Server::launch(&directory, &[shell.as_os_str()], &[], &[], &MANAGED);
        let record_path = suffixed(&server.path, ".record");
        wait_for(&record_path);
        live(&record_path);
        signal(server.pid(), libc::SIGTERM);
        server.communicate(30.0).assert_silent(0);
        let tombstone = tombstone(&record_path);
        assert_eq!(
            (tombstone.reason, tombstone.outcome),
            (
                TerminationReason::ExplicitStop,
                ProcessOutcome::Signal(libc::SIGHUP)
            ),
            "{tombstone:?}"
        );
        assert!(!suffixed(&server.path, ".management").exists());
    }

    // A launcher's empty retained record is claimed once: a held claim stops
    // the start without publishing, and a free one is published in place.
    {
        let directory = TempDir::new();
        let path = directory.join("orbit.sock");
        let record_path = suffixed(&path, ".record");
        let claimed = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&record_path)
            .expect("retained record");
        // SAFETY: the descriptor is open for the duration of the call.
        assert_eq!(unsafe { libc::flock(claimed.as_raw_fd(), libc::LOCK_EX) }, 0);
        run_command(
            &server_command(&path, &[OsStr::new("/bin/true")], &MANAGED),
            &[],
            30.0,
        )
        .assert_refused();
        assert_eq!(fs::metadata(&record_path).expect("record").len(), 0);
        assert!(!path.exists() && !suffixed(&path, ".management").exists());
        drop(claimed);

        run_command(
            &server_command(
                &path,
                &[OsStr::new("/bin/sh"), OsStr::new("-c"), OsStr::new("exit 3")],
                &MANAGED,
            ),
            &[],
            30.0,
        )
        .assert_silent(3);
        let tombstone = tombstone(&record_path);
        assert_eq!(tombstone.outcome, ProcessOutcome::ExitCode(3), "{tombstone:?}");
    }

    // Identities outside the bounds are refused before anything
    // starts.
    {
        let directory = TempDir::new();
        let path = directory.join("orbit.sock");
        let long = "s".repeat(129);
        for options in [
            &["--management-v2", "", "run", "component"][..],
            &["--management-v2", &long, "run", "component"],
            &["--management-v2", "session", "run"],
        ] {
            run_command(
                &server_command(&path, &[OsStr::new("/bin/true")], options),
                &[],
                30.0,
            )
            .assert_refused();
        }
        assert_eq!(fs::read_dir(&*directory).expect("directory").count(), 0);
    }

    // An untrusted object at the record's pathname, or an untrusted runtime
    // directory, stops the start before anything is published, and the
    // object is left as it was.
    {
        let directory = TempDir::new();
        let refused = |path: &Path| {
            run_command(
                &server_command(path, &[OsStr::new("/bin/true")], &MANAGED),
                &[],
                30.0,
            )
            .assert_refused();
            assert!(!path.exists() && !suffixed(path, ".management").exists());
        };
        let private_record = |path: &Path, mode: u32, contents: &[u8]| {
            fs::write(path, contents).expect("record");
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("chmod");
        };

        // A record in another private directory, reached through a symlink.
        let elsewhere = directory.join("elsewhere");
        fs::DirBuilder::new().mode(0o700).create(&elsewhere).expect("directory");
        let target = elsewhere.join("orbit.sock.record");
        private_record(&target, 0o600, b"");

        for name in [
            "symlink",
            "fifo",
            "empty-executable",
            "group-readable",
            "directory",
        ] {
            let run = directory.join(name);
            fs::DirBuilder::new().mode(0o700).create(&run).expect("directory");
            let path = run.join("orbit.sock");
            let record_path = suffixed(&path, ".record");
            match name {
                "symlink" => std::os::unix::fs::symlink(&target, &record_path).expect("symlink"),
                "fifo" => {
                    Gate::new(&run, "orbit.sock.record");
                }
                "empty-executable" => private_record(&record_path, 0o700, b""),
                "group-readable" => private_record(&record_path, 0o640, b"stale"),
                _ => fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&record_path)
                    .expect("directory"),
            }
            let before = fs::symlink_metadata(&record_path).expect("lstat");
            refused(&path);
            let after = fs::symlink_metadata(&record_path).expect("lstat");
            assert_eq!(
                (after.ino(), after.mode(), after.len()),
                (before.ino(), before.mode(), before.len()),
                "{name} record changed"
            );
            assert_eq!(fs::read_dir(&run).expect("directory").count(), 1, "{name}");
        }
        assert!(fs::read(&target).expect("target").is_empty());

        // A runtime directory with a mode other than 0700.
        let sticky = directory.join("sticky");
        fs::create_dir(&sticky).expect("directory");
        fs::set_permissions(&sticky, fs::Permissions::from_mode(0o1700)).expect("chmod");
        refused(&sticky.join("orbit.sock"));
        assert_eq!(fs::read_dir(&sticky).expect("directory").count(), 0);

        // A runtime directory reached through a symlink, holding a launcher's
        // empty retained record.
        let linked = directory.join("linked");
        std::os::unix::fs::symlink(&elsewhere, &linked).expect("symlink");
        refused(&linked.join("orbit.sock"));
        assert!(fs::read(&target).expect("target").is_empty());
        assert_eq!(fs::read_dir(&elsewhere).expect("directory").count(), 1);
    }
}
