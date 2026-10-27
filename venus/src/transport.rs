use eon_workspace_protocol::v21::{
    self as workspace, Action as WorkspaceAction, Request as WorkspaceRequest,
    Response as WorkspaceResponse,
};
use orbit_protocol::session::{self, ClientMessage, ScrollOutcome, ServerMessage, WheelOutcome};
use std::{
    collections::VecDeque,
    fmt,
    io::{self, ErrorKind, Read, Write},
    net::Shutdown,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, SystemTime},
};

/// Events delivered from the local-session worker to the native event loop.
#[derive(Clone, Debug, PartialEq)]
pub enum TransportEvent {
    Server(ServerMessage),
    Incompatible { version: u16 },
    InvalidInput(String),
    RetryableLoss(String),
    Lost(String),
}

/// A semantic event could not enter the bounded transport queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    Full,
    Closed,
}

impl fmt::Display for SendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => formatter.write_str("Venus input queue is full"),
            Self::Closed => formatter.write_str("Venus input channel is closed"),
        }
    }
}

impl std::error::Error for SendError {}

/// Bounded typed input handle for one transient Orbit attachment.
pub struct Transport {
    messages: mpsc::SyncSender<ClientMessage>,
    events: Arc<EventQueue>,
}

/// One complete result from the Eon-owned workspace request boundary.
// The queue holds at most 128 events; snapshot payloads are already heap-owned.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceEvent {
    Response(WorkspaceResponse),
    Unavailable(String),
    /// Follows the event that answered the action `send` numbered `ticket`; `accepted` when that
    /// answer was a snapshot.
    Answered {
        ticket: u64,
        accepted: bool,
    },
}

/// Bounded semantic-action handle for EONW v21.
pub struct WorkspaceTransport {
    actions: mpsc::SyncSender<(WorkspaceAction, u64)>,
    events: Arc<WorkspaceEventQueue>,
    /// The next action's ticket.
    tickets: AtomicU64,
}

const WORKSPACE_REFRESH_INTERVAL: Duration = Duration::from_millis(250);

impl WorkspaceTransport {
    #[must_use]
    pub fn start(socket: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::start_with_refresh(socket, WORKSPACE_REFRESH_INTERVAL, wake)
    }

    fn start_with_refresh(
        socket: PathBuf,
        refresh_interval: Duration,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (actions, receiver) = mpsc::sync_channel(32);
        let events = Arc::new(WorkspaceEventQueue::default());
        let notify = workspace_notifier(Arc::clone(&events), wake);
        let worker_notify = Arc::clone(&notify);
        if let Err(error) = thread::Builder::new()
            .name("venus-eon-workspace".into())
            .spawn(move || run_workspace(socket, receiver, refresh_interval, worker_notify))
        {
            notify(WorkspaceEvent::Unavailable(format!(
                "Cannot start the Eon workspace worker: {error}"
            )));
        }
        Self {
            actions,
            events,
            tickets: AtomicU64::new(0),
        }
    }

    /// Queues `action` and returns the ticket its `WorkspaceEvent::Answered` carries.
    pub fn send(&self, action: WorkspaceAction) -> Result<u64, SendError> {
        let ticket = self.tickets.fetch_add(1, Ordering::Relaxed);
        self.actions
            .try_send((action, ticket))
            .map(|()| ticket)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => SendError::Full,
                mpsc::TrySendError::Disconnected(_) => SendError::Closed,
            })
    }

    pub fn drain_events(&self) -> Vec<WorkspaceEvent> {
        self.events.drain()
    }
}

#[derive(Default)]
struct WorkspaceEventQueue(Mutex<VecDeque<WorkspaceEvent>>);

impl WorkspaceEventQueue {
    fn push(&self, event: WorkspaceEvent) -> bool {
        // A full action channel (32 queued and one in flight) answers with two events each, a
        // response and its `Answered`, and still leaves room for refreshes.
        const CAPACITY: usize = 128;
        let mut events = self.0.lock().expect("workspace event queue lock poisoned");
        let wake = events.is_empty();
        if events.len() == CAPACITY {
            events.clear();
            events.push_back(WorkspaceEvent::Unavailable(
                "Eon workspace event queue exceeded its bounded capacity".into(),
            ));
        } else {
            events.push_back(event);
        }
        wake
    }

    fn drain(&self) -> Vec<WorkspaceEvent> {
        self.0
            .lock()
            .expect("workspace event queue lock poisoned")
            .drain(..)
            .collect()
    }
}

fn workspace_notifier(
    events: Arc<WorkspaceEventQueue>,
    wake: impl Fn() + Send + Sync + 'static,
) -> Arc<dyn Fn(WorkspaceEvent) + Send + Sync> {
    Arc::new(move |event| {
        if events.push(event) {
            wake();
        }
    })
}

