//! The C-free host (docs/HOST.md). It owns the PTY child, the listener, the
//! client sockets and libghostty, runs the event loop, and asks a Bend worker
//! with no custom effects for every decision, over two inherited pipes.

use crate::{
    Step, Timer,
    boundary::{
        Decision, Direction, END, Effect, Fact, FirstMessage, Flags, ObserverLoss, Reason, SCHEMA,
        Selected, SelectionKind, WheelInput,
    },
    cli::{self, Serve},
    errno,
    frame::viewport_preview,
    input::wheel_direction,
    management::Owner as Management,
    platform::{self, signal},
    pty::{Orbit, PtyRead, status_code},
    socket::{
        ClientEvent, ClientState, Listener, ObserverInput, ObserverSend, Peer, Refusal, Role,
        Selection as PendingSelection, Wheel, peer_disconnected, turn_away,
    },
};
use orbit_protocol::management::{FailureCode as ManagementFailure, TerminationReason};
use orbit_protocol::session::{
    self, ClientMessage, ClipboardLocation, Failure, FailureCode, Modifiers, PreviewOutcome,
    ScrollOutcome, SelectionAction, SelectionPosition, ServerMessage, VerticalDirection,
    VerticalPreview, WheelOutcome,
};
use std::{
    collections::VecDeque,
    ffi::{OsString, c_int},
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, IntoRawFd, RawFd},
        unix::{net::UnixStream, process::CommandExt},
    },
    path::Path,
    process::{Child, Command, ExitStatus},
    sync::atomic::{AtomicI32, Ordering},
    thread,
    time::{Duration, Instant},
};

/// The worker's decision pipe, watched in every wait: it becomes readable
/// between questions only when the worker has died.
static WORKER: AtomicI32 = AtomicI32::new(-1);
/// The termination signal the host received, or 0.
static TERMINATION_SIGNAL: AtomicI32 = AtomicI32::new(0);
/// Both ends of the pipe a termination signal writes to. Every wait watches
/// the read end, so a signal that lands just before a wait still ends it.
static SIGNAL_READ: AtomicI32 = AtomicI32::new(-1);
static SIGNAL_WRITE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn note_termination(number: c_int) {
    TERMINATION_SIGNAL.store(number, Ordering::Relaxed);
    platform::wake(SIGNAL_WRITE.load(Ordering::Relaxed));
}

/// A terminated host contains the child and removes the socket pathname.
fn install_termination_handlers() -> io::Result<()> {
    if SIGNAL_READ.load(Ordering::Relaxed) == -1 {
        // Both ends live for the whole process.
        let (read, write) = platform::pipe(true)?;
        SIGNAL_READ.store(read.into_raw_fd(), Ordering::Relaxed);
        SIGNAL_WRITE.store(write.into_raw_fd(), Ordering::Relaxed);
    }
    // A session starts with no termination pending.
    platform::drain(SIGNAL_READ.load(Ordering::Relaxed));
    // SAFETY: The handler only writes an atomic integer and one byte to a
    // pipe.
    unsafe { platform::on_termination(note_termination) }
}

const REAP_DEADLINE: Duration = Duration::from_secs(2);
/// How long the host waits for the worker's whole answer to one fact, and for
/// its schema line and bounds at startup. A mechanism bound, not a Bend
/// decision, since a stalled worker cannot decide; the largest measured
/// answer under load took 7 ms.
const DECISION_DEADLINE: Duration = Duration::from_secs(2);
/// How long a wait may last before the host checks the direct child again,
/// so a descendant that holds the PTY open cannot hide the child's exit.
const CHILD_POLL: Duration = Duration::from_millis(100);
/// The bound on the PTY reads drained after the child exits.
const MAX_EXIT_PTY_READS: usize = 4;
/// How often a half-closed client is checked for a full close.
const CLIENT_PROBE_MS: u32 = 100;

fn protocol() -> io::Error {
    errno(libc::EPROTO)
}

/// The tick position whose coordinates have these bits (fact Tick).
fn position(x: u32, y: u32) -> SelectionPosition {
    SelectionPosition {
        x: f32::from_bits(x),
        y: f32::from_bits(y),
    }
}

impl From<VerticalDirection> for Direction {
    fn from(direction: VerticalDirection) -> Self {
        match direction {
            VerticalDirection::Up => Self::Up,
            VerticalDirection::Down => Self::Down,
        }
    }
}

impl From<Direction> for VerticalDirection {
    fn from(direction: Direction) -> Self {
        match direction {
            Direction::Up => Self::Up,
            Direction::Down => Self::Down,
        }
    }
}

impl Reason {
    /// The Failure sent for this reason, in answer to `fact`, byte for
    /// byte.
    fn failure(self, fact: Fact) -> Failure {
        use FailureCode::{InvalidInput, Protocol, Terminal};
        let (code, detail) = match self {
            Self::VerticalBusy => (InvalidInput, "vertical interaction is already deferred"),
            Self::VerticalAhead => (
                InvalidInput,
                match fact {
                    Fact::PreviewVertical { .. } => "preview frame revision is ahead of authority",
                    _ => "scroll frame revision is ahead of authority",
                },
            ),
            Self::ScrollNoRoom => (Terminal, "client output queue cannot admit scroll outcome"),
            Self::LiveOwned => (
                InvalidInput,
                "live history is unavailable while the terminal owns scrolling",
            ),
            Self::LiveNoRoom => (
                Terminal,
                "client output queue cannot admit the live viewport",
            ),
            Self::WheelOutside => (InvalidInput, "wheel is outside the terminal grid"),
            Self::WheelNoRoom => (Terminal, "client output queue cannot admit wheel outcome"),
            Self::WheelSynchronized => (Terminal, "presentation is synchronized"),
            Self::SelectionAhead => (
                InvalidInput,
                "selection frame revision is ahead of authority",
            ),
            Self::SelectionActive => (InvalidInput, "selection press has an active gesture"),
            Self::UpdateIdle => (InvalidInput, "selection update has no active selection"),
            Self::FinishIdle => (InvalidInput, "selection finish has no active selection"),
            Self::NoCopy => (InvalidInput, "no finished selection to copy"),
            Self::SelectionOutside => (
                InvalidInput,
                "selection position is outside the current viewport",
            ),
            Self::SelectionNoRoom => (Terminal, "client output queue cannot admit selection frame"),
            Self::PointerNoRoom => (Terminal, "client output queue cannot admit pointer result"),
            Self::SelectionFailed => (Terminal, "selection could not be applied"),
            Self::TickNoGesture => (
                InvalidInput,
                "selection autoscroll requires an active primary-screen host gesture",
            ),
            Self::TickNotTopRow => (
                InvalidInput,
                "selection autoscroll position must be in the top row",
            ),
            Self::AnchorLost => (InvalidInput, "selection anchor is no longer valid"),
            Self::TickNoRoom => (
                Terminal,
                "client output queue cannot admit selection scroll outcome",
            ),
            Self::RoleRepeated => (Protocol, "attachment role may only be sent once"),
            Self::PtyFull => (Terminal, "PTY input queue is full"),
            Self::PtyClosed => (Terminal, "PTY is closed"),
        };
        Failure {
            code,
            detail: detail.to_owned(),
        }
    }
}

/// Bend's bounds, sent once when the worker starts.
struct Bounds {
    finish_ms: u32,
    peer_ms: u32,
    sync_ms: u32,
    output_bytes: u32,
}

/// The Bend worker and its two pipes: facts go out on its descriptor 3, and
/// decisions come back on its descriptor 4.
struct Bend {
    worker: Child,
    facts: File,
    decisions: File,
    unread: Vec<u8>,
    /// One read's buffer, as large as a Linux pipe's default capacity, so
    /// one read takes everything the worker has written.
    chunk: Vec<u8>,
}

