//! Keeps the control socket answering while a terminal start waits for its
//! new Orbit (eon-pimct).
//!
//! The supervisor serves every request on one loop thread, and a terminal
//! start runs inside the operation that needs it, with the workspace
//! mutably borrowed. While a start waits, the pump accepts clients: reads it
//! can answer exactly are answered from a capture of the state taken before
//! the operation, and every other request is held, in arrival order, for the
//! loop to serve once the operation ends. Without a capture every request is
//! held, so nothing is ever answered from state that may be wrong.

use super::{
    control::{self, Client, ControlResponse, Dispatched, Incoming},
    diagnostics::DiagnosticLog,
};
use eon_workspace_protocol::v49::{
    Action, BusyTerminals, LifecycleResponse, Request, Response, WorkspaceAction,
};
use std::{
    collections::VecDeque,
    io,
    os::unix::net::{UnixListener, UnixStream},
    sync::Arc,
    thread,
    time::Instant,
};

/// Clients held at once; later ones stay in the kernel's listen backlog, as
/// every client did before the pump.
const HELD_LIMIT: usize = 32;

/// The replies to the reads a capture answers, as normal dispatch gives them
/// in the state before the operation that starts a terminal.
pub(super) struct Capture {
    /// A callerless `Inspect`.
    pub(super) shown: Response,
    /// A caller-bearing `Inspect`, by the caller's workspace id.
    pub(super) workspaces: Vec<(String, Response)>,
    /// Each live caller identity (`EON_TERMINAL`) and its workspace id.
    pub(super) callers: Vec<(String, String)>,
    /// A callerless `InspectRuntime` or `InspectPresentation`, when known.
    pub(super) runtime: Option<LifecycleResponse>,
    /// The busy terminals once every idle reply is stale (EON-C11 fails
    /// closed), and the instant the first one goes stale.
    pub(super) stale_busy: Option<BusyTerminals>,
    pub(super) fresh_until: Option<Instant>,
}

impl Capture {
    /// The reply to `request` at `now`, or `None` when it must be held.
    pub(super) fn answer(&self, request: &Request, now: Instant) -> Option<ControlResponse> {
        let stale = self.fresh_until.is_some_and(|until| now >= until);
        let snapshot = |reply: &Response| {
            let mut reply = reply.clone();
            if stale && let Response::Snapshot(snapshot) = &mut reply {
                snapshot.busy = self.stale_busy.clone();
            }
            ControlResponse::Workspace(reply)
        };
        let Action::Workspace(action) = &request.action else {
            return None;
        };
        match (action, request.caller.as_deref()) {
            (WorkspaceAction::Inspect, None) => Some(snapshot(&self.shown)),
            (WorkspaceAction::Inspect, Some(caller)) => {
                let (_, workspace) = self.callers.iter().find(|(id, _)| id == caller)?;
                let (_, reply) = self.workspaces.iter().find(|(id, _)| id == workspace)?;
                Some(snapshot(reply))
            }
            (WorkspaceAction::InspectRuntime | WorkspaceAction::InspectPresentation, None) => {
                let mut reply = self.runtime.clone()?;
                if stale && let LifecycleResponse::Runtime(runtime) = &mut reply {
                    runtime.busy = self.stale_busy.clone();
                }
                Some(ControlResponse::Lifecycle(reply))
            }
            _ => None,
        }
    }
}

/// The control socket's server while a start waits; see the module text.
#[derive(Default)]
pub(super) struct StartPump {
    /// A clone of the supervisor's non-blocking control listener; `None`
    /// before it exists and in unit fixtures, where waiting only sleeps.
    listener: Option<UnixListener>,
    capture: Option<Capture>,
    held: VecDeque<Client>,
    log: Option<Arc<DiagnosticLog>>,
}

impl StartPump {
    pub(super) fn new(listener: Option<UnixListener>, log: Arc<DiagnosticLog>) -> Self {
        Self {
            listener,
            capture: None,
            held: VecDeque::new(),
            log: Some(log),
        }
    }

    /// Answers reads from `capture` until [`Self::disarm`].
    pub(super) fn arm(&mut self, capture: Option<Capture>) {
        self.capture = capture;
    }

