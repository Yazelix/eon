//! One local window owns Venus, connection attempts and routing, never authoritative layout.
use super::*;
use crate::{
    connections::{self, Cancellation, Connection},
    control::PendingRequest,
};
use eon_workspace_protocol::v13::{
    ConnectionState, RegistryAction, WindowAction, WindowState, WorkspaceGroup, WorkspaceInfo,
    WorkspaceSnapshot,
};
use std::{
    collections::HashMap,
    sync::{Arc, mpsc},
    thread::JoinHandle,
};

struct Task {
    key: u64,
    request: Request,
    deadline: Instant,
}
enum Event {
    Ready(WorkspaceInfo, WorkspaceSnapshot),
    Snapshot(WorkspaceSnapshot),
    Outcome(u64, Result<WorkspaceSnapshot, String>),
    Groups(Vec<WorkspaceGroup>),
    Lost(String),
    Stopped,
}
struct Worker {
    cancel: Arc<Cancellation>,
    tasks: mpsc::SyncSender<Task>,
    events: mpsc::Receiver<Event>,
    thread: JoinHandle<()>,
}

fn emit(
    sender: &mpsc::SyncSender<Event>,
    mut event: Event,
    cancel: &Cancellation,
) -> Result<(), String> {
    loop {
        if cancel.cancelled() {
            return Err("connection cancelled".into());
        }
        match sender.try_send(event) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Full(returned)) => {
                event = returned;
                thread::sleep(Duration::from_millis(10));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return Err("window closed".into()),
        }
    }
}
impl Worker {
    fn start(
        inputs: Inputs,
        target: Option<(String, String)>,
        root: PathBuf,
    ) -> Result<Self, String> {
        let cancel = Arc::new(Cancellation::default());
        let (tasks, requests) = mpsc::sync_channel::<Task>(32);
        let (events, receive) = mpsc::sync_channel(32);
        let cancellation = Arc::clone(&cancel);
        let thread = thread::Builder::new()
            .name("eon-window-connection".into())
            .spawn(move || {
                let result = (|| {
                    let mut stopped = false;
                    let mut last_snapshot = None;
                    let mut connection = if let Some((host, workspace)) = &target {
                        let mut connection =
                            Connection::open(&inputs, host, workspace, &root, &cancellation)?;
                        let snapshot = connection
                            .exchange(Action::Workspace(WorkspaceAction::Inspect), &cancellation)?;
                        last_snapshot = Some(snapshot.clone());
                        emit(
                            &events,
                            Event::Ready(connection.info.clone(), snapshot),
                            &cancellation,
                        )?;
                        Some(connection)
                    } else {
                        None
                    };
                    while !cancellation.cancelled() {
                        match requests.recv_timeout(Duration::from_millis(250)) {
                            Ok(task)
                                if matches!(
                                    task.request.action,
                                    Action::Registry(RegistryAction::List)
                                ) =>
                            {
                                let mut groups = connections::navigation(&inputs)?;
                                for group in
                                    groups.iter_mut().filter(|group| !group.host.is_empty())
                                {
                                    if let Ok(Response::Registry(workspaces)) =
                                        connections::registry(
                                            &group.host,
                                            RegistryAction::List,
                                            &cancellation,
                                        )
                                    {
                                        group.workspaces = workspaces;
                                        group.offline = false;
                                    }
                                    if cancellation.cancelled() {
                                        return Ok(());
                                    }
                                    emit(
                                        &events,
                                        Event::Groups(vec![group.clone()]),
                                        &cancellation,
                                    )?;
                                }
                                emit(&events, Event::Groups(groups), &cancellation)?;
                                if connection.is_none() {
                                    return Ok(());
                                }
                            }
                            Ok(task) => {
                                let is_stop = matches!(&task.request.action, Action::Registry(RegistryAction::Stop { .. }));
                                let result = if Instant::now() >= task.deadline {
                                    Err("workspace action expired before it was sent".into())
                                } else {
                                    connection
                                        .as_mut()
                                        .ok_or_else(|| "window has no connected workspace".into())
                                        .and_then(|connection| {
                                            if stopped {
                                                if is_stop && connection.stopped(&inputs, &cancellation)? {
                                                    Ok(last_snapshot.as_ref().expect("connected workspace has a snapshot").clone())
                                                } else {
                                                    Err("workspace Stop receipt is no longer current; reconnect before stopping it".into())
                                                }
                                            } else {
                                                connection.request(task.request, &cancellation)
                                            }
                                        })
                                };
                                if let Ok(snapshot) = &result {
                                    last_snapshot = Some(snapshot.clone());
                                    stopped |= is_stop;
                                }
                                emit(&events, Event::Outcome(task.key, result), &cancellation)?;
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if let Some(connection) = &mut connection && !stopped {
                                    match connection.exchange(
                                        Action::Workspace(WorkspaceAction::Inspect),
                                        &cancellation,
                                    ) {
                                        Ok(snapshot) => {
                                            last_snapshot = Some(snapshot.clone());
                                            emit(&events, Event::Snapshot(snapshot), &cancellation)?
                                        }
                                        Err(error) => {
                                            if connection
                                                .stopped(&inputs, &cancellation)
                                                .unwrap_or(false)
                                            {
                                                stopped = true;
                                                emit(&events, Event::Stopped, &cancellation)?;
                                            } else {
                                                return Err(error);
                                            }
                                        }
                                    }
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                        }
                    }
                    Ok::<_, String>(())
                })();
                if let Err(error) = result
                    && !cancellation.cancelled()
                {
                    let _ = emit(&events, Event::Lost(error), &cancellation);
                }
                // Connections are gone before their private forwarding directory is removed.
                let _ = fs::remove_dir_all(root);
            })
            .map_err(|error| format!("cannot start workspace connection worker: {error}"))?;
        Ok(Self {
            cancel,
            tasks,
            events: receive,
            thread,
        })
    }
}

struct Pending {
    client: PendingRequest,
    stop: bool,
}
struct Window<'a> {
    inputs: &'a Inputs,
    root: &'a Path,
    state: WindowState,
    requested: Option<(String, String)>,
    worker: Option<Worker>,
    navigation: Option<Worker>,
    pending: HashMap<u64, Pending>,
    next_request: u64,
    refresh_navigation: bool,
    venus: PresentationProcess,
}
impl Window<'_> {
    fn response(&self) -> Response {
        Response::Window(Box::new(self.state.clone()))
    }
    fn cancel_requests(&mut self, detail: &str) {
        for (_, pending) in self.pending.drain() {
            pending
                .client
                .reply(Response::Failure(failure("connection-changed", detail)));
        }
        if let Some(worker) = &self.worker {
            worker.cancel.cancel();
        }
        if let Some(worker) = &self.navigation {
            worker.cancel.cancel();
        }
    }
    fn select(&mut self, target: (String, String)) -> Result<(), String> {
        self.state.generation = self
            .state
            .generation
            .checked_add(1)
            .ok_or("connection generation exhausted")?;
        self.cancel_requests("connection changed; a previous in-flight action may have committed and was not replayed");
        self.state.connection = ConnectionState::Connecting;
        self.state.detail = format!(
            "Connecting to {}/{}",
            if target.0.is_empty() {
                "Local"
            } else {
                &target.0
            },
            target.1
        );
        self.requested = Some(target);
        self.refresh_navigation = false;
        Ok(())
    }
    fn start_worker(&mut self) -> Result<(), String> {
        if self.worker.is_some() {
            return Ok(());
        }
        // Selection cancels discovery and waits for its SSH attempt to end.
        if self.state.connection != ConnectionState::Connecting || self.navigation.is_some() {
            return Ok(());
        }
        let root = self.root.join(format!("c{}", self.state.generation));
        prepare_runtime(&root)?;
        self.worker = Some(Worker::start(
            self.inputs.clone(),
            self.requested.clone(),
            root,
        )?);
        Ok(())
    }
    fn merge_groups(&mut self, groups: Vec<WorkspaceGroup>) {
        for group in groups {
            if let Some(current) = self
                .state
                .groups
                .iter_mut()
                .find(|current| current.host == group.host)
            {
                *current = group;
            } else {
                self.state.groups.push(group);
            }
        }
    }
    fn poll_navigation(&mut self) -> Result<(), String> {
        if let Some(worker) = &self.navigation {
            let finished = worker.thread.is_finished();
            let cancelled = worker.cancel.cancelled();
            let events: Vec<_> = worker.events.try_iter().collect();
            if !cancelled {
                for event in events {
                    match event {
                        Event::Groups(groups) => self.merge_groups(groups),
                        Event::Lost(error) => eprintln!("Eon workspace navigation: {error}"),
                        _ => {}
                    }
                }
            }
            if finished {
                let _ = self.navigation.take().unwrap().thread.join();
            }
        }
        if self.refresh_navigation
            && self.navigation.is_none()
            && self.state.connection != ConnectionState::Connecting
        {
            let root = self.root.join("navigation");
            prepare_runtime(&root)?;
            let worker = Worker::start(self.inputs.clone(), None, root)?;
            worker
                .tasks
                .try_send(Task {
                    key: 0,
                    request: Request {
                        id: request_id(),
                        action: Action::Registry(RegistryAction::List),
                    },
                    deadline: Instant::now() + Duration::from_secs(6),
                })
                .map_err(|_| "workspace navigation queue is unavailable")?;
            self.navigation = Some(worker);
            self.refresh_navigation = false;
        }
        Ok(())
    }
    fn submit(&mut self, client: PendingRequest, action: Action, stop: bool) {
        let key = self.next_request;
        self.next_request += 1;
        let result = self
            .worker
            .as_ref()
            .ok_or("window has no connection worker")
            .and_then(|worker| {
                worker
                    .tasks
                    .try_send(Task {
                        key,
                        request: Request {
                            id: client.request.id.clone(),
                            action,
                        },
                        deadline: Instant::now() + Duration::from_secs(6),
                    })
                    .map_err(|_| "workspace action queue is unavailable or full")
            });
        match result {
            Ok(()) => {
                self.pending.insert(key, Pending { client, stop });
            }
            Err(error) => client.reply(Response::Failure(failure("connection-unavailable", error))),
        }
    }
    fn request(&mut self, client: PendingRequest) -> Result<bool, String> {
        let action = match client.request.action.clone() {
            Action::Workspace(WorkspaceAction::Inspect) => {
                client.reply(self.response());
                return Ok(false);
            }
            Action::Window { generation, action } if generation == self.state.generation => action,
            Action::Window { .. } => {
                client.reply(Response::Failure(failure(
                    "stale-connection",
                    "workspace selection changed; action was not sent",
                )));
                return Ok(false);
            }
            _ => {
                client.reply(Response::Failure(failure(
                    "window-action-required",
                    "workspace mutations require the selected connection generation",
                )));
                return Ok(false);
            }
        };
        match action {
            WindowAction::Select { host, workspace } => {
                self.select((host, workspace))?;
                client.reply(self.response());
            }
            WindowAction::Reconnect => {
                let target = self.requested.clone().or_else(|| {
                    self.state
                        .target
                        .as_ref()
                        .map(|info| (self.state.host.clone(), info.id.clone()))
                });
                if let Some(target) = target {
                    self.select(target)?;
                    client.reply(self.response());
                } else {
                    client.reply(Response::Failure(failure(
                        "no-target",
                        "select a workspace first",
                    )));
                }
            }
            WindowAction::Disconnect | WindowAction::Busy | WindowAction::Offline { .. } => {
                self.cancel_requests("connection detached; an in-flight action may have committed and was not replayed");
                self.state.generation = self
                    .state
                    .generation
                    .checked_add(1)
                    .ok_or("connection generation exhausted")?;
                let (connection, detail) = match action {
                    WindowAction::Busy => (
                        ConnectionState::Busy,
                        "Workspace is busy in another window".into(),
                    ),
                    WindowAction::Offline { detail } => (ConnectionState::Offline, detail),
                    _ => (
                        ConnectionState::Disconnected,
                        "Disconnected; workspace tasks continue running".into(),
                    ),
                };
                self.state.connection = connection;
                self.state.detail = detail;
                self.requested = None;
                client.reply(self.response());
            }
            WindowAction::Present => {
                self.venus.present()?;
                client.reply(self.response());
            }
            WindowAction::Refresh => match connections::navigation(self.inputs) {
                Ok(groups) => {
                    self.state.groups = groups;
                    self.refresh_navigation = true;
                    client.reply(self.response());
                }
                Err(error) => client.reply(Response::Failure(failure("navigation-failed", error))),
            },
            WindowAction::Apply(action) => {
                if self.state.connection != ConnectionState::Connected {
                    client.reply(Response::Failure(failure(
                        "workspace-offline",
                        "reconnect before changing the workspace",
                    )));
                } else {
                    self.submit(client, *action, false);
                }
            }
            WindowAction::Stop => {
                if let Some(target) = &self.state.target {
                    if !matches!(
                        self.state.connection,
                        ConnectionState::Connected | ConnectionState::Stopped
                    ) {
                        client.reply(Response::Failure(failure(
                            "workspace-offline",
                            "reconnect before stopping the selected workspace",
                        )));
                    } else {
                        self.submit(
                            client,
                            Action::Registry(RegistryAction::Stop {
                                workspace: target.id.clone(),
                            }),
                            true,
                        );
                    }
                } else if self.state.connection == ConnectionState::Connecting {
                    client.reply(Response::Failure(failure(
                        "connection-pending",
                        "wait for workspace selection before stopping the window",
                    )));
                } else {
                    client.reply(self.response());
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    fn events(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        // Observe completion before draining: a thread that finishes afterward
        // keeps its receiver until the next iteration consumes its final event.
        let finished = worker.thread.is_finished();
        let cancelled = worker.cancel.cancelled();
        let events: Vec<_> = worker.events.try_iter().collect();
        if finished {
            let _ = self.worker.take().unwrap().thread.join();
        }
        if cancelled {
            return false;
        }
        for event in events {
            match event {
                Event::Ready(info, snapshot) => {
                    self.state.host = self
                        .requested
                        .as_ref()
                        .map(|target| target.0.clone())
                        .unwrap_or_default();
                    self.state.target = Some(info);
                    self.state.workspace = Some(snapshot);
                    self.state.connection = ConnectionState::Connected;
                    self.state.detail.clear();
                    self.refresh_navigation |= !self.state.host.is_empty();
                }
                Event::Snapshot(snapshot) => {
                    self.state.workspace = Some(snapshot);
                }
                Event::Groups(groups) => self.merge_groups(groups),
                Event::Outcome(key, result) => {
                    if let Some(pending) = self.pending.remove(&key) {
                        match result {
                            Ok(snapshot) => {
                                self.state.workspace = Some(snapshot);
                                self.state.detail.clear();
                                if pending.stop {
                                    self.state.connection = ConnectionState::Stopped;
                                    self.state.detail = "Workspace stopped".into();
                                    if let Some(target) = &mut self.state.target {
                                        target.stopped = true;
                                    }
                                }
                                pending.client.reply(self.response());
                                if pending.stop {
                                    return true;
                                }
                            }
                            Err(error) => pending
                                .client
                                .reply(Response::Failure(failure("workspace-action", error))),
                        }
                    }
                }
                Event::Stopped => {
                    self.state.connection = ConnectionState::Stopped;
                    self.state.detail = "Workspace stopped".into();
                    if let Some(target) = &mut self.state.target {
                        target.stopped = true;
                    }
                    for (_, pending) in self.pending.drain() {
                        pending.client.reply(Response::Failure(failure(
                            "workspace-stopped",
                            "workspace stopped; an in-flight action was not replayed",
                        )));
                    }
                }
                Event::Lost(error) => {
                    self.state.connection = ConnectionState::Offline;
                    self.state.detail = failure("connection-lost", error).detail;
                    self.cancel_requests("workspace connection lost; an in-flight action may have committed and was not replayed");
                }
            }
        }
        false
    }
}

pub(crate) fn serve(
    inputs: &Inputs,
    root: &Path,
    host: String,
    workspace: String,
    welcome: bool,
) -> Result<i32, String> {
    inputs.component_report()?;
    validate_private_directory(root)?;
    let _lock = try_lock_supervisor_lifecycle(&root.join("window.lock"))?
        .ok_or("window controller is already running")?;
    let socket = root.join("window.sock");
    let listener = ControlListener::bind(&socket)?;
    let config = configuration_directory()?;
    prepare_configuration(&config)?;
    let terminal = managed_environment::terminal_presentation(&config, &inputs.defaults)?;
    let animation = if welcome {
        managed_environment::startup_animation(&config, &inputs.defaults)?
    } else {
        None
    };
    let initial = WindowState {
        generation: 0,
        connection: ConnectionState::Disconnected,
        host: String::new(),
        target: None,
        groups: connections::navigation(inputs)?,
        workspace: None,
        detail: "Select a workspace".into(),
    };
    let command = venus_command(
        inputs,
        &config,
        &socket,
        LaunchMode::Workspace,
        &terminal,
        "eon",
        false,
    )?;
    let venus = PresentationProcess::start(
        command,
        terminal.requires_startup_admission() || animation.is_some(),
        Some(Response::Window(Box::new(initial.clone()))),
        Instant::now() + SESSION_START_TIMEOUT,
    )?;
    if welcome && host.is_empty() {
        super::named::ensure_for_window(inputs, &workspace)?;
    }
    let mut window = Window {
        inputs,
        root,
        state: initial,
        requested: None,
        worker: None,
        navigation: None,
        pending: HashMap::new(),
        next_request: 1,
        refresh_navigation: false,
        venus,
    };
    window.select((host, workspace))?;
    let result = (|| {
        loop {
            if window
                .venus
                .child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Ok(0);
            }
            if window.events() {
                return Ok(0);
            }
            window.poll_navigation()?;
            window.start_worker()?;
            if let Some(client) = listener.accept_request()?
                && window.request(client)?
            {
                return Ok(0);
            }
            thread::sleep(Duration::from_millis(10));
        }
    })();
    window.cancel_requests("window closed; workspace tasks remain running");
    if let Some(worker) = window.worker.take() {
        let _ = worker.thread.join();
    }
    if let Some(worker) = window.navigation.take() {
        let _ = worker.thread.join();
    }
    drop(window);
    drop(listener);
    result
}