impl Bend {
    fn spawn(worker: &Path, args: &[OsString]) -> io::Result<Self> {
        let (fact_reader, facts) = platform::pipe(false)?;
        let (decisions, decision_writer) = platform::pipe(false)?;
        let (reader, writer) = (fact_reader.as_raw_fd(), decision_writer.as_raw_fd());
        let mut command = Command::new(worker);
        command.args(args);
        // SAFETY: Only async-signal-safe calls run between fork and exec.
        unsafe {
            command.pre_exec(move || platform::configure_worker(reader, writer));
        }
        let worker = command.spawn()?;
        Ok(Self {
            worker,
            facts,
            decisions,
            unread: Vec::new(),
            chunk: vec![0; 1 << 16],
        })
    }

    /// The worker's next line of an answer due by the deadline. A
    /// termination signal ends the wait at once. Lines already read, and
    /// what the worker has written, are taken first, so a host that was
    /// itself paused past the deadline still accepts a complete answer; the
    /// worker has stalled, and is treated as dead, only when the deadline has
    /// passed and what it wrote still lacks the end line. Each read that
    /// leaves the answer incomplete is checked, so a worker that keeps
    /// writing without finishing its answer is cut off too.
    fn line(&mut self, deadline: Instant) -> io::Result<String> {
        let pipe = self.decisions.as_raw_fd();
        loop {
            terminated()?;
            if let Some(end) = self.unread.iter().position(|&byte| byte == b'\n') {
                let mut line: Vec<u8> = self.unread.drain(..=end).collect();
                line.pop();
                return String::from_utf8(line).map_err(|_| protocol());
            }
            if poll_fds(&[(pipe, platform::POLLIN)], Some(deadline))?[0] != 0 {
                match self.decisions.read(&mut self.chunk) {
                    Ok(0) => return Err(io::Error::from_raw_os_error(libc::EPIPE)),
                    Ok(read) => self.unread.extend_from_slice(&self.chunk[..read]),
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(error) => return Err(error),
                }
            }
            if Instant::now() >= deadline && !self.answered() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "worker stalled: no answer within {} s",
                        DECISION_DEADLINE.as_secs()
                    ),
                ));
            }
        }
    }

    /// Whether the lines read hold the end line that completes an answer.
    fn answered(&self) -> bool {
        self.unread
            .split_inclusive(|&byte| byte == b'\n')
            .any(|line| line.strip_suffix(b"\n") == Some(END.as_bytes()))
    }

    /// Bend's next decisions, up to its end line, by the deadline.
    fn decisions(&mut self, deadline: Instant) -> io::Result<Vec<Decision>> {
        let mut decisions = Vec::new();
        loop {
            let line = self.line(deadline)?;
            if line == END {
                return Ok(decisions);
            }
            decisions.push(Decision::decode(&line).ok_or_else(protocol)?);
        }
    }

    /// Sends one fact's line in one write, under PIPE_BUF, and returns
    /// Bend's answer.
    fn ask(&mut self, fact: Fact, flags: Flags) -> io::Result<Vec<Decision>> {
        self.facts.write_all(fact.line(flags).as_bytes())?;
        self.decisions(Instant::now() + DECISION_DEADLINE)
    }

    /// Bend's bounds, after the line naming the schema the worker was
    /// generated from; a worker from another schema is refused before any
    /// fact.
    fn bounds(&mut self) -> io::Result<Bounds> {
        let deadline = Instant::now() + DECISION_DEADLINE;
        let named = self.line(deadline)?;
        if named != SCHEMA {
            return Err(io::Error::other(format!(
                "worker and host come from different boundary schemas \
                 (worker: {named:?}, host: {SCHEMA:?})"
            )));
        }
        match self.decisions(deadline)?[..] {
            [
                Decision::Bounds {
                    finish_ms,
                    peer_ms,
                    sync_ms,
                    output_bytes,
                },
            ] => Ok(Bounds {
                finish_ms,
                peer_ms,
                sync_ms,
                output_bytes,
            }),
            _ => Err(protocol()),
        }
    }
}

/// Polls `fds` and the worker's decision pipe until one is ready or the
/// deadline passes; returns each of `fds`' ready events. A termination signal,
/// even one received just before the wait, ends it early so the caller can
/// check for termination, and a worker that died ends it with EPIPE. The wait
/// for the worker's answer passes the decision pipe in `fds`, where being
/// readable means an answer.
fn poll_fds(fds: &[(RawFd, i16)], deadline: Option<Instant>) -> io::Result<Vec<i16>> {
    let worker = WORKER.load(Ordering::Relaxed);
    // poll skips a negative descriptor.
    let worker = if fds.iter().any(|&(fd, _)| fd == worker) {
        -1
    } else {
        worker
    };
    let timeout = deadline.map_or(-1, |deadline| {
        i32::try_from(
            deadline
                .saturating_duration_since(Instant::now())
                .as_millis(),
        )
        .unwrap_or(i32::MAX)
    });
    let ready = platform::poll(
        fds.iter().copied().chain([
            (SIGNAL_READ.load(Ordering::Relaxed), platform::POLLIN),
            (worker, platform::POLLIN),
        ]),
        timeout,
    );
    let mut revents = match ready {
        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
            return Ok(vec![0; fds.len()]);
        }
        ready => ready?,
    };
    if revents.pop().is_some_and(|worker| worker != 0) {
        return Err(io::Error::from_raw_os_error(libc::EPIPE));
    }
    // A termination signal ends the wait; the pipe stays readable.
    if revents.pop().is_some_and(|signal| signal != 0) {
        return Ok(vec![0; fds.len()]);
    }
    Ok(revents)
}

fn wait_fd(fd: RawFd, events: i16, deadline: Option<Instant>) -> io::Result<()> {
    poll_fds(&[(fd, events)], deadline).map(drop)
}

fn terminated() -> io::Result<()> {
    let number = TERMINATION_SIGNAL.load(Ordering::Relaxed);
    if number == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(libc::EINTR))
    }
}

struct Host {
    bounds: Bounds,
    listener: Listener,
    orbit: Orbit,
    /// The attached interactive client.
    client: Option<Peer>,
    /// A peer negotiating its role, or one awaiting Bend's admission answer.
    peer: Option<Peer>,
    /// Bend's negotiation bound for the peer until its first message arrives.
    peer_deadline: Option<Timer>,
    /// A connection that arrived while the peer negotiates, awaiting Bend's
    /// answer.
    crowded: Option<UnixStream>,
    /// The metadata observer.
    observer: Option<Peer>,
    /// The management socket and record of a managed run (ORB-C13).
    management: Option<Management>,
    /// Facts the host owes Bend before it waits for the next event.
    queued: VecDeque<Fact>,
    /// The containment check's fault: Bend fails as soon as the child starts.
    fault: bool,
    /// The host's exit code once Bend has decided the finish.
    exit_code: Option<u32>,
    /// Where the next wait starts looking for a ready source.
    turn: usize,
    /// Refused clients still writing their last output, each until its
    /// deadline; the interactive slot is already free.
    closing: Vec<(Peer, Timer)>,
    /// Checks a half-closed client for a full close, which macOS reports no
    /// differently from a half-close while nothing waits to be written.
    probe: Timer,
    /// Whether the PTY is still read and written; it closes at end of file,
    /// or at EIO after the child exits (ORB-C1).
    pty_open: bool,
    /// Bend has heard that the direct child exited.
    child_exited: bool,
}

/// A descriptor the host's wait watches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Closing,
    ClientProbe,
    ManagementListener,
    ManagementConnection,
    ManagementDeadline,
    Client,
    Pty,
    Hold,
    Observer,
    Listener,
    Peer,
    PeerDeadline,
}

/// The first ready source at or after `turn`, wrapping, so that each source
/// that stays ready is served in turn.
fn next_ready(revents: &[i16], turn: usize) -> Option<usize> {
    (0..revents.len())
        .map(|offset| (turn + offset) % revents.len())
        .find(|&index| revents[index] != 0)
}

