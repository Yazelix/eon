use eon_workspace_protocol::stop;
use eon_workspace_protocol::v40::{
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
    time::{Duration, Instant, SystemTime},
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
// The queue holds at most 168 events; snapshot payloads are already heap-owned.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum WorkspaceEvent {
    Response(WorkspaceResponse),
    /// A request has waited `Waits::slow` without an answer; its answer or failure follows.
    Slow,
    /// The workspace cannot be reached. `notice` is what the window shows; `detail`, which may
    /// hold operating-system text, goes only to Venus's diagnostics.
    Unavailable {
        notice: String,
        detail: Option<String>,
    },
    /// Follows the event that answered the action `send` numbered `ticket`; `accepted` when that
    /// answer was a snapshot.
    Answered {
        ticket: u64,
        accepted: bool,
    },
}

/// Bounded semantic-action handle for EONW v40.
pub struct WorkspaceTransport {
    actions: mpsc::SyncSender<WorkerItem>,
    events: Arc<WorkspaceEventQueue>,
    /// The next action's ticket.
    tickets: AtomicU64,
    subscription: Arc<Subscription>,
    notify: Arc<Notify>,
}

/// What Venus asks of Eon: a versioned action, or the Reserved Stop of its window's
/// generation, whose bytes no EONW revision changes (EON-C11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outgoing {
    Action(WorkspaceAction),
    Stop { generation: String },
}

impl From<WorkspaceAction> for Outgoing {
    fn from(action: WorkspaceAction) -> Self {
        Self::Action(action)
    }
}

/// A request for the workspace worker and its ticket; a refresh carries none.
type WorkerItem = (Outgoing, Option<u64>);

/// The fallback refresh; an event the subscription reads refreshes at once.
const WORKSPACE_REFRESH_INTERVAL: Duration = Duration::from_millis(250);
/// How long a request waits before the window says Eon is slow. Fixed: VEN-C8 sets this bound,
/// well inside the exchange deadline below, so the notice comes before any loss. A terminal start
/// can still take longer under load; eon-pimct tracks slow starts.
const WORKSPACE_SLOW_THRESHOLD: Duration = Duration::from_secs(2);

/// What the window shows when Venus cannot connect to Eon (VEN-C8).
const UNREACHABLE_NOTICE: &str = "Cannot reach Eon.";
/// What the window shows when a connected request fails or times out (VEN-C8).
const NOT_ANSWERING_NOTICE: &str = "Eon is not answering.";

/// How long the workspace worker waits on Eon.
#[derive(Clone, Copy, Debug)]
struct Waits {
    /// Between refreshes while no action is queued.
    refresh: Duration,
    /// Before a request still unanswered reports `WorkspaceEvent::Slow`.
    slow: Duration,
    /// Before a request other than Stop gives up.
    answer: Duration,
    /// Before a Stop gives up.
    stop: Duration,
}

const WAITS: Waits = Waits {
    refresh: WORKSPACE_REFRESH_INTERVAL,
    slow: WORKSPACE_SLOW_THRESHOLD,
    answer: WORKSPACE_EXCHANGE_TIMEOUT,
    stop: STOP_EXCHANGE_TIMEOUT,
};

impl Waits {
    /// The product's waits, refreshing every `refresh` and never reporting a slow answer, for
    /// tests that are not about one.
    #[cfg(test)]
    fn refreshing(refresh: Duration) -> Self {
        Self {
            refresh,
            slow: Duration::from_secs(3600),
            ..WAITS
        }
    }

    fn timeout(&self, outgoing: &Outgoing) -> Duration {
        if matches!(outgoing, Outgoing::Stop { .. }) {
            self.stop
        } else {
            self.answer
        }
    }
}

impl WorkspaceTransport {
    #[must_use]
    pub fn start(socket: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::start_with(socket, WAITS, wake)
    }

    /// The transport refreshing every `refresh`, for tests that are not about slow answers.
    #[cfg(test)]
    fn start_with_refresh(
        socket: PathBuf,
        refresh: Duration,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self::start_with(socket, Waits::refreshing(refresh), wake)
    }

    fn start_with(socket: PathBuf, waits: Waits, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let transport = Self::start_worker(socket.clone(), waits, wake);
        // Without its subscription the worker still refreshes on its interval.
        let _ = thread::Builder::new()
            .name("venus-eon-events".into())
            .spawn({
                let subscription = Arc::clone(&transport.subscription);
                let notify = Arc::clone(&transport.notify);
                move || run_subscription(socket, subscription, notify)
            });
        transport
    }

    /// The transport without its subscription thread.
    fn start_worker(
        socket: PathBuf,
        waits: Waits,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let (actions, receiver) = mpsc::sync_channel(32);
        let events = Arc::new(WorkspaceEventQueue::default());
        let notify = workspace_notifier(Arc::clone(&events), wake);
        let subscription = Arc::new(Subscription::new(actions.clone()));
        if let Err(error) = thread::Builder::new()
            .name("venus-eon-workspace".into())
            .spawn({
                let (socket, subscription, notify) = (
                    socket.clone(),
                    Arc::clone(&subscription),
                    Arc::clone(&notify),
                );
                move || run_workspace(socket, receiver, waits, subscription, notify)
            })
        {
            notify(WorkspaceEvent::Unavailable {
                notice: UNREACHABLE_NOTICE.into(),
                detail: Some(format!("Cannot start the Eon workspace worker: {error}")),
            });
        }
        Self {
            actions,
            events,
            tickets: AtomicU64::new(0),
            subscription,
            notify,
        }
    }

    /// Queues `action` and returns the ticket its `WorkspaceEvent::Answered` carries.
    pub fn send(&self, outgoing: impl Into<Outgoing>) -> Result<u64, SendError> {
        let ticket = self.tickets.fetch_add(1, Ordering::Relaxed);
        self.actions
            .try_send((outgoing.into(), Some(ticket)))
            .map(|()| ticket)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => SendError::Full,
                mpsc::TrySendError::Disconnected(_) => SendError::Closed,
            })
    }

    pub fn drain_events(&self) -> Vec<WorkspaceEvent> {
        self.events.drain()
    }

    /// Applies, without waiting, every subscription message whose bytes are readable now,
    /// then drains the events to apply, as `drain_events` does. Eon writes a hide before it
    /// answers the action that hid the popup, and the popup's terminal can draw again only
    /// after that answer, so these events hide the popup of any frame already received, whether
    /// this call or the subscription thread read the hide.
    pub fn catch_up(&self) -> Vec<WorkspaceEvent> {
        self.subscription
            .pump(&mut self.subscription.link(), &*self.notify);
        self.events.drain()
    }
}

impl Drop for WorkspaceTransport {
    fn drop(&mut self) {
        self.subscription.stop();
    }
}

#[derive(Default)]
struct WorkspaceEventQueue(Mutex<VecDeque<WorkspaceEvent>>);

impl WorkspaceEventQueue {
    fn push(&self, event: WorkspaceEvent) -> bool {
        // A full action channel (32 queued and one in flight) answers with at most three events
        // each: a slow report, a response and its `Answered`. A snapshot replaces a snapshot
        // still waiting at the end, or one that only a slow report follows, which an `Answered`
        // would otherwise follow, and a slow report adds nothing after another. So refreshes and
        // hides add at most two events between answers: 33 × 5 = 165.
        const CAPACITY: usize = 168;
        let mut events = self.0.lock().expect("workspace event queue lock poisoned");
        let wake = events.is_empty();
        let snapshot = |event: &WorkspaceEvent| {
            matches!(
                event,
                WorkspaceEvent::Response(WorkspaceResponse::Snapshot(_))
            )
        };
        let slow = |event: &WorkspaceEvent| *event == WorkspaceEvent::Slow;
        let len = events.len();
        if snapshot(&event) && events.back().is_some_and(snapshot) {
            *events.back_mut().expect("the queue has a last event") = event;
        } else if slow(&event) && events.back().is_some_and(slow) {
            // The waiting report already says so.
        } else if snapshot(&event)
            && events.back().is_some_and(slow)
            && len >= 2
            && snapshot(&events[len - 2])
        {
            // [.., Snapshot, Slow] becomes [.., Slow, Snapshot], one slow report kept.
            events.remove(len - 2);
            if len >= 3 && slow(&events[len - 3]) {
                events.pop_back();
            }
            events.push_back(event);
        } else if events.len() == CAPACITY {
            events.clear();
            events.push_back(WorkspaceEvent::Unavailable {
                notice: "Eon workspace event queue exceeded its bounded capacity".into(),
                detail: None,
            });
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
) -> Arc<Notify> {
    Arc::new(move |event| {
        if events.push(event) {
            wake();
        }
    })
}

fn run_workspace(
    socket: PathBuf,
    receiver: mpsc::Receiver<WorkerItem>,
    waits: Waits,
    subscription: Arc<Subscription>,
    notify: Arc<Notify>,
) {
    let watch = SlowWatch::default();
    thread::scope(|scope| {
        // Without the watch, answers and failures still arrive; only the slow report is lost.
        let _ = thread::Builder::new()
            .name("venus-eon-slow".into())
            .spawn_scoped(scope, || watch.run(&*notify));
        // Stops the watch even if the worker panics, so the scope can end.
        let _stop = StopWatch(&watch);
        exchange_until_dropped(&socket, &receiver, waits, &subscription, &*notify, &watch);
    });
}

struct StopWatch<'a>(&'a SlowWatch);

impl Drop for StopWatch<'_> {
    fn drop(&mut self) {
        self.0.stop();
    }
}

