//! The desktop notice that a detached Eon window keeps running, with its Quit
//! action (EON-C8). It is a freedesktop notification, so only Linux sends it;
//! on macOS the native route needs the app bundle's notification permission
//! and belongs with that bundle's owner.
//!
//! The text and the notice's state machine are plain data, tested on every
//! platform. On Linux, `Notices` owns one session-bus connection on a worker
//! thread, started at the first detach and kept for the supervisor's life, so
//! the notification server can deliver the notice's actions back to it.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use super::{running::window_label, windows::durable_window_id};
use eon_workspace_protocol::v44::WindowEntry;

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
    /// The issuing owner when replacing, else the capability reply's sender.
    destination: String,
    replaces: u32,
    summary: String,
    body: String,
    /// Identifier and label pairs.
    actions: &'static [&'static str],
    stage: Stage,
}

/// One call to the notification server.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Call {
    Notify(Notification),
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
        Some(Call::Notify(Notification {
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
            stage,
        }))
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
            Signal::OwnerChanged { sender, name, old } => {
                if sender == BUS && name == SERVICE {
                    self.forget(&old);
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
        BUS, Call, Command, Effect, Event, Notice, NoticeWindow, Notification, SERVICE, Signal,
    };
    use crate::diagnostics::DiagnosticLog;
    use std::{
        collections::HashMap,
        sync::{Arc, mpsc},
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

    enum Input {
        Command(Command),
        Signal(Signal),
        /// The bus connection ended.
        Disconnected,
        /// The supervisor is ending; the worker drops the sender as it exits.
        Finish(mpsc::Sender<()>),
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
    }

    /// Opens a server for the desktop entry `application_id`, passing its
    /// signals to the worker's inputs.
    type Connect =
        Arc<dyn Fn(&str, mpsc::Sender<Input>) -> zbus::Result<Box<dyn Server>> + Send + Sync>;

    /// The supervisor's handle on its window's notice. The worker starts at
    /// the first `show`, and again at a later `show` after its connection
    /// failed, so a missing or failing notification service never delays a
    /// detach, and a later detach tries again.
    pub(crate) struct Notices {
        worker: Option<mpsc::Sender<Input>>,
        events: (mpsc::Sender<Event>, mpsc::Receiver<Event>),
        connect: Connect,
    }

    impl Default for Notices {
        fn default() -> Self {
            Self {
                worker: None,
                events: mpsc::channel(),
                connect: Arc::new(DBus::connect),
            }
        }
    }

    impl Notices {
        /// Shows the window's notice; `application_id` names Eon's desktop
        /// entry to the server.
        pub(crate) fn show(
            &mut self,
            window: NoticeWindow,
            application_id: &str,
            log: &Arc<DiagnosticLog>,
        ) {
            let mut input = Input::Command(Command::Show(window));
            if let Some(worker) = &self.worker {
                match worker.send(input) {
                    Ok(()) => return,
                    Err(mpsc::SendError(unsent)) => input = unsent,
                }
            }
            let (sender, receiver) = mpsc::channel();
            let worker = Worker {
                log: Arc::clone(log),
                events: self.events.0.clone(),
                notice: Notice::default(),
                reported: false,
            };
            let connect = Arc::clone(&self.connect);
            let application_id = application_id.to_owned();
            let inputs = sender.clone();
            // A refused thread only loses the notice.
            self.worker = thread::Builder::new()
                .name("eon-detach-notice".into())
                .spawn(move || worker.run(&connect, &application_id, inputs, &receiver))
                .ok()
                .map(|_| sender);
            self.send(input);
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
                let _ = worker.send(input);
            }
        }

        pub(crate) fn next_event(&self) -> Option<Event> {
            self.events.1.try_recv().ok()
        }

        #[cfg(test)]
        pub(crate) fn record_closes() -> (Self, impl Fn() -> bool) {
            let (sender, receiver) = mpsc::channel();
            (
                Self {
                    worker: Some(sender),
                    ..Self::default()
                },
                move || matches!(receiver.try_recv(), Ok(Input::Command(Command::Close))),
            )
        }

        #[cfg(test)]
        pub(crate) fn record_shows() -> (Self, impl Fn() -> Option<NoticeWindow>) {
            let (sender, receiver) = mpsc::channel();
            (
                Self {
                    worker: Some(sender),
                    ..Self::default()
                },
                move || match receiver.try_recv().ok()? {
                    Input::Command(Command::Show(window)) => Some(window),
                    _ => None,
                },
            )
        }

        /// Closes the notice as the supervisor ends, waiting at most
        /// `timeout` for the server.
        pub(crate) fn close_and_wait(&mut self, timeout: Duration) {
            let Some(worker) = self.worker.take() else {
                return;
            };
            let (done, finished) = mpsc::channel();
            if worker.send(Input::Command(Command::Close)).is_ok()
                && worker.send(Input::Finish(done)).is_ok()
            {
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
    }

    impl Worker {
        fn run(
            mut self,
            connect: &Connect,
            application_id: &str,
            signals: mpsc::Sender<Input>,
            inputs: &mpsc::Receiver<Input>,
        ) {
            let server = match connect(application_id, signals) {
                Ok(server) => server,
                Err(error) => {
                    self.log.record(&format!(
                        "desktop notifications are unavailable: cannot connect to the session bus: {error}"
                    ));
                    return;
                }
            };
            while let Ok(input) = inputs.recv() {
                match input {
                    Input::Command(command) => self.carry_out(&*server, command),
                    Input::Signal(signal) => match self.notice.on_signal(signal) {
                        Some(Effect::Event(event)) => {
                            let _ = self.events.send(event);
                        }
                        Some(Effect::Close) => self.carry_out(&*server, Command::Close),
                        None => {}
                    },
                    Input::Disconnected => return,
                    Input::Finish(done) => {
                        // The supervisor stops waiting first; dropping the
                        // server then ends the signal thread too.
                        drop(done);
                        return;
                    }
                }
            }
        }

        fn carry_out(&mut self, server: &dyn Server, command: Command) {
            // Only the detach notice depends on the capability; the server's
            // is read once, and again after that server leaves.
            if matches!(command, Command::Show(_)) && self.notice.actions.is_none() {
                match server.capabilities() {
                    Ok((capabilities, owner)) => {
                        let offered = capabilities.iter().any(|name| name == "actions");
                        self.notice.actions = Some((owner, offered));
                    }
                    Err(failure) => return self.unavailable(&failure),
                }
            }
            let Some(call) = self.notice.call(&command) else {
                return;
            };
            match call {
                Call::Notify(notification) => match server.notify(&notification) {
                    Ok((id, owner)) => {
                        self.reported = false;
                        self.log.record(&format!(
                            "desktop notification {id} sent: {}",
                            notification.summary
                        ));
                        self.notice.sent(id, owner, notification.stage);
                    }
                    // The issuing server left before its owner change
                    // arrived: show the notification afresh.
                    Err(failure) if failure.gone && notification.replaces != 0 => {
                        self.notice.forget(&notification.destination);
                        self.carry_out(server, command);
                    }
                    Err(failure) => self.unavailable(&failure),
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
    }

    impl DBus {
        /// Opens the session bus and subscribes to the server's signals before
        /// any notification, since a fast click can arrive before `Notify`
        /// returns. A server that never answers fails a call after D-Bus's
        /// usual 25-second timeout and holds only the worker.
        fn connect(
            application_id: &str,
            inputs: mpsc::Sender<Input>,
        ) -> zbus::Result<Box<dyn Server>> {
            let connection = zbus::blocking::connection::Builder::session()?
                .method_timeout(Duration::from_secs(25))
                .build()?;
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
            Ok(Box::new(Self {
                connection,
                application_id: application_id.to_owned(),
            }))
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
    }

    impl Drop for DBus {
        /// Closing the connection ends the signal thread's wait.
        fn drop(&mut self) {
            let _ = self.connection.clone().close();
        }
    }

    /// Passes on the bus's notification and owner-change signals; the
    /// worker's notice decides which to believe.
    fn listen(messages: MessageIterator, inputs: &mpsc::Sender<Input>) {
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
                    let (name, old, _new) = body.deserialize::<(String, String, String)>().ok()?;
                    Signal::OwnerChanged { sender, name, old }
                }
                _ => return None,
            },
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::notification::{QUIT, Stage};
        use std::sync::Mutex;

        /// A server at `:1.7` that records each call and numbers its
        /// notifications from 1, keeping a replaced one's ID.
        struct Fake(Arc<Mutex<Vec<Call>>>);

        impl Server for Fake {
            fn capabilities(&self) -> Result<(Vec<String>, String), Failure> {
                Ok((vec!["body".into(), "actions".into()], ":1.7".into()))
            }

            fn notify(&self, notification: &Notification) -> Result<(u32, String), Failure> {
                let mut calls = self.0.lock().unwrap();
                calls.push(Call::Notify(notification.clone()));
                let id = match notification.replaces {
                    0 => u32::try_from(calls.len()).unwrap(),
                    replaces => replaces,
                };
                Ok((id, ":1.7".into()))
            }

            fn close(&self, id: u32, owner: &str) -> Result<(), Failure> {
                self.0.lock().unwrap().push(Call::Close {
                    id,
                    owner: owner.into(),
                });
                Ok(())
            }
        }

        /// Two detaches go through the call `reap_presentation` makes, so
        /// gating the notice on an earlier one, as the once-per-user notice
        /// did, fails here.
        #[test]
        fn every_detach_shows_a_notice() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let recorded = Arc::clone(&calls);
            let mut notices = Notices {
                connect: Arc::new(move |_: &str, _| {
                    Ok(Box::new(Fake(Arc::clone(&recorded))) as Box<dyn Server>)
                }),
                ..Notices::default()
            };
            let log = DiagnosticLog::open_in(Err("no log in this test".into()), "g1", "eon");
            let window = || NoticeWindow {
                name: "w".into(),
                id: None,
                stop_command: "eon window stop default".into(),
                terminals: 1,
            };
            notices.show(window(), "eon", &log);
            notices.show(window(), "eon", &log);
            notices.close_and_wait(Duration::from_secs(30));
            let shown = |destination: &str, replaces| {
                Call::Notify(Notification {
                    destination: destination.into(),
                    replaces,
                    summary: "Eon window w is still running".into(),
                    body: "1 terminal keeps running in the background. Open Eon to return.".into(),
                    actions: &[QUIT, "Quit"],
                    stage: Stage::Shown,
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
                    Call::Close {
                        id: 1,
                        owner: ":1.7".into()
                    },
                ]
            );
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) use other::Notices;

#[cfg(not(target_os = "linux"))]
mod other {
    use super::{Event, NoticeWindow};
    use crate::diagnostics::DiagnosticLog;
    use std::{sync::Arc, time::Duration};

    /// macOS has no freedesktop notification server; a detach sends nothing.
    #[derive(Default)]
    pub(crate) struct Notices {}

    impl Notices {
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
            state: eon_workspace_protocol::v44::WindowState::Live,
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
        }
    }

    #[test]
    fn fresh_notice_uses_the_capability_owner() {
        let notice = Notice {
            actions: Some((":1.7".into(), true)),
            ..Notice::default()
        };
        let Some(Call::Notify(notification)) = notice.call(&Command::Show(window("w", None, 1)))
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
        assert_eq!(notice.on_signal(owner_left(BUS, SERVICE, ":1.5")), None);
        assert_eq!(notice.on_signal(owner_left(BUS, SERVICE, "")), None);
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
        let Some(Call::Notify(replacement)) = notice.call(&Command::Show(window("w", None, 1)))
        else {
            panic!("a detach sends a notification");
        };
        assert_eq!(
            (replacement.destination.as_str(), replacement.replaces),
            (":1.7", 7)
        );
        // The issuing server leaving forgets its notice and capabilities, and
        // the next notification goes to the well-known name afresh.
        assert_eq!(notice.on_signal(owner_left(BUS, SERVICE, ":1.7")), None);
        assert_eq!(notice.actions, None);
        assert_eq!(notice.on_signal(action(":1.7", 7, QUIT_CONFIRM)), None);
        let Some(Call::Notify(fresh)) = notice.call(&Command::Show(window("w", None, 1))) else {
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
        let Some(Call::Notify(first)) = notice.call(&Command::Show(w())) else {
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
        let Some(Call::Notify(next)) = notice.call(&Command::Confirm(w())) else {
            panic!("a confirmation sends a notification");
        };
        assert_eq!(
            (next.destination.as_str(), next.replaces, next.stage),
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
        let Some(Call::Notify(fresh)) = notice.call(&Command::Confirm(w())) else {
            panic!("a confirmation sends a notification");
        };
        assert_eq!((fresh.destination.as_str(), fresh.replaces), (":1.7", 0));
    }
}
