//! Linux desktop notifications for detach (EON-C8) and agent attention
//! (EON-C31). Both use the supervisor's session-bus worker.
//!
//! The text and the notice's state machine are plain data, tested on every
//! platform. On Linux, `Notices` owns one session-bus connection on a worker
//! thread. The notification server can deliver the detach notice's actions
//! back to it, while attention notices have no actions.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::{
    attention::{NoticeDispatch, NoticeKind},
    running::window_label,
    windows::durable_window_id,
};
use eon_workspace_protocol::v50::WindowEntry;
use std::sync::mpsc;

/// Dropping the supervisor's request wakes the command worker even if it
/// acknowledged the park just as the supervisor's deadline expired.
pub(super) struct NoticePark {
    ready: Option<mpsc::Receiver<bool>>,
    _release: mpsc::Sender<()>,
}

impl NoticePark {
    pub(super) fn settled(&mut self) -> Result<bool, &'static str> {
        let Some(ready) = &self.ready else {
            return Ok(true);
        };
        match ready.try_recv() {
            Ok(true) => {
                self.ready = None;
                Ok(true)
            }
            Ok(false) => Err("a detach notice is still live"),
            Err(mpsc::TryRecvError::Empty) => Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => Err("notification state cannot be settled"),
        }
    }
}

/// The notification service's well-known bus name.
const SERVICE: &str = "org.freedesktop.Notifications";
/// The message bus itself, the only sender of `NameOwnerChanged`.
const BUS: &str = "org.freedesktop.DBus";

/// What a notice says about its window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NoticeWindow {
    name: String,
    /// The independent window's ID; `None` in the default namespace.
    id: Option<String>,
    stop_command: String,
    terminals: usize,
}

impl NoticeWindow {
    pub(super) fn new(window: &WindowEntry) -> Self {
        Self {
            name: window_label(window),
            id: window.window.clone(),
            stop_command: format!(
                "eon window stop {}",
                durable_window_id(window.window.as_deref(), &window.generation)
            ),
            terminals: window.live_terminals.unwrap_or(0).into(),
        }
    }
}

/// The supervisor's requests to the notice.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Command {
    /// The window detached: show its notice, replacing any earlier one.
    Show(NoticeWindow),
    /// The person chose Quit: ask once more.
    Confirm(NoticeWindow),
    /// The confirmed Stop failed.
    Failure(NoticeWindow),
    /// The window is shown again or ends: remove the notice.
    Close,
}

/// What the notice asks of the supervisor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Event {
    /// Quit was chosen; answer with `Notices::confirm` and the window's
    /// current facts.
    Confirm,
    /// The confirmation's Quit was chosen; stop the window.
    Quit,
}

/// A signal as the bus delivered it, with the unique name that sent it.
/// Whom to believe is `Notice::on_signal`'s decision.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Signal {
    Action {
        sender: String,
        id: u32,
        key: String,
    },
    Closed {
        sender: String,
        id: u32,
    },
    /// `NameOwnerChanged`: `old` no longer owns `name`.
    OwnerChanged {
        sender: String,
        name: String,
        old: String,
        new: String,
    },
}

/// Which actions the shown notification offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Shown,
    Confirming,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Effect {
    Event(Event),
    Close,
    ServiceAvailable(bool),
}

/// A notification the server shows.
#[derive(Debug)]
struct Shown {
    id: u32,
    /// The unique bus name of the server that issued `id`. Only it can
    /// replace, close or act on the notification, so a server that took over
    /// the well-known name never gets an ID it did not issue.
    owner: String,
    stage: Stage,
}

/// One `Notify` call.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Notification {
    destination: String,
    replaces: u32,
    summary: String,
    body: String,
    /// Identifier and label pairs.
    actions: &'static [&'static str],
    sound: Sound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sound {
    Default,
    Silent,
    Theme,
}

impl Notification {
    fn attention(state: NoticeKind, location: &str) -> Self {
        let (summary, sound) = match state {
            NoticeKind::Finished => ("Agent finished", Sound::Silent),
            NoticeKind::Error => ("Agent failed", Sound::Theme),
            NoticeKind::NeedsInput => ("Agent needs input", Sound::Theme),
        };
        Self {
            destination: SERVICE.into(),
            replaces: 0,
            summary: summary.into(),
            body: escape_body(location),
            actions: &[],
            sound,
        }
    }
}

/// One detach notice call to the notification server.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Notify(Notification, Stage),
    Close { id: u32, owner: String },
}

const QUIT: &str = "quit";
const QUIT_CONFIRM: &str = "quit-confirm";
const KEEP: &str = "keep";

/// One window's notice, and what Eon knows of the server that shows it.
#[derive(Debug, Default)]
struct Notice {
    shown: Option<Shown>,
    /// Whether the server offers actions, with the unique name that said so.
    actions: Option<(String, bool)>,
}

impl Notice {
    /// The call that carries out `command`. A later notification replaces the
    /// shown one at the server that issued it, so a window never stacks
    /// notices.
    fn call(&self, command: &Command) -> Option<Call> {
        let ((summary, body), actions, stage): (_, &'static [&'static str], _) = match command {
            Command::Show(window) => {
                let offered = self.actions.as_ref().is_some_and(|(_, offered)| *offered);
                (
                    notice_text(window, offered),
                    if offered { &[QUIT, "Quit"] } else { &[] },
                    Stage::Shown,
                )
            }
            Command::Confirm(window) => (
                confirm_text(window),
                &[QUIT_CONFIRM, "Quit", KEEP, "Keep running"],
                Stage::Confirming,
            ),
            Command::Failure(window) => (failure_text(window), &[], Stage::Failed),
            Command::Close => {
                return self.shown.as_ref().map(|shown| Call::Close {
                    id: shown.id,
                    owner: shown.owner.clone(),
                });
            }
        };
        Some(Call::Notify(
            Notification {
                destination: self
                    .shown
                    .as_ref()
                    .map(|shown| &shown.owner)
                    .or_else(|| self.actions.as_ref().map(|(owner, _)| owner))
                    .map_or(SERVICE, String::as_str)
                    .to_owned(),
                replaces: self.shown.as_ref().map_or(0, |shown| shown.id),
                summary,
                body,
                actions,
                sound: Sound::Default,
            },
            stage,
        ))
    }

    /// The server `owner` shows the notification as `id`.
    fn sent(&mut self, id: u32, owner: String, stage: Stage) {
        self.shown = Some(Shown { id, owner, stage });
    }

    /// The server `owner` closed `id`, on request or because the person
    /// dismissed it. Until then the notice keeps its ID, so a failed close is
    /// retried by the next close and a later detach replaces the notification
    /// instead of stacking another.
    fn closed(&mut self, id: u32, owner: &str) {
        if self.is_shown(id, owner) {
            self.shown = None;
        }
    }

    /// The server `owner` left the bus: its notification and its capabilities
    /// went with it. An owner that is not the one that issued the notice,
    /// such as one reported by an owner change queued before the notice was
    /// sent, forgets nothing.
    fn forget(&mut self, owner: &str) {
        if self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.owner == owner)
        {
            self.shown = None;
        }
        if self.actions.as_ref().is_some_and(|(said, _)| said == owner) {
            self.actions = None;
        }
    }

    fn is_shown(&self, id: u32, owner: &str) -> bool {
        self.shown
            .as_ref()
            .is_some_and(|shown| shown.id == id && shown.owner == owner)
    }

    /// Believes only the server that issued this notice's ID about it, and
    /// only the bus about owner changes; acts only on the actions the notice's
    /// stage offers. Quit asks first, and only the confirmation's Quit stops
    /// the window.
    fn on_signal(&mut self, signal: Signal) -> Option<Effect> {
        match signal {
            Signal::OwnerChanged {
                sender,
                name,
                old,
                new,
            } => {
                if sender == BUS && name == SERVICE {
                    self.forget(&old);
                    return Some(Effect::ServiceAvailable(!new.is_empty()));
                }
                None
            }
            Signal::Closed { sender, id } => {
                self.closed(id, &sender);
                None
            }
            Signal::Action { sender, id, key } => {
                let shown = self
                    .shown
                    .as_ref()
                    .filter(|shown| shown.id == id && shown.owner == sender)?;
                match (shown.stage, key.as_str()) {
                    (Stage::Shown, QUIT) => Some(Effect::Event(Event::Confirm)),
                    (Stage::Confirming, QUIT_CONFIRM) => Some(Effect::Event(Event::Quit)),
                    (Stage::Confirming, KEEP) => Some(Effect::Close),
                    _ => None,
                }
            }
        }
    }
}

/// The detach notice's title and body. Without the `actions` capability the
/// body names the window's stop command in place of the Quit button.
fn notice_text(window: &NoticeWindow, actions: bool) -> (String, String) {
    let count = match window.terminals {
        0 => "No terminal is running yet.".to_owned(),
        1 => "1 terminal keeps running in the background.".to_owned(),
        terminals => format!("{terminals} terminals keep running in the background."),
    };
    let to_return = match &window.id {
        None => "Open Eon to return.".to_owned(),
        Some(id) => format!("Run `eon window attach {id}` to return."),
    };
    let mut body = format!("{count} {to_return}");
    if !actions {
        body.push_str(&format!(" Run `{}` to stop it.", window.stop_command));
    }
    (
        format!("Eon window {} is still running", window.name),
        escape_body(&body),
    )
}