fn exchange_until_dropped(
    socket: &Path,
    receiver: &mpsc::Receiver<WorkerItem>,
    waits: Waits,
    subscription: &Subscription,
    notify: &Notify,
    watch: &SlowWatch,
) {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut counter = 0;
    // Refreshes carry no ticket.
    let mut action = Outgoing::Action(WorkspaceAction::Workspace(
        workspace::WorkspaceAction::Inspect,
    ));
    let mut ticket = None;
    loop {
        let begun = subscription.presented().hides;
        watch.begin(waits.slow);
        if bench_tracing() {
            bench_trace(
                "workspace_begin",
                format_args!(
                    "ticket={ticket:?} id=venus-{}-{nonce}-{counter}",
                    std::process::id()
                ),
            );
        }
        let result = workspace_exchange(socket, nonce, counter, action, &waits);
        bench_trace("workspace_answer", counter);
        // Ended before the answer is delivered, so a slow report always precedes it.
        watch.end();
        let event = result.map_or_else(ExchangeError::into_event, WorkspaceEvent::Response);
        let accepted = matches!(
            event,
            WorkspaceEvent::Response(WorkspaceResponse::Snapshot(_))
        );
        let answered = ticket.map(|ticket| WorkspaceEvent::Answered { ticket, accepted });
        subscription.deliver(event, answered, begun, notify);
        counter = counter.wrapping_add(1);
        (action, ticket) = match receiver.recv_timeout(waits.refresh) {
            Ok((action, ticket)) => {
                if ticket.is_none() {
                    subscription.link().refresh_due = false;
                }
                (action, ticket)
            }
            Err(mpsc::RecvTimeoutError::Timeout) => (
                Outgoing::Action(WorkspaceAction::Workspace(
                    workspace::WorkspaceAction::Inspect,
                )),
                None,
            ),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
    }
}

/// Reports `WorkspaceEvent::Slow` once for the worker's request still unanswered at its
/// deadline.
#[derive(Default)]
struct SlowWatch {
    state: Mutex<Watched>,
    changed: Condvar,
}

#[derive(Default)]
struct Watched {
    /// When the request in flight becomes slow; None once it is answered or reported.
    deadline: Option<Instant>,
    stopped: bool,
}

impl SlowWatch {
    fn state(&self) -> std::sync::MutexGuard<'_, Watched> {
        self.state
            .lock()
            .expect("workspace slow watch lock poisoned")
    }

    fn begin(&self, slow: Duration) {
        self.state().deadline = Some(Instant::now() + slow);
        self.changed.notify_all();
    }

    /// The request has its answer or failure; no slow report follows.
    fn end(&self) {
        self.state().deadline = None;
    }

    /// Runs while the worker unwinds too, so it never panics on a poisoned lock.
    fn stop(&self) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .stopped = true;
        self.changed.notify_all();
    }

    fn run(&self, notify: &Notify) {
        let mut state = self.state();
        while !state.stopped {
            let now = Instant::now();
            state = match state.deadline {
                None => self
                    .changed
                    .wait(state)
                    .expect("workspace slow watch lock poisoned"),
                Some(deadline) if now < deadline => {
                    self.changed
                        .wait_timeout(state, deadline - now)
                        .expect("workspace slow watch lock poisoned")
                        .0
                }
                Some(_) => {
                    state.deadline = None;
                    // Under the lock, so the worker's `end` and then its answer follow this.
                    notify(WorkspaceEvent::Slow);
                    state
                }
            };
        }
    }
}

/// The window's EON-C39 event subscription: an event makes the workspace worker refresh at once,
/// and a hidden popup stops showing before that refresh answers. Polling keeps the workspace
/// correct while no subscription is open.
struct Subscription {
    link: Mutex<Link>,
    stopped: Condvar,
    presented: Mutex<Presented>,
}

struct Link {
    /// Queues the worker's refreshes. Taken when the transport drops, which stops the
    /// subscription and leaves the transport's sender the worker's last.
    worker: Option<mpsc::SyncSender<WorkerItem>>,
    /// A refresh waits in the worker's queue, so further events add none.
    refresh_due: bool,
    /// The open subscription. It is read only under this lock, and what is read is applied
    /// before the lock is released, so whoever next holds the lock has seen every message whose
    /// bytes were readable when it took it.
    reader: Option<Reader>,
}

struct Reader {
    /// Nonblocking.
    stream: UnixStream,
    /// The start of a message not yet whole.
    partial: Vec<u8>,
    accepted: bool,
}

/// What the worker last delivered, and the popups Eon hid since.
#[derive(Default)]
struct Presented {
    /// The worker's last snapshot, while it is what the window shows.
    snapshot: Option<workspace::Snapshot>,
    /// How many hides the subscription has read.
    hides: u64,
    /// The latest hides that an answer begun before them may still show.
    hidden: VecDeque<Hidden>,
    /// An answer to an exchange begun before this many hides is dropped: a hide it may still
    /// show left `hidden` when that overflowed.
    floor: u64,
}

struct Hidden {
    /// `Presented::hides` once this one was read.
    number: u64,
    workspace: String,
    tab: String,
    popup: String,
}

type Notify = dyn Fn(WorkspaceEvent) + Send + Sync;

/// How many recent hides the worker's answers are checked against.
const HIDDEN_CAPACITY: usize = 32;
/// The first wait before subscribing again, which doubles to the limit while Eon refuses.
const SUBSCRIBE_RETRY: Duration = Duration::from_secs(1);
const SUBSCRIBE_RETRY_LIMIT: Duration = Duration::from_secs(30);

impl Subscription {
    fn new(worker: mpsc::SyncSender<WorkerItem>) -> Self {
        Self {
            link: Mutex::new(Link {
                worker: Some(worker),
                refresh_due: false,
                reader: None,
            }),
            stopped: Condvar::new(),
            presented: Mutex::new(Presented::default()),
        }
    }