impl Host {
    /// Opens the listener and spawns the child; peers may connect at once.
    fn open(bounds: Bounds, serve: Serve) -> io::Result<Self> {
        let listener = Listener::open(&serve.socket)?;
        let prepared = serve
            .management
            .map(|launch| launch.prepare(&serve.socket))
            .transpose()?;
        let orbit = Orbit::spawn(&serve.command, serve.palette, 80, 24)?;
        // Ready: the live record names both sockets and this process.
        let management = prepared
            .map(|prepared| prepared.publish(&serve.socket))
            .transpose()?;
        Ok(Self {
            bounds,
            listener,
            orbit,
            client: None,
            peer: None,
            peer_deadline: None,
            crowded: None,
            observer: None,
            management,
            queued: VecDeque::new(),
            fault: cli::guardian_fault(),
            exit_code: None,
            turn: 0,
            probe: Timer::default(),
            closing: Vec::new(),
            pty_open: true,
            child_exited: false,
        })
    }

    fn serve(&mut self, bend: &mut Bend) -> io::Result<()> {
        let pid = self.orbit.pid;
        let mut fact = if self.fault {
            Fact::Fault { pid }
        } else {
            Fact::Started { pid }
        };
        loop {
            let flags = self.flags(fact)?;
            let decisions = bend.ask(fact, flags)?;
            self.carry_out(fact, &decisions)?;
            fact = match self.queued.pop_front() {
                Some(next) => next,
                None if self.exit_code.is_some() => return Ok(()),
                None => self.wait()?,
            };
        }
    }

    /// The facts every decision may need. Reading the synchronized-output mode
    /// arms Bend's watchdog the first time it is set, as Bend's own check did.
    fn flags(&mut self, fact: Fact) -> io::Result<Flags> {
        // A pointer request carries the flags its phase was decided with.
        let mut flags = match (fact, self.client.as_ref()) {
            (Fact::Selection { .. }, Some(client)) => client
                .selection
                .as_ref()
                .map_or_else(Flags::default, |selection| selection.flags),
            _ => Flags::default(),
        };
        flags.held |= self.orbit.synchronized(self.bounds.sync_ms)?;
        flags.unanchored |= self.orbit.pointer.unanchored;
        // Without an observer, nothing was queued for the one Bend may admit.
        let (changed, fresh) = match &self.observer {
            Some(observer) => {
                let (title, directory) = self.orbit.title_and_directory()?;
                (
                    observer.metadata_changed(title, directory),
                    observer.metadata_fresh(),
                )
            }
            None => (true, true),
        };
        flags.metadata_changed |= changed;
        flags.metadata_fresh |= fresh;
        let Some(client) = self.client.as_ref() else {
            return Ok(flags);
        };
        flags.routed |= self.orbit.routed()?;
        flags.alternate |= self.orbit.alternate()?;
        flags.room |= client.output.can_push_scroll_outcome(client.output_bound);
        flags.frame_room |= client.output.can_push_frame(client.output_bound);
        flags.above_live |= fact == Fact::Applied(Effect::Key) && self.orbit.above_live()?;
        let clipboard = self.orbit.clipboard.borrow();
        flags.clipboard |= !clipboard.writes.is_empty();
        flags.clipboard_overflow |= clipboard.overflowed;
        Ok(flags)
    }

    /// The next fact for Bend. Queued PTY input is written whenever the loop
    /// turns, and every wait watches the PTY for room while some remains.
    fn wait(&mut self) -> io::Result<Fact> {
        loop {
            terminated()?;
            if self.pty_open {
                self.orbit.flush_writes()?;
            }
            if let Some(fact) = self.wake()? {
                return Ok(fact);
            }
        }
    }

    /// One fact per PTY turn. An attached client gets a frame per 8 KiB read;
    /// without one, nobody sees frames, so the host drains the PTY, up to
    /// 512 KiB, before asking Bend, since each fact costs a round trip
    /// through the worker. The foreground group is sampled before each read,
    /// and a change ends the turn, so the turn's sample held for all of its
    /// output (ORB-C15).
    fn read_pty(&mut self) -> io::Result<Option<Fact>> {
        self.orbit.start_turn();
        let turns = if self.client.is_some() { 1 } else { 64 };
        let foreground = self.orbit.foreground();
        let mut data = [0; 8192];
        let mut read = false;
        let mut closed = false;
        for _ in 0..turns {
            if read && self.orbit.foreground() != foreground {
                break;
            }
            match self.orbit.read_into(&mut data)? {
                PtyRead::Bytes(_) => read = true,
                PtyRead::Closed => {
                    closed = true;
                    break;
                }
                PtyRead::Blocked(_) => break,
            }
        }
        let output = read.then(|| Fact::Output {
            foreground,
            marks: self.orbit.status_marks(),
        });
        if closed {
            return self.pty_closed(output).map(Some);
        }
        Ok(output)
    }

    /// The PTY closed after a turn that may have read output. Once the
    /// direct child has exited, the run stops; while it lives, nothing more
    /// is read or written and the run waits for its exit (ORB-C1).
    fn pty_closed(&mut self, output: Option<Fact>) -> io::Result<Fact> {
        self.pty_open = false;
        self.orbit.writes.discard();
        if self.orbit.try_reap()?.is_some() {
            self.child_exited = true;
            return Ok(Fact::ChildExited {
                drained: output.is_some(),
            });
        }
        let Some(output) = output else {
            return Ok(Fact::PtyClosed);
        };
        self.queued.push_back(Fact::PtyClosed);
        Ok(output)
    }

    /// The direct child exited: the PTY's remaining output, at most four
    /// reads, reaches the terminal before the stop, so a descendant that
    /// keeps writing cannot hold the run open (ORB-C12).
    fn drain_exited(&mut self) -> io::Result<Fact> {
        self.child_exited = true;
        self.orbit.start_turn();
        let mut drained = false;
        if self.pty_open {
            let mut data = [0; 8192];
            for _ in 0..MAX_EXIT_PTY_READS {
                match self.orbit.read_into(&mut data)? {
                    PtyRead::Bytes(_) => drained = true,
                    PtyRead::Closed | PtyRead::Blocked(_) => break,
                }
            }
            self.pty_open = false;
            self.orbit.writes.discard();
        }
        Ok(Fact::ChildExited { drained })
    }

