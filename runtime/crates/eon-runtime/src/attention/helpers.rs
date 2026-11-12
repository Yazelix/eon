//! Positive infrastructure roles; Orbit's unrecognized processes keep background.

use orbit_protocol::management::{ProgramState, ProgramStatus};
use std::{
    sync::mpsc::{self, Receiver, SyncSender},
    thread,
    time::Instant,
};

pub(super) fn needs_inspection(status: &ProgramStatus) -> bool {
    cfg!(target_os = "linux")
        && status.background > 0
        && status.background < u8::MAX
        && status.processes.complete
        && status.processes.foreground.is_some()
        && status.processes.background.len() == usize::from(status.background)
        && status
            .records
            .iter()
            .any(|record| matches!(record.state, ProgramState::Done | ProgramState::Idle))
}

/// One bounded reader, detached on drop so a target mmap lock cannot hold shutdown.
pub(super) struct Worker {
    sender: SyncSender<(u64, ProgramStatus, Instant)>,
    pub receiver: Receiver<(u64, bool)>,
}

impl Worker {
    pub fn start() -> Option<Self> {
        let (sender, jobs) = mpsc::sync_channel::<(u64, ProgramStatus, Instant)>(16);
        let (results, receiver) = mpsc::sync_channel(16);
        thread::Builder::new()
            .name("eon-agent-helpers".into())
            .spawn(move || {
                while let Ok((sample, status, deadline)) = jobs.recv() {
                    let helpers = only_helpers(&status, deadline);
                    if results.send((sample, helpers)).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { sender, receiver })
    }

    pub fn submit(&self, sample: u64, status: ProgramStatus, deadline: Instant) -> bool {
        self.sender.try_send((sample, status, deadline)).is_ok()
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn only_helpers(_: &ProgramStatus, _: Instant) -> bool {
    false
}

#[cfg(target_os = "linux")]
pub(super) fn only_helpers(status: &ProgramStatus, deadline: Instant) -> bool {
    use crate::platform::{HelperProcess, helper_process};
    use std::{collections::HashMap, os::unix::ffi::OsStrExt};
    if !needs_inspection(status) {
        return false;
    }
    let leader = status.processes.foreground.unwrap();
    let mut cache = HashMap::<(u32, bool), Option<HelperProcess>>::new();
    // At most 16 candidates and two ancestors each, plus the leader. Every
    // reader revalidates identity; a deadline miss never becomes known idle.
    let mut read = |pid, arguments| {
        if Instant::now() >= deadline {
            return None;
        }
        cache
            .entry((pid, arguments))
            .or_insert_with(|| helper_process(pid, arguments))
            .clone()
    };
    let Some(anchor) = read(leader.pid, false).filter(|p| p.identity == leader) else {
        return false;
    };
    if anchor.group != leader.pid {
        return false;
    }
    let provider = |p: &HelperProcess| {
        let Some(bin) = p.executable.parent() else {
            return false;
        };
        let Some(release) = bin.parent() else {
            return false;
        };
        let Some(releases) = release.parent() else {
            return false;
        };
        let Some(standalone) = releases.parent() else {
            return false;
        };
        let Some(packages) = standalone.parent() else {
            return false;
        };
        p.group == leader.pid
            && p.executable.file_name() == Some("codex".as_ref())
            && bin.file_name() == Some("bin".as_ref())
            && release
                .file_name()
                .is_some_and(|name| name.as_bytes().contains(&b'-'))
            && releases.file_name() == Some("releases".as_ref())
            && standalone.file_name() == Some("standalone".as_ref())
            && packages.file_name() == Some("packages".as_ref())
    };
    let no_arguments = |p: &HelperProcess| {
        p.arguments.len() == 1 && p.arguments[0] == p.executable.as_os_str().as_bytes()
    };
    let wrapper = |p: &HelperProcess| {
        p.executable.ends_with("resources/cua_node/bin/node")
            && p.arguments.len() == 2
            && p.arguments[0] == p.executable.as_os_str().as_bytes()
            && p.arguments[1]
                == p.executable
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("lib/node_modules/@oai/cua-repl/bin/cua-repl.mjs")
                    .as_os_str()
                    .as_bytes()
    };
    for identity in &status.processes.background {
        let Some(process) = read(identity.pid, true).filter(|p| p.identity == *identity) else {
            return false;
        };
        let Some(parent) = read(process.parent, false) else {
            return false;
        };
        if provider(&parent) {
            if no_arguments(&process)
                && (process.executable == parent.executable.with_file_name("codex-code-mode-host")
                    || process
                        .executable
                        .ends_with("resources/cua_node/bin/node_repl"))
            {
                continue;
            }
            if wrapper(&process) {
                continue;
            }
        } else if process
            .executable
            .ends_with("resources/cua_node/bin/node_repl")
            && no_arguments(&process)
            && let Some(parent) = read(process.parent, true)
            && wrapper(&parent)
            && process.executable.parent() == parent.executable.parent()
            && read(parent.parent, false).is_some_and(|root| provider(&root))
        {
            continue;
        }
        return false;
    }
    cache.iter().all(|(&(pid, arguments), cached)| {
        cached.as_ref().is_some_and(|process| {
            Instant::now() < deadline && helper_process(pid, arguments).as_ref() == Some(process)
        })
    }) && Instant::now() < deadline
}