    fn link(&self) -> std::sync::MutexGuard<'_, Link> {
        self.link
            .lock()
            .expect("workspace subscription lock poisoned")
    }

    fn presented(&self) -> std::sync::MutexGuard<'_, Presented> {
        self.presented
            .lock()
            .expect("workspace presentation lock poisoned")
    }

    /// Delivers the worker's answer to an exchange begun after `begun` hides, then its ticket's
    /// `answered`. Eon wrote those hides before it read the exchange, so the answer holds them;
    /// one read since may have come after the answer was written, so its popup is hidden here
    /// too.
    fn deliver(
        &self,
        mut event: WorkspaceEvent,
        answered: Option<WorkspaceEvent>,
        begun: u64,
        notify: &Notify,
    ) {
        // Eon writes an action's events before its reply, so applying those already readable
        // first keeps a late hide from undoing what this answer shows.
        let link = &mut self.link();
        self.pump(link, notify);
        let mut presented = self.presented();
        presented.hidden.retain(|hidden| hidden.number > begun);
        let stale = begun < presented.floor;
        match &mut event {
            // The refresh the overflow queued answers instead.
            WorkspaceEvent::Response(WorkspaceResponse::Snapshot(_)) if stale => {}
            WorkspaceEvent::Response(WorkspaceResponse::Snapshot(snapshot)) => {
                for hidden in &presented.hidden {
                    hidden.apply(snapshot);
                }
                presented.snapshot = Some(snapshot.clone());
            }
            // The window keeps its snapshot after a refused action; a patch would undo
            // unavailability.
            WorkspaceEvent::Response(WorkspaceResponse::Confirm(_)) => {}
            WorkspaceEvent::Response(WorkspaceResponse::Failure(failure))
                if failure.code != "stop-incomplete" => {}
            _ => presented.snapshot = None,
        }
        // Under the lock, so a hide read meanwhile follows this answer.
        if !(stale
            && matches!(
                event,
                WorkspaceEvent::Response(WorkspaceResponse::Snapshot(_))
            ))
        {
            notify(event);
        }
        if let Some(answered) = answered {
            notify(answered);
        }
    }

    /// Connects and asks for an unfiltered subscription; a handle for awaiting its bytes.
    fn open(&self, socket: &Path) -> Option<UnixStream> {
        let request = WorkspaceRequest {
            id: format!("venus-{}-events", std::process::id()),
            caller: None,
            action: WorkspaceAction::Subscribe {
                filter: workspace::Filter::default(),
                after: None,
            },
        };
        let encoded = workspace::encode_request(&request).ok()?;
        let mut stream = UnixStream::connect(socket).ok()?;
        stream
            .set_write_timeout(Some(WORKSPACE_EXCHANGE_TIMEOUT))
            .and_then(|()| stream.write_all(&encoded))
            .and_then(|()| stream.set_nonblocking(true))
            .ok()?;
        let readiness = stream.try_clone().ok()?;
        let mut link = self.link();
        link.worker.as_ref()?;
        link.reader = Some(Reader {
            stream,
            partial: Vec::new(),
            accepted: false,
        });
        Some(readiness)
    }

    /// Applies the subscription's messages as they become readable until it ends, fails or the
    /// transport drops; true when Eon accepted it.
    fn follow(&self, readiness: &UnixStream, notify: &Notify) -> bool {
        loop {
            // Waits without the lock, so the window can read meanwhile.
            let mut ready = [rustix::event::PollFd::new(
                readiness,
                rustix::event::PollFlags::IN,
            )];
            match rustix::event::poll(&mut ready, None) {
                Ok(_) | Err(rustix::io::Errno::INTR) => {}
                Err(_) => break,
            }
            if !self.pump(&mut self.link(), notify) {
                break;
            }
        }
        self.link()
            .reader
            .take()
            .is_some_and(|reader| reader.accepted)
    }

    /// Reads what the subscription holds without waiting and applies each whole message; false
    /// once it has ended, failed or closed.
    fn pump(&self, link: &mut Link, notify: &Notify) -> bool {
        let Link {
            worker,
            refresh_due,
            reader,
        } = link;
        let Some(reader) = reader else {
            return false;
        };
        let mut refresh = || queue_refresh(worker.as_ref(), refresh_due);
        let mut buffer = [0; 4096];
        let open = loop {
            match reader.stream.read(&mut buffer) {
                Ok(0) => break false,
                Ok(read) => reader.partial.extend_from_slice(&buffer[..read]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break true,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => break false,
            }
            let mut ended = false;
            while let Some(length) = whole_message(&reader.partial) {
                let decoded = length.and_then(|length| {
                    workspace::decode_subscription_message(&reader.partial[..length]).ok()
                });
                let Some((length, message)) = length.zip(decoded) else {
                    ended = true;
                    break;
                };
                reader.partial.drain(..length);
                match message {
                    workspace::SubscriptionMessage::Subscribed(_) => {
                        reader.accepted = true;
                        // Changes since the last refresh began reach no event.
                        refresh();
                    }
                    workspace::SubscriptionMessage::Tabs(_) => {}
                    workspace::SubscriptionMessage::Event(event) => {
                        if let workspace::Change::Popup {
                            change,
                            workspace,
                            tab,
                            popup,
                            ..
                        } = event.change
                        {
                            let mut presented = self.presented();
                            if change == workspace::PopupChange::Hidden {
                                let dropped = presented.hide(workspace, tab, popup.id, notify);
                                // An answer begun before a dropped hide may still show its
                                // popup; the refresh below replaces it.
                                if let Some(dropped) = dropped {
                                    presented.floor = dropped;
                                }
                            } else if matches!(
                                change,
                                workspace::PopupChange::Shown | workspace::PopupChange::Created
                            ) {
                                // A late answer must not hide a popup shown again; any other
                                // window is corrected by the refresh every event queues.
                                presented.hidden.retain(|hidden| {
                                    (&hidden.workspace, &hidden.tab, &hidden.popup)
                                        != (&workspace, &tab, &popup.id)
                                });
                            }
                        }
                        refresh();
                    }
                    workspace::SubscriptionMessage::Gap(_) => refresh(),
                    workspace::SubscriptionMessage::End(_)
                    | workspace::SubscriptionMessage::Failure(_) => {
                        ended = true;
                        break;
                    }
                }
            }
            if ended {
                break false;
            }
        };
        if !open {
            // Wakes a waiting `follow`, which retires the reader.
            let _ = reader.stream.shutdown(Shutdown::Both);
        }
        open
    }

    /// Waits `delay` unless the transport drops; true when the subscription may try again.
    fn wait(&self, delay: Duration) -> bool {
        let (link, _) = self
            .stopped
            .wait_timeout_while(self.link(), delay, |link| link.worker.is_some())
            .expect("workspace subscription lock poisoned");
        link.worker.is_some()
    }

    fn stop(&self) {
        let mut link = self.link();
        link.worker = None;
        if let Some(reader) = link.reader.take() {
            let _ = reader.stream.shutdown(Shutdown::Both);
        }
        self.stopped.notify_all();
    }
}

/// The length of the whole message `partial` starts with: None while it is not whole, and
/// `Some(None)` when its header is invalid.
fn whole_message(partial: &[u8]) -> Option<Option<usize>> {
    if partial.len() < workspace::HEADER_BYTES {
        return None;
    }
    match workspace::declared_message_len(&partial[..workspace::HEADER_BYTES]) {
        Ok(length) if partial.len() < length => None,
        Ok(length) => Some(Some(length)),
        Err(_) => Some(None),
    }
}

/// Queues a refresh unless one already waits. A full queue's actions answer with snapshots
/// begun after this, so none is queued then.
fn queue_refresh(worker: Option<&mpsc::SyncSender<WorkerItem>>, due: &mut bool) {
    if !*due {
        *due = worker.is_some_and(|worker| {
            worker
                .try_send((
                    Outgoing::Action(WorkspaceAction::Workspace(
                        workspace::WorkspaceAction::Inspect,
                    )),
                    None,
                ))
                .is_ok()
        });
    }
}

impl Presented {
    /// Records that Eon hid `popup` in `tab`; when the window shows it, a snapshot without it
    /// goes out at once. Returns the number of the oldest hide when it overflowed.
    fn hide(
        &mut self,
        workspace: String,
        tab: String,
        popup: String,
        notify: &Notify,
    ) -> Option<u64> {
        self.hides += 1;
        let dropped = if self.hidden.len() == HIDDEN_CAPACITY {
            self.hidden.pop_front().map(|hidden| hidden.number)
        } else {
            None
        };
        let hidden = Hidden {
            number: self.hides,
            workspace,
            tab,
            popup,
        };
        let patched = self
            .snapshot
            .as_mut()
            .is_some_and(|snapshot| snapshot.active_tab == hidden.tab && hidden.apply(snapshot));
        if patched && let Some(snapshot) = &self.snapshot {
            notify(WorkspaceEvent::Response(WorkspaceResponse::Snapshot(
                snapshot.clone(),
            )));
        }
        self.hidden.push_back(hidden);
        dropped
    }
}

impl Hidden {
    /// Hides the popup in `snapshot`; true when it showed there.
    fn apply(&self, snapshot: &mut workspace::Snapshot) -> bool {
        if snapshot.workspace != self.workspace {
            return false;
        }
        snapshot
            .tabs
            .iter_mut()
            .find(|tab| tab.id == self.tab && tab.selected_popup.as_deref() == Some(&self.popup))
            .map(|tab| tab.selected_popup = None)
            .is_some()
    }
}