    /// One turn of the event loop. The host settles the client and the
    /// pending peer, then waits on every source of the current state: the
    /// client, the PTY, the synchronized-output timer, the observer while its
    /// queue holds output, and the listener or, while a peer negotiates, that
    /// peer and its deadline. It serves one
    /// ready source, rotating among those that stay ready, so a busy PTY
    /// cannot starve the client or a peer.
    fn wake(&mut self) -> io::Result<Option<Fact>> {
        if !self.child_exited && self.orbit.try_reap()?.is_some() {
            return self.drain_exited().map(Some);
        }
        if let Some(client) = self.client.as_mut() {
            match client.prepare_wait()? {
                ClientState::Open => {}
                ClientState::Closed => return Ok(Some(Fact::Closed { full: true })),
                ClientState::Pressured => return Ok(Some(Fact::Pressured)),
            }
        }
        if let Some(fact) = self.negotiate()? {
            return Ok(Some(fact));
        }
        self.closing
            .retain_mut(|(peer, deadline)| !peer.drain() && !deadline.expired());
        if let Some(management) = self.management.as_mut()
            && let Some(event) = management.flush()?
        {
            return Ok(Some(Fact::Management(event)));
        }
        let pty_events = platform::POLLIN
            | if self.orbit.writes_pending() {
                platform::POLLOUT
            } else {
                0
            };
        let mut sources = Vec::with_capacity(8);
        // Deadlines wait alongside descriptors: the wait polls until the
        // earliest, and each that has passed counts as a ready source.
        let mut timers = Vec::with_capacity(3);
        if let Some(management) = &self.management {
            if let Some(connection) = &management.connection {
                // One request at a time: the next is read once the answer is
                // written.
                let events = if connection.writing() {
                    platform::POLLOUT
                } else {
                    platform::POLLIN
                };
                sources.push((
                    Source::ManagementConnection,
                    connection.stream.as_raw_fd(),
                    events,
                ));
                timers.push((Source::ManagementDeadline, connection.deadline()));
            }
            // A connection waits for Bend's admission before the next is taken.
            if management.incoming.is_none() {
                sources.push((
                    Source::ManagementListener,
                    management.listener.listener.as_raw_fd(),
                    platform::POLLIN,
                ));
            }
        }
        match &self.client {
            Some(client) => {
                sources.push((
                    Source::Client,
                    client.stream.as_raw_fd(),
                    client.wait_events(),
                ));
                if client.half_closed() {
                    if self.probe.deadline().is_none() {
                        self.probe.set(Some(CLIENT_PROBE_MS));
                    }
                    timers.push((Source::ClientProbe, self.probe.deadline()));
                }
            }
            None => self.probe.set(None),
        }
        if self.pty_open {
            sources.push((Source::Pty, self.orbit.master.as_raw_fd(), pty_events));
        }
        timers.push((Source::Hold, self.orbit.hold.deadline()));
        for (peer, deadline) in &self.closing {
            sources.push((Source::Closing, peer.stream.as_raw_fd(), platform::POLLOUT));
            timers.push((Source::Closing, deadline.deadline()));
        }
        // The observer is read for its close or a message it may not send,
        // and written while its queue holds output.
        if let Some(observer) = &self.observer {
            let events = platform::POLLIN
                | if observer.output.is_empty() {
                    0
                } else {
                    platform::POLLOUT
                };
            sources.push((Source::Observer, observer.stream.as_raw_fd(), events));
        }
        if let Some(peer) = &self.peer {
            sources.push((Source::Peer, peer.stream.as_raw_fd(), platform::POLLIN));
            timers.push((
                Source::PeerDeadline,
                self.peer_deadline.as_ref().and_then(Timer::deadline),
            ));
        }
        // While a peer negotiates, a later connection is still accepted, for
        // Bend to turn away (ORB-C3).
        if self.crowded.is_none() {
            sources.push((
                Source::Listener,
                self.listener.listener.as_raw_fd(),
                platform::POLLIN,
            ));
        }
        let fds: Vec<_> = sources
            .iter()
            .map(|&(_, fd, events)| (fd, events))
            .collect();
        let armed: Vec<_> = timers
            .into_iter()
            .filter_map(|(source, deadline)| Some((source, deadline?)))
            .collect();
        // The wait also ends in time to check the direct child again.
        let child_poll = Instant::now() + CHILD_POLL;
        let deadline = armed
            .iter()
            .map(|&(_, at)| at)
            .fold(child_poll, Instant::min);
        let mut revents = poll_fds(&fds, Some(deadline))?;
        let now = Instant::now();
        for &(source, at) in &armed {
            sources.push((source, -1, 0));
            revents.push(if now >= at { platform::POLLIN } else { 0 });
        }
        let Some(index) = next_ready(&revents, self.turn) else {
            return Ok(None);
        };
        self.turn = index + 1;
        match sources[index].0 {
            Source::ManagementListener => Ok(self
                .management
                .as_mut()
                .ok_or_else(protocol)?
                .accept()?
                .map(Fact::Management)),
            // Writing waits for the next turn's flush; reading takes a request.
            Source::ManagementConnection => {
                let management = self.management.as_mut().ok_or_else(protocol)?;
                if revents[index] & (platform::POLLIN | platform::POLLHUP | platform::POLLERR) == 0
                {
                    return Ok(None);
                }
                Ok(management.read()?.map(Fact::Management))
            }
            Source::ManagementDeadline => Ok(self
                .management
                .as_mut()
                .ok_or_else(protocol)?
                .expire()
                .map(Fact::Management)),
            Source::Client => {
                let client = self.client.as_mut().ok_or_else(protocol)?;
                match client.on_ready(revents[index])? {
                    None => Ok(None),
                    Some(ClientEvent::Request) => self.request(),
                    Some(ClientEvent::Closed { full }) => Ok(Some(Fact::Closed { full })),
                }
            }
            // The next turn writes or drops the refused clients.
            Source::Closing => Ok(None),
            Source::ClientProbe => {
                let client = self.client.as_ref().ok_or_else(protocol)?;
                Ok((self.probe.expired() && client.closed_fully())
                    .then_some(Fact::Closed { full: true }))
            }
            Source::Pty => self.read_pty(),
            Source::Hold => Ok(self.orbit.hold.expired().then_some(Fact::Expired)),
            Source::Observer => Ok(self.flush_observer()),
            Source::Listener => self.accept(),
            // The next turn reads the peer or drops it once its bound passes.
            Source::Peer | Source::PeerDeadline => Ok(None),
        }
    }

    /// Reads the pending peer's first message. A peer that keeps the slot
    /// past Bend's bound is closed unanswered, so the next connection can
    /// negotiate (ORB-C3).
    fn negotiate(&mut self) -> io::Result<Option<Fact>> {
        let Some(peer) = self.peer.as_mut() else {
            return Ok(None);
        };
        if let Some(deadline) = self.peer_deadline.as_mut()
            && deadline.expired()
        {
            self.peer = None;
            self.peer_deadline = None;
            return Ok(None);
        }
        let message = match peer.recv_role() {
            Ok(message) => message,
            // While a client is attached, a failed peer just goes.
            Err(_) if self.client.is_some() => {
                self.peer = None;
                self.peer_deadline = None;
                return Ok(None);
            }
            Err(error) if peer_disconnected(&error) => Some(peer.abort_hello()),
            Err(error) => return Err(error),
        };
        let Some(message) = message else {
            return Ok(None);
        };
        self.peer_deadline = None;
        Ok(Some(self.peer_fact(message)))
    }