fn run_workspace(
    socket: PathBuf,
    receiver: mpsc::Receiver<(WorkspaceAction, u64)>,
    refresh_interval: Duration,
    notify: Arc<dyn Fn(WorkspaceEvent) + Send + Sync>,
) {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut counter = 0;
    // Refreshes carry no ticket.
    let mut action = WorkspaceAction::Workspace(workspace::WorkspaceAction::Inspect);
    let mut ticket = None;
    loop {
        let event = workspace_exchange(&socket, nonce, counter, action)
            .map(WorkspaceEvent::Response)
            .unwrap_or_else(WorkspaceEvent::Unavailable);
        let accepted = matches!(
            event,
            WorkspaceEvent::Response(WorkspaceResponse::Snapshot(_))
        );
        notify(event);
        if let Some(ticket) = ticket {
            notify(WorkspaceEvent::Answered { ticket, accepted });
        }
        counter = counter.wrapping_add(1);
        (action, ticket) = match receiver.recv_timeout(refresh_interval) {
            Ok((action, ticket)) => (action, Some(ticket)),
            Err(mpsc::RecvTimeoutError::Timeout) => (
                WorkspaceAction::Workspace(workspace::WorkspaceAction::Inspect),
                None,
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
    }
}

/// Eon answers an action that starts a terminal only after Orbit is Ready, within its shared
/// five-second start deadline. Wait that deadline plus one second, so one slow but successful
/// terminal start does not read as workspace loss; an action queued behind other slow work, or one
/// that stops a terminal before starting another, can still exceed it.
const WORKSPACE_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(6);

fn workspace_exchange(
    socket: &Path,
    nonce: u128,
    counter: usize,
    action: WorkspaceAction,
) -> Result<WorkspaceResponse, String> {
    let request = WorkspaceRequest {
        id: format!("venus-{}-{nonce}-{counter}", std::process::id()),
        caller: None,
        action,
    };
    let encoded = workspace::encode_request(&request)
        .map_err(|error| format!("Cannot encode Eon workspace action: {error}"))?;
    let mut stream = UnixStream::connect(socket).map_err(|error| {
        format!(
            "Cannot connect to Eon workspace at {}: {error}",
            socket.display()
        )
    })?;
    let timeout = Some(WORKSPACE_EXCHANGE_TIMEOUT);
    stream
        .set_read_timeout(timeout)
        .and_then(|()| stream.set_write_timeout(timeout))
        .map_err(|error| format!("Cannot bound Eon workspace I/O: {error}"))?;
    stream
        .write_all(&encoded)
        .map_err(|error| format!("Cannot send Eon workspace action: {error}"))?;

    read_workspace_response(&mut stream)
}

/// Read one bounded canonical response, leaving subsequent control bytes unread.
pub fn read_workspace_response(stream: &mut impl Read) -> Result<WorkspaceResponse, String> {
    let mut response = vec![0; workspace::HEADER_BYTES];
    stream
        .read_exact(&mut response)
        .map_err(|error| format!("Cannot read Eon workspace response header: {error}"))?;
    let length = workspace::declared_message_len(&response)
        .map_err(|error| format!("Eon sent an invalid workspace response: {error}"))?;
    response.resize(length, 0);
    stream
        .read_exact(&mut response[workspace::HEADER_BYTES..])
        .map_err(|error| format!("Cannot read complete Eon workspace response: {error}"))?;
    workspace::decode_response(&response)
        .map_err(|error| format!("Eon sent an invalid workspace response: {error}"))
}

impl Transport {
    /// Connect on a worker thread, queue decoded server messages, and wake the event loop.
    #[must_use]
    pub fn start(socket: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (messages, receiver) = mpsc::sync_channel(256);
        let events = Arc::new(EventQueue::default());
        let notify = notifier(Arc::clone(&events), wake);
        if let Err(error) = thread::Builder::new()
            .name("venus-orbit-reader".into())
            .spawn({
                let notify = Arc::clone(&notify);
                move || run(socket, receiver, notify)
            })
        {
            notify(
                TransportEvent::Lost(format!("Cannot start the Orbit transport worker: {error}")),
                0,
            );
        }
        Self { messages, events }
    }

    /// Queue one canonical Orbit message without blocking the native event loop.
    pub fn send(&self, message: ClientMessage) -> Result<(), SendError> {
        self.messages
            .try_send(message)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => SendError::Full,
                mpsc::TrySendError::Disconnected(_) => SendError::Closed,
            })
    }

    /// Drain ordered server events, with consecutive complete frames reduced to the latest one.
    pub fn drain_events(&self) -> Vec<TransportEvent> {
        self.events.drain()
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        self.events.close();
    }
}

/// Server events awaiting the native event loop. A push that would exceed a
/// bound waits for the next drain, so the reader stops reading and Orbit's
/// bounded slow-reader handling keeps its newest frame.
#[derive(Default)]
struct EventQueue(Mutex<QueuedEvents>, Condvar);

#[derive(Default)]
struct QueuedEvents {
    events: VecDeque<QueuedEvent>,
    retained_bytes: usize,
    delivered_frame_high_water: Option<u64>,
    stopped: bool,
}

struct QueuedEvent {
    event: TransportEvent,
    framed_bytes: usize,
}

const EVENT_QUEUE_CAPACITY: usize = 256;
const FRAME_QUEUE_CAPACITY: usize = 2;
const EVENT_QUEUE_BYTE_CAPACITY: usize = 3 * (session::MAX_PAYLOAD_BYTES + session::HEADER_BYTES);

impl EventQueue {
    #[cfg(test)]
    fn push(&self, event: TransportEvent) -> bool {
        self.push_framed(event, 0)
    }

    fn push_framed(&self, event: TransportEvent, framed_bytes: usize) -> bool {
        self.push_with_byte_capacity(event, framed_bytes, EVENT_QUEUE_BYTE_CAPACITY)
    }