/// Subscribes while the transport lives, waiting longer after each refusal.
fn run_subscription(socket: PathBuf, subscription: Arc<Subscription>, notify: Arc<Notify>) {
    let mut retry = SUBSCRIBE_RETRY;
    loop {
        if let Some(readiness) = subscription.open(&socket)
            && subscription.follow(&readiness, &*notify)
        {
            retry = SUBSCRIBE_RETRY;
        }
        if !subscription.wait(retry) {
            return;
        }
        retry = (retry * 2).min(SUBSCRIBE_RETRY_LIMIT);
    }
}

// Runtime owns the budgets; a slow but admitted start must not look like workspace loss.
const WORKSPACE_EXCHANGE_TIMEOUT: Duration = eon_workspace_protocol::CONTROL_TIMEOUT;
const STOP_EXCHANGE_TIMEOUT: Duration = eon_workspace_protocol::STOP_TIMEOUT;

/// Why a workspace request has no answer.
#[derive(Debug, PartialEq)]
enum ExchangeError {
    /// Venus could not connect to Eon; the detail may hold operating-system text.
    Unreachable(String),
    /// Connected, but the request or its answer did not get through: a timeout, a read or write
    /// failure, or an early end. The detail may hold operating-system text.
    NotAnswering(String),
    /// Venus refused the request or its answer; the detail is Venus's own.
    Invalid(String),
}

impl ExchangeError {
    fn into_event(self) -> WorkspaceEvent {
        let (notice, detail) = match self {
            Self::Unreachable(detail) => (UNREACHABLE_NOTICE.into(), Some(detail)),
            Self::NotAnswering(detail) => (NOT_ANSWERING_NOTICE.into(), Some(detail)),
            Self::Invalid(detail) => (NOT_ANSWERING_NOTICE.into(), Some(detail)),
        };
        WorkspaceEvent::Unavailable { notice, detail }
    }

    fn into_detail(self) -> String {
        match self {
            Self::Unreachable(detail) | Self::NotAnswering(detail) | Self::Invalid(detail) => {
                detail
            }
        }
    }
}

fn workspace_exchange(
    socket: &Path,
    nonce: u128,
    counter: usize,
    outgoing: Outgoing,
    waits: &Waits,
) -> Result<WorkspaceResponse, ExchangeError> {
    let timeout = Some(waits.timeout(&outgoing));
    let encoded = match &outgoing {
        Outgoing::Action(action) => workspace::encode_request(&WorkspaceRequest {
            id: format!("venus-{}-{nonce}-{counter}", std::process::id()),
            caller: None,
            action: action.clone(),
        })
        .map_err(|error| error.to_string()),
        Outgoing::Stop { generation } => {
            stop::encode_request(generation).map_err(|error| error.to_string())
        }
    }
    .map_err(|error| {
        ExchangeError::Invalid(format!("Cannot encode Eon workspace action: {error}"))
    })?;
    let mut stream = UnixStream::connect(socket).map_err(|error| {
        ExchangeError::Unreachable(format!(
            "Cannot connect to Eon window at {}: {error}",
            socket.display()
        ))
    })?;
    stream
        .set_read_timeout(timeout)
        .and_then(|()| stream.set_write_timeout(timeout))
        .map_err(|error| {
            ExchangeError::NotAnswering(format!("Cannot bound Eon workspace I/O: {error}"))
        })?;
    bench_trace("workspace_write", counter);
    stream.write_all(&encoded).map_err(|error| {
        ExchangeError::NotAnswering(format!("Cannot send Eon workspace action: {error}"))
    })?;

    if !matches!(outgoing, Outgoing::Stop { .. }) {
        let response = read_workspace_message(&mut stream)?;
        return workspace::decode_response(&response).map_err(invalid_response);
    }
    // `Stopped` ends the generation as `Ended` does.
    match read_stop_reply(&mut stream)? {
        stop::Reply::Stopped { generation, .. } => {
            Ok(WorkspaceResponse::Ended(workspace::Ended { generation }))
        }
        stop::Reply::Failure { code, detail } => {
            Ok(WorkspaceResponse::Failure(workspace::Failure {
                code,
                detail,
            }))
        }
    }
}

/// Reads one bounded Reserved Stop reply.
fn read_stop_reply(stream: &mut impl Read) -> Result<stop::Reply, ExchangeError> {
    let invalid = |error: stop::Error| {
        ExchangeError::Invalid(format!("Eon sent an invalid Stop reply: {error}"))
    };
    let mut reply = vec![0; stop::HEADER_BYTES];
    stream.read_exact(&mut reply).map_err(|error| {
        ExchangeError::NotAnswering(format!("Cannot read Eon Stop reply header: {error}"))
    })?;
    let length = stop::reply_len(&reply).map_err(invalid)?;
    reply.resize(length, 0);
    stream
        .read_exact(&mut reply[stop::HEADER_BYTES..])
        .map_err(|error| {
            ExchangeError::NotAnswering(format!("Cannot read complete Eon Stop reply: {error}"))
        })?;
    stop::decode_reply(&reply).map_err(invalid)
}

fn invalid_response(error: workspace::Error) -> ExchangeError {
    ExchangeError::Invalid(format!("Eon sent an invalid workspace response: {error}"))
}

/// Read one bounded canonical response, leaving subsequent control bytes unread.
pub fn read_workspace_response(stream: &mut impl Read) -> Result<WorkspaceResponse, String> {
    let response = read_workspace_message(stream).map_err(ExchangeError::into_detail)?;
    workspace::decode_response(&response).map_err(|error| invalid_response(error).into_detail())
}

fn read_workspace_message(stream: &mut impl Read) -> Result<Vec<u8>, ExchangeError> {
    let mut response = vec![0; workspace::HEADER_BYTES];
    stream.read_exact(&mut response).map_err(|error| {
        ExchangeError::NotAnswering(format!(
            "Cannot read Eon workspace response header: {error}"
        ))
    })?;
    let length = workspace::declared_message_len(&response).map_err(invalid_response)?;
    response.resize(length, 0);
    stream
        .read_exact(&mut response[workspace::HEADER_BYTES..])
        .map_err(|error| {
            ExchangeError::NotAnswering(format!(
                "Cannot read complete Eon workspace response: {error}"
            ))
        })?;
    Ok(response)
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
                bench_trace("terminal_read", framed_bytes);
                if let ServerMessage::Frame(frame) = &message {
                    bench_trace("frame_read", frame.revision);
                }
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
        bench_trace("terminal_dequeue", "");
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
    bench_trace("terminal_write_begin", "");
    let result = stream.write_all(&encoded);
    bench_trace("terminal_write_end", result.is_ok());
    result
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

/// Opt-in benchmark diagnostics; never waits for a reader or writes the pressured disk.
// The Runtime and Orbit copies mirror this format at their own ownership boundaries.
#[doc(hidden)]
pub fn bench_trace(stage: &str, detail: impl std::fmt::Display) {
    if !bench_tracing() {
        return;
    }
    use std::sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    };
    static SOCKET: OnceLock<Option<std::os::unix::net::UnixDatagram>> = OnceLock::new();
    static NEXT: AtomicU64 = AtomicU64::new(0);
    static LOST: AtomicU64 = AtomicU64::new(0);
    let socket = SOCKET.get_or_init(|| {
        let path = std::env::var_os("EON_BENCH_TRACE")?;
        let socket = std::os::unix::net::UnixDatagram::unbound().ok()?;
        socket.set_nonblocking(true).ok()?;
        socket.connect(path).ok()?;
        Some(socket)
    });
    let Some(socket) = socket else { return };
    let time = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    let nanos = time.tv_sec as u64 * 1_000_000_000 + time.tv_nsec as u64;
    let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
    let packet = format!(
        "venus\t{}\t{sequence}\t{nanos}\t{}\t{stage}\t{detail}",
        std::process::id(),
        LOST.load(Ordering::Relaxed)
    );
    if packet.len() > 4096 || socket.send(packet.as_bytes()).is_err() {
        LOST.fetch_add(1, Ordering::Relaxed);
    }
}

/// The final event lets the benchmark reject incomplete diagnostics.
#[doc(hidden)]
pub struct BenchTrace;
impl BenchTrace {
    #[doc(hidden)]
    pub fn start() -> Self {
        bench_trace("start", "");
        Self
    }
}
impl Drop for BenchTrace {
    fn drop(&mut self) {
        bench_trace("end", "");
    }
}