    /// Takes the next connection as the negotiating peer, or, while one
    /// negotiates, reports it for Bend's answer.
    fn accept(&mut self) -> io::Result<Option<Fact>> {
        match self.listener.listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(true)?;
                if self.peer.is_some() {
                    self.crowded = Some(stream);
                    return Ok(Some(Fact::Crowded));
                }
                let mut deadline = Timer::default();
                deadline.set(Some(self.bounds.peer_ms));
                self.peer = Some(Peer::new(stream));
                self.peer_deadline = Some(deadline);
                Ok(None)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Reports a peer's first message. Before an observer request, Bend first
    /// hears whether the current observer has gone or sent a message, so its
    /// slot is current.
    fn peer_fact(&mut self, message: FirstMessage) -> Fact {
        let fact = Fact::Peer(message);
        if message != FirstMessage::ObserveMetadata {
            return fact;
        }
        match self.observer_event() {
            Some(outcome) => {
                self.queued.push_back(fact);
                outcome
            }
            None => fact,
        }
    }

    /// The fact reporting that the observer closed, failed or sent a message,
    /// if it has.
    fn observer_event(&mut self) -> Option<Fact> {
        let observer = self.observer.as_mut()?;
        match observer.observer_input() {
            Ok(ObserverInput::Quiet) => None,
            Ok(ObserverInput::Closed) => Some(Fact::ObserverGone(ObserverLoss::Closed)),
            Ok(ObserverInput::Message) => Some(Fact::ObserverMessage),
            Ok(ObserverInput::Unsupported) | Err(_) => {
                Some(Fact::ObserverGone(ObserverLoss::Failed))
            }
        }
    }

    /// Writes the observer's queued metadata as its socket takes it; a gone or
    /// failed observer is reported for Bend to drop.
    fn flush_observer(&mut self) -> Option<Fact> {
        if let Some(gone) = self.observer_event() {
            return Some(gone);
        }
        let observer = self.observer.as_mut()?;
        observer
            .flush_observer()
            .err()
            .map(|_| Fact::ObserverGone(ObserverLoss::Failed))
    }

    /// Queues the observer current metadata at `revision`, `ordered` or
    /// replaceable as Bend decides. A gone, pressured or failed observer is
    /// reported for Bend to drop; the host never drops it itself.
    fn publish_metadata(&mut self, revision: u64, ordered: bool) -> io::Result<()> {
        if self.observer.is_none() {
            return Ok(());
        }
        if let Some(outcome) = self.observer_event() {
            self.queued.push_back(outcome);
            return Ok(());
        }
        let metadata = self.orbit.metadata(revision)?;
        let observer = self.observer.as_mut().ok_or_else(protocol)?;
        match observer.send_metadata(metadata, ordered) {
            Ok(ObserverSend::Sent) => {}
            Ok(ObserverSend::Pressured) => self
                .queued
                .push_back(Fact::ObserverGone(ObserverLoss::Pressured)),
            Err(_) => self
                .queued
                .push_back(Fact::ObserverGone(ObserverLoss::Failed)),
        }
        Ok(())
    }

    fn observe(&mut self) -> io::Result<()> {
        let mut peer = self.peer.take().ok_or_else(protocol)?;
        loop {
            match peer.send_role(Role::Metadata) {
                Ok(Step::Blocked(fd)) => {
                    wait_fd(fd, platform::POLLOUT, None)?;
                    terminated()?;
                }
                Ok(Step::Done) => {
                    peer.output_bound = self.bounds.output_bytes as usize;
                    self.observer = Some(peer);
                    return Ok(());
                }
                Err(_) => {
                    self.queued
                        .push_back(Fact::ObserverGone(ObserverLoss::Failed));
                    return Ok(());
                }
            }
        }
    }

    /// Classifies the client's request; plain input and resizes are applied
    /// before Bend answers.
    fn request(&mut self) -> io::Result<Option<Fact>> {
        let client = self.client.as_mut().ok_or_else(protocol)?;
        if let Some(refusal) = &client.refused {
            return Ok(Some(Fact::Refused {
                version: matches!(refusal, Refusal::Version),
            }));
        }
        match client.request.take().ok_or_else(protocol)? {
            ClientMessage::Hello | ClientMessage::ObserveMetadata => Ok(Some(Fact::RoleRepeated)),
            // Once the PTY closes, no request is applied.
            _ if !self.pty_open => Ok(Some(Fact::Applied(Effect::Closed))),
            // Bend compares a request's revision with authority.
            ClientMessage::PreviewVertical {
                frame_revision,
                direction,
            } => Ok(Some(Fact::PreviewVertical {
                revision: frame_revision,
                direction: direction.into(),
            })),
            ClientMessage::ScrollVertical {
                frame_revision,
                rows,
            } => Ok(Some(Fact::ScrollVertical {
                revision: frame_revision,
                rows: u32::from(rows.cast_unsigned()),
            })),
            ClientMessage::ReturnToLive => Ok(Some(Fact::ReturnToLive)),
            ClientMessage::Mouse(input) if wheel_direction(input.button).is_some() => {
                self.wheel_fact(input).map(Some)
            }
            ClientMessage::Selection(action) => self.selection_fact(action),
            request => Ok(Some(Fact::Applied(self.orbit.apply(request)?))),
        }
    }

    /// A pointer phase becomes Bend's to decide, with every fact either route
    /// needs: the terminal's mouse tracking and Shift for the route, and
    /// whether each route's answer fits. An upward tick is a vertical request
    /// of its own.
    fn selection_fact(&mut self, action: SelectionAction) -> io::Result<Option<Fact>> {
        let kind = match action {
            SelectionAction::Begin { .. } => SelectionKind::Begin,
            SelectionAction::Update { .. } => SelectionKind::Update,
            SelectionAction::Finish { .. } => SelectionKind::Finish,
            SelectionAction::Cancel => SelectionKind::Cancel,
            SelectionAction::Copy => SelectionKind::Copy,
            SelectionAction::AutoscrollUp { position } => {
                return self.tick_fact(position).map(Some);
            }
        };
        let orbit = &self.orbit;
        let client = self.client.as_mut().ok_or_else(protocol)?;
        let mut flags = Flags::default();
        let mut routed = Vec::new();
        if orbit.tracking()? {
            flags.tracking = true;
        }
        let bound = client.output_bound;
        if client.output.can_push_result_frame(bound) {
            flags.result_frame_room = true;
        }
        let revision = match action {
            SelectionAction::Begin { frame_revision, .. } => frame_revision,
            _ => 0,
        };
        if let SelectionAction::Begin { modifiers, .. }
        | SelectionAction::Update { modifiers, .. }
        | SelectionAction::Finish { modifiers, .. } = action
        {
            if modifiers.contains(Modifiers::SHIFT) {
                flags.shift = true;
            }
            if orbit.outside(action) {
                flags.outside = true;
            }
            let finishing = kind == SelectionKind::Finish;
            if client.output.can_push_host_selection(bound, finishing) {
                flags.host_room = true;
            }
            // Any revision sizes the answer: it crosses as eight bytes.
            let response = if finishing {
                ServerMessage::SelectionFinished { frame_revision: 0 }
            } else {
                ServerMessage::Accepted
            };
            if client.output.can_push_ordered(&response, bound)? {
                flags.terminal_room = true;
            }
            routed = orbit.pointer_input(action)?;
            if !orbit.writes.fits(routed.len()) {
                flags.pty_full = true;
            }
        }
        client.selection = Some(PendingSelection {
            action,
            flags,
            routed,
        });
        Ok(Some(Fact::Selection { kind, revision }))
    }

    /// An upward selection tick, with whether it may run.
    fn tick_fact(&mut self, position: SelectionPosition) -> io::Result<Fact> {
        Ok(Fact::Tick {
            state: self.orbit.tick_state(position)?,
            x: position.x.to_bits(),
            y: position.y.to_bits(),
        })
    }

    /// Carries out Bend's host phase and reports how it went.
    fn select_host(&mut self) -> io::Result<()> {
        let client = self.client.as_mut().ok_or_else(protocol)?;
        let selection = client.selection.take().ok_or_else(protocol)?;
        let fact = match self.orbit.select_host(selection.action) {
            None => Fact::Selected {
                outcome: Selected::Failed,
                presented: false,
            },
            Some(presented) => {
                client.output.supersede_frames();
                let outcome = match selection.action {
                    SelectionAction::Begin { .. } => Selected::Began,
                    SelectionAction::Finish { .. } => Selected::Finished,
                    _ => Selected::Applied,
                };
                Fact::Selected { outcome, presented }
            }
        };
        self.queued.push_front(fact);
        Ok(())
    }

    /// Carries out Bend's terminal phase: the pointer input goes to the PTY.
    fn select_terminal(&mut self) -> io::Result<()> {
        let client = self.client.as_mut().ok_or_else(protocol)?;
        let selection = client.selection.take().ok_or_else(protocol)?;
        if !self.orbit.writes.push(&selection.routed) {
            return Err(protocol());
        }
        self.orbit.flush_writes()
    }

    /// Sends the frozen copy: the selection clipboard's after a finished host
    /// sequence, the standard clipboard's for Copy.
    fn send_copied(&mut self, selection: bool) -> io::Result<()> {
        let text = self.orbit.pointer.copied.clone();
        let location = if selection {
            ClipboardLocation::Selection
        } else {
            ClipboardLocation::Standard
        };
        self.send(move |_| Ok(ServerMessage::CopiedText { location, text }))
    }

    /// Ends the attachment after Bend refused it: the client keeps writing
    /// what it queued, its Failure included, for at most the finish bound;
    /// the slot is free at once.
    fn close_client(&mut self) -> io::Result<()> {
        if let Some(client) = self.client.take() {
            self.close_after_flush(client);
        }
        self.drop_client()
    }

    /// Refuses the observer's message with Failure(Protocol) and closes it
    /// once that is written, within the finish bound; its slot is free at once.
    fn refuse_observer(&mut self) -> io::Result<()> {
        let mut observer = self.observer.take().ok_or_else(protocol)?;
        observer.refuse_observer()?;
        self.close_after_flush(observer);
        Ok(())
    }

    /// Keeps writing a refused peer's queued output for at most the finish
    /// bound before its socket closes.
    fn close_after_flush(&mut self, mut peer: Peer) {
        if !peer.drain() {
            let mut deadline = Timer::default();
            deadline.set(Some(self.bounds.finish_ms));
            self.closing.push((peer, deadline));
        }
    }

    /// Ends the attachment.
    fn drop_client(&mut self) -> io::Result<()> {
        self.client = None;
        Ok(())
    }

    /// A physical vertical wheel becomes Bend's to decide, with the state of
    /// the input it would send the terminal.
    fn wheel_fact(&mut self, input: session::MouseEvent) -> io::Result<Fact> {
        let direction = wheel_direction(input.button).ok_or_else(protocol)?;
        let routed = self.orbit.routed_wheel(input, direction)?;
        let client = self.client.as_mut().ok_or_else(protocol)?;
        let outcome = ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted);
        let state = if routed.is_empty() {
            WheelInput::Empty
        } else if !client
            .output
            .can_push_ordered(&outcome, client.output_bound)?
        {
            WheelInput::NoOutcomeRoom
        } else if !self.orbit.writes.fits(routed.len()) {
            WheelInput::PtyFull
        } else {
            WheelInput::Fits
        };
        client.wheel = Some(Wheel { direction, routed });
        Ok(Fact::Wheel(state))
    }