fn confirm_text(window: &NoticeWindow) -> (String, String) {
    let body = match window.terminals {
        0 => "No terminal is running yet.".to_owned(),
        1 => "This stops its 1 terminal. Eon cannot restore it.".to_owned(),
        terminals => format!("This stops its {terminals} terminals. Eon cannot restore them."),
    };
    (format!("Quit {}?", window.name), escape_body(&body))
}

/// A failed Stop may have stopped some terminals, and the window stays for a
/// retried Stop (EON-C11).
fn failure_text(window: &NoticeWindow) -> (String, String) {
    (
        format!("Eon could not quit {}", window.name),
        escape_body(&format!(
            "Some terminals may still run; run `{}` to try again.",
            window.stop_command
        )),
    )
}

/// Servers with `body-markup` parse a body as markup; titles are plain text.
fn escape_body(body: &str) -> String {
    let mut escaped = String::with_capacity(body.len());
    for character in body.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(target_os = "linux")]
pub(super) use linux::Notices;

#[cfg(target_os = "linux")]
mod linux {
    use super::{
        BUS, Call, Command, Effect, Event, Notice, NoticeDispatch, NoticeKind, NoticePark,
        NoticeWindow, Notification, SERVICE, Signal, Sound,
    };
    use crate::{AttentionNotifications, diagnostics::DiagnosticLog};
    use std::{
        cell::Cell,
        collections::HashMap,
        future::Future,
        pin::pin,
        sync::{
            Arc, Mutex, MutexGuard,
            atomic::{AtomicUsize, Ordering::Relaxed},
            mpsc,
        },
        task::{Context, Wake, Waker},
        thread,
        time::Duration,
    };
    use zbus::{
        Message,
        blocking::{Connection, MessageIterator},
        message::Type,
        zvariant::Value,
    };

    const SERVICE_PATH: &str = "/org/freedesktop/Notifications";
    const BUS_PATH: &str = "/org/freedesktop/DBus";
    const ATTENTION_CAPACITY: usize = 16;

    enum Input {
        Command(Command),
        Signal(Signal),
        Attention(PendingAttention),
        /// The bus connection ended.
        Disconnected,
        /// The supervisor is ending; the worker drops the sender as it exits.
        Finish(mpsc::Sender<()>),
        Park {
            ready: mpsc::Sender<bool>,
            released: mpsc::Receiver<()>,
        },
        #[cfg(test)]
        Barrier(mpsc::Sender<()>),
    }

    #[derive(Clone)]
    struct InputSender {
        sender: mpsc::Sender<Input>,
        wake: thread::Thread,
    }

    impl InputSender {
        fn send(&self, input: Input) -> Result<(), mpsc::SendError<Input>> {
            self.sender.send(input)?;
            self.wake.unpark();
            Ok(())
        }
    }

    struct PendingAttention {
        notification: Notification,
        epoch: usize,
        dispatch: Arc<NoticeDispatch>,
    }

    #[derive(Default)]
    struct AttentionEpoch(AtomicUsize);

    impl AttentionEpoch {
        fn current(&self) -> usize {
            self.0.load(Relaxed)
        }

        /// Odd accepts new offers. Advancing on both transitions prevents a
        /// concurrent offer from an earlier cycle from surviving recovery.
        fn set_available(&self, available: bool) {
            if self.current() % 2 != usize::from(available) {
                self.0.fetch_add(1, Relaxed);
            }
        }
    }

    struct WorkerHandle {
        inputs: InputSender,
        attention: mpsc::SyncSender<PendingAttention>,
        epoch: Arc<AttentionEpoch>,
    }

    /// A failed call to the notification server.
    struct Failure {
        /// The addressed server has left the bus.
        gone: bool,
        detail: String,
    }