#[doc(hidden)]
pub fn bench_tracing() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("EON_BENCH_TRACE").is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use eon_workspace_protocol::v40::{
        InvokeIntent, Pane, Popup, PopupEntry, PopupKey, Shortcut, Snapshot, Tab,
    };
    use orbit_protocol::{
        Capabilities, Colors, Cursor, CursorShape, Dimensions, Frame, Rgb, Screen,
        session::FocusEvent,
    };

    use std::{
        fs,
        os::unix::net::UnixListener,
        time::{Duration, Instant, SystemTime},
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
            WorkspaceAction::Workspace(workspace::WorkspaceAction::Inspect).into(),
            &WAITS,
        )
        .unwrap_err();

        assert!(
            matches!(&error, ExchangeError::Invalid(text) if text.contains("unsupported EONW version")),
            "{error:?}"
        );
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
            WorkspaceAction::Workspace(workspace::WorkspaceAction::CreatePane { focus: true })
                .into(),
            &WAITS,
        );

        assert_eq!(result, Ok(snapshot));
        server.join().unwrap();
    }

    /// Waits short enough for a test: a request is slow after 100 ms and gives up after
    /// `answer`.
    fn quick_waits(answer: Duration) -> Waits {
        Waits {
            refresh: Duration::from_secs(60),
            slow: Duration::from_millis(100),
            answer,
            stop: answer,
        }
    }

    fn worker(socket: &Path, waits: Waits) -> (WorkspaceTransport, mpsc::Receiver<()>) {
        let (wakes, receiver) = mpsc::channel();
        let transport = WorkspaceTransport::start_worker(socket.to_path_buf(), waits, move || {
            let _ = wakes.send(());
        });
        (transport, receiver)
    }

    /// The events through the first that `last` matches, or all that arrive within 5 s.
    fn take_through(
        transport: &WorkspaceTransport,
        receiver: &mpsc::Receiver<()>,
        last: impl Fn(&WorkspaceEvent) -> bool,
    ) -> Vec<WorkspaceEvent> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut events = Vec::new();
        while !events.last().is_some_and(&last) {
            let Some(wait) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            if receiver.recv_timeout(wait).is_err() {
                break;
            }
            events.extend(transport.drain_events());
        }
        events
    }

    fn is_response(event: &WorkspaceEvent) -> bool {
        matches!(event, WorkspaceEvent::Response(_))
    }

    fn is_answered(event: &WorkspaceEvent) -> bool {
        matches!(event, WorkspaceEvent::Answered { .. })
    }

    fn is_unavailable(event: &WorkspaceEvent) -> bool {
        matches!(event, WorkspaceEvent::Unavailable { .. })
    }

    #[test]
    fn a_late_answer_reports_slow_before_it_arrives() {
        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let snapshot = workspace_snapshot();
        let late = snapshot.clone();
        let server = thread::spawn(move || {
            // The refresh, then a queued action: each request is slow, not only refreshes.
            for _ in 0..2 {
                let (mut stream, _) = accept_workspace_action(&listener);
                // A second past the slow threshold, so a loaded machine still reports it.
                thread::sleep(Duration::from_millis(1100));
                write_response(&mut stream, &late);
            }
        });
        let (transport, receiver) = worker(&socket.path, quick_waits(Duration::from_secs(10)));

        assert_eq!(
            take_through(&transport, &receiver, is_response),
            [WorkspaceEvent::Slow, snapshot_event(&snapshot)]
        );
        let ticket = transport.send(focus_pane()).unwrap();
        assert_eq!(
            take_through(&transport, &receiver, is_answered),
            [
                WorkspaceEvent::Slow,
                snapshot_event(&snapshot),
                WorkspaceEvent::Answered {
                    ticket,
                    accepted: true,
                },
            ]
        );
        server.join().unwrap();
    }

    #[test]
    fn a_request_past_its_timeout_reports_that_eon_is_not_answering() {
        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let (held, release) = mpsc::channel::<()>();
        let server = thread::spawn(move || {
            // The refresh waits without an answer until Venus gives up.
            let (silent, _) = accept_workspace_action(&listener);
            // The queued action's connection ends without an answer.
            drop(accept_workspace_action(&listener));
            let _ = release.recv();
            drop(silent);
        });
        // The slow report has 900 ms to arrive before the timeout.
        let (transport, receiver) = worker(&socket.path, quick_waits(Duration::from_secs(1)));

        let events = take_through(&transport, &receiver, is_unavailable);
        let ticket = transport.send(focus_pane()).unwrap();
        let mut ended = take_through(&transport, &receiver, is_answered);
        // The early end may still take past the slow threshold on a loaded machine.
        ended.retain(|event| *event != WorkspaceEvent::Slow);
        held.send(()).unwrap();
        server.join().unwrap();

        // The operating system's text goes only to the diagnostics.
        let not_answering = |events: &[WorkspaceEvent]| match events {
            [
                WorkspaceEvent::Unavailable {
                    notice,
                    detail: Some(detail),
                },
                ..,
            ] => {
                assert_eq!(notice, "Eon is not answering.");
                assert!(
                    detail.starts_with("Cannot read Eon workspace response header: "),
                    "{detail}"
                );
                detail.clone()
            }
            _ => panic!("expected Eon not answering, got {events:?}"),
        };
        assert_eq!(events[0], WorkspaceEvent::Slow, "{events:?}");
        let timed_out = not_answering(&events[1..]);
        assert!(timed_out.contains("(os error "), "{timed_out}");
        not_answering(&ended);
        assert_eq!(
            ended[1],
            WorkspaceEvent::Answered {
                ticket,
                accepted: false,
            }
        );
    }

    #[test]
    fn an_invalid_answer_reports_that_eon_is_not_answering() {
        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = accept_workspace_action(&listener);
            let mut response =
                workspace::encode_response(&WorkspaceResponse::Snapshot(workspace_snapshot()))
                    .unwrap();
            response[4..6].copy_from_slice(&(workspace::VERSION + 1).to_le_bytes());
            stream.write_all(&response).unwrap();
        });
        let (transport, receiver) = worker(&socket.path, quick_waits(Duration::from_secs(10)));

        let events = take_through(&transport, &receiver, is_unavailable);
        server.join().unwrap();
        // Venus's own account of the answer goes only to the diagnostics.
        assert!(
            matches!(
                events.last(),
                Some(WorkspaceEvent::Unavailable { notice, detail: Some(detail) })
                    if notice == "Eon is not answering."
                        && detail.contains("unsupported EONW version")
            ),
            "{events:?}"
        );
    }

    #[test]
    fn a_missing_or_refused_socket_reports_that_venus_cannot_reach_eon() {
        let missing = TestSocket::new();
        let refused = TestSocket::new();
        // Its file stays after the listener closes, so connecting is refused.
        drop(UnixListener::bind(&refused.path).unwrap());
        for socket in [missing, refused] {
            let (transport, receiver) = worker(&socket.path, quick_waits(Duration::from_secs(10)));
            let events = take(&transport, &receiver, 1);
            // The operating system's text goes only to the diagnostics.
            assert!(
                matches!(
                    &events[..],
                    [WorkspaceEvent::Unavailable { notice, detail: Some(detail) }]
                        if notice == "Cannot reach Eon."
                            && detail.starts_with("Cannot connect to Eon window at ")
                            && detail.contains("(os error ")
                ),
                "{events:?}"
            );
        }
    }

    #[test]
    fn workspace_worker_sends_the_reserved_stop() {
        const GENERATION: &str = "g1-00000000000000000000000000000007";
        let stop = || Outgoing::Stop {
            generation: GENERATION.into(),
        };
        let failure = workspace::Failure {
            code: "stop-failed".into(),
            detail: "an Orbit host refused".into(),
        };
        let replies = [
            stop::Reply::Stopped {
                generation: GENERATION.into(),
                terminals: vec!["terminal-1".into()],
            },
            stop::Reply::Failure {
                code: failure.code.clone(),
                detail: failure.detail.clone(),
            },
        ];
        let socket = TestSocket::new();
        let listener = UnixListener::bind(&socket.path).unwrap();
        let server = thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; stop::REQUEST_BYTES];
                stream.read_exact(&mut request).unwrap();
                assert_eq!(
                    request.as_slice(),
                    stop::encode_request(GENERATION).unwrap()
                );
                stream
                    .write_all(&stop::encode_reply(&reply).unwrap())
                    .unwrap();
            }
        });

        assert_eq!(
            workspace_exchange(&socket.path, 1, 0, stop(), &WAITS),
            Ok(WorkspaceResponse::Ended(workspace::Ended {
                generation: GENERATION.into()
            }))
        );
        assert_eq!(
            workspace_exchange(&socket.path, 1, 1, stop(), &WAITS),
            Ok(WorkspaceResponse::Failure(failure))
        );
        server.join().unwrap();
        // Stop waits as long as `eon quit` does: one in-flight operation, then the Stop.
        assert_eq!(WAITS.timeout(&stop()), eon_workspace_protocol::STOP_TIMEOUT);
        assert_eq!(
            WAITS.timeout(&WorkspaceAction::Workspace(workspace::WorkspaceAction::Inspect).into()),
            WORKSPACE_EXCHANGE_TIMEOUT
        );
    }

    #[test]
    fn a_subscription_event_refreshes_the_workspace_at_once() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let (wakes, receiver) = mpsc::channel();
        // No refresh falls due during the test.
        let transport = WorkspaceTransport::start_with_refresh(
            socket.path.clone(),
            Duration::from_secs(60),
            move || {
                let _ = wakes.send(());
            },
        );
        let first = workspace_snapshot();
        let mut second = first.clone();
        second.active_tab = "t2".into();
        second.tabs.push(second_workspace_tab());
        second.workspaces[0].tab_count = second.tabs.len();
        eon.answer_refresh(&first);
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(transport.drain_events(), [snapshot_event(&first)]);

        // Accepting the subscription refreshes too: no event covers what changed before it.
        let mut subscription = eon.subscription();
        write_subscription(&mut subscription, &subscribed());
        eon.answer_refresh(&first);
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(transport.drain_events(), [snapshot_event(&first)]);
        write_subscription(
            &mut subscription,
            &workspace::SubscriptionMessage::Event(workspace::Event {
                seq: 1,
                change: workspace::Change::Tab {
                    change: workspace::TabChange::Activated,
                    workspace: "s1".into(),
                    tab: second_workspace_tab(),
                    tabs: None,
                },
            }),
        );
        eon.answer_refresh(&second);
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(transport.drain_events(), [snapshot_event(&second)]);
    }

    #[test]
    fn a_hidden_presented_popup_stops_showing_before_the_refresh_answers() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let (wakes, receiver) = mpsc::channel();
        let transport = WorkspaceTransport::start_with_refresh(
            socket.path.clone(),
            Duration::from_secs(60),
            move || {
                let _ = wakes.send(());
            },
        );
        let shown = shown_popup();
        let hidden = workspace_snapshot();
        eon.answer_refresh(&shown);
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(transport.drain_events(), [snapshot_event(&shown)]);
        let mut subscription = eon.subscription();
        write_subscription(&mut subscription, &subscribed());
        eon.answer_refresh(&shown);
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(transport.drain_events(), [snapshot_event(&shown)]);

        // Eon hides the presented popup: Venus stops showing it with no answer from Eon.
        write_subscription(&mut subscription, &popup_hidden(1));
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(transport.drain_events(), [snapshot_event(&hidden)]);
        // The refresh the event queued was begun after the hide, so its answer stands.
        let (mut refresh, action) = eon.exchange();
        assert_eq!(
            action,
            WorkspaceAction::Workspace(workspace::WorkspaceAction::Inspect)
        );
        assert!(eon.exchanges.is_empty());
        write_response(&mut refresh, &shown);
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(transport.drain_events(), [snapshot_event(&shown)]);
        drop(eon);
    }

    #[test]
    fn an_answer_begun_before_a_hide_does_not_show_its_popup() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let (transport, receiver, mut subscription) =
            held_subscription(&socket, &mut eon, &shown_popup());
        let ticket = transport.send(focus_pane()).unwrap();
        let (mut stale, _) = eon.exchange();
        write_subscription(&mut subscription, &popup_hidden(1));
        assert_eq!(
            transport.catch_up(),
            [snapshot_event(&workspace_snapshot())]
        );
        write_response(&mut stale, &shown_popup());
        assert_eq!(
            take(&transport, &receiver, 2),
            [
                snapshot_event(&workspace_snapshot()),
                WorkspaceEvent::Answered {
                    ticket,
                    accepted: true
                }
            ]
        );
    }

    #[test]
    fn a_refused_action_keeps_the_snapshot_a_hide_patches() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let (transport, receiver, mut subscription) =
            held_subscription(&socket, &mut eon, &shown_popup());
        let ticket = transport.send(focus_pane()).unwrap();
        let (mut refused, _) = eon.exchange();
        let failure = workspace::Failure {
            code: "unknown-pane".into(),
            detail: "no pane pane-1".into(),
        };
        refused
            .write_all(
                &workspace::encode_response(&WorkspaceResponse::Failure(failure.clone())).unwrap(),
            )
            .unwrap();
        assert_eq!(
            take(&transport, &receiver, 2),
            [
                WorkspaceEvent::Response(WorkspaceResponse::Failure(failure)),
                WorkspaceEvent::Answered {
                    ticket,
                    accepted: false
                }
            ]
        );
        write_subscription(&mut subscription, &popup_hidden(1));
        assert_eq!(
            transport.catch_up(),
            [snapshot_event(&workspace_snapshot())]
        );
    }

    #[test]
    fn a_slow_answer_keeps_the_snapshot_a_hide_patches() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let waits = Waits {
            slow: Duration::from_secs(1),
            ..quick_waits(Duration::from_secs(10))
        };
        let (transport, receiver, mut subscription) =
            held_subscription_waiting(&socket, &mut eon, &shown_popup(), waits);
        let ticket = transport.send(focus_pane()).unwrap();
        let (mut late, _) = eon.exchange();
        assert_eq!(
            take_through(&transport, &receiver, |event| *event
                == WorkspaceEvent::Slow),
            [WorkspaceEvent::Slow]
        );
        write_subscription(&mut subscription, &popup_hidden(1));
        assert_eq!(
            transport.catch_up(),
            [snapshot_event(&workspace_snapshot())]
        );
        write_response(&mut late, &workspace_snapshot());
        assert_eq!(
            take(&transport, &receiver, 2),
            [
                snapshot_event(&workspace_snapshot()),
                WorkspaceEvent::Answered {
                    ticket,
                    accepted: true
                }
            ]
        );
    }

    #[test]
    fn a_popup_shown_again_is_not_hidden_by_a_late_answer() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let (transport, receiver, mut subscription) =
            held_subscription(&socket, &mut eon, &shown_popup());
        let ticket = transport.send(focus_pane()).unwrap();
        let (mut late, _) = eon.exchange();
        write_subscription(&mut subscription, &popup_hidden(1));
        write_subscription(
            &mut subscription,
            &popup_event(2, workspace::PopupChange::Shown, "u1"),
        );
        assert_eq!(
            transport.catch_up(),
            [snapshot_event(&workspace_snapshot())]
        );
        write_response(&mut late, &shown_popup());
        assert_eq!(
            take(&transport, &receiver, 2),
            [
                snapshot_event(&shown_popup()),
                WorkspaceEvent::Answered {
                    ticket,
                    accepted: true
                }
            ]
        );
    }

    #[test]
    fn overflowing_hides_drop_an_answer_that_needs_one_and_refresh() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let (transport, receiver, mut subscription) =
            held_subscription(&socket, &mut eon, &shown_popup());
        let ticket = transport.send(focus_pane()).unwrap();
        let (mut stale, _) = eon.exchange();
        write_subscription(&mut subscription, &popup_hidden(1));
        // Hides of other popups push the first out of the bound.
        for seq in 2..=(HIDDEN_CAPACITY as u64 + 1) {
            write_subscription(
                &mut subscription,
                &popup_event(seq, workspace::PopupChange::Hidden, &format!("x{seq}")),
            );
        }
        assert_eq!(
            transport.catch_up(),
            [snapshot_event(&workspace_snapshot())]
        );
        write_response(&mut stale, &shown_popup());
        assert_eq!(
            take(&transport, &receiver, 1),
            [WorkspaceEvent::Answered {
                ticket,
                accepted: true
            }]
        );
        let (_, refresh) = eon.exchange();
        assert_eq!(
            refresh,
            WorkspaceAction::Workspace(workspace::WorkspaceAction::Inspect)
        );
    }

    /// The window applies the subscription before an Orbit frame: Eon wrote a hide before the
    /// answer that let the popup's terminal draw again, so `catch_up` after the frame arrived
    /// finds it, with no subscription thread to run first.
    #[test]
    fn a_frame_drawn_after_a_hide_finds_the_hide_applied() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let (transport, _receiver, mut subscription) =
            held_subscription(&socket, &mut eon, &shown_popup());
        let mut popup = PopupTerminal::new();
        write_subscription(&mut subscription, &popup_hidden(1));
        popup.redraw();
        assert_eq!(
            transport.catch_up(),
            [snapshot_event(&workspace_snapshot())]
        );
    }

    /// The same order when the subscription thread read the hide first: its patch waits for
    /// the window, which `catch_up` hands over before the frame.
    #[test]
    fn a_frame_drawn_after_a_hide_the_thread_read_finds_it_applied() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let (wakes, receiver) = mpsc::channel();
        let transport = WorkspaceTransport::start_with_refresh(
            socket.path.clone(),
            Duration::from_secs(60),
            move || {
                let _ = wakes.send(());
            },
        );
        eon.answer_refresh(&shown_popup());
        assert_eq!(
            take(&transport, &receiver, 1),
            [snapshot_event(&shown_popup())]
        );
        let mut subscription = eon.subscription();
        write_subscription(&mut subscription, &subscribed());
        eon.answer_refresh(&shown_popup());
        assert_eq!(
            take(&transport, &receiver, 1),
            [snapshot_event(&shown_popup())]
        );
        let mut popup = PopupTerminal::new();
        write_subscription(&mut subscription, &popup_hidden(1));
        // The thread queued the patch; the window has not drained it.
        receiver.recv_timeout(Duration::from_secs(5)).unwrap();
        popup.redraw();
        assert_eq!(
            transport.catch_up(),
            [snapshot_event(&workspace_snapshot())]
        );
    }

    /// Eon hid and showed the popup again before answering an action that shows it: the
    /// answer goes out as it is, and no late hide follows it.
    #[test]
    fn a_late_hide_does_not_undo_a_reopened_popup() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let (transport, receiver, mut subscription) =
            held_subscription(&socket, &mut eon, &workspace_snapshot());
        let ticket = transport.send(focus_pane()).unwrap();
        let (mut reopening, _) = eon.exchange();
        write_subscription(&mut subscription, &popup_hidden(1));
        write_subscription(
            &mut subscription,
            &popup_event(2, workspace::PopupChange::Shown, "u1"),
        );
        write_response(&mut reopening, &shown_popup());
        assert_eq!(
            take(&transport, &receiver, 2),
            [
                snapshot_event(&shown_popup()),
                WorkspaceEvent::Answered {
                    ticket,
                    accepted: true
                }
            ]
        );
        assert_eq!(transport.catch_up(), []);
    }

    /// A popup's Orbit terminal, attached through the window's Orbit transport.
    struct PopupTerminal {
        _socket: TestSocket,
        orbit: Transport,
        arrived: mpsc::Receiver<()>,
        terminal: UnixStream,
    }

    impl PopupTerminal {
        fn new() -> Self {
            let socket = TestSocket::new();
            let listener = UnixListener::bind(&socket.path).unwrap();
            let (frames, arrived) = mpsc::channel();
            let orbit = Transport::start(socket.path.clone(), move || {
                let _ = frames.send(());
            });
            let (mut terminal, _) = listener.accept().unwrap();
            assert_eq!(read_client(&mut terminal), ClientMessage::Hello);
            Self {
                _socket: socket,
                orbit,
                arrived,
                terminal,
            }
        }

        /// Yazi draws one cell, and the window receives it.
        fn redraw(&mut self) {
            let mut redraw = frame(1);
            redraw.dimensions = Dimensions { cols: 1, rows: 1 };
            redraw.capabilities.hyperlinks = true;
            redraw.rows = vec![orbit_protocol::Row {
                wrapped: false,
                wrap_continuation: false,
                cells: vec![orbit_protocol::Cell {
                    width: orbit_protocol::CellWidth::Narrow,
                    text: "y".into(),
                    hyperlink: String::new(),
                    style: orbit_protocol::CellStyle {
                        foreground: orbit_protocol::StyleColor::None,
                        background: orbit_protocol::StyleColor::None,
                        underline_color: orbit_protocol::StyleColor::None,
                        underline: orbit_protocol::Underline::None,
                        bold: false,
                        italic: false,
                        faint: false,
                        blink: false,
                        inverse: false,
                        invisible: false,
                        strikethrough: false,
                        overline: false,
                        selected: false,
                        protected: false,
                    },
                }],
            }];
            let redraw = ServerMessage::Frame(Box::new(redraw));
            self.terminal
                .write_all(&session::encode_server_message(&redraw).unwrap())
                .unwrap();
            self.arrived.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(self.orbit.drain_events(), [TransportEvent::Server(redraw)]);
        }
    }

    #[test]
    fn a_snapshot_replaces_a_waiting_snapshot() {
        let events = WorkspaceEventQueue::default();
        let first = snapshot_event(&workspace_snapshot());
        let second = snapshot_event(&shown_popup());
        let answered = WorkspaceEvent::Answered {
            ticket: 0,
            accepted: true,
        };
        assert!(events.push(first.clone()));
        for _ in 0..200 {
            assert!(!events.push(second.clone()));
        }
        assert!(!events.push(answered.clone()));
        assert!(!events.push(first.clone()));
        assert_eq!(events.drain(), [second, answered, first]);
    }

    #[test]
    fn slow_reports_keep_waiting_snapshots_coalesced() {
        let events = WorkspaceEventQueue::default();
        let first = snapshot_event(&workspace_snapshot());
        let second = snapshot_event(&shown_popup());
        assert!(events.push(first.clone()));
        // Each slow refresh adds a slow report and its snapshot.
        for _ in 0..200 {
            assert!(!events.push(WorkspaceEvent::Slow));
            assert!(!events.push(second.clone()));
        }
        assert!(!events.push(WorkspaceEvent::Slow));
        assert!(!events.push(WorkspaceEvent::Slow));
        assert_eq!(
            events.drain(),
            [WorkspaceEvent::Slow, second, WorkspaceEvent::Slow]
        );
    }

    /// A transport whose subscription only `catch_up` reads, accepted while `presented` shows,
    /// with no event waiting.
    fn held_subscription(
        socket: &TestSocket,
        eon: &mut FakeEon,
        presented: &Snapshot,
    ) -> (WorkspaceTransport, mpsc::Receiver<()>, UnixStream) {
        held_subscription_waiting(
            socket,
            eon,
            presented,
            Waits::refreshing(Duration::from_secs(60)),
        )
    }

    /// `held_subscription` with the worker waiting `waits`.
    fn held_subscription_waiting(
        socket: &TestSocket,
        eon: &mut FakeEon,
        presented: &Snapshot,
        waits: Waits,
    ) -> (WorkspaceTransport, mpsc::Receiver<()>, UnixStream) {
        let (transport, receiver) = worker(&socket.path, waits);
        eon.answer_refresh(presented);
        assert_eq!(take(&transport, &receiver, 1), [snapshot_event(presented)]);
        transport.subscription.open(&socket.path).unwrap();
        let mut subscription = eon.subscription();
        write_subscription(&mut subscription, &subscribed());
        assert_eq!(transport.catch_up(), []);
        eon.answer_refresh(presented);
        assert_eq!(take(&transport, &receiver, 1), [snapshot_event(presented)]);
        (transport, receiver, subscription)
    }

    /// The next `count` events, waiting for each wake.
    fn take(
        transport: &WorkspaceTransport,
        receiver: &mpsc::Receiver<()>,
        count: usize,
    ) -> Vec<WorkspaceEvent> {
        let mut events = Vec::new();
        while events.len() < count {
            receiver.recv_timeout(Duration::from_secs(5)).unwrap();
            events.extend(transport.drain_events());
        }
        events
    }

    fn shown_popup() -> Snapshot {
        let mut shown = workspace_snapshot();
        shown.tabs[0].selected_popup = Some("u1".into());
        shown
    }

    fn focus_pane() -> WorkspaceAction {
        WorkspaceAction::Workspace(workspace::WorkspaceAction::FocusId("pane-1".into()))
    }

    #[test]
    fn a_refused_or_ended_subscription_falls_back_to_polling() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let transport = WorkspaceTransport::start_with_refresh(
            socket.path.clone(),
            Duration::from_millis(10),
            || {},
        );
        let snapshot = workspace_snapshot();
        let refused = workspace::SubscriptionMessage::Failure(workspace::Failure {
            code: "subscriber-limit".into(),
            detail: "this window already serves 32 event subscribers".into(),
        });
        let ended = workspace::SubscriptionMessage::End(workspace::End::TooSlow);
        for closing in [vec![refused], vec![subscribed(), ended]] {
            let mut subscription = eon.subscription();
            for message in &closing {
                write_subscription(&mut subscription, message);
            }
            drop(subscription);
            // Refreshes go on while Venus waits a second before it subscribes again.
            let closed = Instant::now();
            let mut refreshes = 0;
            while eon.subscriptions.is_empty() {
                if let Some((mut stream, _)) = eon.exchanges.pop_front() {
                    write_response(&mut stream, &snapshot);
                    refreshes += 1;
                } else if !eon.accept() {
                    assert!(closed.elapsed() < Duration::from_secs(5));
                    thread::sleep(Duration::from_millis(1));
                }
            }
            assert!(closed.elapsed() >= Duration::from_millis(900));
            assert!(refreshes >= 10, "{refreshes} refreshes");
        }
        drop(eon);
        drop(transport);
    }

    #[test]
    fn dropping_the_transport_stops_its_subscription_and_worker() {
        let socket = TestSocket::new();
        let mut eon = FakeEon::new(UnixListener::bind(&socket.path).unwrap());
        let transport = WorkspaceTransport::start_with_refresh(
            socket.path.clone(),
            Duration::from_millis(10),
            || {},
        );
        let mut subscription = eon.subscription();
        // Darwin rejects options once the subscription thread closes Venus's last socket descriptor.
        subscription
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write_subscription(&mut subscription, &subscribed());
        eon.answer_refresh(&workspace_snapshot());
        drop(transport);

        let mut rest = Vec::new();
        subscription.read_to_end(&mut rest).unwrap();
        // The worker finishes an exchange already begun, then makes no more.
        let dropped = Instant::now();
        let mut last = Instant::now();
        while dropped.elapsed() < Duration::from_millis(500) {
            if let Some((mut stream, _)) = eon.exchanges.pop_front() {
                write_response(&mut stream, &workspace_snapshot());
                last = Instant::now();
            } else if !eon.accept() {
                thread::sleep(Duration::from_millis(1));
            }
        }
        assert!(eon.subscriptions.is_empty());
        assert!(
            last.duration_since(dropped) < Duration::from_millis(100),
            "the worker refreshed {:?} after the drop",
            last.duration_since(dropped)
        );
    }

    /// A fake Eon that sorts its clients into exchanges and subscriptions.
    struct FakeEon {
        listener: UnixListener,
        exchanges: VecDeque<(UnixStream, WorkspaceAction)>,
        subscriptions: VecDeque<UnixStream>,
    }

    impl FakeEon {
        fn new(listener: UnixListener) -> Self {
            listener.set_nonblocking(true).unwrap();
            Self {
                listener,
                exchanges: VecDeque::new(),
                subscriptions: VecDeque::new(),
            }
        }

        /// Accepts one waiting client; false when none waits.
        fn accept(&mut self) -> bool {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let (stream, action) = read_workspace_action(stream);
                    if matches!(action, WorkspaceAction::Subscribe { .. }) {
                        self.subscriptions.push_back(stream);
                    } else {
                        self.exchanges.push_back((stream, action));
                    }
                    true
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => false,
                Err(error) => panic!("{error}"),
            }
        }

        fn until<T>(&mut self, mut next: impl FnMut(&mut Self) -> Option<T>) -> T {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if let Some(item) = next(self) {
                    return item;
                }
                if !self.accept() {
                    assert!(Instant::now() < deadline, "no client within 5 s");
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }

        fn exchange(&mut self) -> (UnixStream, WorkspaceAction) {
            self.until(|eon| eon.exchanges.pop_front())
        }

        fn subscription(&mut self) -> UnixStream {
            self.until(|eon| eon.subscriptions.pop_front())
        }

        fn answer_refresh(&mut self, snapshot: &Snapshot) {
            let (mut stream, action) = self.exchange();
            assert_eq!(
                action,
                WorkspaceAction::Workspace(workspace::WorkspaceAction::Inspect)
            );
            write_response(&mut stream, snapshot);
        }
    }

    fn write_response(stream: &mut UnixStream, snapshot: &Snapshot) {
        stream
            .write_all(
                &workspace::encode_response(&WorkspaceResponse::Snapshot(snapshot.clone()))
                    .unwrap(),
            )
            .unwrap();
    }

    fn write_subscription(stream: &mut UnixStream, message: &workspace::SubscriptionMessage) {
        stream
            .write_all(&workspace::encode_subscription_message(message).unwrap())
            .unwrap();
    }

    fn snapshot_event(snapshot: &Snapshot) -> WorkspaceEvent {
        WorkspaceEvent::Response(WorkspaceResponse::Snapshot(snapshot.clone()))
    }

    fn subscribed() -> workspace::SubscriptionMessage {
        workspace::SubscriptionMessage::Subscribed(workspace::Subscribed {
            stream: "s1".into(),
            seq: 0,
            generation: "g1".into(),
            window: None,
            caller: None,
            snapshot: None,
        })
    }

    fn popup_hidden(seq: u64) -> workspace::SubscriptionMessage {
        popup_event(seq, workspace::PopupChange::Hidden, "u1")
    }

    fn popup_event(
        seq: u64,
        change: workspace::PopupChange,
        id: &str,
    ) -> workspace::SubscriptionMessage {
        let mut popup = workspace_snapshot().tabs[0].popups[0].clone();
        popup.id = id.into();
        workspace::SubscriptionMessage::Event(workspace::Event {
            seq,
            change: workspace::Change::Popup {
                change,
                workspace: "s1".into(),
                tab: "t1".into(),
                popup,
                chosen: change == workspace::PopupChange::Shown,
            },
        })
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

    /// The next exchange; this fake refuses each subscription by closing it.
    fn accept_workspace_action(listener: &UnixListener) -> (UnixStream, WorkspaceAction) {
        loop {
            let (stream, _) = listener.accept().unwrap();
            let (stream, action) = read_workspace_action(stream);
            if !matches!(action, WorkspaceAction::Subscribe { .. }) {
                return (stream, action);
            }
        }
    }

    fn read_workspace_action(mut stream: UnixStream) -> (UnixStream, WorkspaceAction) {
        stream.set_nonblocking(false).unwrap();
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
            workspace: "s1".into(),
            active_workspace: "s1".into(),
            workspaces: vec![eon_workspace_protocol::v40::WorkspaceInfo {
                id: "s1".into(),
                persistent_id: Some("s1".into()),
                registration_error: None,
                name: None,
                tab_count: 1,
                attention: None,
                recency: 0,
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
                columns: vec![eon_workspace_protocol::v40::Column {
                    panes: vec![Pane {
                        label: "pane-1".into(),
                        pinned_name: None,
                        id: "pane-1".into(),
                        session: "terminal-1".into(),
                        endpoint: b"/run/eon/orbit.sock".to_vec(),
                        live: true,
                        attention: None,
                    }],
                    expanded: "pane-1".into(),
                }],
                attention_rollup: None,
            }],
            geometry: eon_workspace_protocol::v40::PopupGeometry {
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
            codex_quota_failure: None,
            claude_quota_failure: None,
            running: None,
            busy: None,
            sidebar: eon_workspace_protocol::v40::Sidebar::Open,
            attention_queue: Vec::new(),
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
            columns: vec![eon_workspace_protocol::v40::Column {
                panes: vec![Pane {
                    label: "pane-2".into(),
                    pinned_name: None,
                    id: "pane-2".into(),
                    session: "terminal-2".into(),
                    endpoint: b"/run/eon/terminal-2.sock".to_vec(),
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