    fn push_with_byte_capacity(
        &self,
        event: TransportEvent,
        framed_bytes: usize,
        byte_capacity: usize,
    ) -> bool {
        let incoming_revision = frame_revision(&event);
        let mut state = self.0.lock().expect("transport event queue lock poisoned");
        let (replaced, retained_bytes) = loop {
            if state.stopped {
                return false;
            }
            let mut replaced = None;
            if let Some(revision) = incoming_revision {
                for (index, queued) in state.events.iter().enumerate().rev() {
                    match &queued.event {
                        TransportEvent::Server(
                            ServerMessage::Accepted
                            | ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted),
                        ) => {}
                        TransportEvent::Server(ServerMessage::ScrollOutcome(_)) => break,
                        queued => {
                            let Some(queued_revision) = frame_revision(queued) else {
                                break;
                            };
                            let previous_revision = state
                                .events
                                .iter()
                                .take(index)
                                .filter_map(|queued| frame_revision(&queued.event))
                                .chain(state.delivered_frame_high_water)
                                .max();
                            if revision > queued_revision
                                && previous_revision
                                    .is_none_or(|previous| queued_revision > previous)
                            {
                                replaced = Some(index);
                            }
                            break;
                        }
                    }
                }
            }
            let replaced_bytes = replaced.map_or(0, |index| state.events[index].framed_bytes);
            let retained_bytes =
                (state.retained_bytes - replaced_bytes).saturating_add(framed_bytes);
            let full = retained_bytes > byte_capacity
                || (replaced.is_none()
                    && (state.events.len() == EVENT_QUEUE_CAPACITY
                        || (incoming_revision.is_some()
                            && state
                                .events
                                .iter()
                                .filter_map(|queued| frame_revision(&queued.event))
                                .count()
                                == FRAME_QUEUE_CAPACITY)));
            if !full {
                break (replaced, retained_bytes);
            }
            state = self
                .1
                .wait(state)
                .expect("transport event queue lock poisoned");
        };
        let wake = state.events.is_empty();
        if let Some(index) = replaced {
            state.events.remove(index);
        }
        state.stopped = matches!(
            &event,
            TransportEvent::RetryableLoss(_)
                | TransportEvent::Lost(_)
                | TransportEvent::Incompatible { .. }
                | TransportEvent::Server(ServerMessage::Busy | ServerMessage::Exited { .. })
        );
        state.retained_bytes = retained_bytes;
        state.events.push_back(QueuedEvent {
            event,
            framed_bytes,
        });
        wake
    }

    fn drain(&self) -> Vec<TransportEvent> {
        let mut state = self.0.lock().expect("transport event queue lock poisoned");
        state.delivered_frame_high_water = state
            .events
            .iter()
            .filter_map(|queued| frame_revision(&queued.event))
            .chain(state.delivered_frame_high_water)
            .max();
        state.retained_bytes = 0;
        let events = state.events.drain(..).map(|queued| queued.event).collect();
        self.1.notify_all();
        events
    }

    /// Releases a waiting push once the client has gone.
    fn close(&self) {
        self.0
            .lock()
            .expect("transport event queue lock poisoned")
            .stopped = true;
        self.1.notify_all();
    }
}

fn frame_revision(event: &TransportEvent) -> Option<u64> {
    match event {
        TransportEvent::Server(
            ServerMessage::Frame(frame)
            | ServerMessage::WheelOutcome(WheelOutcome::Viewport { frame, .. })
            | ServerMessage::ScrollOutcome(ScrollOutcome::Viewport { frame, .. }),
        ) => Some(frame.revision),
        _ => None,
    }
}

fn notifier(
    events: Arc<EventQueue>,
    wake: impl Fn() + Send + Sync + 'static,
) -> Arc<dyn Fn(TransportEvent, usize) + Send + Sync> {
    Arc::new(move |event, framed_bytes| {
        if events.push_framed(event, framed_bytes) {
            wake();
        }
    })
}

fn run(
    socket: PathBuf,
    receiver: mpsc::Receiver<ClientMessage>,
    notify: Arc<dyn Fn(TransportEvent, usize) + Send + Sync>,
) {
    let mut stream = match UnixStream::connect(&socket) {
        Ok(stream) => stream,
        Err(error) => {
            let kind = error.kind();
            notify(
                socket_loss(
                    format!("Cannot connect to Orbit at {}: {error}", socket.display()),
                    kind,
                ),
                0,
            );
            return;
        }
    };
    let writer = match stream.try_clone() {
        Ok(writer) => writer,
        Err(error) => {
            notify(
                TransportEvent::Lost(format!("Cannot open the Orbit input channel: {error}")),
                0,
            );
            return;
        }
    };
    if let Err(error) = write_message(&mut stream, &ClientMessage::Hello) {
        let kind = error.kind();
        notify(
            socket_loss(format!("Cannot start the Orbit attachment: {error}"), kind),
            0,
        );
        return;
    }

    let writer_notify = Arc::clone(&notify);
    if let Err(error) = thread::Builder::new()
        .name("venus-orbit-writer".into())
        .spawn(move || write_loop(writer, receiver, writer_notify))
    {
        let _ = stream.shutdown(Shutdown::Both);
        notify(
            TransportEvent::Lost(format!("Cannot start the Orbit input worker: {error}")),
            0,
        );
        return;
    }

    loop {
        match read_message(&mut stream) {
            Ok(Some((message, framed_bytes))) => {
                notify(TransportEvent::Server(message), framed_bytes)
            }
            Ok(None) => {
                notify(
                    TransportEvent::RetryableLoss("Orbit closed the local session".into()),
                    0,
                );
                return;
            }
            Err(event) => {
                notify(event, 0);
                return;
            }
        }
    }
}

fn write_loop(
    mut stream: UnixStream,
    receiver: mpsc::Receiver<ClientMessage>,
    notify: Arc<dyn Fn(TransportEvent, usize) + Send + Sync>,
) {
    while let Ok(message) = receiver.recv() {
        if let Err(error) = write_message(&mut stream, &message) {
            if error.kind() != ErrorKind::InvalidInput {
                // The reader owns peer completion so buffered terminal output wins.
                let _ = stream.shutdown(Shutdown::Write);
                return;
            }
            notify(TransportEvent::InvalidInput(error.to_string()), 0);
        }
    }
    let _ = stream.shutdown(Shutdown::Both);
}