    /// Carries out Bend's wheel answer: the routed input and TerminalRouted,
    /// or a one-row viewport move with its frame at `revision`.
    fn send_wheel(&mut self, routed: bool, revision: u64) -> io::Result<()> {
        let client = self.client.as_mut().ok_or_else(protocol)?;
        let wheel = client.wheel.take().ok_or_else(protocol)?;
        let orbit = &mut self.orbit;
        if routed {
            if !orbit.writes.push(&wheel.routed) {
                return Err(protocol());
            }
            orbit.flush_writes()?;
            client.send_server(|| Ok(ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted)))
        } else {
            // The viewport moves even for a client found lost meanwhile, since
            // Bend advanced the revision for the move.
            let outcome = orbit.wheel(wheel.direction, revision)?;
            client.send_server(|| Ok(outcome))
        }
    }

    /// Carries out Bend's answer to a preview in this direction.
    fn send_preview(
        &mut self,
        direction: Direction,
        routed: bool,
        revision: u64,
    ) -> io::Result<()> {
        let direction = direction.into();
        self.send(|orbit| {
            let outcome = if routed {
                PreviewOutcome::TerminalRouted
            } else {
                viewport_preview(&orbit.terminal, &orbit.extractor, direction)?
            };
            Ok(ServerMessage::VerticalPreview(VerticalPreview {
                frame_revision: revision,
                direction,
                outcome,
            }))
        })
    }

    /// Carries out Bend's answer to a scroll of these rows (fact ScrollVertical).
    fn send_scroll(&mut self, rows: u32, routed: bool, revision: u64) -> io::Result<()> {
        let rows = u16::try_from(rows).map_err(|_| protocol())?.cast_signed();
        if routed {
            self.send(|_| {
                Ok(ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned {
                    requested_rows: rows,
                }))
            })
        } else {
            let outcome = self.orbit.scroll(rows, revision)?;
            self.send(|_| Ok(outcome))
        }
    }

    /// Carries out Bend's answer to an upward selection tick from this
    /// position (fact Tick), whose frame is at `revision`.
    fn send_tick(&mut self, position: SelectionPosition, revision: u64) -> io::Result<()> {
        let client = self.client.as_mut().ok_or_else(protocol)?;
        let fact = match self.orbit.tick(position, revision)? {
            Some((outcome, presented)) => {
                client.send_server(|| Ok(outcome))?;
                Fact::Selected {
                    outcome: Selected::Ticked,
                    presented,
                }
            }
            None => Fact::Selected {
                outcome: Selected::Failed,
                presented: false,
            },
        };
        self.queued.push_front(fact);
        Ok(())
    }

    fn send(
        &mut self,
        message: impl FnOnce(&mut Orbit) -> io::Result<ServerMessage>,
    ) -> io::Result<()> {
        let client = self.client.as_mut().ok_or_else(protocol)?;
        let orbit = &mut self.orbit;
        client.send_server(|| message(orbit))
    }

    /// Sends the PTY turn's clipboard writes in order, each as one ordered
    /// message that no later frame replaces. A write the client's queue cannot
    /// take ends the attachment through the next wait's pressure fact.
    fn send_clipboard(&mut self) -> io::Result<()> {
        let writes = std::mem::take(&mut self.orbit.clipboard.borrow_mut().writes);
        for (location, text) in writes {
            self.send(move |_| Ok(ServerMessage::ClipboardWrite { location, text }))?;
        }
        Ok(())
    }

    /// Carries out Bend's decisions on `fact` in order, queueing the facts
    /// they produce.
    fn carry_out(&mut self, fact: Fact, decisions: &[Decision]) -> io::Result<()> {
        for &decision in decisions {
            match decision {
                // Bend sends its bounds only before the first fact.
                Decision::Bounds { .. } => return Err(protocol()),
                Decision::Attach => self.attach()?,
                Decision::Frame { revision } => {
                    self.send(|orbit| Ok(ServerMessage::Frame(Box::new(orbit.frame(revision)?))))?
                }
                Decision::Accepted => self.send(|_| Ok(ServerMessage::Accepted))?,
                Decision::Clipboard => self.send_clipboard()?,
                Decision::Capture { on } => self.orbit.capture_clipboard(on),
                Decision::StatusLifetime { edge, marks } => self.orbit.apply_status(edge, marks),
                Decision::Detach => self.drop_client()?,
                Decision::Dismiss => self.close_client()?,
                Decision::Reject(reason) => {
                    self.send(move |_| Ok(ServerMessage::Failure(reason.failure(fact))))?;
                    // A rejected wheel or pointer phase is answered and gone.
                    if let Some(client) = self.client.as_mut() {
                        client.wheel = None;
                        client.selection = None;
                    }
                }
                Decision::Release => {
                    self.orbit.release()?;
                }
                Decision::Bottom => self.orbit.bottom(),
                Decision::WheelAnswer { routed, revision } => self.send_wheel(routed, revision)?,
                Decision::SelectHost => self.select_host()?,
                Decision::SelectTerminal => self.select_terminal()?,
                Decision::Unselect { .. } => self.orbit.pointer.clear(&self.orbit.terminal)?,
                Decision::SelectionFinished { revision } => self.send(move |_| {
                    Ok(ServerMessage::SelectionFinished {
                        frame_revision: revision,
                    })
                })?,
                Decision::Copied { selection } => self.send_copied(selection)?,
                Decision::PreviewAnswer {
                    direction,
                    routed,
                    revision,
                } => self.send_preview(direction, routed, revision)?,
                Decision::ScrollAnswer {
                    rows,
                    routed,
                    revision,
                } => self.send_scroll(rows, routed, revision)?,
                Decision::LiveAnswer { revision } => {
                    let outcome = self.orbit.return_to_live(revision)?;
                    self.send(|_| Ok(outcome))?
                }
                Decision::TickAnswer { x, y, revision } => {
                    self.send_tick(position(x, y), revision)?
                }
                Decision::Retick { x, y } => {
                    let fact = self.tick_fact(position(x, y))?;
                    self.queued.push_front(fact);
                }
                Decision::RefuseRequest => {
                    let client = self.client.as_mut().ok_or_else(protocol)?;
                    client.answer_refused()?;
                }
                Decision::Observing => self.observe()?,
                Decision::ManageAdmit => self.managed()?.admit()?,
                Decision::ManageBusy => self.managed()?.busy()?,
                Decision::ManageLease => self.managed()?.lease()?,
                Decision::ManageStatus => self.managed()?.status()?,
                Decision::ManageRejectIdentity => {
                    self.managed()?.reject(ManagementFailure::InvalidIdentity)?
                }
                Decision::ManageReject => {
                    self.managed()?.reject(ManagementFailure::InvalidRequest)?
                }
                Decision::ManageStop => {
                    let fact = self.stop_explicitly()?;
                    self.queued.push_back(fact);
                }
                Decision::Metadata { revision, ordered } => {
                    self.publish_metadata(revision, ordered)?
                }
                Decision::DropObserver => self.observer = None,
                Decision::RefuseObserver => self.refuse_observer()?,
                Decision::Busy => self.answer_peer(Peer::reject_busy)?,
                Decision::TurnAway => turn_away(self.crowded.take().ok_or_else(protocol)?),
                Decision::Refuse => self.answer_peer(Peer::refuse_peer)?,
                Decision::Drop => self.peer = None,
                Decision::Reap => {
                    let fact = self.reap()?;
                    self.queued.push_back(fact);
                }
                Decision::Finish { status, explicit } => {
                    self.exit(status, explicit)?;
                    // An explicit stop succeeds once the child is reaped.
                    self.exit_code = Some(if explicit { 0 } else { status });
                    self.queued.push_back(Fact::Finished);
                }
            }
        }
        Ok(())
    }

    fn managed(&mut self) -> io::Result<&mut Management> {
        self.management.as_mut().ok_or_else(protocol)
    }

    /// Carries out Bend's management stop: the child is hung up and reaped
    /// with the bounded shutdown of ORB-C12. A child that already exited ends
    /// the run naturally instead.
    fn stop_explicitly(&mut self) -> io::Result<Fact> {
        if let Some(status) = self.orbit.try_reap()? {
            return Ok(Fact::ExitedBeforeStop { status });
        }
        if !self.contain() {
            return Ok(Fact::NotReaped);
        }
        let status = self.orbit.try_reap()?.ok_or_else(protocol)?;
        Ok(Fact::Reaped { status })
    }

    fn answer_peer(&mut self, answer: fn(Peer) -> io::Result<()>) -> io::Result<()> {
        answer(self.peer.take().ok_or_else(protocol)?)
    }

    fn attach(&mut self) -> io::Result<()> {
        let mut client = self.peer.take().ok_or_else(protocol)?;
        client.output_bound = self.bounds.output_bytes as usize;
        while let Step::Blocked(fd) = client.send_role(Role::Interactive)? {
            wait_fd(fd, platform::POLLOUT, None)?;
            terminated()?;
        }
        self.client = Some(client);
        Ok(())
    }

    fn reap(&mut self) -> io::Result<Fact> {
        let deadline = Instant::now() + REAP_DEADLINE;
        loop {
            if let Some(status) = self.orbit.try_reap()? {
                return Ok(Fact::Reaped { status });
            }
            if Instant::now() >= deadline {
                return Ok(Fact::NotReaped);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Gives the client, then a live observer, Bend's finish bound each to
    /// take its output and Exited; closes every peer, and removes the socket
    /// pathname.
    fn exit(&mut self, status: u32, explicit: bool) -> io::Result<()> {
        let status = i32::try_from(status).map_err(|_| protocol())?;
        // After a management stop the client closes without Exited.
        if explicit {
            self.client = None;
        }
        if let Some(mut client) = self.client.take() {
            let deadline = Instant::now() + Duration::from_millis(self.bounds.finish_ms.into());
            while let Step::Blocked(fd) = client.send_exited(status)? {
                if Instant::now() >= deadline {
                    break;
                }
                wait_fd(fd, platform::POLLOUT, Some(deadline))?;
            }
        }
        self.finish_observer(status);
        // An unnegotiated peer is closed without an exit event.
        self.peer = None;
        let reason = if explicit {
            TerminationReason::ExplicitStop
        } else {
            TerminationReason::NaturalExit
        };
        self.finish_management(reason)?;
        self.listener.unlink_owned()?;
        Ok(())
    }

    /// Queues Exited behind a live observer's output and writes the queue for
    /// at most the finish bound, then closes the observer. The wait watches
    /// only the observer, so it also serves the stop after a termination
    /// signal, whose pipe stays readable.
    fn finish_observer(&mut self, code: i32) {
        let Some(mut observer) = self.observer.take() else {
            return;
        };
        if !matches!(observer.observer_input(), Ok(ObserverInput::Quiet))
            || observer.send_observer_exited(code).is_err()
        {
            return;
        }
        let deadline = Instant::now() + Duration::from_millis(self.bounds.finish_ms.into());
        while !observer.output.is_empty() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return;
            }
            let timeout = i32::try_from(left.as_millis().max(1)).unwrap_or(i32::MAX);
            let _ = platform::poll([(observer.stream.as_raw_fd(), platform::POLLOUT)], timeout);
            if observer.flush_observer().is_err() {
                return;
            }
        }
    }

    /// Replaces a managed run's live record with its tombstone, answers an
    /// explicit stop's lease with it, then closes the management socket; the
    /// record stays.
    fn finish_management(&mut self, reason: TerminationReason) -> io::Result<()> {
        if let (Some(owner), Some(status)) = (self.management.as_mut(), self.orbit.exit_status()) {
            let tombstone = owner.finish(reason, status)?;
            if reason == TerminationReason::ExplicitStop {
                owner.reply_stopped(tombstone);
            }
        }
        self.management = None;
        Ok(())
    }

    /// After a host failure: hang up the child, escalate to SIGKILL, and reap it.
    /// The bounded shutdown of ORB-C12: an exited child is
    /// reaped without a signal to its recyclable PID; otherwise the child's
    /// group and a distinct foreground group get SIGHUP, then, after 500 ms,
    /// SIGKILL with the foreground group read again, and the child must be
    /// reaped within two seconds. Reports whether it was reaped.
    fn contain(&mut self) -> bool {
        if self.orbit.child.is_none() || !matches!(self.orbit.try_reap(), Ok(None)) {
            return true;
        }
        let pid = self.orbit.pid as platform::pid_t;
        let reaped_by = |orbit: &mut Orbit, deadline: Instant| {
            while Instant::now() < deadline {
                if !matches!(orbit.try_reap(), Ok(None)) {
                    return true;
                }
                thread::sleep(Duration::from_millis(10));
            }
            false
        };
        let hang_up = |orbit: &Orbit, number| {
            let foreground = orbit.foreground_group();
            let child = signal(pid, number);
            if let Some(group) = foreground.filter(|group| *group != pid) {
                platform::signal_group(group, number);
            }
            child
        };
        if hang_up(&self.orbit, platform::SIGHUP).is_ok()
            && reaped_by(&mut self.orbit, Instant::now() + Duration::from_millis(500))
        {
            return true;
        }
        let _ = hang_up(&self.orbit, platform::SIGKILL);
        reaped_by(&mut self.orbit, Instant::now() + Duration::from_secs(2))
    }
}

