//! EON-C39 event subscriptions: each subscriber's connection, its bounded
//! nonblocking writer and the EONW v36 messages it reads.

use super::events::{
    ColumnOrder, Event, Events, Filter, FilterSpec, Kind, PaneChange, PopupChange, Projection,
    QueueItem, SnapshotState, TabChange, WorkspaceChange, WorkspaceItem,
};
use eon_workspace_protocol::v36::{
    self as eonw, Change, End, EventKind, Failure, Gap, GapReason, Resume, Subscribed,
    SubscriptionMessage, Tabs, WindowSnapshot, WorkspaceEntry, WorkspaceInfo,
    encode_subscription_message,
};
use std::{
    collections::VecDeque,
    io::{ErrorKind, Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    rc::Rc,
    time::{Duration, Instant},
};

/// How many subscribers one supervisor serves.
pub(super) const MAX_SUBSCRIBERS: usize = 32;
/// The event output a subscriber may hold pending.
const PENDING_BOUND: usize = 1024 * 1024;
/// How long pending output may stand still before the subscriber ends.
const TOO_SLOW: Duration = Duration::from_secs(30);
/// The most one write hands the connection, so a reader's small reads free
/// buffer space, and so move output, within a few KiB.
const WRITE_CHUNK: usize = 4096;

/// The supervisor's event stream and the subscribers it writes to.
pub(super) struct Subscribers {
    events: Events,
    subscribers: Vec<Subscriber>,
    /// A turn or maintenance ran since the supervisor last asked.
    served: bool,
}

/// One subscription: its filter and the frames not yet written to it.
pub(super) struct Subscriber {
    filter: Filter,
    stream: Option<UnixStream>,
    frames: VecDeque<Frame>,
    /// The bytes of pending event frames, which the bound counts.
    counted: usize,
    /// Events were dropped, so a gap follows once the pending frames drain;
    /// until it is queued, it is output that waits.
    gap_due: bool,
    /// Its `end` is queued; the connection closes once that is written.
    closing: bool,
    /// When pending output last moved, or when it began to wait.
    moved: Instant,
    /// While a gap waits with nothing queued: the bytes the connection
    /// last held unread, so a reader that drains it moves the clock.
    unread: Option<usize>,
}

struct Frame {
    /// Shared by every subscriber that admits the same event.
    bytes: Rc<[u8]>,
    written: usize,
    /// Whether the frame is an event, which counts against the bound.
    counted: bool,
    /// Whether the frame is the stream's `end`.
    end: bool,
}

/// How a subscriber's turn ended.
#[derive(Debug, PartialEq, Eq)]
enum Flushed {
    Open,
    Closed,
}

impl Subscribers {
    pub(super) fn new(generation: &str) -> Self {
        Self {
            events: Events::new(generation),
            subscribers: Vec::new(),
            served: false,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.subscribers.is_empty()
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.subscribers.len()
    }

    /// Refuses a further subscriber past the limit.
    pub(super) fn room(&self) -> Result<(), Failure> {
        if self.subscribers.len() >= MAX_SUBSCRIBERS {
            return Err(failure(
                "subscriber-limit",
                format!("this window already serves {MAX_SUBSCRIBERS} event subscribers"),
            ));
        }
        Ok(())
    }

    /// Accepts a `Subscribe` from the terminal `caller` over `projection`,
    /// the window now: its first frames answer `after` from the retained
    /// events when it can, and otherwise carry a full snapshot. Subscribers
    /// already served first receive every event up to now.
    #[expect(clippy::too_many_arguments, reason = "the parts of SUBSCRIBED")]
    pub(super) fn subscribe(
        &mut self,
        projection: Projection,
        filter: &eonw::Filter,
        after: Option<&Resume>,
        caller: Option<&str>,
        generation: &str,
        window: Option<String>,
        now: Instant,
    ) -> Result<Subscriber, Failure> {
        // After the turn, which lets subscribers that hung up go first.
        self.turn(projection, now);
        self.room()?;
        let projection = self
            .events
            .projection()
            .expect("a published window has a projection");
        let filter = Filter::resolve(&filter_spec(filter), caller, projection)?;
        let caller_target = caller.and_then(|session| projection.target(session));
        let subscribed = |seq, snapshot| {
            SubscriptionMessage::Subscribed(Subscribed {
                stream: self.events.stream().to_owned(),
                seq,
                generation: generation.to_owned(),
                window: window.clone(),
                caller: caller_target.clone(),
                snapshot,
            })
        };
        let unrepresentable = |error: eonw::Error| {
            failure(
                "unrepresentable-state",
                format!("cannot encode the event stream: {error}"),
            )
        };
        let mut subscriber = Subscriber {
            filter,
            stream: None,
            frames: VecDeque::new(),
            counted: 0,
            gap_due: false,
            closing: false,
            moved: now,
            unread: None,
        };
        let resumed = after.and_then(|after| {
            let events = self.events.resume(&after.stream, after.seq)?;
            Some((after.seq, events))
        });
        if let Some((seq, events)) = resumed {
            subscriber.queue(
                encode(&subscribed(seq, None)).map_err(unrepresentable)?,
                false,
            );
            for numbered in events {
                // Past the bound, the gap's snapshot covers the rest, so
                // nothing more is encoded.
                if subscriber.gap_due {
                    break;
                }
                if subscriber.filter.admits(&numbered.event) {
                    let frame = encode(&event_message(numbered.seq, &numbered.event))
                        .map_err(unrepresentable)?;
                    subscriber.queue(frame, true);
                }
            }
        } else {
            let (snapshot, tabs) = window_snapshot(&subscriber.filter.snapshot(projection));
            let head = subscribed(self.events.seq(), Some(snapshot));
            subscriber
                .queue_snapshot(head, tabs)
                .map_err(unrepresentable)?;
        }
        Ok(subscriber)
    }

    /// Starts writing to `subscriber` on its own connection, `stream`.
    pub(super) fn attach(&mut self, stream: UnixStream, mut subscriber: Subscriber, now: Instant) {
        if stream.set_nonblocking(true).is_err() {
            return;
        }
        subscriber.stream = Some(stream);
        subscriber.moved = now;
        if subscriber.flush(now) == Flushed::Open {
            self.subscribers.push(subscriber);
        }
    }

    /// Whether a turn or maintenance ran since the last call.
    pub(super) fn take_served(&mut self) -> bool {
        std::mem::take(&mut self.served)
    }

    /// Pauses the stream once no subscriber is left, so the supervisor does
    /// no work for it until the next subscribes and no `--after` resumes
    /// across the turns nobody saw.
    pub(super) fn idle(&mut self) {
        if self.subscribers.is_empty() {
            self.events.pause();
        }
    }

    /// One supervisor loop turn: publishes `projection`, the window now,
    /// queues each subscriber's admitted events or its due gap, then an
    /// `end` for those whose named objects are gone, and writes without
    /// waiting.
    pub(super) fn turn(&mut self, projection: Projection, now: Instant) {
        self.served = true;
        // Each event is encoded once, which weighs it for the retained log
        // and serves every subscriber that admits it. One that cannot be
        // encoded weighs nothing and ends those subscribers.
        let mut encoded: Vec<Option<Rc<[u8]>>> = Vec::new();
        let published = self.events.publish(projection, |numbered| {
            let bytes = encode(&event_message(numbered.seq, &numbered.event)).ok();
            let weight = bytes.as_ref().map_or(0, |bytes| bytes.len());
            encoded.push(bytes);
            weight
        });
        // `weigh` saw each published event once, in order.
        debug_assert_eq!(encoded.len(), published.len());
        let projection = self
            .events
            .projection()
            .expect("a published window has a projection");
        let seq = self.events.seq();
        self.subscribers.retain_mut(|subscriber| {
            if subscriber.closing {
                return subscriber.flush(now) == Flushed::Open;
            }
            if subscriber.gap_due {
                // The gap's snapshot already holds this turn's events.
                if !subscriber.serve_gap(projection, seq) {
                    return false;
                }
            } else {
                for (numbered, frame) in published.iter().zip(&encoded) {
                    // Past the bound, the gap's snapshot covers the rest.
                    if subscriber.gap_due {
                        break;
                    }
                    if subscriber.filter.admits(&numbered.event) {
                        let Some(bytes) = frame else {
                            return false;
                        };
                        subscriber.queue(Rc::clone(bytes), true);
                    }
                }
            }
            // After the events that made them go, and after a due gap, so
            // nothing is skipped silently.
            if !subscriber.gap_due && subscriber.filter.gone(projection) {
                subscriber.close(End::TargetsGone);
            }
            subscriber.flush(now) == Flushed::Open
        });
    }

    /// A loop turn that publishes nothing, while a Stop is incomplete:
    /// writes what each connection takes and ends subscribers that hung up
    /// or went too slow.
    pub(super) fn maintain(&mut self, now: Instant) {
        self.served = true;
        let (projection, seq) = (self.events.projection(), self.events.seq());
        self.subscribers.retain_mut(|subscriber| {
            // A due gap still goes out, from the window last published, and
            // an `end` follows it when the targets are gone there.
            if let Some(projection) = projection
                && subscriber.gap_due
            {
                if !subscriber.serve_gap(projection, seq) {
                    return false;
                }
                if !subscriber.gap_due && subscriber.filter.gone(projection) {
                    subscriber.close(End::TargetsGone);
                }
            }
            subscriber.flush(now) == Flushed::Open
        });
    }

    /// Ends every subscriber with `ended`, as the window ends: one write
    /// attempt each, never waiting for a slow one.
    pub(super) fn end(&mut self, generation: &str) {
        for mut subscriber in self.subscribers.drain(..) {
            subscriber.end(End::Ended {
                generation: generation.to_owned(),
            });
        }
        self.events.pause();
    }
}

impl Subscriber {
    /// Whether the subscriber reads again: its connection takes output.
    fn reads(&self) -> bool {
        let Some(stream) = &self.stream else {
            return false;
        };
        let mut poll = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        // SAFETY: polls one descriptor this subscriber owns, without waiting.
        unsafe { libc::poll(&mut poll, 1, 0) == 1 && poll.revents & libc::POLLOUT != 0 }
    }

    /// Queues the due gap at `seq` with a fresh snapshot of `projection`
    /// once the subscriber reads again; false when it cannot be encoded.
    fn serve_gap(&mut self, projection: &Projection, seq: u64) -> bool {
        if !self.frames.is_empty() || !self.reads() {
            return true;
        }
        self.gap_due = false;
        let (snapshot, tabs) = window_snapshot(&self.filter.snapshot(projection));
        let gap = SubscriptionMessage::Gap(Gap {
            seq,
            reason: GapReason::Overflow,
            snapshot,
        });
        self.queue_snapshot(gap, tabs).is_ok()
    }

    /// Queues a snapshot's `head` and its `TABS`, which never count
    /// against the bound; all or, when one cannot be encoded, none.
    fn queue_snapshot(&mut self, head: SubscriptionMessage, tabs: Vec<Tabs>) -> eonw::Result<()> {
        for frame in snapshot_frames(head, tabs)? {
            self.queue(frame, false);
        }
        Ok(())
    }

    fn queue(&mut self, bytes: Rc<[u8]>, counted: bool) {
        if counted {
            self.counted += bytes.len();
        }
        self.frames.push_back(Frame {
            bytes,
            written: 0,
            counted,
            end: false,
        });
        if self.counted > PENDING_BOUND {
            self.drop_unsent(|frame| frame.counted);
            self.gap_due = true;
        }
    }

    /// Drops every whole unsent frame that `which` names; a frame partly
    /// written stays, so the stream stays framed.
    fn drop_unsent(&mut self, which: impl Fn(&Frame) -> bool) {
        self.frames
            .retain(|frame| frame.written != 0 || !which(frame));
        self.counted = self
            .frames
            .iter()
            .filter(|frame| frame.counted)
            .map(|frame| frame.bytes.len() - frame.written)
            .sum();
    }

    /// Writes what the connection takes now, then ends a subscriber whose
    /// pending output stood still for 30 s or whose peer is gone.
    fn flush(&mut self, now: Instant) -> Flushed {
        let Some(stream) = &mut self.stream else {
            return Flushed::Open;
        };
        if write_pending(
            stream,
            &mut self.frames,
            &mut self.counted,
            &mut self.moved,
            now,
        )
        .is_err()
            || peer_closed(stream)
        {
            self.stream = None;
            return Flushed::Closed;
        }
        if self.closing && self.frames.is_empty() {
            self.stream = None;
            return Flushed::Closed;
        }
        // A reader draining what the connection holds moves output too,
        // while its gap waits for it with nothing queued; with frames
        // queued, writes of at most `WRITE_CHUNK` move as it reads.
        let unread = (self.gap_due && self.frames.is_empty())
            .then(|| unread(stream))
            .flatten();
        if let (Some(unread), Some(before)) = (unread, self.unread)
            && unread < before
        {
            self.moved = now;
        }
        self.unread = unread;
        if self.frames.is_empty() && !self.gap_due {
            // Output queued later starts to wait from here.
            self.moved = now;
        } else if now.saturating_duration_since(self.moved) >= TOO_SLOW {
            return self.end(End::TooSlow);
        }
        Flushed::Open
    }

    /// Queues `end` after the pending frames, which are written as usual;
    /// the connection then closes.
    fn close(&mut self, end: End) {
        self.queue_end(end);
        self.closing = true;
    }

    fn queue_end(&mut self, end: End) {
        if let Ok(bytes) = encode(&SubscriptionMessage::End(end)) {
            self.frames.push_back(Frame {
                bytes,
                written: 0,
                counted: false,
                end: true,
            });
        }
    }

    /// Drops unsent whole frames, then writes what it can of a partly
    /// written frame and of `end` without waiting, and closes.
    fn end(&mut self, end: End) -> Flushed {
        self.drop_unsent(|_| true);
        // A stream ends once: an `end` partly written stays the only one.
        if !self.frames.iter().any(|frame| frame.end) {
            self.queue_end(end);
        }
        if let Some(stream) = &mut self.stream {
            let (mut moved, now) = (self.moved, self.moved);
            let _ = write_pending(stream, &mut self.frames, &mut self.counted, &mut moved, now);
        }
        self.stream = None;
        Flushed::Closed
    }
}

/// Writes `frames` until the connection would block.
fn write_pending(
    stream: &mut UnixStream,
    frames: &mut VecDeque<Frame>,
    counted: &mut usize,
    moved: &mut Instant,
    now: Instant,
) -> std::io::Result<()> {
    while let Some(frame) = frames.front_mut() {
        let chunk = frame.bytes.len().min(frame.written + WRITE_CHUNK);
        match stream.write(&frame.bytes[frame.written..chunk]) {
            Ok(0) => return Err(ErrorKind::WriteZero.into()),
            Ok(written) => {
                frame.written += written;
                if frame.counted {
                    *counted -= written;
                }
                *moved = now;
                if frame.written == frame.bytes.len() {
                    frames.pop_front();
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(()),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// The bytes `stream` holds that its reader has not read yet, from Linux's
/// `TIOCOUTQ` (`SIOCOUTQ`).
#[cfg(target_os = "linux")]
fn unread(stream: &UnixStream) -> Option<usize> {
    let mut unread: libc::c_int = 0;
    // SAFETY: asks about one descriptor this subscriber owns, into a c_int.
    let asked = unsafe { libc::ioctl(stream.as_raw_fd(), libc::TIOCOUTQ, &mut unread) };
    (asked == 0).then_some(unread as usize)
}

/// Elsewhere the connection turning writable is the measure of progress.
/// By XNU's source, a macOS Unix stream holds no unsent bytes of its own
/// (`SO_NWRITE` stays 0); its reader's reads give back send space, and it
/// polls writable at the 2 KiB low-water mark, so `reads` sees a reader
/// that drains it. eon-ciwn.8's Mac run proves this.
#[cfg(not(target_os = "linux"))]
fn unread(_stream: &UnixStream) -> Option<usize> {
    None
}

/// Whether the subscriber closed its end; it sends nothing after
/// subscribing, so anything it sends is discarded.
fn peer_closed(stream: &mut UnixStream) -> bool {
    let mut discard = [0; 256];
    loop {
        match stream.read(&mut discard) {
            Ok(0) => return true,
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => return false,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return true,
        }
    }
}

fn failure(code: &str, detail: String) -> Failure {
    Failure {
        code: code.into(),
        detail,
    }
}

fn encode(message: &SubscriptionMessage) -> eonw::Result<Rc<[u8]>> {
    #[cfg(test)]
    tests::ENCODED.with(|encoded| encoded.set(encoded.get() + 1));
    encode_subscription_message(message).map(Rc::from)
}

/// A snapshot's head message followed by its `TABS` messages, encoded.
fn snapshot_frames(head: SubscriptionMessage, tabs: Vec<Tabs>) -> eonw::Result<Vec<Rc<[u8]>>> {
    std::iter::once(head)
        .chain(tabs.into_iter().map(SubscriptionMessage::Tabs))
        .map(|message| encode(&message))
        .collect()
}

fn filter_spec(filter: &eonw::Filter) -> FilterSpec {
    FilterSpec {
        workspaces: filter.workspaces.clone(),
        tabs: filter.tabs.clone(),
        targets: filter.targets.clone(),
        kinds: filter
            .kinds
            .iter()
            .map(|kind| match kind {
                EventKind::Workspace => Kind::Workspace,
                EventKind::Tab => Kind::Tab,
                EventKind::Pane => Kind::Pane,
                EventKind::Popup => Kind::Popup,
                EventKind::Attention => Kind::Attention,
                EventKind::Queue => Kind::Queue,
                EventKind::Busy => Kind::Busy,
            })
            .collect(),
        states: filter.states.clone(),
    }
}

/// A subscription snapshot as EONW v36 carries it: the head's snapshot and
/// one `TABS` message per workspace with admitted tabs.
fn window_snapshot(state: &SnapshotState) -> (WindowSnapshot, Vec<Tabs>) {
    let tabs: Vec<_> = state
        .tabs
        .iter()
        .map(|group| Tabs {
            workspace: group.workspace.clone(),
            tabs: group.tabs.clone(),
        })
        .collect();
    let snapshot = WindowSnapshot {
        workspaces: state.workspaces.iter().map(workspace_entry).collect(),
        attention_queue: state.attention_queue.iter().map(queue_item).collect(),
        busy: Some(state.busy.clone()),
        tab_messages: tabs.len(),
    };
    (snapshot, tabs)
}

fn workspace_entry(item: &WorkspaceItem) -> WorkspaceEntry {
    WorkspaceEntry {
        workspace: WorkspaceInfo {
            id: item.id.clone(),
            name: item.name.clone(),
            tab_count: item.tabs,
            attention: item.attention,
            recency: item.recency,
        },
        active_tab: item.active_tab.clone(),
    }
}

fn queue_item(item: &QueueItem) -> eonw::QueueItem {
    eonw::QueueItem {
        workspace: item.workspace.clone(),
        tab: item.tab.clone(),
        target: item.target.clone(),
        state: item.state,
    }
}

fn column_order(column: &ColumnOrder) -> eonw::ColumnOrder {
    eonw::ColumnOrder {
        panes: column.panes.clone(),
        expanded: column.expanded.clone(),
    }
}

/// An event as EONW v36 carries it; the filter-only `hidden` and queue
/// `previous` stay behind.
fn event_message(seq: u64, event: &Event) -> SubscriptionMessage {
    let change = match event {
        Event::Workspace(change, item) => Change::Workspace {
            change: match change {
                WorkspaceChange::Created => eonw::WorkspaceChange::Created,
                WorkspaceChange::Renamed => eonw::WorkspaceChange::Renamed,
                WorkspaceChange::Closed => eonw::WorkspaceChange::Closed,
                WorkspaceChange::Shown { .. } => eonw::WorkspaceChange::Shown,
            },
            workspace: workspace_entry(item),
        },
        Event::Tab(change, item) => Change::Tab {
            change: match change {
                TabChange::Created => eonw::TabChange::Created,
                TabChange::Moved => eonw::TabChange::Moved,
                TabChange::Directory => eonw::TabChange::Directory,
                TabChange::Closed => eonw::TabChange::Closed,
                TabChange::Activated => eonw::TabChange::Activated,
            },
            workspace: item.workspace.clone(),
            tab: item.tab.clone(),
            tabs: item.tabs.clone(),
        },
        Event::Pane(change, item) => Change::Pane {
            change: match change {
                PaneChange::Created => eonw::PaneChange::Created,
                PaneChange::Moved => eonw::PaneChange::Moved,
                PaneChange::Renamed => eonw::PaneChange::Renamed,
                PaneChange::Ended => eonw::PaneChange::Ended,
                PaneChange::Closed => eonw::PaneChange::Closed,
                PaneChange::Selected => eonw::PaneChange::Selected,
            },
            workspace: item.workspace.clone(),
            tab: item.tab.clone(),
            pane: item.pane.clone(),
            column: item.column,
            expanded: item.expanded,
            columns: item.columns.iter().map(column_order).collect(),
        },
        Event::Popup(change, item) => Change::Popup {
            change: match change {
                PopupChange::Created => eonw::PopupChange::Created,
                PopupChange::Shown => eonw::PopupChange::Shown,
                PopupChange::Hidden => eonw::PopupChange::Hidden,
                PopupChange::Closed => eonw::PopupChange::Closed,
            },
            workspace: item.workspace.clone(),
            tab: item.tab.clone(),
            popup: item.popup.clone(),
            chosen: item.chosen,
        },
        Event::Attention {
            target,
            previous,
            attention,
        } => Change::Attention {
            target: target.clone(),
            previous: *previous,
            attention: attention.clone(),
        },
        Event::Queue { queue, .. } => Change::Queue(queue.iter().map(queue_item).collect()),
        Event::Busy { target, busy, name } => Change::Busy {
            target: target.clone(),
            busy: *busy,
            name: name.clone(),
        },
    };
    SubscriptionMessage::Event(eonw::Event { seq, change })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::events::tests::{attention, base, pane_mut, tab_mut};
    use eon_workspace_protocol::v36::{
        AttentionState as State, HEADER_BYTES, declared_message_len, decode_subscription_message,
    };
    use std::cell::Cell;

    const GENERATION: &str = "g1-0123";

    thread_local! {
        /// How many messages this thread encoded.
        pub(super) static ENCODED: Cell<usize> = const { Cell::new(0) };
    }

    /// Hangs `peer` up at once: a terminal another test forks may hold a
    /// copy of its descriptor until it execs, so dropping it is not enough.
    pub(crate) fn hang_up(peer: UnixStream) {
        peer.shutdown(std::net::Shutdown::Both).unwrap();
    }

    /// A subscriber's end as the CLI holds it. It keeps the start of a frame that a
    /// non-blocking read cut short for the next read, since a write can end inside a frame
    /// (on macOS often).
    #[derive(Debug)]
    struct Peer {
        stream: UnixStream,
        carry: Vec<u8>,
    }

    impl std::ops::Deref for Peer {
        type Target = UnixStream;

        fn deref(&self) -> &UnixStream {
            &self.stream
        }
    }

    impl Read for Peer {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            if self.carry.is_empty() {
                return self.stream.read(buffer);
            }
            let count = buffer.len().min(self.carry.len());
            buffer[..count].copy_from_slice(&self.carry[..count]);
            self.carry.drain(..count);
            Ok(count)
        }
    }

    /// A subscriber's connection as the CLI reads it.
    fn connection() -> (UnixStream, Peer) {
        let (stream, peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        (
            stream,
            Peer {
                stream: peer,
                carry: Vec::new(),
            },
        )
    }

    fn read(peer: &mut Peer) -> SubscriptionMessage {
        let mut message = vec![0; HEADER_BYTES];
        peer.read_exact(&mut message).unwrap();
        message.resize(declared_message_len(&message).unwrap(), 0);
        peer.read_exact(&mut message[HEADER_BYTES..]).unwrap();
        decode_subscription_message(&message).unwrap()
    }

    /// Every whole message already written, without waiting for more; the start of a frame
    /// still being written waits in `peer` for the next read.
    fn read_available(peer: &mut Peer) -> Vec<SubscriptionMessage> {
        let bytes = read_available_bytes(peer);
        let mut messages = Vec::new();
        let mut rest = &bytes[..];
        while rest.len() >= HEADER_BYTES {
            let length = declared_message_len(&rest[..HEADER_BYTES]).unwrap();
            if rest.len() < length {
                break;
            }
            messages.push(decode_subscription_message(&rest[..length]).unwrap());
            rest = &rest[length..];
        }
        peer.carry = rest.to_vec();
        messages
    }

    /// Reads `peer`, the subscriber at `index`, as a continuously draining
    /// reader would: on through turns that publish nothing until nothing is
    /// pending, since a turn's events can outgrow the connection (macOS gives
    /// it 8 KiB). Output that stops moving fails the test rather than spin:
    /// a whole PENDING_BOUND takes about 130 turns at 8 KiB each.
    fn read_through(
        subscribers: &mut Subscribers,
        index: usize,
        peer: &mut Peer,
        window: &Projection,
        now: Instant,
    ) -> Vec<SubscriptionMessage> {
        let mut messages = read_available(peer);
        for _ in 0..1000 {
            if subscribers.subscribers[index].frames.is_empty() {
                return messages;
            }
            subscribers.turn(window.clone(), now);
            messages.extend(read_available(peer));
        }
        panic!("subscriber {index} still has output pending after 1000 turns");
    }

    /// Every byte already written, without waiting for more.
    fn read_available_bytes(peer: &mut Peer) -> Vec<u8> {
        let mut bytes = Vec::new();
        peer.set_nonblocking(true).unwrap();
        let mut buffer = [0; 65536];
        loop {
            match peer.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => bytes.extend_from_slice(&buffer[..read]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break,
                Err(error) => panic!("{error}"),
            }
        }
        peer.set_nonblocking(false).unwrap();
        bytes
    }

    fn subscribe(
        subscribers: &mut Subscribers,
        projection: &Projection,
        filter: eonw::Filter,
        after: Option<Resume>,
        now: Instant,
    ) -> Result<Peer, Failure> {
        let subscriber = subscribers.subscribe(
            projection.clone(),
            &filter,
            after.as_ref(),
            None,
            GENERATION,
            Some("0123abcd".into()),
            now,
        )?;
        let (stream, peer) = connection();
        subscribers.attach(stream, subscriber, now);
        Ok(peer)
    }

    fn event_seqs(messages: &[SubscriptionMessage]) -> Vec<u64> {
        messages
            .iter()
            .filter_map(|message| match message {
                SubscriptionMessage::Event(event) => Some(event.seq),
                _ => None,
            })
            .collect()
    }

    /// From `before`, p1 needs input and p4 is renamed.
    fn changed(before: &Projection) -> Projection {
        let mut after = before.clone();
        pane_mut(&mut after, "p1").attention = Some(attention(State::NeedsInput, false));
        // The wire checks each tab's rollup against its terminals.
        tab_mut(&mut after, "t1").attention_rollup = Some(State::NeedsInput);
        pane_mut(&mut after, "p4").label = "p4  tests".into();
        after
    }

    #[test]
    fn a_subscriber_reads_its_snapshot_then_its_admitted_events() {
        let now = Instant::now();
        let before = base();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut peer = subscribe(
            &mut subscribers,
            &before,
            eonw::Filter {
                kinds: vec![EventKind::Attention, EventKind::Pane],
                ..eonw::Filter::default()
            },
            None,
            now,
        )
        .unwrap();
        let SubscriptionMessage::Subscribed(subscribed) = read(&mut peer) else {
            panic!("no SUBSCRIBED");
        };
        assert!(subscribed.stream.starts_with("g1-0123-"));
        assert_eq!(
            (subscribed.seq, subscribed.generation.as_str()),
            (0, GENERATION)
        );
        assert_eq!(subscribed.window.as_deref(), Some("0123abcd"));
        assert_eq!(subscribed.caller, None);
        let snapshot = subscribed.snapshot.unwrap();
        assert_eq!(
            snapshot
                .workspaces
                .iter()
                .map(|entry| (entry.workspace.id.as_str(), entry.active_tab.as_str()))
                .collect::<Vec<_>>(),
            [("s1", "t1"), ("s2", "t3")]
        );
        assert_eq!(snapshot.tab_messages, 2);
        let tabs: Vec<_> = (0..2)
            .map(|_| match read(&mut peer) {
                SubscriptionMessage::Tabs(tabs) => tabs,
                message => panic!("{message:?}"),
            })
            .collect();
        assert_eq!(
            tabs.iter()
                .map(|tabs| (tabs.workspace.as_str(), tabs.tabs.len()))
                .collect::<Vec<_>>(),
            [("s1", 2), ("s2", 1)]
        );

        subscribers.turn(changed(&before), now);
        let messages = read_available(&mut peer);
        let changes: Vec<_> = messages
            .iter()
            .map(|message| match message {
                SubscriptionMessage::Event(event) => (event.seq, event.change.kind()),
                message => panic!("{message:?}"),
            })
            .collect();
        assert_eq!(changes, [(1, EventKind::Pane), (2, EventKind::Attention)]);
        let SubscriptionMessage::Event(eonw::Event {
            change:
                Change::Pane {
                    change,
                    pane,
                    columns,
                    column,
                    expanded,
                    ..
                },
            ..
        }) = &messages[0]
        else {
            panic!("{messages:?}");
        };
        assert_eq!(*change, eonw::PaneChange::Renamed);
        assert_eq!(
            (pane.label.as_str(), *column, *expanded),
            ("p4  tests", 1, true)
        );
        assert_eq!(columns[0].panes, ["p4"]);
    }

    #[test]
    fn a_resume_replays_the_admitted_events_after_seq_on_this_stream_only() {
        let now = Instant::now();
        let before = base();
        let after = changed(&before);
        let mut subscribers = Subscribers::new(GENERATION);
        let mut first = subscribe(
            &mut subscribers,
            &before,
            eonw::Filter::default(),
            None,
            now,
        )
        .unwrap();
        subscribers.turn(after.clone(), now);
        let SubscriptionMessage::Subscribed(subscribed) = read(&mut first) else {
            panic!("no SUBSCRIBED");
        };
        let stream = subscribed.stream;

        let resume = |seq| Resume {
            stream: stream.clone(),
            seq,
        };
        let only_attention = eonw::Filter {
            kinds: vec![EventKind::Attention],
            ..eonw::Filter::default()
        };
        let mut resumed = subscribe(
            &mut subscribers,
            &after,
            only_attention.clone(),
            Some(resume(0)),
            now,
        )
        .unwrap();
        let messages = read_available(&mut resumed);
        let SubscriptionMessage::Subscribed(head) = &messages[0] else {
            panic!("{messages:?}");
        };
        assert_eq!((head.seq, head.snapshot.is_none()), (0, true));
        assert_eq!(event_seqs(&messages), [2]);

        // Another stream's cursor, or one past the counter, gets a snapshot.
        for after in [
            Resume {
                stream: format!("{stream}0"),
                seq: 0,
            },
            resume(3),
        ] {
            let mut fresh = subscribe(
                &mut subscribers,
                &changed(&before),
                only_attention.clone(),
                Some(after),
                now,
            )
            .unwrap();
            let SubscriptionMessage::Subscribed(head) = read(&mut fresh) else {
                panic!("no SUBSCRIBED");
            };
            assert_eq!((head.seq, head.snapshot.is_some()), (2, true));
        }
    }

    #[test]
    fn subscribing_resolves_its_filter_and_a_33rd_subscriber_is_refused() {
        let now = Instant::now();
        let window = base();
        let mut subscribers = Subscribers::new(GENERATION);
        let missing = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter {
                targets: vec!["p9".into()],
                ..eonw::Filter::default()
            },
            None,
            now,
        )
        .unwrap_err();
        assert_eq!(missing.code, "unknown-target");

        let peers: Vec<_> = (0..MAX_SUBSCRIBERS)
            .map(|_| {
                subscribe(
                    &mut subscribers,
                    &window,
                    eonw::Filter::default(),
                    None,
                    now,
                )
            })
            .collect::<Result<_, _>>()
            .unwrap();
        let refused = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            now,
        )
        .unwrap_err();
        assert_eq!(refused.code, "subscriber-limit");
        // A subscriber that hung up frees its place for the next at once.
        let mut peers: Vec<_> = peers;
        hang_up(peers.pop().unwrap().stream);
        peers.push(
            subscribe(
                &mut subscribers,
                &window,
                eonw::Filter::default(),
                None,
                now,
            )
            .unwrap(),
        );
        peers.into_iter().for_each(|peer| hang_up(peer.stream));
        subscribers.turn(window.clone(), now);
        assert!(subscribers.is_empty());
        subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            now,
        )
        .unwrap();
    }

    /// Each turn renames 40 panes to names of EONW's 80 characters, so a
    /// subscriber that never reads passes 1 MiB within a few dozen turns.
    fn busy_window() -> (Projection, impl FnMut(&Projection, usize) -> Projection) {
        let mut window = base();
        let t1 = tab_mut(&mut window, "t1");
        t1.columns[0].panes = (0..40)
            .map(|index| crate::events::tests::pane(&format!("q{index}"), &format!("r{index}")))
            .collect();
        t1.columns[0].expanded = "q0".into();
        t1.selected_pane = Some("q0".into());
        let rename = |window: &Projection, turn: usize| {
            let mut next = window.clone();
            for pane in &mut tab_mut(&mut next, "t1").columns[0].panes {
                let name = "€".repeat(eonw::MAX_PANE_NAME_CHARS - 3);
                pane.label = format!("{}  {turn:03}{name}", pane.id);
            }
            next
        };
        (window, rename)
    }

    /// EON-C39: snapshot and `TABS` frames never count against the 1 MiB
    /// bound, so neither a first snapshot nor a gap's can overflow it.
    #[test]
    fn snapshot_and_gap_frames_do_not_count_against_the_bound() {
        let now = Instant::now();
        let (window, _) = busy_window();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut subscriber = subscribers
            .subscribe(
                window.clone(),
                &eonw::Filter::default(),
                None,
                None,
                GENERATION,
                None,
                now,
            )
            .unwrap();
        assert!(subscriber.frames.len() > 1);
        assert_eq!(subscriber.counted, 0);

        // A gap's snapshot queues the same way.
        subscriber.frames.clear();
        let (snapshot, tabs) = window_snapshot(&subscriber.filter.snapshot(&window));
        subscriber
            .queue_snapshot(
                SubscriptionMessage::Gap(Gap {
                    seq: 0,
                    reason: GapReason::Overflow,
                    snapshot,
                }),
                tabs,
            )
            .unwrap();
        assert!(subscriber.frames.len() > 1);
        assert_eq!(subscriber.counted, 0);
    }

    #[test]
    fn a_stalled_subscriber_overflows_into_one_gap_while_another_reads_every_event() {
        let now = Instant::now();
        let (mut window, mut rename) = busy_window();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut stalled = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            now,
        )
        .unwrap();
        let mut reader = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            now,
        )
        .unwrap();
        let mut read_by_reader = read_available(&mut reader);
        for turn in 0..80 {
            window = rename(&window, turn);
            subscribers.turn(window.clone(), now);
            read_by_reader.extend(read_through(&mut subscribers, 1, &mut reader, &window, now));
        }
        // The reader saw every event in order, and no gap.
        assert_eq!(
            event_seqs(&read_by_reader),
            (1..=80 * 40).collect::<Vec<_>>()
        );
        assert!(
            !read_by_reader
                .iter()
                .any(|message| matches!(message, SubscriptionMessage::Gap(_)))
        );
        let stalled_subscriber = &subscribers.subscribers[0];
        assert!(stalled_subscriber.gap_due);
        assert!(stalled_subscriber.counted <= PENDING_BOUND);

        // Once the stalled one reads everything, one gap follows at the
        // counter, then live events.
        let mut messages = Vec::new();
        for _ in 0..100 {
            messages.extend(read_available(&mut stalled));
            if messages
                .iter()
                .any(|message| matches!(message, SubscriptionMessage::Gap(_)))
            {
                break;
            }
            subscribers.turn(window.clone(), now);
        }
        window = rename(&window, 80);
        subscribers.turn(window.clone(), now);
        messages.extend(read_through(
            &mut subscribers,
            0,
            &mut stalled,
            &window,
            now,
        ));
        let gaps: Vec<_> = messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| match message {
                SubscriptionMessage::Gap(gap) => Some((index, gap)),
                _ => None,
            })
            .collect();
        assert_eq!(gaps.len(), 1);
        let (index, gap) = gaps[0];
        assert_eq!(gap.seq, 80 * 40);
        assert_eq!(gap.reason, GapReason::Overflow);
        assert_eq!(gap.snapshot.tab_messages, 2);
        assert!(matches!(messages[index + 1], SubscriptionMessage::Tabs(_)));
        let before: Vec<_> = event_seqs(&messages[..index]);
        assert!(before.windows(2).all(|pair| pair[1] == pair[0] + 1));
        assert!(*before.last().unwrap() < gap.seq);
        assert_eq!(
            event_seqs(&messages[index..]),
            (80 * 40 + 1..=81 * 40).collect::<Vec<_>>()
        );
    }

    #[test]
    fn output_that_stands_still_for_30_s_ends_the_subscriber_too_slow() {
        let start = Instant::now();
        let (mut window, mut rename) = busy_window();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut stalled = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            start,
        )
        .unwrap();
        // An idle subscriber is never too slow.
        let mut idle = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter {
                kinds: vec![EventKind::Queue],
                ..eonw::Filter::default()
            },
            None,
            start,
        )
        .unwrap();
        read_available(&mut idle);
        // About 500 KiB: more than the connection buffers, less than the
        // bound.
        for turn in 0..20 {
            window = rename(&window, turn);
            subscribers.turn(window.clone(), start);
        }
        assert!(!subscribers.subscribers[0].gap_due);
        assert!(!subscribers.subscribers[0].frames.is_empty());
        subscribers.turn(window.clone(), start + Duration::from_secs(29));
        assert_eq!(subscribers.subscribers.len(), 2);
        subscribers.turn(window.clone(), start + Duration::from_secs(30));
        assert_eq!(subscribers.subscribers.len(), 1);
        subscribers.turn(window.clone(), start + Duration::from_secs(90));
        assert_eq!(subscribers.subscribers.len(), 1);

        // The stalled one's connection closed: its buffer was full, so its
        // `end` could not be written without waiting, and its CLI exits 4.
        let mut rest = Vec::new();
        stalled.read_to_end(&mut rest).unwrap();
        assert!(!rest.is_empty());
        assert!(read_available(&mut idle).is_empty());
    }

    #[test]
    fn subscribers_end_when_their_targets_are_gone_or_the_window_ends() {
        let now = Instant::now();
        let window = base();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut p3 = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter {
                targets: vec!["p3".into()],
                ..eonw::Filter::default()
            },
            None,
            now,
        )
        .unwrap();
        let mut everything = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            now,
        )
        .unwrap();
        read_available(&mut p3);
        read_available(&mut everything);

        let mut closed = window.clone();
        let t2 = tab_mut(&mut closed, "t2");
        t2.columns.clear();
        t2.selected_pane = None;
        // p3's pane ends with its column; its stream ends targets-gone.
        subscribers.turn(closed.clone(), now);
        assert_eq!(
            read_available(&mut p3).last(),
            Some(&SubscriptionMessage::End(End::TargetsGone))
        );
        assert_eq!(subscribers.subscribers.len(), 1);

        subscribers.end(GENERATION);
        let messages = read_available(&mut everything);
        assert_eq!(
            messages.last(),
            Some(&SubscriptionMessage::End(End::Ended {
                generation: GENERATION.into()
            }))
        );
        assert!(subscribers.is_empty());
        // The stream paused: no cursor from before resumes.
        assert!(subscribers.events.projection().is_none());
    }

    #[test]
    fn a_subscriber_that_hangs_up_leaves_and_the_stream_pauses() {
        let now = Instant::now();
        let window = base();
        let mut subscribers = Subscribers::new(GENERATION);
        let peer = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            now,
        )
        .unwrap();
        hang_up(peer.stream);
        subscribers.turn(window.clone(), now);
        assert!(subscribers.is_empty());
        subscribers.idle();
        assert!(subscribers.events.projection().is_none());
        // With nobody subscribed, idle keeps it paused.
        subscribers.idle();
        assert!(subscribers.events.projection().is_none());
    }

    /// A message as the final-sequence tests name it.
    fn describe(message: &SubscriptionMessage) -> String {
        match message {
            SubscriptionMessage::Event(event) => match &event.change {
                Change::Pane { change, pane, .. } => format!("pane {change:?} {}", pane.id),
                Change::Attention {
                    target, attention, ..
                } => format!(
                    "attention {} {:?}",
                    target.terminal,
                    attention.as_ref().map(|attention| attention.state)
                ),
                Change::Queue(items) => format!("queue {}", items.len()),
                change => format!("{change:?}"),
            },
            SubscriptionMessage::Gap(_) => "gap".into(),
            SubscriptionMessage::Tabs(_) => "tabs".into(),
            SubscriptionMessage::End(end) => format!("end {end:?}"),
            SubscriptionMessage::Subscribed(_) => "subscribed".into(),
            SubscriptionMessage::Failure(failure) => format!("failure {}", failure.code),
        }
    }

    fn target(terminal: &str) -> eonw::Filter {
        eonw::Filter {
            targets: vec![terminal.into()],
            ..eonw::Filter::default()
        }
    }

    /// A subscriber past the bound with its gap still due holds output
    /// that waits, so it ends too-slow 30 s after output last moved; one
    /// that reads again before then gets its gap and stays (EON-C39).
    #[test]
    fn a_stalled_subscriber_past_the_bound_still_ends_too_slow() {
        let start = Instant::now();
        let (mut window, mut rename) = busy_window();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut stalled = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            start,
        )
        .unwrap();
        let mut slow = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            start,
        )
        .unwrap();
        for turn in 0..80 {
            window = rename(&window, turn);
            subscribers.turn(window.clone(), start);
        }
        for subscriber in &subscribers.subscribers {
            assert!(subscriber.gap_due);
            assert!(subscriber.frames.is_empty());
        }
        subscribers.turn(window.clone(), start + Duration::from_secs(20));
        assert_eq!(subscribers.subscribers.len(), 2);

        // The slow one reads again at 20 s and gets its gap.
        let mut read = read_available(&mut slow);
        subscribers.turn(window.clone(), start + Duration::from_secs(20));
        read.extend(read_available(&mut slow));
        assert!(read.iter().any(|message| describe(message) == "gap"));

        subscribers.turn(window.clone(), start + Duration::from_secs(29));
        assert_eq!(subscribers.subscribers.len(), 2);
        subscribers.turn(window.clone(), start + Duration::from_secs(30));
        assert_eq!(subscribers.subscribers.len(), 1);
        assert!(!subscribers.subscribers[0].gap_due);
        subscribers.turn(window.clone(), start + Duration::from_secs(3600));
        assert_eq!(subscribers.subscribers.len(), 1);
        let mut rest = Vec::new();
        stalled.read_to_end(&mut rest).unwrap();
    }

    /// A subscriber whose targets go reads the events that made them go,
    /// then `end` `targets-gone` (EON-C39).
    #[test]
    fn a_subscriber_whose_targets_go_reads_their_last_events_then_the_end() {
        let now = Instant::now();
        let mut window = base();
        pane_mut(&mut window, "p4").attention = Some(attention(State::NeedsInput, false));
        tab_mut(&mut window, "t3").attention_rollup = Some(State::NeedsInput);
        let mut subscribers = Subscribers::new(GENERATION);
        let mut ended = subscribe(&mut subscribers, &window, target("p3"), None, now).unwrap();
        let mut closed = subscribe(&mut subscribers, &window, target("p4"), None, now).unwrap();
        read_available(&mut ended);
        read_available(&mut closed);

        // In one turn p3 finishes and ends, and p4's column closes.
        let mut next = window.clone();
        let p3 = pane_mut(&mut next, "p3");
        p3.attention = Some(attention(State::Finished, false));
        p3.live = false;
        tab_mut(&mut next, "t2").attention_rollup = Some(State::Finished);
        let t3 = tab_mut(&mut next, "t3");
        t3.columns.clear();
        t3.selected_pane = None;
        t3.attention_rollup = None;
        subscribers.turn(next, now);
        let names = |peer: &mut Peer| {
            read_available(peer)
                .iter()
                .map(describe)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(&mut ended),
            [
                "pane Ended p3",
                "attention s4 Some(Finished)",
                "end TargetsGone",
            ]
        );
        assert_eq!(
            names(&mut closed),
            ["attention s5 None", "pane Closed p4", "end TargetsGone"]
        );
        assert!(subscribers.is_empty());
        let mut rest = [0; 1];
        assert_eq!(ended.read(&mut rest).unwrap(), 0);
        assert_eq!(closed.read(&mut rest).unwrap(), 0);
    }

    /// Renames p3 at 80 characters, so a subscriber to it that never reads
    /// fills its connection.
    fn rename_p3(window: &Projection, turn: usize) -> Projection {
        let mut next = window.clone();
        pane_mut(&mut next, "p3").label = format!("p3  {turn:05}{}", "€".repeat(75));
        next
    }

    /// `targets-gone` comes after output already pending, which is written
    /// as usual before the connection closes.
    #[test]
    fn targets_gone_keeps_pending_events_ahead_of_its_end() {
        let now = Instant::now();
        let mut window = base();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut peer = subscribe(&mut subscribers, &window, target("p3"), None, now).unwrap();
        read_available(&mut peer);
        for turn in 0..1000 {
            window = rename_p3(&window, turn);
            subscribers.turn(window.clone(), now);
        }
        assert!(!subscribers.subscribers[0].frames.is_empty());
        assert!(!subscribers.subscribers[0].gap_due);
        let t2 = tab_mut(&mut window, "t2");
        t2.columns.clear();
        t2.selected_pane = None;
        subscribers.turn(window.clone(), now);
        assert_eq!(subscribers.subscribers.len(), 1);

        let mut messages = Vec::new();
        while !subscribers.is_empty() {
            messages.extend(read_available(&mut peer));
            subscribers.turn(window.clone(), now);
        }
        messages.extend(read_available(&mut peer));
        let names: Vec<_> = messages.iter().map(describe).collect();
        assert_eq!(names.len(), 1002);
        assert!(names[..1000].iter().all(|name| name == "pane Renamed p3"));
        assert_eq!(names[1000..], ["pane Closed p3", "end TargetsGone"]);
        assert_eq!(
            event_seqs(&messages)[..1000],
            (1..=1000).collect::<Vec<_>>()
        );
    }

    /// A subscriber past the bound whose targets go gets its gap when it
    /// reads again, then its end, so no event is skipped silently.
    #[test]
    fn a_due_gap_comes_before_targets_gone() {
        let now = Instant::now();
        let mut window = base();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut peer = subscribe(&mut subscribers, &window, target("p3"), None, now).unwrap();
        read_available(&mut peer);
        let mut turn = 0;
        while !subscribers.subscribers[0].gap_due {
            window = rename_p3(&window, turn);
            subscribers.turn(window.clone(), now);
            turn += 1;
        }
        let t2 = tab_mut(&mut window, "t2");
        t2.columns.clear();
        t2.selected_pane = None;
        subscribers.turn(window.clone(), now);
        assert_eq!(subscribers.subscribers.len(), 1);

        let mut messages = Vec::new();
        while !subscribers.is_empty() {
            messages.extend(read_available(&mut peer));
            subscribers.turn(window.clone(), now);
        }
        messages.extend(read_available(&mut peer));
        let names: Vec<_> = messages.iter().map(describe).collect();
        let gap = names.iter().position(|name| name == "gap").unwrap();
        let after: Vec<_> = names[gap + 1..]
            .iter()
            .filter(|name| *name != "tabs")
            .collect();
        assert_eq!(after, ["end TargetsGone"]);
        assert!(names[..gap].iter().all(|name| name == "pane Renamed p3"));
    }

    /// A resume stops encoding the retained events once they pass the
    /// bound; the gap that follows covers the rest.
    #[test]
    fn a_resume_encodes_no_more_than_the_bound_holds() {
        let now = Instant::now();
        let mut window = base();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut reader = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            now,
        )
        .unwrap();
        let SubscriptionMessage::Subscribed(subscribed) = read(&mut reader) else {
            panic!("no SUBSCRIBED");
        };
        // 1024 directory changes of about 4 KiB each: about 4 MiB retained.
        for turn in 0..1024 {
            let letter = if turn % 2 == 0 { "d" } else { "e" };
            tab_mut(&mut window, "t1").directory = format!("/{}", letter.repeat(4000)).into();
            subscribers.turn(window.clone(), now);
            read_available(&mut reader);
        }
        ENCODED.with(|encoded| encoded.set(0));
        let resumed = subscribers
            .subscribe(
                window.clone(),
                &eonw::Filter::default(),
                Some(&Resume {
                    stream: subscribed.stream,
                    seq: subscribed.seq,
                }),
                None,
                GENERATION,
                None,
                now,
            )
            .unwrap();
        assert!(resumed.gap_due);
        // SUBSCRIBED, then the events up to the first past 1 MiB.
        let encoded = ENCODED.with(Cell::get);
        assert!(encoded <= 2 + PENDING_BOUND / 4000, "{encoded} encoded");
    }

    /// While a Stop is incomplete nothing is published, but subscribers
    /// that hung up leave and stalled ones still end too-slow.
    #[test]
    fn maintenance_ends_hung_up_and_stalled_subscribers_while_publishing_nothing() {
        let start = Instant::now();
        let (mut window, mut rename) = busy_window();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut stalled = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            start,
        )
        .unwrap();
        let hung_up = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            start,
        )
        .unwrap();
        let mut idle = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter {
                kinds: vec![EventKind::Queue],
                ..eonw::Filter::default()
            },
            None,
            start,
        )
        .unwrap();
        read_available(&mut idle);
        for turn in 0..20 {
            window = rename(&window, turn);
            subscribers.turn(window.clone(), start);
        }
        hang_up(hung_up.stream);
        subscribers.maintain(start);
        assert_eq!(subscribers.subscribers.len(), 2);
        subscribers.maintain(start + Duration::from_secs(29));
        assert_eq!(subscribers.subscribers.len(), 2);
        subscribers.maintain(start + Duration::from_secs(30));
        assert_eq!(subscribers.subscribers.len(), 1);
        let mut rest = Vec::new();
        stalled.read_to_end(&mut rest).unwrap();
        assert!(read_available(&mut idle).is_empty());
    }

    /// Every whole message in `bytes`.
    fn decode_all(bytes: &[u8]) -> Vec<SubscriptionMessage> {
        let mut messages = Vec::new();
        let mut rest = bytes;
        while rest.len() >= HEADER_BYTES {
            let length = declared_message_len(rest).unwrap();
            if rest.len() < length {
                break;
            }
            messages.push(decode_subscription_message(&rest[..length]).unwrap());
            rest = &rest[length..];
        }
        messages
    }

    /// A reader past the bound when a Stop fails still gets its gap once
    /// it drains, and is not ended too-slow.
    #[test]
    fn a_due_gap_goes_out_while_a_stop_is_incomplete() {
        let start = Instant::now();
        let (mut window, mut rename) = busy_window();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut peer = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            start,
        )
        .unwrap();
        for turn in 0..80 {
            window = rename(&window, turn);
            subscribers.turn(window.clone(), start);
        }
        assert!(subscribers.subscribers[0].gap_due);
        // The Stop fails; from here only maintenance runs.
        subscribers.maintain(start + Duration::from_secs(10));
        let mut messages = read_available(&mut peer);
        subscribers.maintain(start + Duration::from_secs(20));
        messages.extend(read_available(&mut peer));
        assert!(messages.iter().any(|message| describe(message) == "gap"));
        subscribers.maintain(start + Duration::from_secs(60));
        assert_eq!(subscribers.subscribers.len(), 1);
        assert!(
            !read_available(&mut peer)
                .iter()
                .any(|message| matches!(message, SubscriptionMessage::End(_)))
        );
    }

    /// EON-C39: too-slow is output that has not moved for 30 s, so a reader
    /// past the bound that drains slowly but steadily keeps its stream and
    /// gets its gap.
    #[test]
    fn a_slow_steady_reader_past_the_bound_is_never_too_slow() {
        let start = Instant::now();
        let (mut window, mut rename) = busy_window();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut peer = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            start,
        )
        .unwrap();
        for turn in 0..80 {
            window = rename(&window, turn);
            subscribers.turn(window.clone(), start);
        }
        assert!(subscribers.subscribers[0].gap_due);
        // 1 KiB a second, far below what makes the connection writable
        // within 30 s, for 150 s and on until the gap arrives: how many bytes
        // come before it depends on the platform's socket buffer.
        peer.set_nonblocking(true).unwrap();
        let mut bytes = Vec::new();
        let mut names = Vec::new();
        for second in 1..=600 {
            let mut buffer = [0; 1024];
            if let Ok(read) = peer.read(&mut buffer) {
                bytes.extend_from_slice(&buffer[..read]);
            }
            subscribers.turn(window.clone(), start + Duration::from_secs(second));
            assert_eq!(subscribers.subscribers.len(), 1, "ended at {second} s");
            if second >= 150 {
                names = decode_all(&bytes).iter().map(describe).collect();
                if names.contains(&"gap".to_owned()) {
                    break;
                }
            }
        }
        assert!(names.contains(&"gap".to_owned()));
        assert!(!names.iter().any(|name| name.starts_with("end")));
    }

    /// A stream ends with one `end`: the window ending after a partly
    /// written `end` `targets-gone` adds no `ended`.
    #[test]
    fn a_stream_ends_once() {
        let now = Instant::now();
        let window = base();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut subscriber = subscribers
            .subscribe(window, &target("p3"), None, None, GENERATION, None, now)
            .unwrap();
        subscriber.frames.clear();
        subscriber.close(End::TargetsGone);
        subscriber.frames[0].written = 1;
        subscriber.end(End::Ended {
            generation: GENERATION.into(),
        });
        assert_eq!(subscriber.frames.len(), 1);
        let rest = &subscriber.frames[0].bytes;
        assert_eq!(
            decode_subscription_message(rest).unwrap(),
            SubscriptionMessage::End(End::TargetsGone)
        );
    }

    /// A reader past the bound whose targets go, when a Stop then fails,
    /// gets its gap once it drains, then its `end`.
    #[test]
    fn a_gap_during_a_failed_stop_is_followed_by_targets_gone() {
        let now = Instant::now();
        let mut window = base();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut peer = subscribe(&mut subscribers, &window, target("p3"), None, now).unwrap();
        read_available(&mut peer);
        let mut turn = 0;
        while !subscribers.subscribers[0].gap_due {
            window = rename_p3(&window, turn);
            subscribers.turn(window.clone(), now);
            turn += 1;
        }
        let t2 = tab_mut(&mut window, "t2");
        t2.columns.clear();
        t2.selected_pane = None;
        subscribers.turn(window.clone(), now);
        assert_eq!(subscribers.subscribers.len(), 1);

        // The Stop fails; from here only maintenance runs.
        let mut messages = Vec::new();
        for _ in 0..1000 {
            if subscribers.is_empty() {
                break;
            }
            messages.extend(read_available(&mut peer));
            subscribers.maintain(now);
        }
        assert!(subscribers.is_empty());
        messages.extend(read_available(&mut peer));
        let names: Vec<_> = messages.iter().map(describe).collect();
        let gap = names.iter().position(|name| name == "gap").unwrap();
        let after: Vec<_> = names[gap + 1..]
            .iter()
            .filter(|name| *name != "tabs")
            .collect();
        assert_eq!(after, ["end TargetsGone"]);
    }

    /// 200 panes whose terminals have 1500-byte endpoints, so one snapshot
    /// of the window is larger than a connection's buffer; each turn
    /// renames every pane.
    fn big_window() -> (Projection, impl FnMut(&Projection, usize) -> Projection) {
        let mut window = base();
        let t1 = tab_mut(&mut window, "t1");
        t1.columns[0].panes = (0..200)
            .map(|index| {
                let mut pane =
                    crate::events::tests::pane(&format!("q{index}"), &format!("r{index}"));
                pane.endpoint = format!("/{}/{index}", "e".repeat(1500)).into_bytes();
                pane
            })
            .collect();
        t1.columns[0].expanded = "q0".into();
        t1.selected_pane = Some("q0".into());
        let rename = |window: &Projection, turn: usize| {
            let mut next = window.clone();
            for pane in &mut tab_mut(&mut next, "t1").columns[0].panes {
                let name = "€".repeat(eonw::MAX_PANE_NAME_CHARS - 3);
                pane.label = format!("{}  {turn:03}{name}", pane.id);
            }
            next
        };
        (window, rename)
    }

    /// EON-C39: output that moves is never too slow, however small the
    /// reads: a reader past the bound taking 512 bytes a second drains its
    /// connection and then a gap snapshot larger than the connection holds.
    #[test]
    fn a_reader_taking_small_reads_through_a_large_gap_snapshot_is_never_too_slow() {
        let start = Instant::now();
        let (mut window, mut rename) = big_window();
        let mut subscribers = Subscribers::new(GENERATION);
        let mut peer = subscribe(
            &mut subscribers,
            &window,
            eonw::Filter::default(),
            None,
            start,
        )
        .unwrap();
        // The first snapshot, larger than the connection holds, read whole.
        while !subscribers.subscribers[0].frames.is_empty() {
            read_available_bytes(&mut peer);
            subscribers.turn(window.clone(), start);
        }
        read_available_bytes(&mut peer);
        let mut turn = 0;
        while !subscribers.subscribers[0].gap_due {
            window = rename(&window, turn);
            subscribers.turn(window.clone(), start);
            turn += 1;
        }
        peer.set_nonblocking(true).unwrap();
        let mut bytes = Vec::new();
        let mut second = 0;
        loop {
            second += 1;
            let mut buffer = [0; 512];
            if let Ok(read) = peer.read(&mut buffer) {
                bytes.extend_from_slice(&buffer[..read]);
            }
            subscribers.turn(window.clone(), start + Duration::from_secs(second));
            assert_eq!(subscribers.subscribers.len(), 1, "ended at {second} s");
            let subscriber = &subscribers.subscribers[0];
            if !subscriber.gap_due && subscriber.frames.is_empty() {
                break;
            }
            assert!(second < 5000, "no gap in {second} s");
        }
        peer.set_nonblocking(false).unwrap();
        bytes.extend(read_available_bytes(&mut peer));
        let messages = decode_all(&bytes);
        let gap = messages
            .iter()
            .find_map(|message| match message {
                SubscriptionMessage::Gap(gap) => Some(gap),
                _ => None,
            })
            .unwrap();
        assert_eq!(gap.snapshot.tab_messages, 2);
        // Reading it took far longer than 30 s.
        assert!(second > 60, "{second} s");
    }

    /// Two tabs of 120 terminals with endpoints and labels near EONW's
    /// bounds, so each `tab-activated` event carries hundreds of KB.
    fn large_tabs() -> Projection {
        let mut window = base();
        for (tab, prefix) in [("t1", "a"), ("t2", "b")] {
            let tab = tab_mut(&mut window, tab);
            tab.columns[0].panes = (0..120)
                .map(|index| {
                    let id = format!("{prefix}{index}");
                    let mut pane = crate::events::tests::pane(&id, &format!("{prefix}s{index}"));
                    pane.endpoint = format!("/{}/{id}", "e".repeat(4000)).into_bytes();
                    pane.label = format!("{id}  {}", "€".repeat(eonw::MAX_PANE_NAME_CHARS));
                    pane
                })
                .collect();
            tab.columns[0].expanded = format!("{prefix}0");
            tab.selected_pane = Some(format!("{prefix}0"));
        }
        window
    }

    /// The bead's worst case: flipping between two large tabs. The retained
    /// events stay within 8 MiB as EONW frames (EON-C39), so far fewer than
    /// 1024 of them are kept; each turn encodes its event once.
    #[test]
    fn flipping_two_large_tabs_keeps_the_retained_events_within_8_mib() {
        let now = Instant::now();
        let window = large_tabs();
        let mut flipped = window.clone();
        crate::events::tests::activate(&mut flipped, "s1", "t2");
        let mut subscribers = Subscribers::new(GENERATION);
        let only_queue = eonw::Filter {
            kinds: vec![EventKind::Queue],
            ..eonw::Filter::default()
        };
        let mut peer = subscribe(&mut subscribers, &window, only_queue.clone(), None, now).unwrap();
        let SubscriptionMessage::Subscribed(first) = read(&mut peer) else {
            panic!("no SUBSCRIBED");
        };
        read_available_bytes(&mut peer);
        let stream = subscribers.events.stream().to_owned();
        let mut largest = 0;
        // The byte bound binds from the 17th event; weighing what is kept
        // costs a full encode, so it is checked every 8 turns.
        for turn in 0..48 {
            ENCODED.with(|encoded| encoded.set(0));
            subscribers.turn(if turn % 2 == 0 { &flipped } else { &window }.clone(), now);
            assert_eq!(ENCODED.with(Cell::get), 1, "turn {turn}");
            if turn % 8 != 7 {
                continue;
            }
            let seq = subscribers.events.seq();
            let retained: Vec<_> = (0..=seq)
                .find_map(|after| subscribers.events.resume(&stream, after))
                .unwrap()
                .map(|numbered| {
                    encode_subscription_message(&event_message(numbered.seq, &numbered.event))
                        .unwrap()
                        .len()
                })
                .collect();
            let bytes: usize = retained.iter().sum();
            largest = largest.max(*retained.last().unwrap());
            assert!(
                bytes <= 8 * 1024 * 1024,
                "turn {turn}: {bytes} bytes retained"
            );
        }
        // What a count bound alone would have kept.
        eprintln!(
            "tab-activated frames up to {largest} bytes; 1024 of them would be {} MiB",
            largest * 1024 / (1024 * 1024)
        );
        assert!(largest > 500 * 1024);

        // A cursor whose events the bound dropped gets a full snapshot; one
        // ten events back, about 5 MiB, still resumes. The last turn showed
        // `window`, as these subscribers do.
        let seq = subscribers.events.seq();
        for (after, resumed) in [(first.seq, false), (seq - 10, true)] {
            let mut again = subscribe(
                &mut subscribers,
                &window,
                only_queue.clone(),
                Some(Resume {
                    stream: stream.clone(),
                    seq: after,
                }),
                now,
            )
            .unwrap();
            let SubscriptionMessage::Subscribed(head) = read(&mut again) else {
                panic!("no SUBSCRIBED");
            };
            assert_eq!(head.snapshot.is_none(), resumed, "after {after}");
        }
    }
}