fn write_message(stream: &mut UnixStream, message: &ClientMessage) -> io::Result<()> {
    let encoded = session::encode_client_message(message)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    stream.write_all(&encoded)
}

fn read_message(stream: &mut impl Read) -> Result<Option<(ServerMessage, usize)>, TransportEvent> {
    let mut header = [0; session::HEADER_BYTES];
    match stream.read_exact(&mut header[..1]) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(io_loss(error)),
    }
    stream.read_exact(&mut header[1..]).map_err(io_loss)?;
    let length = session::server_message_len(&header)
        .map_err(protocol_loss)?
        .expect("a complete ORBS header declares a message length");
    let mut bytes = Vec::with_capacity(length);
    bytes.extend_from_slice(&header);
    bytes.resize(length, 0);
    stream
        .read_exact(&mut bytes[session::HEADER_BYTES..])
        .map_err(io_loss)?;
    session::decode_server_message(&bytes)
        .map(|message| Some((message, length)))
        .map_err(protocol_loss)
}

fn io_loss(error: io::Error) -> TransportEvent {
    let kind = error.kind();
    socket_loss(format!("Cannot read from Orbit: {error}"), kind)
}

fn socket_loss(detail: String, kind: ErrorKind) -> TransportEvent {
    if matches!(
        kind,
        ErrorKind::NotFound
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::NotConnected
            | ErrorKind::BrokenPipe
            | ErrorKind::TimedOut
            | ErrorKind::Interrupted
            | ErrorKind::UnexpectedEof
            | ErrorKind::WriteZero
    ) {
        TransportEvent::RetryableLoss(detail)
    } else {
        TransportEvent::Lost(detail)
    }
}