    /// The notification server as the worker calls it: D-Bus in use, a fake
    /// in tests. Each answer names the unique bus name that gave it.
    trait Server {
        fn capabilities(&self) -> Result<(Vec<String>, String), Failure>;
        fn notify(&self, notification: &Notification) -> Result<(u32, String), Failure>;
        fn close(&self, id: u32, owner: &str) -> Result<(), Failure>;
        fn pause_reader(&self) -> Option<MutexGuard<'_, bool>> {
            None
        }
    }

    /// Opens a server for the desktop entry `application_id`, passing its
    /// signals to the worker's inputs.
    type Connect = Arc<dyn Fn(&str, InputSender) -> zbus::Result<Box<dyn Server>> + Send + Sync>;

    /// The supervisor's handle on its window's notifications. A later detach
    /// can replace a lost worker.
    pub(crate) struct Notices {
        worker: Option<WorkerHandle>,
        attempted: bool,
        detach_requested: bool,
        events: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
        pending_event: Cell<Option<Event>>,
        connect: Connect,
        settings: AttentionNotifications,
    }

    impl Default for Notices {
        fn default() -> Self {
            Self::new(AttentionNotifications::default())
        }
    }

    impl Drop for Notices {
        fn drop(&mut self) {
            self.close_and_wait(Duration::ZERO);
        }
    }

    impl Notices {
        pub(crate) fn new(settings: AttentionNotifications) -> Self {
            Self {
                worker: None,
                attempted: false,
                detach_requested: false,
                events: mpsc::channel(),
                pending_event: Cell::default(),
                connect: Arc::new(|application_id, inputs| {
                    if cfg!(test) {
                        return Err(zbus::Error::Failure(
                            "tests inject the notification server".into(),
                        ));
                    }
                    DBus::connect(application_id, inputs)
                }),
                settings,
            }
        }

        /// Shows the window's notice; `application_id` names Eon's desktop
        /// entry to the server.
        pub(crate) fn show(
            &mut self,
            window: NoticeWindow,
            application_id: &str,
            log: &Arc<DiagnosticLog>,
        ) {
            self.detach_requested = true;
            let input = Input::Command(Command::Show(window));
            let input = match &self.worker {
                Some(worker) => match worker.inputs.send(input) {
                    Ok(()) => return,
                    Err(mpsc::SendError(unsent)) => unsent,
                },
                None => input,
            };
            self.start(application_id, log, false);
            self.send(input);
        }

        pub(crate) fn attention(
            &mut self,
            state: NoticeKind,
            application_id: &str,
            log: &Arc<DiagnosticLog>,
            dispatch: Arc<NoticeDispatch>,
            location: impl FnOnce(bool) -> String,
        ) -> bool {
            if !self.settings.notify {
                return false;
            }
            if !self.attempted {
                self.start(application_id, log, true);
            }
            let Some(worker) = &self.worker else {
                return false;
            };
            let epoch = worker.epoch.current();
            if epoch.is_multiple_of(2) {
                return false;
            }
            let notification =
                Notification::attention(state, &location(self.settings.notify_names));
            let queued = worker
                .attention
                .try_send(PendingAttention {
                    notification,
                    epoch,
                    dispatch,
                })
                .is_ok();
            if queued {
                worker.inputs.wake.unpark();
            }
            queued
        }

        fn start(&mut self, application_id: &str, log: &Arc<DiagnosticLog>, connect_now: bool) {
            let (sender, receiver) = mpsc::channel();
            let (attention, pending) = mpsc::sync_channel(ATTENTION_CAPACITY);
            let epoch = Arc::new(AttentionEpoch::default());
            epoch.set_available(!std::mem::replace(&mut self.attempted, true));
            let worker = Worker {
                log: Arc::clone(log),
                events: self.events.0.clone(),
                notice: Notice::default(),
                reported: false,
                attention_reported: false,
                epoch: Arc::clone(&epoch),
            };
            let connect = Arc::clone(&self.connect);
            let application_id = application_id.to_owned();
            let signals = sender.clone();
            match thread::Builder::new()
                .name("eon-notifications".into())
                .spawn(move || {
                    let inputs = InputSender {
                        sender: signals,
                        wake: thread::current(),
                    };
                    let server = if connect_now {
                        worker.connect(&connect, &application_id, &inputs)
                    } else {
                        None
                    };
                    worker.run(
                        &connect,
                        &application_id,
                        inputs,
                        &receiver,
                        &pending,
                        server,
                    );
                }) {
                Ok(thread) => {
                    self.worker = Some(WorkerHandle {
                        inputs: InputSender {
                            sender,
                            wake: thread.thread().clone(),
                        },
                        attention,
                        epoch,
                    })
                }
                Err(_) => {
                    self.worker = None;
                    log.record("desktop notifications are unavailable: cannot start the worker");
                }
            }
        }

        pub(crate) fn confirm(&self, window: NoticeWindow) {
            self.send(Input::Command(Command::Confirm(window)));
        }

        pub(crate) fn failure(&self, window: NoticeWindow) {
            self.send(Input::Command(Command::Failure(window)));
        }

        pub(crate) fn close(&self) {
            self.send(Input::Command(Command::Close));
        }

        fn send(&self, input: Input) {
            if let Some(worker) = &self.worker {
                let _ = worker.inputs.send(input);
            }
        }

        pub(crate) fn next_event(&self) -> Option<Event> {
            self.pending_event
                .take()
                .or_else(|| self.events.1.try_recv().ok())
        }

        /// Inspect the head after the worker acknowledges, leaving it for the
        /// normal supervisor turn if Commit refuses.
        pub(crate) fn has_pending_event(&self) -> bool {
            let event = self.next_event();
            self.pending_event.set(event);
            event.is_some()
        }

        pub(crate) fn park(&self) -> Option<NoticePark> {
            let worker = self.worker.as_ref()?;
            let (ready, ack) = mpsc::channel();
            let (release, released) = mpsc::channel();
            let _ = worker.inputs.send(Input::Park { ready, released });
            Some(NoticePark {
                ready: Some(ack),
                _release: release,
            })
        }

        #[cfg(test)]
        pub(crate) fn record_closes() -> (Self, impl Fn() -> bool) {
            let (notices, receiver) = Self::record_commands();
            (notices, move || {
                matches!(receiver.try_recv(), Ok(Input::Command(Command::Close)))
            })
        }

        #[cfg(test)]
        pub(crate) fn record_shows() -> (Self, impl Fn() -> Option<NoticeWindow>) {
            let (notices, receiver) = Self::record_commands();
            (notices, move || match receiver.try_recv().ok()? {
                Input::Command(Command::Show(window)) => Some(window),
                _ => None,
            })
        }

        #[cfg(test)]
        fn record_commands() -> (Self, mpsc::Receiver<Input>) {
            let (sender, receiver) = mpsc::channel();
            let (attention, _) = mpsc::sync_channel(0);
            let worker = WorkerHandle {
                inputs: InputSender {
                    sender,
                    wake: thread::current(),
                },
                attention,
                epoch: Arc::default(),
            };
            let mut notices = Self::default();
            notices.worker = Some(worker);
            notices.attempted = true;
            (notices, receiver)
        }

        /// Stops the worker, allowing up to `timeout` for detach cleanup.
        pub(crate) fn close_and_wait(&mut self, timeout: Duration) {
            let Some(worker) = self.worker.take() else {
                return;
            };
            let (done, finished) = mpsc::channel();
            if worker.inputs.send(Input::Finish(done)).is_ok() && self.detach_requested {
                let _ = finished.recv_timeout(timeout);
            }
        }
    }

    struct Worker {
        log: Arc<DiagnosticLog>,
        events: mpsc::Sender<Event>,
        notice: Notice,
        /// Whether the log already holds the current failure.
        reported: bool,
        attention_reported: bool,
        epoch: Arc<AttentionEpoch>,
    }

    fn next_input(
        inputs: &mpsc::Receiver<Input>,
        pending: &mpsc::Receiver<PendingAttention>,
        selected: &mut Option<PendingAttention>,
        mut park: impl FnMut(),
    ) -> Option<Input> {
        loop {
            match inputs.try_recv() {
                Ok(input) => return Some(input),
                Err(mpsc::TryRecvError::Disconnected) => return None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if let Some(attention) = selected.take() {
                return Some(Input::Attention(attention));
            }
            // Recheck controls after receiving an alert from the other queue.
            *selected = pending.try_recv().ok();
            if selected.is_none() {
                park();
            }
        }
    }

    impl Worker {
        fn run(
            mut self,
            connect: &Connect,
            application_id: &str,
            signals: InputSender,
            inputs: &mpsc::Receiver<Input>,
            pending: &mpsc::Receiver<PendingAttention>,
            mut server: Option<Box<dyn Server>>,
        ) {
            let mut selected = None;
            while let Some(input) = next_input(inputs, pending, &mut selected, thread::park) {
                match input {
                    Input::Command(command) => {
                        if server.is_none() && matches!(command, Command::Show(_)) {
                            server = self.connect(connect, application_id, &signals);
                        }
                        if let Some(server) = &server {
                            self.carry_out(&**server, command);
                        }
                    }
                    Input::Attention(attention) => {
                        if let Some(server) = &server {
                            self.notify_attention(&**server, attention);
                        }
                    }
                    Input::Signal(signal) => match self.notice.on_signal(signal) {
                        Some(Effect::Event(event)) => {
                            let _ = self.events.send(event);
                        }
                        Some(Effect::Close) => {
                            if let Some(server) = &server {
                                self.carry_out(&**server, Command::Close);
                            }
                        }
                        Some(Effect::ServiceAvailable(available)) => {
                            self.epoch.set_available(available)
                        }
                        None => {}
                    },
                    Input::Disconnected => {
                        self.epoch.set_available(false);
                        server = None;
                        self.notice = Notice::default();
                    }
                    Input::Finish(done) => {
                        if let Some(server) = &server {
                            self.carry_out(&**server, Command::Close);
                        }
                        drop(done);
                        return;
                    }
                    Input::Park { ready, released } => {
                        // Earlier controls have run. The selected alert and
                        // the separate pending lane still own actual D-Bus calls.
                        while released.try_recv() == Err(mpsc::TryRecvError::Empty) {
                            let Some(attention) =
                                selected.take().or_else(|| pending.try_recv().ok())
                            else {
                                if self.notice.shown.is_some() {
                                    let _ = ready.send(false);
                                } else {
                                    let _reader =
                                        server.as_ref().and_then(|server| server.pause_reader());
                                    if ready.send(true).is_ok() {
                                        let _ = released.recv();
                                    }
                                }
                                break;
                            };
                            if let Some(server) = &server {
                                self.notify_attention(&**server, attention);
                            }
                        }
                    }
                    #[cfg(test)]
                    Input::Barrier(done) => {
                        let _ = done.send(());
                    }
                }
            }
        }

        fn connect(
            &self,
            connect: &Connect,
            application_id: &str,
            signals: &InputSender,
        ) -> Option<Box<dyn Server>> {
            match connect(application_id, signals.clone()) {
                Ok(server) => Some(server),
                Err(error) => {
                    self.epoch.set_available(false);
                    self.log.record(&format!(
                        "desktop notifications are unavailable: cannot connect to the session bus: {error}"
                    ));
                    None
                }
            }
        }

        fn notify_attention(&mut self, server: &dyn Server, pending: PendingAttention) {
            if pending.epoch != self.epoch.current() || !pending.dispatch.begin() {
                return;
            }
            match server.notify(&pending.notification) {
                Ok(_) => self.attention_reported = false,
                Err(failure) => {
                    if failure.gone {
                        self.epoch.set_available(false);
                    }
                    if !std::mem::replace(&mut self.attention_reported, true) {
                        self.log
                            .record("desktop agent attention notification could not be sent");
                    }
                }
            }
        }

        fn carry_out(&mut self, server: &dyn Server, command: Command) {
            let show = matches!(command, Command::Show(_));
            // Only the detach notice depends on the capability; the server's
            // is read once, and again after that server leaves.
            if show && self.notice.actions.is_none() {
                match server.capabilities() {
                    Ok((capabilities, owner)) => {
                        let offered = capabilities.iter().any(|name| name == "actions");
                        self.notice.actions = Some((owner, offered));
                    }
                    Err(failure) => {
                        if failure.gone {
                            self.epoch.set_available(false);
                        }
                        return self.unavailable(&failure);
                    }
                }
            }
            let Some(call) = self.notice.call(&command) else {
                return;
            };
            match call {
                Call::Notify(notification, stage) => match server.notify(&notification) {
                    Ok((id, owner)) => {
                        self.reported = false;
                        if show {
                            self.epoch.set_available(true);
                        }
                        self.log.record(&format!(
                            "desktop notification {id} sent: {}",
                            notification.summary
                        ));
                        self.notice.sent(id, owner, stage);
                    }
                    // The issuing server left before its owner change
                    // arrived: show the notification afresh.
                    Err(failure) if failure.gone && notification.replaces != 0 => {
                        self.notice.forget(&notification.destination);
                        self.carry_out(server, command);
                    }
                    Err(failure) => {
                        if failure.gone && notification.destination == SERVICE {
                            self.epoch.set_available(false);
                        }
                        self.unavailable(&failure);
                    }
                },
                Call::Close { id, owner } => match server.close(id, &owner) {
                    Ok(()) => self.notice.closed(id, &owner),
                    Err(failure) if failure.gone => self.notice.forget(&owner),
                    // Such as a notification the person dismissed just now,
                    // whose `NotificationClosed` is still on its way: the
                    // service works, so this is not `unavailable`.
                    Err(failure) => self.log.record(&format!(
                        "desktop notification {id} was not closed: {}",
                        failure.detail
                    )),
                },
            }
        }

        /// Logs a failing notification service once until a call succeeds.
        fn unavailable(&mut self, failure: &Failure) {
            if !std::mem::replace(&mut self.reported, true) {
                self.log.record(&format!(
                    "desktop notifications are unavailable: {}",
                    failure.detail
                ));
            }
        }
    }

    /// The session bus's notification service.
    struct DBus {
        connection: Connection,
        application_id: String,
        reader: BusReader,
    }

    /// zbus 5.19 receives ancillary descriptors without MSG_CMSG_CLOEXEC.
    /// Drive its executor here so Commit can exclude all socket reads
    /// during audit and exec and keep the connection for rollback.
    struct BusReader {
        stopped: Arc<Mutex<bool>>,
        worker: thread::JoinHandle<()>,
    }

    struct ReaderWake(thread::Thread);

    impl Wake for ReaderWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }
    }

    impl BusReader {
        fn start(connection: zbus::Connection) -> std::io::Result<Self> {
            let stopped = Arc::new(Mutex::new(false));
            let stop = Arc::clone(&stopped);
            let worker = thread::Builder::new()
                .name("eon-notification-bus".into())
                .spawn(move || {
                    let waker = Waker::from(Arc::new(ReaderWake(thread::current())));
                    let mut context = Context::from_waker(&waker);
                    loop {
                        let mut tick = pin!(connection.executor().tick());
                        loop {
                            let result = {
                                let stopped = stop.lock().expect("notification reader");
                                if *stopped {
                                    return;
                                }
                                tick.as_mut().poll(&mut context)
                            };
                            if result.is_ready() {
                                break;
                            }
                            thread::park();
                        }
                    }
                })?;
            Ok(Self { stopped, worker })
        }
    }

    impl Drop for BusReader {
        fn drop(&mut self) {
            *self.stopped.lock().expect("notification reader") = true;
            self.worker.thread().unpark();
        }
    }

    impl DBus {
        /// Opens the session bus and subscribes to the server's signals before
        /// any notification, since a fast click can arrive before `Notify`
        /// returns. A server that never answers fails a call after D-Bus's
        /// usual 25-second timeout and holds only the worker.
        fn connect(application_id: &str, inputs: InputSender) -> zbus::Result<Box<dyn Server>> {
            let builder =
                zbus::connection::Builder::session()?.method_timeout(Duration::from_secs(25));
            Ok(Box::new(Self::connect_on(builder, application_id, inputs)?))
        }

        fn connect_on(
            builder: zbus::connection::Builder<'_>,
            application_id: &str,
            inputs: InputSender,
        ) -> zbus::Result<Self> {
            let connection = builder.internal_executor(false).build();
            let connection = zbus::block_on(connection)?;
            let reader = BusReader::start(connection.clone())
                .map_err(|error| zbus::Error::Failure(error.to_string()))?;
            let connection = Connection::from(connection);
            // Subscribed before the match rules, so nothing they admit is
            // missed.
            let messages = MessageIterator::from(&connection);
            for rule in [
                format!("type='signal',sender='{SERVICE}',interface='{SERVICE}'"),
                format!(
                    "type='signal',sender='{BUS}',interface='{BUS}',member='NameOwnerChanged',arg0='{SERVICE}'"
                ),
            ] {
                connection.call_method(Some(BUS), BUS_PATH, Some(BUS), "AddMatch", &rule)?;
            }
            thread::Builder::new()
                .name("eon-detach-notice-signals".into())
                .spawn(move || listen(messages, &inputs))
                .map_err(|error| zbus::Error::Failure(error.to_string()))?;
            Ok(Self {
                connection,
                application_id: application_id.to_owned(),
                reader,
            })
        }

        fn call<B>(&self, destination: &str, method: &str, body: &B) -> Result<Message, Failure>
        where
            B: serde::Serialize + zbus::zvariant::DynamicType,
        {
            self.connection
                .call_method(Some(destination), SERVICE_PATH, Some(SERVICE), method, body)
                .map_err(|error| Failure {
                    gone: matches!(
                        &error,
                        zbus::Error::MethodError(name, ..) if matches!(
                            name.as_str(),
                            "org.freedesktop.DBus.Error.ServiceUnknown"
                                | "org.freedesktop.DBus.Error.NameHasNoOwner"
                        )
                    ),
                    detail: error.to_string(),
                })
        }
    }

    /// The reply's body and the unique name that sent it.
    fn answer<T>(reply: &Message) -> Result<(T, String), Failure>
    where
        T: serde::de::DeserializeOwned + zbus::zvariant::Type,
    {
        let failure = |detail: String| Failure {
            gone: false,
            detail,
        };
        let sender = reply
            .header()
            .sender()
            .map(ToString::to_string)
            .ok_or_else(|| failure("the reply names no sender".into()))?;
        let body = reply
            .body()
            .deserialize::<T>()
            .map_err(|error| failure(error.to_string()))?;
        Ok((body, sender))
    }

    impl Server for DBus {
        fn capabilities(&self) -> Result<(Vec<String>, String), Failure> {
            answer(&self.call(SERVICE, "GetCapabilities", &())?)
        }

        fn notify(&self, notification: &Notification) -> Result<(u32, String), Failure> {
            // Eon raises no window for it; `desktop-entry` only names Eon's
            // icon and launcher to the server. `resident` keeps a notice with
            // actions after one is chosen, as GNOME otherwise removes it, so
            // Quit's confirmation can replace it in place.
            let mut hints =
                HashMap::from([("desktop-entry", Value::from(self.application_id.as_str()))]);
            if !notification.actions.is_empty() {
                hints.insert("resident", Value::from(true));
            }
            match notification.sound {
                Sound::Default => {}
                Sound::Silent => {
                    hints.insert("suppress-sound", Value::from(true));
                }
                Sound::Theme => {
                    hints.insert("sound-name", Value::from("message-new-instant"));
                }
            }
            answer(&self.call(
                &notification.destination,
                "Notify",
                &(
                    "Eon",
                    notification.replaces,
                    "",
                    notification.summary.as_str(),
                    notification.body.as_str(),
                    notification.actions.to_vec(),
                    hints,
                    -1i32,
                ),
            )?)
        }

        fn close(&self, id: u32, owner: &str) -> Result<(), Failure> {
            self.call(owner, "CloseNotification", &id).map(drop)
        }

        fn pause_reader(&self) -> Option<MutexGuard<'_, bool>> {
            Some(self.reader.stopped.lock().expect("notification reader"))
        }
    }

    impl Drop for DBus {
        /// Closing the connection ends the signal thread's wait.
        fn drop(&mut self) {
            let _ = self.connection.clone().close();
        }
    }

    /// Passes on the bus's notification and owner-change signals; the
    /// worker's notice decides which to believe.
    fn listen(messages: MessageIterator, inputs: &InputSender) {
        for message in messages {
            let Ok(message) = message else {
                break;
            };
            let Some(signal) = signal(&message) else {
                continue;
            };
            if inputs.send(Input::Signal(signal)).is_err() {
                return;
            }
        }
        let _ = inputs.send(Input::Disconnected);
    }

    fn signal(message: &Message) -> Option<Signal> {
        let header = message.header();
        if header.message_type() != Type::Signal {
            return None;
        }
        let sender = header.sender()?.to_string();
        let body = message.body();
        Some(
            match (header.interface()?.as_str(), header.member()?.as_str()) {
                (SERVICE, "ActionInvoked") => {
                    let (id, key) = body.deserialize::<(u32, String)>().ok()?;
                    Signal::Action { sender, id, key }
                }
                (SERVICE, "NotificationClosed") => {
                    let (id, _reason) = body.deserialize::<(u32, u32)>().ok()?;
                    Signal::Closed { sender, id }
                }
                (BUS, "NameOwnerChanged") => {
                    let (name, old, new) = body.deserialize::<(String, String, String)>().ok()?;
                    Signal::OwnerChanged {
                        sender,
                        name,
                        old,
                        new,
                    }
                }
                _ => return None,
            },
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::notification::QUIT;
        use crate::notification::QUIT_CONFIRM;
        use std::{
            cell::Cell,
            sync::{Mutex, atomic::AtomicBool},
            time::Instant,
        };

        #[derive(Debug, PartialEq, Eq)]
        enum Recorded {
            Notify(Notification),
            Close(u32, String),
        }

        /// A server at `:1.7` that records each call and numbers its
        /// notifications from 1, keeping a replaced one's ID.
        struct Fake(Arc<Mutex<Vec<Recorded>>>);

        impl Server for Fake {
            fn capabilities(&self) -> Result<(Vec<String>, String), Failure> {
                Ok((vec!["body".into(), "actions".into()], ":1.7".into()))
            }

            fn notify(&self, notification: &Notification) -> Result<(u32, String), Failure> {
                let mut calls = self.0.lock().unwrap();
                calls.push(Recorded::Notify(notification.clone()));
                let id = match notification.replaces {
                    0 => u32::try_from(calls.len()).unwrap(),
                    replaces => replaces,
                };
                Ok((id, ":1.7".into()))
            }

            fn close(&self, id: u32, owner: &str) -> Result<(), Failure> {
                self.0
                    .lock()
                    .unwrap()
                    .push(Recorded::Close(id, owner.into()));
                Ok(())
            }
        }

        fn window() -> NoticeWindow {
            NoticeWindow {
                name: "w".into(),
                id: None,
                stop_command: "eon window stop default".into(),
                terminals: 1,
            }
        }

        fn log() -> Arc<DiagnosticLog> {
            DiagnosticLog::open_in(Err("no log in this test".into()), "g1", "eon")
        }

        /// Two detaches go through the call `reap_presentation` makes, so
        /// gating the notice on an earlier one, as the once-per-user notice
        /// did, fails here.
        #[test]
        fn every_detach_shows_a_notice() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&calls);
            let mut notices = Notices::default();
            notices.connect = Arc::new(move |_, _| Ok(Box::new(Fake(Arc::clone(&recorded)))));
            let log = log();
            notices.show(window(), "eon", &log);
            notices.show(window(), "eon", &log);
            notices.close_and_wait(Duration::from_secs(30));
            let shown = |destination: &str, replaces| {
                Recorded::Notify(Notification {
                    destination: destination.into(),
                    replaces,
                    summary: "Eon window w is still running".into(),
                    body: "1 terminal keeps running in the background. Open Eon to return.".into(),
                    actions: &[QUIT, "Quit"],
                    sound: Sound::Default,
                })
            };
            assert_eq!(
                *calls.lock().unwrap(),
                [
                    shown(":1.7", 0),
                    // The second detach replaces the first notice at the
                    // server that issued it.
                    shown(":1.7", 1),
                    // The supervisor's end closes it there.
                    Recorded::Close(1, ":1.7".into()),
                ]
            );
        }

        const WAIT: Duration = Duration::from_secs(5);

        /// Report each call before reading the test's selected result.
        struct GatedServer {
            calls: mpsc::Sender<Notification>,
            replies: mpsc::Receiver<Result<(), Failure>>,
            fail_capabilities: Cell<bool>,
        }

        impl Server for GatedServer {
            fn capabilities(&self) -> Result<(Vec<String>, String), Failure> {
                if self.fail_capabilities.replace(false) {
                    return Err(Failure {
                        gone: false,
                        detail: "timeout".into(),
                    });
                }
                Ok((vec!["actions".into()], ":1.8".into()))
            }

            fn notify(&self, notification: &Notification) -> Result<(u32, String), Failure> {
                self.calls.send(notification.clone()).unwrap();
                self.replies.recv_timeout(WAIT).unwrap()?;
                Ok((1, ":1.8".into()))
            }

            fn close(&self, _: u32, _: &str) -> Result<(), Failure> {
                Ok(())
            }
        }

        fn gated(
            notify_names: bool,
        ) -> (
            Notices,
            mpsc::Receiver<Notification>,
            mpsc::Sender<Result<(), Failure>>,
        ) {
            let (calls, observed) = mpsc::channel();
            let (replies, answers) = mpsc::channel();
            let answers = Mutex::new(Some(answers));
            let mut notices = Notices::new(AttentionNotifications {
                notify: true,
                notify_names,
            });
            notices.connect = Arc::new(move |_, _| {
                Ok(Box::new(GatedServer {
                    calls: calls.clone(),
                    replies: answers.lock().unwrap().take().unwrap(),
                    fail_capabilities: Cell::default(),
                }))
            });
            (notices, observed, replies)
        }

        fn barrier(inputs: &InputSender) {
            let (done, finished) = mpsc::channel();
            assert!(inputs.send(Input::Barrier(done)).is_ok());
            finished.recv_timeout(WAIT).unwrap();
        }

        #[test]
        fn update_park_drains_attention_calls_and_preserves_detach_actions() {
            let (mut notices, observed, replies) = gated(true);
            let log = log();
            let offer = |notices: &mut Notices, name: &str| {
                assert!(
                    notices.attention(NoticeKind::Error, "eon", &log, Arc::default(), |_| name
                        .into())
                );
            };
            offer(&mut notices, "in flight");
            assert!(
                observed
                    .recv_timeout(WAIT)
                    .unwrap()
                    .body
                    .contains("in flight")
            );
            offer(&mut notices, "queued");
            let mut parked = notices.park().unwrap();
            assert_eq!(parked.settled(), Ok(false));
            replies.send(Ok(())).unwrap();
            assert!(observed.recv_timeout(WAIT).unwrap().body.contains("queued"));
            assert_eq!(
                parked.settled(),
                Ok(false),
                "dispatch is earlier than call completion"
            );
            replies.send(Ok(())).unwrap();
            assert!(parked.ready.as_ref().unwrap().recv_timeout(WAIT).unwrap());
            drop(parked);

            notices.show(window(), "eon", &log);
            assert_eq!(
                observed.recv_timeout(WAIT).unwrap().actions,
                &[QUIT, "Quit"]
            );
            replies.send(Ok(())).unwrap();
            let parked = notices.park().unwrap();
            assert!(!parked.ready.as_ref().unwrap().recv_timeout(WAIT).unwrap());
            drop(parked);
            let inject = |signal| notices.send(Input::Signal(signal));
            let worker = &notices.worker.as_ref().unwrap().inputs;
            let action = |key: &str| Signal::Action {
                sender: ":1.8".into(),
                id: 1,
                key: key.into(),
            };
            inject(action(QUIT));
            barrier(worker);
            assert!(notices.has_pending_event());
            assert_eq!(notices.next_event(), Some(Event::Confirm));
            notices.confirm(window());
            assert_eq!(
                observed.recv_timeout(WAIT).unwrap().actions[0],
                QUIT_CONFIRM
            );
            replies.send(Ok(())).unwrap();
            inject(action(QUIT_CONFIRM));
            inject(Signal::Closed {
                sender: ":1.8".into(),
                id: 1,
            });
            let parked = notices.park().unwrap();
            assert!(parked.ready.as_ref().unwrap().recv_timeout(WAIT).unwrap());
            assert!(notices.has_pending_event());
            assert!(notices.has_pending_event());
            drop(parked);
            assert_eq!(notices.next_event(), Some(Event::Quit));
            assert_eq!(notices.next_event(), None);
            notices.close_and_wait(WAIT);
        }

        #[derive(Debug, PartialEq, Eq)]
        enum BusCall {
            Capabilities,
            Notify(u32, Vec<String>),
            Close(u32),
        }

        struct BusService(mpsc::Sender<BusCall>);

        #[zbus::interface(name = "org.freedesktop.Notifications")]
        impl BusService {
            fn get_capabilities(&self) -> Vec<String> {
                self.0.send(BusCall::Capabilities).unwrap();
                vec!["actions".into()]
            }

            #[expect(clippy::too_many_arguments, reason = "the D-Bus Notify signature")]
            fn notify(
                &self,
                _app_name: &str,
                replaces: u32,
                _app_icon: &str,
                _summary: &str,
                _body: &str,
                actions: Vec<String>,
                _hints: HashMap<String, zbus::zvariant::OwnedValue>,
                _expire_timeout: i32,
            ) -> u32 {
                let id = if actions.is_empty() { 2 } else { 1 };
                self.0.send(BusCall::Notify(replaces, actions)).unwrap();
                id
            }

            fn close_notification(&self, id: u32) {
                self.0.send(BusCall::Close(id)).unwrap();
            }
        }

        struct PrivateBus {
            child: std::process::Child,
            root: std::path::PathBuf,
        }

        impl Drop for PrivateBus {
            fn drop(&mut self) {
                let _ = self.child.kill();
                let _ = self.child.wait();
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }

        #[test]
        fn update_park_stops_and_resumes_the_real_notification_bus_reader() {
            use std::{fs, os::unix::net::UnixStream, process::Stdio};

            const BUS_ADDRESS: &str = "EON_TEST_PRIVATE_NOTIFICATION_BUS";
            if let Ok(address) = std::env::var(BUS_ADDRESS) {
                notification_bus_roundtrip(address);
                return;
            }
            let root = crate::supervisor::temporary_directory();
            let socket = root.join("bus");
            let address = format!("unix:path={}", socket.display());
            let config = root.join("dbus.conf");
            // Use an isolated bus with no activation directories or host services.
            fs::write(
                &config,
                "<busconfig><type>session</type><listen>unix:tmpdir=/tmp</listen>
                 <auth>EXTERNAL</auth><policy context='default'>
                 <allow send_destination='*'/><allow receive_sender='*'/><allow own='*'/>
                 </policy></busconfig>",
            )
            .unwrap();
            let mut daemon = PrivateBus {
                child: std::process::Command::new("dbus-daemon")
                    .arg("--nofork")
                    .arg(format!("--config-file={}", config.display()))
                    .arg(format!("--address={address}"))
                    .stdin(Stdio::null())
                    .spawn()
                    .unwrap(),
                root,
            };
            let deadline = Instant::now() + WAIT;
            while UnixStream::connect(&socket).is_err() {
                assert!(
                    daemon.child.try_wait().unwrap().is_none(),
                    "private bus exited"
                );
                assert!(Instant::now() < deadline, "private bus did not start");
                thread::sleep(Duration::from_millis(5));
            }

            // zbus's method timeout does not cover its initial bus handshake.
            // Bound the whole round trip in a child so even a failed connection
            // cannot hold the test runner or leave the private bus running.
            let mut test = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "notification::linux::tests::update_park_stops_and_resumes_the_real_notification_bus_reader",
                    "--nocapture",
                ])
                .env(BUS_ADDRESS, address)
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + WAIT * 3;
            loop {
                if let Ok(Some(status)) = test.try_wait() {
                    assert!(status.success(), "notification bus round trip: {status}");
                    break;
                }
                if Instant::now() >= deadline {
                    let _ = test.kill();
                    let _ = test.wait();
                    panic!("notification bus round trip exceeded its deadline");
                }
                thread::sleep(Duration::from_millis(5));
            }
        }

        fn notification_bus_roundtrip(address: String) {
            use zbus::blocking::connection::Builder;

            let (calls, called) = mpsc::channel();
            let server = Builder::address(address.as_str())
                .unwrap()
                .name(SERVICE)
                .unwrap()
                .serve_at(SERVICE_PATH, BusService(calls))
                .unwrap()
                .build()
                .unwrap();
            let expect_call = |call| assert_eq!(called.recv_timeout(WAIT).unwrap(), call);
            let (client_tx, client_rx) = mpsc::channel();
            let mut notices = Notices::new(AttentionNotifications {
                notify: true,
                notify_names: true,
            });
            notices.connect = Arc::new(move |application_id, inputs| {
                let builder =
                    zbus::connection::Builder::address(address.as_str())?.method_timeout(WAIT);
                let bus = DBus::connect_on(builder, application_id, inputs)?;
                client_tx.send(bus.connection.clone()).unwrap();
                Ok(Box::new(bus))
            });
            let log = log();
            let action = |notices: &Notices, key: &str, event| {
                server
                    .emit_signal(
                        None::<&str>,
                        SERVICE_PATH,
                        SERVICE,
                        "ActionInvoked",
                        &(1u32, key),
                    )
                    .unwrap();
                assert_eq!(notices.events.1.recv_timeout(WAIT).unwrap(), event);
            };
            let buttons = |keys: &[&str]| keys.iter().map(|key| (*key).to_owned()).collect();

            notices.show(window(), "eon", &log);
            let client = client_rx.recv_timeout(WAIT).unwrap();
            expect_call(BusCall::Capabilities);
            expect_call(BusCall::Notify(0, buttons(&[QUIT, "Quit"])));
            action(&notices, QUIT, Event::Confirm);
            notices.confirm(window());
            expect_call(BusCall::Notify(
                1,
                buttons(&[QUIT_CONFIRM, "Quit", super::super::KEEP, "Keep running"]),
            ));
            action(&notices, QUIT_CONFIRM, Event::Quit);
            notices.close();
            expect_call(BusCall::Close(1));

            let park = notices.park().unwrap();
            assert!(park.ready.as_ref().unwrap().recv_timeout(WAIT).unwrap());
            let (reply, received) = mpsc::channel();
            let worker = thread::spawn(move || {
                let capabilities = client.call_method(
                    Some(SERVICE),
                    SERVICE_PATH,
                    Some(SERVICE),
                    "GetCapabilities",
                    &(),
                );
                reply.send(capabilities).unwrap();
            });
            expect_call(BusCall::Capabilities);
            // The actual peer has handled the call. The same client cannot read
            // the reply until the supervisor releases the executor.
            assert!(matches!(
                received.recv_timeout(Duration::from_millis(100)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            drop(park);
            assert!(received.recv_timeout(WAIT).unwrap().is_ok());
            worker.join().unwrap();

            assert!(
                notices.attention(NoticeKind::Finished, "eon", &log, Arc::default(), |_| {
                    "private bus test".into()
                })
            );
            expect_call(BusCall::Notify(0, Vec::new()));
            notices.close_and_wait(WAIT);
        }

        #[test]
        fn attention_recovers_on_service_replacement_without_replaying_or_detaching() {
            let (mut notices, observed, replies) = gated(true);
            let log = log();
            let notify = |notices: &mut Notices, location: &str| {
                notices.attention(NoticeKind::Error, "eon", &log, Arc::default(), |names| {
                    assert!(names);
                    location.into()
                });
            };
            let read = || observed.recv_timeout(WAIT).unwrap();
            notify(&mut notices, "error while the service was down");
            assert_eq!(read().body, "error while the service was down");
            let worker = notices.worker.as_ref().unwrap().inputs.clone();
            notify(&mut notices, "old queued error");
            replies
                .send(Err(Failure {
                    gone: true,
                    detail: "ServiceUnknown".into(),
                }))
                .unwrap();
            barrier(&worker);
            notify(&mut notices, "error while paused");
            barrier(&worker);
            let restored = |sender: &str| Signal::OwnerChanged {
                sender: sender.into(),
                name: SERVICE.into(),
                old: String::new(),
                new: ":1.8".into(),
            };
            assert!(worker.send(Input::Signal(restored(":1.7"))).is_ok());
            barrier(&worker);
            notify(&mut notices, "error after spoofed owner signal");
            assert!(worker.send(Input::Signal(restored(BUS))).is_ok());
            barrier(&worker);
            notify(&mut notices, "fresh error under new owner");
            assert_eq!(read().body, "fresh error under new owner");
            replies
                .send(Err(Failure {
                    gone: false,
                    detail: "timeout".into(),
                }))
                .unwrap();
            barrier(&worker);
            notify(&mut notices, "fresh error after timeout");
            assert_eq!(read().body, "fresh error after timeout");
            replies.send(Ok(())).unwrap();
            notices.close_and_wait(WAIT);
            assert!(observed.try_recv().is_err());
        }

        #[test]
        fn full_attention_queue_keeps_detach_ahead_of_pending_alerts() {
            let (mut notices, observed, replies) = gated(false);
            let log = log();
            let notify = |notices: &mut Notices, label: String| {
                notices.attention(NoticeKind::Finished, "eon", &log, Arc::default(), |names| {
                    assert!(!names);
                    label
                })
            };
            let read = || observed.recv_timeout(WAIT).unwrap();
            assert!(notify(&mut notices, "active".into()));
            assert_eq!(read().body, "active");
            for index in 0..ATTENTION_CAPACITY {
                assert!(notify(&mut notices, format!("queued {index}")));
            }
            assert!(!notify(&mut notices, "overflow".into()));
            notices.show(window(), "eon", &log);
            replies.send(Ok(())).unwrap();
            let detach = read();
            assert_eq!(detach.actions, &[QUIT, "Quit"]);
            replies.send(Ok(())).unwrap();
            for index in 0..ATTENTION_CAPACITY {
                assert_eq!(read().body, format!("queued {index}"));
                replies.send(Ok(())).unwrap();
            }
            notices.close_and_wait(WAIT);
            assert!(observed.try_recv().is_err());
        }

        #[test]
        fn cancelled_attention_is_skipped_before_dispatch() {
            let (mut notices, observed, replies) = gated(false);
            let log = log();
            let mut offer = |label: &str, cancelled| {
                assert!(
                    notices.attention(NoticeKind::Finished, "eon", &log, cancelled, |_| label
                        .into())
                );
            };
            offer("active", Arc::default());
            assert_eq!(observed.recv_timeout(WAIT).unwrap().body, "active");
            let dispatch = Arc::new(NoticeDispatch::default());
            offer("now seen", Arc::clone(&dispatch));
            assert!(dispatch.cancel());
            offer("still unseen", Arc::default());
            replies.send(Ok(())).unwrap();
            assert_eq!(observed.recv_timeout(WAIT).unwrap().body, "still unseen");

            // With no detach to close, shutdown returns even while the last call hangs.
            let (returned, done) = mpsc::channel();
            let closer = thread::spawn(move || {
                notices.close_and_wait(WAIT * 2);
                returned.send(()).unwrap();
            });
            let stopped_without_waiting = done.recv_timeout(WAIT);
            replies.send(Ok(())).unwrap();
            closer.join().unwrap();
            stopped_without_waiting.unwrap();
        }

        #[test]
        fn controls_precede_an_alert_selected_from_the_other_queue() {
            let (sender, receiver) = mpsc::channel();
            let (_attention, pending) = mpsc::sync_channel(1);
            let mut selected = Some(PendingAttention {
                notification: Notification::attention(NoticeKind::Finished, "queued"),
                epoch: 1,
                dispatch: Arc::default(),
            });
            assert!(sender.send(Input::Command(Command::Show(window()))).is_ok());
            let park = || panic!("input is ready");
            assert!(matches!(
                next_input(&receiver, &pending, &mut selected, park),
                Some(Input::Command(Command::Show(_)))
            ));
            assert!(matches!(
                next_input(&receiver, &pending, &mut selected, park),
                Some(Input::Attention(_))
            ));
        }

        #[test]
        fn input_arriving_between_empty_check_and_park_wakes_the_worker() {
            let (sender, receiver) = mpsc::channel();
            let (announce, announced) = mpsc::channel();
            let may_park = Arc::new(AtomicBool::new(false));
            let release = Arc::clone(&may_park);
            let (done, finished) = mpsc::channel();
            let waiter = thread::spawn(move || {
                let (_attention, pending) = mpsc::sync_channel(1);
                let input = next_input(&receiver, &pending, &mut None, || {
                    announce.send(()).unwrap();
                    let deadline = Instant::now() + WAIT;
                    while !may_park.load(Relaxed) {
                        assert!(Instant::now() < deadline);
                        thread::yield_now();
                    }
                    thread::park_timeout(WAIT * 2);
                });
                done.send(matches!(
                    input,
                    Some(Input::Signal(Signal::Closed { id: 42, .. }))
                ))
                .unwrap();
            });
            let inputs = InputSender {
                sender,
                wake: waiter.thread().clone(),
            };
            announced.recv_timeout(WAIT).unwrap();
            assert!(
                inputs
                    .send(Input::Signal(Signal::Closed {
                        sender: SERVICE.into(),
                        id: 42
                    }))
                    .is_ok()
            );
            release.store(true, Relaxed);
            assert!(finished.recv_timeout(WAIT).unwrap());
            waiter.join().unwrap();
        }

        #[test]
        fn default_test_connector_refuses_the_bus() {
            let (sender, _) = mpsc::channel();
            let inputs = InputSender {
                sender,
                wake: thread::current(),
            };
            assert!((Notices::default().connect)("eon", inputs).is_err());
        }

        #[test]
        fn detach_retries_an_initial_bus_connection_failure() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&calls);
            let (attempted, attempts) = mpsc::channel();
            let count = AtomicUsize::new(0);
            let mut notices = Notices::new(AttentionNotifications {
                notify: true,
                notify_names: false,
            });
            notices.connect = Arc::new(move |_, _| {
                let index = count.fetch_add(1, Relaxed);
                attempted.send(index).unwrap();
                if index == 0 {
                    return Err(zbus::Error::Failure("private bus unavailable".into()));
                }
                Ok(Box::new(Fake(Arc::clone(&recorded))))
            });
            let log = log();
            let notify = |notices: &mut Notices| {
                notices.attention(NoticeKind::Error, "eon", &log, Arc::default(), |_| {
                    "lost".into()
                })
            };

            notify(&mut notices);
            assert_eq!(attempts.recv_timeout(WAIT).unwrap(), 0);
            barrier(&notices.worker.as_ref().unwrap().inputs);
            assert!(!notify(&mut notices));
            notices.show(window(), "eon", &log);
            assert_eq!(attempts.recv_timeout(WAIT).unwrap(), 1);
            notices.close_and_wait(WAIT);
            let records = calls.lock().unwrap();
            assert!(
                matches!(&records[..], [Recorded::Notify(notification), Recorded::Close(..)] if notification.actions == [QUIT, "Quit"])
            );
            assert!(attempts.try_recv().is_err());
        }

        #[test]
        fn detach_already_queued_behind_disconnect_reconnects_and_stays_paused_if_it_fails() {
            let (mut notices, observed, replies) = gated(false);
            let first = Arc::clone(&notices.connect);
            // Two fake connections, both attached to the same worker control queue.
            let (calls, recovered_calls) = mpsc::channel();
            let (answers, replies_after_reconnect) = mpsc::channel();
            let replies_after_reconnect = Mutex::new(Some(replies_after_reconnect));
            let (connected, connections) = mpsc::channel();
            let attempts = AtomicUsize::new(0);
            notices.connect = Arc::new(move |application, inputs| {
                let attempt = attempts.fetch_add(1, Relaxed);
                connected.send(attempt).unwrap();
                if attempt == 0 {
                    first(application, inputs)
                } else {
                    Ok(Box::new(GatedServer {
                        calls: calls.clone(),
                        replies: replies_after_reconnect.lock().unwrap().take().unwrap(),
                        fail_capabilities: Cell::new(true),
                    }))
                }
            });
            let log = log();
            let offer = |notices: &mut Notices, label: &str| {
                notices.attention(NoticeKind::Error, "eon", &log, Arc::default(), |_| {
                    label.into()
                })
            };
            assert!(offer(&mut notices, "holding the worker"));
            assert_eq!(connections.recv_timeout(WAIT).unwrap(), 0);
            assert_eq!(
                observed.recv_timeout(WAIT).unwrap().body,
                "holding the worker"
            );
            let inputs = notices.worker.as_ref().unwrap().inputs.clone();
            assert!(inputs.send(Input::Disconnected).is_ok());
            notices.show(window(), "eon", &log);
            assert!(offer(&mut notices, "old epoch"));
            replies.send(Ok(())).unwrap();
            assert_eq!(connections.recv_timeout(WAIT).unwrap(), 1);
            barrier(&inputs);
            assert!(!offer(
                &mut notices,
                "after reconnect, before a successful detach"
            ));
            notices.show(window(), "eon", &log);
            assert_eq!(
                recovered_calls.recv_timeout(WAIT).unwrap().actions,
                [QUIT, "Quit"]
            );
            answers.send(Ok(())).unwrap();
            barrier(&inputs);
            assert!(offer(&mut notices, "fresh after detach"));
            assert_eq!(
                recovered_calls.recv_timeout(WAIT).unwrap().body,
                "fresh after detach"
            );
            answers.send(Ok(())).unwrap();
            notices.close_and_wait(WAIT);
        }

        #[test]
        fn a_lost_worker_replacement_stays_paused_if_the_detach_fails() {
            let (mut notices, observed, answers) = gated(false);
            let (mut lost, receiver) = Notices::record_commands();
            notices.worker = lost.worker.take();
            notices.attempted = true;
            drop(receiver);
            let log = log();
            notices.show(window(), "eon", &log);
            assert!(
                !notices.attention(NoticeKind::Error, "eon", &log, Arc::default(), |_| {
                    "paused".into()
                })
            );
            assert_eq!(observed.recv_timeout(WAIT).unwrap().actions, [QUIT, "Quit"]);
            answers
                .send(Err(Failure {
                    gone: false,
                    detail: "timeout".into(),
                }))
                .unwrap();
            barrier(&notices.worker.as_ref().unwrap().inputs);
            assert!(
                !notices.attention(NoticeKind::Error, "eon", &log, Arc::default(), |_| {
                    "still paused".into()
                })
            );
            notices.close_and_wait(WAIT);
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) use other::Notices;

#[cfg(not(target_os = "linux"))]
mod other {
    use super::{Event, NoticeDispatch, NoticeKind, NoticePark, NoticeWindow};
    use crate::{AttentionNotifications, diagnostics::DiagnosticLog};
    use std::{sync::Arc, time::Duration};

    /// macOS has no freedesktop notification server; a detach sends nothing.
    #[derive(Default)]
    pub(crate) struct Notices {}

    impl Notices {
        pub(crate) fn new(_settings: AttentionNotifications) -> Self {
            Self::default()
        }
        pub(crate) fn attention(
            &mut self,
            _state: NoticeKind,
            _application_id: &str,
            _log: &Arc<DiagnosticLog>,
            _dispatch: Arc<NoticeDispatch>,
            _location: impl FnOnce(bool) -> String,
        ) -> bool {
            false
        }
        pub(crate) fn show(
            &mut self,
            _window: NoticeWindow,
            _application_id: &str,
            _log: &Arc<DiagnosticLog>,
        ) {
        }
        pub(crate) fn confirm(&self, _window: NoticeWindow) {}
        pub(crate) fn failure(&self, _window: NoticeWindow) {}
        pub(crate) fn close(&self) {}
        pub(crate) fn next_event(&self) -> Option<Event> {
            None
        }
        pub(crate) fn has_pending_event(&self) -> bool {
            false
        }
        pub(crate) fn park(&self) -> Option<NoticePark> {
            None
        }
        pub(crate) fn close_and_wait(&mut self, _timeout: Duration) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(name: &str, id: Option<&str>, terminals: u16) -> NoticeWindow {
        NoticeWindow::new(&WindowEntry {
            created_build: Some("g1-00000000000000000000000000000001".into()),
            active_build: Some("g1-00000000000000000000000000000001".into()),
            name: Some(name.into()),
            window: id.map(Into::into),
            generation: "g1-0123456789abcdef0123456789abcdef".into(),
            started_at: None,
            live_terminals: Some(terminals),
            state: eon_workspace_protocol::v50::WindowState::Live,
        })
    }

    #[test]
    fn notice_names_the_window_counts_terminals_and_falls_back_to_targeted_stop() {
        assert_eq!(
            notice_text(&window("refs", None, 1), true),
            (
                "Eon window refs is still running".into(),
                "1 terminal keeps running in the background. Open Eon to return.".into()
            )
        );
        assert_eq!(
            notice_text(&window("~", None, 0), true).1,
            "No terminal is running yet. Open Eon to return."
        );
        assert_eq!(
            notice_text(&window("refs", None, 1), false).1,
            "1 terminal keeps running in the background. Open Eon to return. \
             Run `eon window stop 01234567` to stop it."
        );
        assert_eq!(
            notice_text(&window("eon", Some("0a1b2c3d"), 3), false),
            (
                "Eon window eon is still running".into(),
                "3 terminals keep running in the background. \
                 Run `eon window attach 0a1b2c3d` to return. Run `eon window stop 0a1b2c3d` to stop it."
                    .into()
            )
        );
        // Titles are plain text; bodies escape markup.
        assert_eq!(
            notice_text(&window("<a&b>", Some("<&>"), 2), false),
            (
                "Eon window <a&b> is still running".into(),
                "2 terminals keep running in the background. \
                 Run `eon window attach &lt;&amp;&gt;` to return. \
                 Run `eon window stop &lt;&amp;&gt;` to stop it."
                    .into()
            )
        );
        assert_eq!(
            confirm_text(&window("<a&b>", None, 1)),
            (
                "Quit <a&b>?".into(),
                "This stops its 1 terminal. Eon cannot restore it.".into()
            )
        );
        assert_eq!(
            confirm_text(&window("w", None, 4)).1,
            "This stops its 4 terminals. Eon cannot restore them."
        );
    }

    #[test]
    fn failed_stop_names_the_exact_window_to_retry() {
        assert_eq!(
            failure_text(&window("w", None, 4)),
            (
                "Eon could not quit w".into(),
                "Some terminals may still run; run `eon window stop 01234567` to try again.".into()
            )
        );
        assert_eq!(
            failure_text(&window("w", Some("0a1b2c3d"), 4)).1,
            "Some terminals may still run; run `eon window stop 0a1b2c3d` to try again."
        );
        assert_eq!(
            failure_text(&window("<a&b>", Some("<&>"), 2)).1,
            "Some terminals may still run; run `eon window stop &lt;&amp;&gt;` to try again."
        );
    }

    fn action(sender: &str, id: u32, key: &str) -> Signal {
        Signal::Action {
            sender: sender.into(),
            id,
            key: key.into(),
        }
    }

    fn closed(sender: &str, id: u32) -> Signal {
        Signal::Closed {
            sender: sender.into(),
            id,
        }
    }

    fn owner_left(sender: &str, name: &str, old: &str) -> Signal {
        Signal::OwnerChanged {
            sender: sender.into(),
            name: name.into(),
            old: old.into(),
            new: String::new(),
        }
    }

    #[test]
    fn fresh_notice_uses_the_capability_owner() {
        let notice = Notice {
            actions: Some((":1.7".into(), true)),
            ..Notice::default()
        };
        let Some(Call::Notify(notification, _)) = notice.call(&Command::Show(window("w", None, 1)))
        else {
            panic!("a detach sends a notification");
        };
        // The well-known name may have changed hands since GetCapabilities;
        // its new owner need not support these buttons.
        assert_eq!(notification.destination, ":1.7");
        assert_eq!(notification.actions, &[QUIT, "Quit"]);
    }

    #[test]
    fn notice_acts_only_on_its_own_notification_and_confirms_before_quit() {
        let mut notice = Notice::default();
        assert_eq!(notice.on_signal(action(":1.7", 7, QUIT)), None);
        notice.sent(7, ":1.7".into(), Stage::Shown);
        // Another notification's action, such as another window's under a
        // broadcasting server, does nothing.
        assert_eq!(notice.on_signal(action(":1.7", 8, QUIT)), None);
        // The notice's own Quit never stops without the confirmation.
        assert_eq!(notice.on_signal(action(":1.7", 7, QUIT_CONFIRM)), None);
        assert_eq!(notice.on_signal(action(":1.7", 7, KEEP)), None);
        assert_eq!(
            notice.on_signal(action(":1.7", 7, QUIT)),
            Some(Effect::Event(Event::Confirm))
        );
        notice.sent(7, ":1.7".into(), Stage::Confirming);
        assert_eq!(notice.on_signal(action(":1.7", 8, QUIT_CONFIRM)), None);
        assert_eq!(notice.on_signal(action(":1.7", 7, QUIT)), None);
        assert_eq!(
            notice.on_signal(action(":1.7", 7, QUIT_CONFIRM)),
            Some(Effect::Event(Event::Quit))
        );
        assert_eq!(
            notice.on_signal(action(":1.7", 7, KEEP)),
            Some(Effect::Close)
        );
        // A failure notice offers nothing.
        notice.sent(9, ":1.7".into(), Stage::Failed);
        assert_eq!(notice.on_signal(action(":1.7", 9, QUIT)), None);
        assert_eq!(notice.on_signal(action(":1.7", 9, QUIT_CONFIRM)), None);
        // A dismissed notice is forgotten.
        notice.sent(10, ":1.7".into(), Stage::Confirming);
        assert_eq!(notice.on_signal(closed(":1.7", 11)), None);
        assert_eq!(notice.on_signal(closed(":1.7", 10)), None);
        assert_eq!(notice.on_signal(action(":1.7", 10, QUIT_CONFIRM)), None);
        assert_eq!(notice.call(&Command::Close), None);
    }

    /// EON-C8's security clause: only the server that issued the ID can act
    /// on it or close it, only the bus reports owner changes, and an owner
    /// change forgets only what the leaving owner issued.
    #[test]
    fn notice_believes_only_the_issuing_server_and_the_bus() {
        let mut notice = Notice {
            actions: Some((":1.7".into(), true)),
            ..Notice::default()
        };
        notice.sent(7, ":1.7".into(), Stage::Confirming);
        // Another client's signals, or the well-known name's, prove nothing.
        assert_eq!(notice.on_signal(action(":1.66", 7, QUIT_CONFIRM)), None);
        assert_eq!(notice.on_signal(action(SERVICE, 7, QUIT_CONFIRM)), None);
        assert_eq!(notice.on_signal(closed(":1.66", 7)), None);
        // A spoofed owner change, or one for another name, forgets nothing.
        assert_eq!(notice.on_signal(owner_left(":1.66", SERVICE, ":1.7")), None);
        assert_eq!(
            notice.on_signal(owner_left(BUS, "org.example", ":1.7")),
            None
        );
        // An owner change queued before this notice was sent names an
        // earlier owner, and forgets nothing.
        assert_eq!(
            notice.on_signal(owner_left(BUS, SERVICE, ":1.5")),
            Some(Effect::ServiceAvailable(false))
        );
        assert_eq!(
            notice.on_signal(owner_left(BUS, SERVICE, "")),
            Some(Effect::ServiceAvailable(false))
        );
        assert!(notice.actions.is_some());
        assert_eq!(
            notice.on_signal(action(":1.7", 7, QUIT_CONFIRM)),
            Some(Effect::Event(Event::Quit))
        );
        // Replacements and closes go to the issuing server, not to whoever
        // owns the well-known name now.
        assert_eq!(
            notice.call(&Command::Close),
            Some(Call::Close {
                id: 7,
                owner: ":1.7".into()
            })
        );
        let Some(Call::Notify(replacement, _)) = notice.call(&Command::Show(window("w", None, 1)))
        else {
            panic!("a detach sends a notification");
        };
        assert_eq!(
            (replacement.destination.as_str(), replacement.replaces),
            (":1.7", 7)
        );
        // The issuing server leaving forgets its notice and capabilities, and
        // the next notification goes to the well-known name afresh.
        assert_eq!(
            notice.on_signal(owner_left(BUS, SERVICE, ":1.7")),
            Some(Effect::ServiceAvailable(false))
        );
        assert_eq!(notice.actions, None);
        assert_eq!(notice.on_signal(action(":1.7", 7, QUIT_CONFIRM)), None);
        let Some(Call::Notify(fresh, _)) = notice.call(&Command::Show(window("w", None, 1))) else {
            panic!("a detach sends a notification");
        };
        assert_eq!((fresh.destination.as_str(), fresh.replaces), (SERVICE, 0));
        // Without the capability read again, the notice names its stop command.
        assert!(fresh.actions.is_empty());
    }

    #[test]
    fn notice_keeps_its_id_until_the_close_succeeds() {
        let w = || window("w", None, 1);
        let mut notice = Notice {
            actions: Some((":1.7".into(), true)),
            ..Notice::default()
        };
        let Some(Call::Notify(first, _)) = notice.call(&Command::Show(w())) else {
            panic!("a detach sends a notification");
        };
        assert_eq!(
            (first.destination.as_str(), first.replaces, first.actions),
            (":1.7", 0, &[QUIT, "Quit"][..])
        );
        notice.sent(5, ":1.7".into(), Stage::Shown);
        let close = Some(Call::Close {
            id: 5,
            owner: ":1.7".into(),
        });
        // Asking for the close keeps the ID: a failed close is retried by
        // the next close, and a detach before then replaces the notice.
        assert_eq!(notice.call(&Command::Close), close);
        assert_eq!(notice.call(&Command::Close), close);
        let Some(Call::Notify(next, stage)) = notice.call(&Command::Confirm(w())) else {
            panic!("a confirmation sends a notification");
        };
        assert_eq!(
            (next.destination.as_str(), next.replaces, stage),
            (":1.7", 5, Stage::Confirming)
        );
        assert_eq!(
            (next.summary.as_str(), next.body.as_str(), next.actions),
            (
                "Quit w?",
                "This stops its 1 terminal. Eon cannot restore it.",
                &[QUIT_CONFIRM, "Quit", KEEP, "Keep running"][..]
            )
        );
        // Another server's close of the same ID is not this close.
        notice.closed(5, ":1.8");
        assert_eq!(notice.call(&Command::Close), close);
        notice.closed(5, ":1.7");
        assert_eq!(notice.call(&Command::Close), None);
        // A server that removed the notice on the action gets a fresh
        // confirmation.
        let Some(Call::Notify(fresh, _)) = notice.call(&Command::Confirm(w())) else {
            panic!("a confirmation sends a notification");
        };
        assert_eq!((fresh.destination.as_str(), fresh.replaces), (":1.7", 0));
    }
}
