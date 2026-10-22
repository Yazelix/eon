use crate::management_record::{live, suffixed, tombstone};
use crate::support::*;
use orbit_protocol::management::{
    self, ClientMessage, Failure, FailureCode, LiveIdentity, ObjectIdentity, ProcessOutcome,
    ServerMessage, TerminationReason,
};
use std::{
    ffi::OsStr,
    fs,
    io::{self, Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{MetadataExt, PermissionsExt},
        net::UnixStream,
    },
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

const MANAGED: [&str; 4] = ["--management-v2", "session-2", "run-2", "component-2"];

/// What a management connection reads next.
#[derive(Debug, PartialEq)]
pub enum Reply {
    Message(ServerMessage),
    Closed,
    Timeout,
    Malformed(management::Error),
}

/// One management client of the run whose live record is at a path.
pub struct Manager {
    record: PathBuf,
    identity: LiveIdentity,
    stream: Option<UnixStream>,
}

impl Manager {
    pub fn new(record: &Path) -> Self {
        Self {
            record: record.to_path_buf(),
            identity: live(record),
            stream: None,
        }
    }

    /// Connects to the run's management socket.
    pub fn connect(&mut self) {
        let path = OsStr::from_bytes(&self.identity.management.path);
        let stream = UnixStream::connect(path).expect("connected");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("timeout");
        self.stream = Some(stream);
    }

    /// Reads one whole management message, or reports the connection closed.
    pub fn read(&mut self) -> Reply {
        let stream = self.stream.as_mut().expect("connected");
        let mut bytes = Vec::new();
        loop {
            match management::server_message_len(&bytes) {
                Ok(Some(length)) if bytes.len() >= length => break,
                Ok(_) => {}
                Err(error) => return Reply::Malformed(error),
            }
            let mut chunk = [0; 4096];
            match stream.read(&mut chunk) {
                Ok(0) => return Reply::Closed,
                Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Reply::Timeout;
                }
                Err(_) => return Reply::Closed,
            }
        }
        match management::decode_server_message(&bytes) {
            Ok(message) => Reply::Message(message),
            Err(error) => Reply::Malformed(error),
        }
    }

    /// Sends bytes and reads the answer; an answer the server wrote before
    /// closing, such as Busy, is read even when the bytes could not be sent.
    fn send_bytes(&mut self, bytes: &[u8]) -> Reply {
        let _ = self.stream.as_mut().expect("connected").write_all(bytes);
        self.read()
    }

    pub fn send(&mut self, message: &ClientMessage) -> Reply {
        self.send_bytes(&management::encode_client_message(message).expect("encodable request"))
    }

    /// Sends Acquire with this identity and the record's inode.
    fn acquire_as(&mut self, expected: LiveIdentity) -> Reply {
        let metadata = fs::symlink_metadata(&self.record).expect("record");
        self.send(&ClientMessage::Acquire {
            expected,
            record: ObjectIdentity {
                device: metadata.dev(),
                inode: metadata.ino(),
            },
        })
    }

    /// Connects and acquires with the record's identity.
    pub fn acquire(&mut self) -> Reply {
        self.connect();
        self.acquire_as(self.identity.clone())
    }

    /// A header that announces a message over the bound.
    fn oversized(&mut self) -> Reply {
        let mut header = [0; management::HEADER_BYTES];
        header[..4].copy_from_slice(b"ORBM");
        header[4..6].copy_from_slice(&management::VERSION.to_le_bytes());
        header[6] = 2;
        header[8..12].copy_from_slice(&(management::MAX_MESSAGE_BYTES as u32).to_le_bytes());
        self.send_bytes(&header)
    }
}

fn failure(reply: &Reply, code: FailureCode) -> bool {
    matches!(reply, Reply::Message(ServerMessage::Failure(failure)) if failure.code == code)
}