fn protocol_loss(error: session::Error) -> TransportEvent {
    match error {
        session::Error::UnsupportedVersion { version } => TransportEvent::Incompatible { version },
        error => TransportEvent::Lost(format!(
            "Orbit sent an invalid local-session message: {error}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eon_workspace_protocol::v21::{
        InvokeIntent, Pane, Popup, PopupEntry, PopupKey, Shortcut, Snapshot, Tab,
    };
    use orbit_protocol::{
        Capabilities, Colors, Cursor, CursorShape, Dimensions, Frame, Rgb, Screen,
        session::FocusEvent,
    };

    use std::{
        fs,
        os::unix::net::UnixListener,
        time::{Duration, SystemTime},
    };

    #[test]
    fn local_failures_name_their_owner() {
        assert_eq!(SendError::Full.to_string(), "Venus input queue is full");
        assert_eq!(
            SendError::Closed.to_string(),
            "Venus input channel is closed"
        );
        let error = io::Error::new(ErrorKind::ConnectionReset, "socket reset");
        assert_eq!(
            io_loss(error),
            TransportEvent::RetryableLoss("Cannot read from Orbit: socket reset".into())
        );
        assert!(matches!(
            protocol_loss(session::Error::InvalidMagic),
            TransportEvent::Lost(_)
        ));
        assert_eq!(
            protocol_loss(session::Error::UnsupportedVersion { version: 3 }),
            TransportEvent::Incompatible { version: 3 }
        );
        assert!(matches!(
            socket_loss("missing".into(), ErrorKind::NotFound),
            TransportEvent::RetryableLoss(_)
        ));
        assert!(matches!(
            socket_loss("resource".into(), ErrorKind::OutOfMemory),
            TransportEvent::Lost(_)
        ));
    }

    #[test]
    fn interrupted_first_read_retries_the_message() {
        struct InterruptedOnce<R>(R, bool);

        impl<R: Read> Read for InterruptedOnce<R> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if !self.1 {
                    self.1 = true;
                    return Err(ErrorKind::Interrupted.into());
                }
                self.0.read(buffer)
            }
        }

        let encoded = session::encode_server_message(&ServerMessage::Accepted).unwrap();
        let mut reader = InterruptedOnce(encoded.as_slice(), false);
        assert_eq!(
            read_message(&mut reader),
            Ok(Some((ServerMessage::Accepted, encoded.len())))
        );
    }

    #[test]
    fn first_terminal_loss_is_the_final_transport_event() {
        let events = Arc::new(EventQueue::default());
        let queued = Arc::clone(&events);
        let (wakes, receiver) = mpsc::channel();
        let notify = notifier(queued, move || wakes.send(()).unwrap());

        notify(TransportEvent::Lost("writer failed".into()), 0);
        notify(TransportEvent::Lost("socket closed".into()), 0);
        notify(TransportEvent::InvalidInput("late input error".into()), 0);
        notify(TransportEvent::Server(ServerMessage::Accepted), 0);

        receiver.recv().unwrap();
        assert!(receiver.try_recv().is_err());
        assert_eq!(
            events.drain(),
            [TransportEvent::Lost("writer failed".into())]
        );

        let events = EventQueue::default();
        assert!(events.push_framed(
            TransportEvent::Server(ServerMessage::Busy),
            session::HEADER_BYTES,
        ));
        assert!(!events.push(TransportEvent::RetryableLoss("socket closed".into())));
        assert_eq!(
            events.0.lock().unwrap().retained_bytes,
            session::HEADER_BYTES
        );
        assert_eq!(
            events.drain(),
            [TransportEvent::Server(ServerMessage::Busy)]
        );
        assert_eq!(events.0.lock().unwrap().retained_bytes, 0);

        let events = EventQueue::default();
        assert!(events.push(TransportEvent::Incompatible { version: 3 }));
        assert!(!events.push(TransportEvent::Server(ServerMessage::Accepted)));
        assert_eq!(
            events.drain(),
            [TransportEvent::Incompatible { version: 3 }]
        );
    }

    #[test]
    fn writer_failure_preserves_buffered_authoritative_completion() {
        let (mut reader, mut orbit) = UnixStream::pair().unwrap();
        orbit
            .write_all(
                &session::encode_server_message(&ServerMessage::Exited { code: 17 }).unwrap(),
            )
            .unwrap();
        orbit.shutdown(Shutdown::Both).unwrap();

        let writer = reader.try_clone().unwrap();
        let (messages, receiver) = mpsc::channel();
        messages
            .send(ClientMessage::Focus(FocusEvent::Gained))
            .unwrap();
        drop(messages);
        let events = Arc::new(EventQueue::default());
        write_loop(writer, receiver, notifier(Arc::clone(&events), || {}));

        assert!(events.drain().is_empty());
        assert_eq!(
            read_message(&mut reader),
            Ok(Some((
                ServerMessage::Exited { code: 17 },
                session::HEADER_BYTES + 4,
            )))
        );
    }

    #[test]
    fn frame_reduction_preserves_message_and_revision_order() {
        let events = EventQueue::default();
        assert!(events.push(server_frame(1)));
        assert!(!events.push(TransportEvent::Server(ServerMessage::Accepted)));
        assert!(!events.push(server_wheel_frame(2)));
        assert!(!events.push(TransportEvent::InvalidInput("input".into())));
        assert!(!events.push(server_frame(3)));
        assert!(!events.push(server_frame(4)));

        assert_eq!(
            events.drain(),
            [
                TransportEvent::Server(ServerMessage::Accepted),
                server_wheel_frame(2),
                TransportEvent::InvalidInput("input".into()),
                server_frame(4),
            ]
        );

        let retirement = TransportEvent::Server(ServerMessage::ResourceRetire { resource: 1 });
        assert!(events.push(server_frame(5)));
        assert!(!events.push(retirement.clone()));
        assert!(!events.push(server_frame(6)));
        assert_eq!(
            events.drain(),
            [server_frame(5), retirement, server_frame(6)]
        );

        assert!(events.push(server_frame(9)));
        assert!(!events.push(server_scroll_frame(10)));
        assert_eq!(events.drain(), [server_scroll_frame(10)]);

        assert!(events.push(server_scroll_frame(11)));
        assert!(!events.push(server_frame(12)));
        assert_eq!(events.drain(), [server_scroll_frame(11), server_frame(12)]);

        assert!(events.push(server_frame(3)));
        assert!(!events.push(server_frame(5)));
        assert_eq!(events.drain(), [server_frame(3), server_frame(5)]);

        assert!(events.push(server_frame(6)));
        assert!(!events.push(server_frame(5)));
        assert_eq!(events.drain(), [server_frame(6), server_frame(5)]);

        assert!(events.push(server_frame(13)));
        assert!(
            !events.push(TransportEvent::Server(ServerMessage::WheelOutcome(
                WheelOutcome::TerminalRouted
            )))
        );
        assert!(!events.push(server_wheel_frame(14)));
        assert_eq!(
            events.drain(),
            [
                TransportEvent::Server(ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted)),
                server_wheel_frame(14),
            ]
        );
    }

    #[test]
    fn full_event_queue_holds_the_reader_until_drained_or_dropped() {
        const DEADLINE: Duration = Duration::from_secs(5);
        let events = Arc::new(EventQueue::default());
        let (wakes, woken) = mpsc::channel();
        let notify = notifier(Arc::clone(&events), move || wakes.send(()).unwrap());
        let (steps, step) = mpsc::channel();
        let (resume, resumed) = mpsc::channel();
        let reader = thread::spawn(move || {
            // During a scroll Orbit answers each frame's preview between frames.
            for event in [
                server_frame(1),
                server_preview(1),
                server_wheel_frame(2),
                server_preview(2),
            ] {
                notify(event, 0);
            }
            steps.send("held").unwrap();
            notify(server_frame(3), 0);
            steps.send("pushed").unwrap();

            for _ in 0..2 {
                resumed.recv().unwrap();
                notify(accepted(), EVENT_QUEUE_BYTE_CAPACITY);
                steps.send("held").unwrap();
                notify(accepted(), 1);
                steps.send("pushed").unwrap();
            }
        });
        let next = || step.recv_timeout(DEADLINE).unwrap();
        let held = || {
            assert_eq!(next(), "held");
            assert!(step.recv_timeout(Duration::from_millis(100)).is_err());
        };

        held();
        assert_eq!(
            events.drain(),
            [
                server_frame(1),
                server_preview(1),
                server_wheel_frame(2),
                server_preview(2),
            ]
        );
        assert_eq!(next(), "pushed");
        woken.recv_timeout(DEADLINE).unwrap();
        // The released frame wakes the event loop again.
        woken.recv_timeout(DEADLINE).unwrap();
        assert_eq!(events.drain(), [server_frame(3)]);
        resume.send(()).unwrap();

        held();
        assert_eq!(
            events.0.lock().unwrap().retained_bytes,
            EVENT_QUEUE_BYTE_CAPACITY
        );
        assert_eq!(events.drain(), [accepted()]);
        assert_eq!(next(), "pushed");
        assert_eq!(events.drain(), [accepted()]);
        resume.send(()).unwrap();

        held();
        drop(Transport {
            messages: mpsc::sync_channel(1).0,
            events: Arc::clone(&events),
        });
        assert_eq!(next(), "pushed");
        assert_eq!(events.drain(), [accepted()]);
        reader.join().unwrap();
    }

    #[test]
    fn event_queue_bytes_are_bounded_replaced_and_reset() {
        const BYTE_CAPACITY: usize = 10;
        let events = EventQueue::default();
        assert!(events.push_with_byte_capacity(server_frame(1), 6, BYTE_CAPACITY));
        assert!(!events.push_with_byte_capacity(
            TransportEvent::Server(ServerMessage::Accepted),
            1,
            BYTE_CAPACITY,
        ));
        assert!(!events.push_with_byte_capacity(server_frame(2), 9, BYTE_CAPACITY));
        assert_eq!(events.0.lock().unwrap().retained_bytes, BYTE_CAPACITY);
        assert_eq!(
            events.drain(),
            [
                TransportEvent::Server(ServerMessage::Accepted),
                server_frame(2),
            ]
        );
        assert_eq!(events.0.lock().unwrap().retained_bytes, 0);
    }

    #[test]
    fn typed_attachment_is_duplex_and_client_drop_leaves_server_listener() {
        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let path = socket.path.clone();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            assert_eq!(read_client(&mut stream), ClientMessage::Hello);
            stream
                .write_all(&session::encode_server_message(&ServerMessage::Attached).unwrap())
                .unwrap();
            assert_eq!(
                read_client(&mut stream),
                ClientMessage::Focus(FocusEvent::Gained)
            );
            let mut eof = [0];
            assert_eq!(stream.read(&mut eof).unwrap(), 0);

            let (mut reopened, _) = listener.accept().unwrap();
            reopened
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            read_client(&mut reopened)
        });

        let (wakes, receiver) = mpsc::channel();
        let transport = Transport::start(path.clone(), move || {
            let _ = wakes.send(());
        });
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            transport.drain_events(),
            [TransportEvent::Server(ServerMessage::Attached)]
        );
        transport
            .send(ClientMessage::Focus(FocusEvent::Gained))
            .unwrap();
        drop(transport);

        let mut reopened = UnixStream::connect(path).unwrap();
        write_message(&mut reopened, &ClientMessage::Hello).unwrap();
        assert_eq!(server.join().unwrap(), ClientMessage::Hello);
    }

    #[test]
    fn missing_and_dropped_sockets_can_attach_on_later_attempts() {
        let socket = TestSocket::new();
        let (first_wake, first_events) = mpsc::channel();
        let first = Transport::start(socket.path.clone(), move || {
            let _ = first_wake.send(());
        });
        first_events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            first.drain_events().as_slice(),
            [TransportEvent::RetryableLoss(detail)] if detail.contains("Cannot connect to Orbit")
        ));
        drop(first);

        let listener = UnixListener::bind(&socket.path).unwrap();
        let (release, releases) = mpsc::channel();
        let server = thread::spawn(move || {
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                assert_eq!(read_client(&mut stream), ClientMessage::Hello);
                stream
                    .write_all(&session::encode_server_message(&ServerMessage::Attached).unwrap())
                    .unwrap();
                releases.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        });

        let (second_wake, second_events) = mpsc::channel();
        let second = Transport::start(socket.path.clone(), move || {
            let _ = second_wake.send(());
        });
        second_events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            second.drain_events().as_slice(),
            [TransportEvent::Server(ServerMessage::Attached)]
        ));
        release.send(()).unwrap();
        second_events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            second.drain_events().as_slice(),
            [TransportEvent::RetryableLoss(detail)] if detail == "Orbit closed the local session"
        ));
        drop(second);

        let (third_wake, third_events) = mpsc::channel();
        let third = Transport::start(socket.path.clone(), move || {
            let _ = third_wake.send(());
        });
        third_events.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            third.drain_events().as_slice(),
            [TransportEvent::Server(ServerMessage::Attached)]
        ));
        release.send(()).unwrap();
        drop(third);
        server.join().unwrap();
    }

    #[test]
    fn invalid_server_header_becomes_one_bounded_loss_event() {
        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_client(&mut stream);
            stream.write_all(&[0; session::HEADER_BYTES]).unwrap();
        });
        let (wakes, receiver) = mpsc::channel();
        let transport = Transport::start(socket.path.clone(), move || {
            let _ = wakes.send(());
        });
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let event = transport.drain_events().pop().unwrap();
        let expected =
            "Orbit sent an invalid local-session message: invalid local-session message magic";
        assert_eq!(event, TransportEvent::Lost(expected.into()));
        server.join().unwrap();
    }

    #[test]
    fn workspace_worker_carries_actions_and_popup_transition() {
        use WorkspaceAction::Workspace;
        use workspace::Direction::{Down, Left, Right, Up};
        use workspace::WorkspaceAction::{CloseTab, Focus, FocusId, Inspect, Move};

        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let snapshot = WorkspaceResponse::Snapshot(workspace_snapshot());
        let mut popup = workspace_snapshot();
        popup.tabs.push(second_workspace_tab());
        popup.workspaces[0].tab_count = popup.tabs.len();
        popup.tabs[0].selected_popup = Some("u1".into());
        let mut inactive_popup = popup.clone();
        inactive_popup.active_tab = "t2".into();
        let exchanges = [
            (Workspace(Inspect), snapshot.clone()),
            (Workspace(FocusId("pane-1".into())), snapshot.clone()),
            (Workspace(Focus(Up)), snapshot.clone()),
            (Workspace(Focus(Down)), snapshot),
            (
                Workspace(Move(Left)),
                WorkspaceResponse::Snapshot(workspace_snapshot()),
            ),
            (
                Workspace(Move(Right)),
                WorkspaceResponse::Snapshot(workspace_snapshot()),
            ),
            (
                Workspace(Move(Up)),
                WorkspaceResponse::Snapshot(workspace_snapshot()),
            ),
            (
                Workspace(Move(Down)),
                WorkspaceResponse::Snapshot(workspace_snapshot()),
            ),
            (
                Workspace(CloseTab { tab: "t2".into() }),
                WorkspaceResponse::Snapshot(workspace_snapshot()),
            ),
            (
                WorkspaceAction::InvokePopup {
                    tab: "t1".into(),
                    entry: "agent".into(),
                    expected_instance: Some("u1".into()),
                    intent: InvokeIntent::Toggle,
                },
                WorkspaceResponse::Snapshot(popup.clone()),
            ),
            (
                Workspace(Focus(Right)),
                WorkspaceResponse::Snapshot(inactive_popup),
            ),
            (Workspace(Focus(Left)), WorkspaceResponse::Snapshot(popup)),
        ];
        let server_exchanges = exchanges.clone();
        let server = thread::spawn(move || {
            for (expected_action, response) in server_exchanges {
                let (mut stream, action) = accept_workspace_action(&listener);
                assert_eq!(action, expected_action);
                stream
                    .write_all(&workspace::encode_response(&response).unwrap())
                    .unwrap();
            }
        });
        let (wakes, receiver) = mpsc::channel();
        let transport = WorkspaceTransport::start_with_refresh(
            socket.path.clone(),
            Duration::from_secs(5),
            move || {
                let _ = wakes.send(());
            },
        );

        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        let mut exchanges = exchanges.into_iter();
        let (_, expected) = exchanges.next().unwrap();
        assert_eq!(
            transport.drain_events(),
            [WorkspaceEvent::Response(expected)]
        );
        // Each action's answer is followed by its ticket; the refresh above carried none.
        for (ticket, (action, expected)) in (0..).zip(exchanges) {
            assert_eq!(transport.send(action).unwrap(), ticket);
            // A drain between the two pushes sees only the response; the answer then wakes
            // the queue again.
            let mut events = Vec::new();
            while events.len() < 2 {
                receiver.recv_timeout(Duration::from_secs(5)).unwrap();
                events.extend(transport.drain_events());
            }
            assert_eq!(
                events,
                [
                    WorkspaceEvent::Response(expected),
                    WorkspaceEvent::Answered {
                        ticket,
                        accepted: true,
                    },
                ]
            );
        }
        server.join().unwrap();
    }

    #[test]
    fn workspace_worker_refreshes_without_a_queued_action() {
        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let first = workspace_snapshot();
        let mut second = first.clone();
        second.active_tab = "t2".into();
        second.tabs.push(second_workspace_tab());
        second.workspaces[0].tab_count = second.tabs.len();
        let snapshots = [first, second];
        let expected = snapshots.clone();
        let server = thread::spawn(move || {
            for snapshot in snapshots {
                let (mut stream, action) = accept_workspace_action(&listener);
                assert_eq!(
                    action,
                    WorkspaceAction::Workspace(workspace::WorkspaceAction::Inspect)
                );
                stream
                    .write_all(
                        &workspace::encode_response(&WorkspaceResponse::Snapshot(snapshot))
                            .unwrap(),
                    )
                    .unwrap();
            }
        });
        let (wakes, receiver) = mpsc::channel();
        let transport = WorkspaceTransport::start_with_refresh(
            socket.path.clone(),
            Duration::from_millis(10),
            move || {
                let _ = wakes.send(());
            },
        );

        for snapshot in expected {
            receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(
                transport.drain_events(),
                [WorkspaceEvent::Response(WorkspaceResponse::Snapshot(
                    snapshot
                ))]
            );
        }
        drop(transport);
        server.join().unwrap();
    }

    #[test]
    fn startup_snapshot_leaves_following_presentation_control_unread() {
        let snapshot = WorkspaceResponse::Snapshot(workspace_snapshot());
        let mut bytes = workspace::encode_response(&snapshot).unwrap();
        bytes.extend_from_slice(b"present\n");
        let mut input = bytes.as_slice();
        assert_eq!(read_workspace_response(&mut input).unwrap(), snapshot);
        assert_eq!(input, b"present\n");
        assert!(read_workspace_response(&mut &bytes[..workspace::HEADER_BYTES]).is_err());
    }

    #[test]
    fn incompatible_workspace_response_is_rejected_before_model_state() {
        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; workspace::HEADER_BYTES];
            stream.read_exact(&mut request).unwrap();
            let mut response =
                workspace::encode_response(&WorkspaceResponse::Failure(workspace::Failure {
                    code: "unused".into(),
                    detail: "unused".into(),
                }))
                .unwrap();
            response[4..6].copy_from_slice(&(workspace::VERSION + 1).to_le_bytes());
            stream.write_all(&response).unwrap();
        });

        let error = workspace_exchange(
            &socket.path,
            1,
            0,
            WorkspaceAction::Workspace(workspace::WorkspaceAction::Inspect),
        )
        .unwrap_err();

        assert!(error.contains("unsupported EONW version"));
        server.join().unwrap();
    }

    #[test]
    fn slow_successful_workspace_action_is_not_reported_as_loss() {
        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let snapshot = WorkspaceResponse::Snapshot(workspace_snapshot());
        let response = workspace::encode_response(&snapshot).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = accept_workspace_action(&listener);
            // Slower than the old two-second bound, inside Eon's terminal-start deadline.
            thread::sleep(Duration::from_millis(2500));
            stream.write_all(&response).unwrap();
        });

        let result = workspace_exchange(
            &socket.path,
            1,
            0,
            WorkspaceAction::Workspace(workspace::WorkspaceAction::CreatePane { focus: true }),
        );

        assert_eq!(result, Ok(snapshot));
        server.join().unwrap();
    }

    fn read_client(stream: &mut UnixStream) -> ClientMessage {
        let mut header = [0; session::HEADER_BYTES];
        stream.read_exact(&mut header).unwrap();
        let length = session::client_message_len(&header).unwrap().unwrap();
        let mut bytes = Vec::from(header);
        bytes.resize(length, 0);
        stream
            .read_exact(&mut bytes[session::HEADER_BYTES..])
            .unwrap();
        session::decode_client_message(&bytes).unwrap()
    }

    fn accept_workspace_action(listener: &UnixListener) -> (UnixStream, WorkspaceAction) {
        let (mut stream, _) = listener.accept().unwrap();
        let mut header = [0; workspace::HEADER_BYTES];
        stream.read_exact(&mut header).unwrap();
        let length = workspace::declared_message_len(&header).unwrap();
        let mut request = Vec::from(header);
        request.resize(length, 0);
        stream
            .read_exact(&mut request[workspace::HEADER_BYTES..])
            .unwrap();
        (stream, workspace::decode_request(&request).unwrap().action)
    }

    fn workspace_snapshot() -> Snapshot {
        Snapshot {
            workspace: "w1".into(),
            active_workspace: "w1".into(),
            workspaces: vec![eon_workspace_protocol::v21::WorkspaceInfo {
                id: "w1".into(),
                name: None,
                tab_count: 1,
            }],
            active_tab: "t1".into(),
            tabs: vec![Tab {
                pending: false,
                selected_popup: None,
                popups: vec![Popup {
                    id: "u1".into(),
                    entry: "agent".into(),
                    session: "popup-session".into(),
                    endpoint: b"/run/agent.sock".to_vec(),
                    attention: None,
                }],
                id: "t1".into(),
                directory: b"/tmp/eon".to_vec(),
                selected_pane: Some("pane-1".into()),
                columns: vec![eon_workspace_protocol::v21::Column {
                    panes: vec![Pane {
                        label: "pane-1".into(),
                        pinned_name: None,
                        id: "pane-1".into(),
                        session: "session-1".into(),
                        endpoint: b"/run/eon/orbit.sock".to_vec(),
                        live: true,
                        attention: None,
                    }],
                    expanded: "pane-1".into(),
                }],
                attention_rollup: None,
            }],
            geometry: eon_workspace_protocol::v21::PopupGeometry {
                side_margin: 8.0,
                vertical_margin: 4.0,
            },
            entries: vec![PopupEntry {
                id: "agent".into(),
                label: "Agent".into(),
                shortcut: PopupKey::Bound(Shortcut {
                    modifiers: workspace::ALT | workspace::SHIFT,
                    key: "KeyL".into(),
                }),
            }],
            codex_quota: None,
            claude_quota: None,
            sidebar: eon_workspace_protocol::v21::Sidebar::Open,
        }
    }

    fn second_workspace_tab() -> Tab {
        Tab {
            pending: false,
            selected_popup: None,
            popups: Vec::new(),
            id: "t2".into(),
            directory: b"/tmp/nova".to_vec(),
            selected_pane: Some("pane-2".into()),
            columns: vec![eon_workspace_protocol::v21::Column {
                panes: vec![Pane {
                    label: "pane-2".into(),
                    pinned_name: None,
                    id: "pane-2".into(),
                    session: "session-2".into(),
                    endpoint: b"/run/eon/session-2.sock".to_vec(),
                    live: true,
                    attention: None,
                }],
                expanded: "pane-2".into(),
            }],
            attention_rollup: None,
        }
    }

    fn server_frame(revision: u64) -> TransportEvent {
        TransportEvent::Server(ServerMessage::Frame(Box::new(frame(revision))))
    }

    fn server_preview(revision: u64) -> TransportEvent {
        TransportEvent::Server(ServerMessage::VerticalPreview(session::VerticalPreview {
            frame_revision: revision,
            direction: session::VerticalDirection::Up,
            outcome: session::PreviewOutcome::TerminalRouted,
        }))
    }

    fn accepted() -> TransportEvent {
        TransportEvent::Server(ServerMessage::Accepted)
    }

    fn server_wheel_frame(revision: u64) -> TransportEvent {
        TransportEvent::Server(ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows: -1,
            frame: Box::new(frame(revision)),
        }))
    }

    fn server_scroll_frame(revision: u64) -> TransportEvent {
        TransportEvent::Server(ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
            requested_rows: -1,
            applied_rows: -1,
            frame: Box::new(frame(revision)),
            next: session::PreviewOutcome::Viewport {
                cols: 0,
                edge_reached: true,
                rows: Vec::new(),
            },
        }))
    }

    fn frame(revision: u64) -> Frame {
        Frame {
            revision,
            dimensions: Dimensions { cols: 0, rows: 0 },
            screen: Screen::Primary,
            scroll_position: orbit_protocol::ScrollPosition::default(),
            title: String::new(),
            working_directory: String::new(),
            capabilities: Capabilities { hyperlinks: false },
            colors: Colors {
                background: Rgb::BLACK,
                foreground: Rgb::BLACK,
                cursor: None,
                palette: [Rgb::BLACK; orbit_protocol::PALETTE_LEN],
            },
            cursor: Cursor {
                visible: false,
                blinking: false,
                password_input: false,
                shape: CursorShape::Block,
                viewport: None,
            },
            rows: Vec::new(),
            graphics: None,
        }
    }

    struct TestSocket {
        directory: PathBuf,
        path: PathBuf,
    }

    impl TestSocket {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let directory = std::env::temp_dir()
                .join(format!("venus-transport-{}-{nonce}", std::process::id()));
            fs::create_dir(&directory).unwrap();
            let path = directory.join("orbit.sock");
            Self { directory, path }
        }
    }

    impl Drop for TestSocket {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_dir(&self.directory);
        }
    }
}