    /// Ends the operation's capture; held clients stay held.
    pub(super) fn disarm(&mut self) {
        self.capture = None;
    }

    #[cfg(test)]
    pub(super) fn armed(&self) -> bool {
        self.capture.is_some()
    }

    /// Whether it holds no client and no start capture.
    pub(super) fn is_idle(&self) -> bool {
        self.held.is_empty() && self.capture.is_none()
    }

    /// The oldest held client, which the loop serves before any new one.
    pub(super) fn next_held(&mut self) -> Option<Client> {
        self.held.pop_front()
    }

    /// Serves the control socket until `until`, the next instant a start
    /// checks its Orbit again.
    pub(super) fn wait(&mut self, until: Instant) {
        loop {
            let Some(left) = until
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
            else {
                return;
            };
            let Some(listener) = self
                .listener
                .as_ref()
                .filter(|_| self.held.len() < HELD_LIMIT)
            else {
                thread::sleep(left);
                return;
            };
            let mut readable = [rustix::event::PollFd::new(
                listener,
                rustix::event::PollFlags::IN,
            )];
            let timeout = rustix::event::Timespec {
                tv_sec: left.as_secs() as i64,
                tv_nsec: i64::from(left.subsec_nanos()),
            };
            match rustix::event::poll(&mut readable, Some(&timeout)) {
                Ok(0) => return,
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(error) => {
                    self.pause(&format!("cannot poll Eon control socket: {error}"), until);
                    return;
                }
            }
            match listener.accept() {
                Ok((stream, _)) => self.serve(stream),
                // A client that left between connect and accept ends nothing.
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::Interrupted
                            | io::ErrorKind::ConnectionAborted
                    ) => {}
                Err(error) => {
                    self.pause(
                        &format!(
                            "cannot accept Eon control client while a terminal starts: {error}"
                        ),
                        until,
                    );
                    return;
                }
            }
        }
    }

    /// Serves nothing more in this wait: waiting clients stay in the
    /// backlog, and the next wait tries again. Never fatal here; the loop's
    /// own accept reports a lasting error.
    fn pause(&self, detail: &str, until: Instant) {
        if let Some(log) = &self.log {
            log.note_once(detail);
        }
        super::sessions::sleep_until(until);
    }

    fn serve(&mut self, stream: UnixStream) {
        let Some(client) = control::read_client(stream) else {
            return;
        };
        let reply = match &client.incoming {
            // A request that could not be read gets its one reply now.
            Err(_) => {
                control::serve_client(client, |_, _| -> Dispatched {
                    unreachable!("an unread request is never dispatched")
                });
                return;
            }
            Ok(Incoming::Request(request)) => self
                .capture
                .as_ref()
                .and_then(|capture| capture.answer(request, Instant::now())),
            Ok(Incoming::Stop(_)) => None,
        };
        match reply {
            Some(reply) => {
                control::finish_client(client.stream, Dispatched::Reply(reply, false));
            }
            None => self.held.push_back(client),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Capture, HELD_LIMIT, StartPump};
    use crate::{
        control::{Incoming, failure},
        supervisor::temporary_directory,
    };
    use eon_workspace_protocol::v49::{
        Action, HEADER_BYTES, Request, Response, WorkspaceAction, declared_message_len,
        decode_response, encode_request,
    };
    use std::{
        fs,
        io::{Read, Write},
        os::unix::net::{UnixListener, UnixStream},
        path::Path,
        time::{Duration, Instant},
    };

    fn pump(root: &Path) -> (UnixListener, StartPump) {
        let listener = UnixListener::bind(root.join("eon.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let pump = StartPump {
            listener: Some(listener.try_clone().unwrap()),
            ..StartPump::default()
        };
        (listener, pump)
    }

    fn send(root: &Path, id: &str, caller: Option<&str>, action: WorkspaceAction) -> UnixStream {
        let mut stream = UnixStream::connect(root.join("eon.sock")).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(
                &encode_request(&Request {
                    id: id.into(),
                    caller: caller.map(str::to_owned),
                    action: Action::Workspace(action),
                })
                .unwrap(),
            )
            .unwrap();
        stream
    }

    fn reply(stream: &mut UnixStream) -> Response {
        let mut message = vec![0; HEADER_BYTES];
        stream.read_exact(&mut message).unwrap();
        message.resize(declared_message_len(&message).unwrap(), 0);
        stream.read_exact(&mut message[HEADER_BYTES..]).unwrap();
        decode_response(&message).unwrap()
    }

    fn held_ids(pump: &mut StartPump) -> Vec<String> {
        std::iter::from_fn(|| pump.next_held())
            .map(|client| match client.incoming {
                Ok(Incoming::Request(request)) => request.id,
                _ => panic!("a held client is not a request"),
            })
            .collect()
    }

    fn named(detail: &str) -> Response {
        Response::Failure(failure("captured", detail))
    }

    fn capture() -> Capture {
        Capture {
            shown: named("shown"),
            workspaces: vec![("w1".into(), named("w1"))],
            callers: vec![("terminal-1.run".into(), "w1".into())],
            runtime: None,
            stale_busy: None,
            fresh_until: None,
        }
    }

    #[test]
    fn without_a_capture_every_request_is_held_in_arrival_order() {
        let root = temporary_directory();
        let (_listener, mut pump) = pump(&root);
        let _clients =
            ["first", "second", "third"].map(|id| send(&root, id, None, WorkspaceAction::Inspect));
        pump.wait(Instant::now() + Duration::from_millis(300));
        assert_eq!(held_ids(&mut pump), ["first", "second", "third"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_capture_answers_its_reads_and_holds_the_rest() {
        let root = temporary_directory();
        let (_listener, mut pump) = pump(&root);
        pump.arm(Some(capture()));
        let mut shown = send(&root, "shown", None, WorkspaceAction::Inspect);
        let mut from_pane = send(
            &root,
            "pane",
            Some("terminal-1.run"),
            WorkspaceAction::Inspect,
        );
        let _clients = [
            send(
                &root,
                "stranger",
                Some("terminal-9.run"),
                WorkspaceAction::Inspect,
            ),
            send(&root, "runtime", None, WorkspaceAction::InspectRuntime),
            send(
                &root,
                "close",
                None,
                WorkspaceAction::CloseTab { tab: "t1".into() },
            ),
        ];
        pump.wait(Instant::now() + Duration::from_millis(300));
        assert_eq!(reply(&mut shown), named("shown"));
        assert_eq!(reply(&mut from_pane), named("w1"));
        assert_eq!(held_ids(&mut pump), ["stranger", "runtime", "close"]);
        // Disarmed, the same read is held.
        pump.disarm();
        let _late = send(&root, "late", None, WorkspaceAction::Inspect);
        pump.wait(Instant::now() + Duration::from_millis(300));
        assert_eq!(held_ids(&mut pump), ["late"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unreadable_request_is_answered_at_once() {
        let root = temporary_directory();
        let (_listener, mut pump) = pump(&root);
        let mut garbage = UnixStream::connect(root.join("eon.sock")).unwrap();
        garbage
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        garbage.write_all(&[0xff; HEADER_BYTES]).unwrap();
        pump.wait(Instant::now() + Duration::from_millis(400));
        assert!(matches!(reply(&mut garbage), Response::Failure(_)));
        assert!(pump.next_held().is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn held_clients_past_the_limit_stay_in_the_backlog() {
        let root = temporary_directory();
        let (listener, mut pump) = pump(&root);
        let _clients = (0..HELD_LIMIT + 3)
            .map(|index| send(&root, &format!("c{index}"), None, WorkspaceAction::Inspect))
            .collect::<Vec<_>>();
        let started = Instant::now();
        pump.wait(started + Duration::from_millis(500));
        // The wait runs to its end, sleeping once the queue is full.
        assert!(started.elapsed() >= Duration::from_millis(500));
        assert_eq!(held_ids(&mut pump).len(), HELD_LIMIT);
        let mut backlog = 0;
        while listener.accept().is_ok() {
            backlog += 1;
        }
        assert_eq!(backlog, 3);
        fs::remove_dir_all(root).unwrap();
    }
}