fn wait_worker(worker: &mut Child, deadline: Instant) -> io::Result<Option<ExitStatus>> {
    loop {
        if let Some(status) = worker.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Serves one session from Orbit's command line and returns the
/// process exit status: the child's exit code after a natural exit, 0 after an
/// orderly stop on a termination signal that reaped the child or came before
/// it started, or a dead worker's status.
pub fn run(worker: &Path, arguments: &[OsString]) -> io::Result<u32> {
    let serve = cli::parse(arguments)
        .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
    TERMINATION_SIGNAL.store(0, Ordering::Relaxed);
    install_termination_handlers()?;
    // Bend's policy is sequential, so one runtime thread serves it.
    let mut bend = Bend::spawn(worker, &["--threads".into(), "1".into()])?;
    WORKER.store(bend.decisions.as_raw_fd(), Ordering::Relaxed);
    let opened = bend.bounds().and_then(|bounds| Host::open(bounds, serve));
    let mut host = match opened {
        Ok(host) => host,
        Err(error) => {
            let _ = bend.worker.kill();
            let _ = bend.worker.wait();
            // Before the child starts, a termination signal leaves nothing
            // to stop.
            if TERMINATION_SIGNAL.load(Ordering::Relaxed) != 0 {
                return Ok(0);
            }
            return Err(error);
        }
    };
    match host.serve(&mut bend) {
        Ok(()) => {
            drop(bend.facts);
            match wait_worker(&mut bend.worker, Instant::now() + Duration::from_secs(2))? {
                Some(status) if status.success() => host.exit_code.ok_or_else(protocol),
                Some(status) => Ok(status_code(status)),
                None => {
                    let _ = bend.worker.kill();
                    Err(io::Error::from_raw_os_error(libc::ETIMEDOUT))
                }
            }
        }
        // A failed host, a terminated host, or a dead or stalled worker:
        // contain the child and remove the socket pathname before exiting.
        Err(error) => {
            let reaped = host.contain();
            let number = TERMINATION_SIGNAL.load(Ordering::Relaxed);
            // A termination signal is an explicit stop: once the child is
            // reaped, a live observer hears its exit, and then a managed run's
            // tombstone records the stop. The interactive
            // client closes without Exited.
            if number != 0 && reaped {
                if let Ok(Some(code)) = host.orbit.try_reap() {
                    host.finish_observer(code as i32);
                }
                let _ = host.finish_management(TerminationReason::ExplicitStop);
            }
            drop(host);
            // A stalled worker will not exit on its own, so it is killed at
            // once, and no status it reaches replaces the stall's diagnostic.
            let died = if error.kind() == io::ErrorKind::TimedOut {
                None
            } else {
                wait_worker(
                    &mut bend.worker,
                    Instant::now() + Duration::from_millis(500),
                )?
            };
            if died.is_none() {
                let _ = bend.worker.kill();
                let _ = bend.worker.wait();
            }
            // A termination signal is an orderly stop: it succeeds once the
            // child is reaped.
            if number != 0 {
                return Ok(if reaped { 0 } else { 128 + number as u32 });
            }
            match died {
                Some(status) => Ok(status_code(status)),
                None => Err(error),
            }
        }
    }
}

/// The interactive client's output bound that this build's worker sends, for
/// tests of the mechanisms it bounds.
#[cfg(test)]
pub(crate) fn built_worker_output_bound() -> usize {
    let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
    let mut bend = Bend::spawn(
        Path::new(env!("ORBIT_BUILT_WORKER")),
        &["--threads".into(), "1".into()],
    )
    .unwrap();
    let bounds = bend.bounds();
    bend.worker.kill().unwrap();
    bend.worker.wait().unwrap();
    bounds.unwrap().output_bytes as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::Foreground;

    /// The deadline judges only an incomplete answer: one the worker already
    /// wrote, or whose lines were already read, is accepted after it, as when
    /// the host itself was paused; a worker that keeps writing decisions
    /// without its end line is cut off at it.
    #[test]
    fn the_decision_deadline_cuts_off_only_an_incomplete_answer() {
        let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
        let worker = |script: &str| {
            Bend::spawn(Path::new("/bin/sh"), &["-c".into(), script.into()]).unwrap()
        };
        let mut written = worker("printf '9\\nend\\n' >&4");
        assert!(written.worker.wait().unwrap().success());
        let passed = Instant::now();
        assert_eq!(written.decisions(passed).unwrap(), [Decision::Accepted]);
        written.unread.extend_from_slice(b"9\nend\n");
        assert_eq!(written.decisions(passed).unwrap(), [Decision::Accepted]);

        // yes writes faster than the host reads, so the pipe never runs dry.
        let mut streaming = worker("exec yes 9 >&4");
        let asked = Instant::now();
        let error = streaming
            .decisions(asked + Duration::from_millis(20))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(
            asked.elapsed() < Duration::from_secs(1),
            "{:?}",
            asked.elapsed()
        );
        streaming.worker.kill().unwrap();
        streaming.worker.wait().unwrap();
    }

    /// The built worker leads each PTY turn's answer with the program status
    /// lifetime of ORB-C15 and carries its tracker from turn to turn.
    #[test]
    fn the_worker_leads_each_output_with_its_status_lifetime() {
        fn lifetime(bend: &mut Bend, foreground: Foreground, marks: bool) -> (bool, bool) {
            let decisions = bend
                .ask(Fact::Output { foreground, marks }, Flags::default())
                .unwrap();
            let lifetimes: Vec<_> = decisions
                .iter()
                .filter_map(|decision| match *decision {
                    Decision::StatusLifetime { edge, marks } => Some((edge, marks)),
                    _ => None,
                })
                .collect();
            assert!(
                matches!(decisions[0], Decision::StatusLifetime { .. }) && lifetimes.len() == 1,
                "{decisions:?}"
            );
            lifetimes[0]
        }
        let _guard = crate::test_support::FORK_AND_SOCKET_TEST.lock().unwrap();
        let mut bend = Bend::spawn(
            Path::new(env!("ORBIT_BUILT_WORKER")),
            &["--threads".into(), "1".into()],
        )
        .unwrap();
        bend.bounds().unwrap();
        bend.ask(Fact::Started { pid: 1 }, Flags::default())
            .unwrap();
        // The child leads its foreground at the start, so its own group drops
        // nothing until another group has led.
        assert_eq!(
            lifetime(&mut bend, Foreground::Child, false),
            (false, false)
        );
        assert_eq!(
            lifetime(&mut bend, Foreground::Other, false),
            (false, false)
        );
        // A sample that finds no group keeps the last one.
        assert_eq!(
            lifetime(&mut bend, Foreground::Unknown, false),
            (false, false)
        );
        assert_eq!(lifetime(&mut bend, Foreground::Child, false), (true, false));
        assert_eq!(
            lifetime(&mut bend, Foreground::Child, false),
            (false, false)
        );
        // A prompt mark drops, and from then on a return does not.
        assert_eq!(lifetime(&mut bend, Foreground::Other, true), (false, true));
        assert_eq!(
            lifetime(&mut bend, Foreground::Child, false),
            (false, false)
        );
        bend.worker.kill().unwrap();
        bend.worker.wait().unwrap();
    }

    #[test]
    fn waits_rotate_among_ready_sources() {
        let ready = [platform::POLLIN, 0, platform::POLLIN, platform::POLLHUP];
        assert_eq!(next_ready(&ready, 0), Some(0));
        assert_eq!(next_ready(&ready, 1), Some(2));
        assert_eq!(next_ready(&ready, 3), Some(3));
        assert_eq!(next_ready(&ready, 4), Some(0));
        assert_eq!(next_ready(&[0, 0], 1), None);
        // A source that stays ready waits for every other ready source.
        let mut turn = 0;
        let served: Vec<_> = (0..6)
            .map(|_| {
                let index = next_ready(&ready, turn).unwrap();
                turn = index + 1;
                index
            })
            .collect();
        assert_eq!(served, [0, 2, 3, 0, 2, 3]);
    }

    #[test]
    fn a_peer_deadline_expires_once() {
        let mut timer = Timer::default();
        assert!(!timer.expired());
        timer.set(Some(1));
        assert!(timer.deadline().is_some());
        thread::sleep(Duration::from_millis(5));
        assert!(timer.expired());
        assert!(!timer.expired());
        assert_eq!(timer.deadline(), None);
    }
}