pub fn run() {
    let directory = TempDir::new();
    let work = directory.join("work");
    let shell = executable(
        &directory,
        "child.sh",
        "#!/bin/sh\n\
         while [ ! -e \"$ORBIT_WORK\" ]; do sleep 0.01; done\n\
         printf '\\033]0;SURVIVED\\007'\n\
         while :; do sleep 1; done\n",
    );
    let mut server = Server::launch(
        &directory,
        &[shell.as_os_str()],
        &[("ORBIT_WORK", &work)],
        &[],
        &MANAGED,
    );
    let record_path = suffixed(&server.path, ".record");
    let management_path = suffixed(&server.path, ".management");
    wait_for(&record_path);
    let identity = live(&record_path);

    // A wrong identity fails closed and closes the connection.
    let mut wrong = Manager::new(&record_path);
    wrong.connect();
    let mut expected = identity.clone();
    expected.process_start += 1;
    assert_eq!(
        wrong.acquire_as(expected),
        Reply::Message(ServerMessage::Failure(Failure {
            code: FailureCode::InvalidIdentity,
            detail: "live Orbit identity did not match".into(),
        }))
    );
    assert_eq!(wrong.read(), Reply::Closed);
    drop(wrong);

    // A silent connection holds the one slot: a contender is Busy until the
    // one-second negotiation bound passes.
    let mut silent = UnixStream::connect(&management_path).expect("connect");
    sleep(0.2);
    let mut contender = Manager::new(&record_path);
    assert_eq!(contender.acquire(), Reply::Message(ServerMessage::Busy));
    drop(contender);
    sleep(1.1);
    assert_eq!(silent.read(&mut [0]).expect("read"), 0); // The bound closed it.
    drop(silent);

    // A request out of place or over the bound is refused and closes.
    let mut early = Manager::new(&record_path);
    early.connect();
    let reply = early.send(&ClientMessage::Status);
    assert!(failure(&reply, FailureCode::InvalidRequest), "status before the lease: {reply:?}");
    drop(early);
    let mut oversized = Manager::new(&record_path);
    oversized.connect();
    let reply = oversized.oversized();
    assert!(failure(&reply, FailureCode::InvalidRequest), "oversized request: {reply:?}");
    drop(oversized);

    // A record that is no longer exact fails the identity check.
    let mode = |mode| fs::set_permissions(&record_path, fs::Permissions::from_mode(mode));
    mode(0o640).expect("chmod");
    let mut changed = Manager::new(&record_path);
    let reply = changed.acquire();
    assert!(failure(&reply, FailureCode::InvalidIdentity), "{reply:?}");
    drop(changed);
    mode(0o600).expect("chmod");

    // The lease reports status; its loss does not stop the run.
    let mut lease = Manager::new(&record_path);
    assert_eq!(lease.acquire(), Reply::Message(ServerMessage::Lease(identity.clone())));
    assert_eq!(
        lease.send(&ClientMessage::Status),
        Reply::Message(ServerMessage::Status(identity.clone()))
    );
    drop(lease);
    sleep(0.15);
    fs::write(&work, "").expect("work");
    let mut client = Peer::attach(&server.path, 4.0);
    client.frame_titled("SURVIVED");
    drop(client);

    // Two replacements race for the lease: one wins and one is Busy.
    let mut managers = [Manager::new(&record_path), Manager::new(&record_path)];
    for manager in &mut managers {
        manager.connect();
    }
    let racing: Vec<_> = managers
        .into_iter()
        .map(|mut manager| {
            thread::spawn(move || {
                let identity = manager.identity.clone();
                let reply = manager.acquire_as(identity);
                (manager, reply)
            })
        })
        .collect();
    let raced: Vec<_> = racing
        .into_iter()
        .map(|racer| racer.join().expect("racer"))
        .collect();
    let replies: Vec<_> = raced.iter().map(|(_, reply)| reply).collect();
    let lease = Reply::Message(ServerMessage::Lease(identity.clone()));
    let busy = Reply::Message(ServerMessage::Busy);
    assert!(
        replies == [&lease, &busy] || replies == [&busy, &lease],
        "{replies:?}"
    );
    let status = Reply::Message(ServerMessage::Status(identity.clone()));
    let mut winner = None;
    for (mut manager, _) in raced {
        if manager.send(&ClientMessage::Status) == status {
            winner = Some(manager);
        }
    }
    let mut winner = winner.expect("a lease holder");

    // Stop through the lease runs the bounded shutdown: the child is hung up,
    // the lease hears the tombstone, and the server exits 0.
    match winner.send(&ClientMessage::Stop) {
        Reply::Message(ServerMessage::Stopped(stopped)) => assert_eq!(
            (stopped.reason, stopped.outcome),
            (
                TerminationReason::ExplicitStop,
                ProcessOutcome::Signal(libc::SIGHUP)
            ),
            "{stopped:?}"
        ),
        other => panic!("expected Stopped, received {other:?}"),
    }
    drop(winner);
    server.communicate(10.0).assert_silent(0);
    let tombstone = tombstone(&record_path);
    assert_eq!(
        (tombstone.reason, tombstone.outcome),
        (
            TerminationReason::ExplicitStop,
            ProcessOutcome::Signal(libc::SIGHUP)
        ),
        "{tombstone:?}"
    );
    assert_eq!(tombstone.identity, identity);
    assert!(!server.path.exists() && !management_path.exists());
    assert!(!alive(server.child));
}
