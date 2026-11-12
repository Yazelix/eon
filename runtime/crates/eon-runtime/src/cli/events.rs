use super::super::{
    control as endpoint, sessions::bound_io, supervisor::request_id, workspace as projection,
};
use super::*;
use eon_workspace_protocol::v42::{self as wire, EventKind, Filter, Resume};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::os::fd::{AsFd, BorrowedFd};

pub(super) struct Options {
    pub(super) filter: Filter,
    pub(super) after: Option<Resume>,
    pub(super) target: WorkspaceTarget,
    pub(super) json: bool,
}

pub(super) fn parse(arguments: &[OsString]) -> Result<Options, Error> {
    let mut options = Options {
        filter: Filter::default(),
        after: None,
        target: WorkspaceTarget {
            command: arguments.to_vec(),
            ..WorkspaceTarget::default()
        },
        json: false,
    };
    let mut args = arguments.iter();
    if arguments.first().is_some_and(|word| word == "--window") {
        args.next();
        options.target.window =
            Some(windows::window_id_argument(args.next().ok_or_else(usage)?)?.into());
    }
    if args.next().is_none_or(|word| word != "events") {
        return Err(usage());
    }
    while let Some(flag) = args.next() {
        if flag == "--json" && !options.json {
            options.json = true;
            continue;
        }
        let value = args
            .next()
            .and_then(|word| word.to_str())
            .ok_or_else(usage)?;
        match flag.to_str() {
            Some("--session") => options.filter.workspaces.push(value.into()),
            Some("--tab") => options.filter.tabs.push(value.into()),
            Some("--target") => options.filter.targets.push(value.into()),
            Some("--kind") => options.filter.kinds.push(match value {
                "session" => EventKind::Workspace,
                "tab" => EventKind::Tab,
                "pane" => EventKind::Pane,
                "popup" => EventKind::Popup,
                "attention" => EventKind::Attention,
                "queue" => EventKind::Queue,
                "busy" => EventKind::Busy,
                _ => return Err(usage()),
            }),
            Some("--state") => options.filter.states.push(match value {
                "needs-input" => AttentionState::NeedsInput,
                "error" => AttentionState::Error,
                "finished" => AttentionState::Finished,
                "working" => AttentionState::Working,
                "background" => AttentionState::Background,
                "idle" => AttentionState::Idle,
                "unavailable" => AttentionState::Unavailable,
                _ => return Err(usage()),
            }),
            Some("--after") if options.after.is_none() => {
                let (stream, seq) = value.split_once(':').ok_or_else(usage)?;
                if seq.is_empty() || !seq.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(usage());
                }
                options.after = Some(Resume {
                    stream: stream.into(),
                    seq: seq.parse().map_err(|_| usage())?,
                });
            }
            Some("--window") if options.target.window.is_none() => {
                options.target.window =
                    Some(windows::window_id_argument(OsStr::new(value))?.into());
            }
            _ => return Err(usage()),
        }
    }
    distinct(&mut options.filter.workspaces);
    distinct(&mut options.filter.tabs);
    distinct(&mut options.filter.targets);
    distinct(&mut options.filter.kinds);
    distinct(&mut options.filter.states);
    validate_action(
        None,
        Action::Subscribe {
            filter: options.filter.clone(),
            after: options.after.clone(),
        },
    )
    .map_err(|error| Error::usage(error.detail))?;
    Ok(options)
}

fn distinct<T: PartialEq>(items: &mut Vec<T>) {
    for index in (0..items.len()).rev() {
        if items[..index].contains(&items[index]) {
            items.remove(index);
        }
    }
}

fn usage() -> Error {
    Error::usage(
        "usage: eon events [--json] [--window ID] [--session SESSION] [--tab TAB] [--target ID] [--kind KIND] [--state STATE] [--after STREAM:SEQ]",
    )
}

pub(super) fn execute(inputs: &Inputs, arguments: &[OsString]) -> Result<i32, Error> {
    // _exit is async-signal-safe, including during startup or a blocked stdout write.
    extern "C" fn interrupt(_: libc::c_int) {
        unsafe { libc::_exit(130) }
    }
    let previous =
        unsafe { libc::signal(libc::SIGINT, interrupt as *const () as libc::sighandler_t) };
    if previous == libc::SIG_ERR {
        return Err(lost(std::io::Error::last_os_error()));
    }
    let output = std::io::stdout();
    let stdout = Output::watch(output.as_fd()).map_err(lost)?;
    let result = if stdout.closed().map_err(lost)? {
        Ok(0)
    } else {
        execute_inner(inputs, arguments, &stdout)
    };
    unsafe {
        libc::signal(libc::SIGINT, previous);
    }
    if stdout.closed().map_err(lost)? {
        Ok(0)
    } else {
        result
    }
}

fn execute_inner(inputs: &Inputs, arguments: &[OsString], stdout: &Output) -> Result<i32, Error> {
    let options = parse(arguments)?;
    let mut output = std::io::stdout().lock();
    let mut errors = std::io::stderr().lock();
    let caller = match calling_terminal() {
        Ok(caller) => caller,
        Err(error) => {
            return stream_result(refuse(
                &mut output,
                &mut errors,
                stdout,
                &error,
                options.json,
            ));
        }
    };
    let action = Action::Subscribe {
        filter: options.filter,
        after: options.after,
    };
    validate_action(caller.clone(), action.clone()).map_err(|error| Error::usage(error.detail))?;
    let resolved = match resolve_workspace(inputs, options.target, caller.as_deref()) {
        Ok(resolved) => resolved,
        Err(error) => {
            return stream_result(refuse(
                &mut output,
                &mut errors,
                stdout,
                &error,
                options.json,
            ));
        }
    };
    let request = wire::encode_request(&wire::Request {
        id: request_id(),
        caller,
        action,
    })
    .map_err(|error| Error::usage(error.to_string()))?;
    let mut stream = endpoint::connect_control(&resolved.socket)
        .map_err(|error| Error::unreachable(error.detail))?;
    stream.write_all(&request).map_err(lost)?;
    stream_result(consume(
        &mut stream,
        &mut output,
        &mut errors,
        stdout,
        options.json,
    ))
}

fn stream_result(result: std::io::Result<i32>) -> Result<i32, Error> {
    match result {
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(0),
        result => result.map_err(lost),
    }
}

/// How often a half-closed socket output is asked whether its reader is gone,
/// as Orbit probes a half-closed client.
const PROBE_INTERVAL: Duration = Duration::from_millis(100);

/// Standard output, watched for a reader that goes away while the stream waits.
#[derive(Default)]
struct Output<'fd> {
    /// The descriptor when it is watched.
    fd: Option<BorrowedFd<'fd>>,
    socket: bool,
    /// When the output is next probed. Darwin reports a socket output that is
    /// readable, or hung up while its reader still reads (the reader shut down
    /// only its writing), and would then wake poll at once every time. Such an
    /// output leaves poll's set and is probed on this fixed schedule instead,
    /// which reads and messages do not move.
    probe_at: std::cell::Cell<Option<Instant>>,
}

impl<'fd> Output<'fd> {
    /// Linux reports a gone reader on any output. Darwin watches only a pipe or
    /// socket: its poll would wake for a file's or a terminal's readable data.
    fn watch(fd: BorrowedFd<'fd>) -> std::io::Result<Self> {
        let status = rustix::fs::fstat(fd)?;
        let kind = rustix::fs::FileType::from_raw_mode(status.st_mode);
        let socket = kind == rustix::fs::FileType::Socket;
        let watched =
            !cfg!(target_vendor = "apple") || socket || kind == rustix::fs::FileType::Fifo;
        Ok(Self {
            fd: watched.then_some(fd),
            socket,
            probe_at: Default::default(),
        })
    }

    /// What poll asks of the output. Linux reports a gone reader whatever is
    /// asked. Darwin's kqueue-backed poll registers nothing for no events; a
    /// read-band request registers the read filter, which reports a gone
    /// reader as a hangup. Linux never sets read-band on an output.
    fn poll(&self) -> Option<PollFd<'fd>> {
        self.fd
            .filter(|_| self.probe_at.get().is_none())
            .map(|fd| PollFd::from_borrowed_fd(fd, PollFlags::RDBAND))
    }

    /// How long poll may wait before the output is due a probe.
    fn probe_after(&self) -> Option<Duration> {
        self.probe_at
            .get()
            .map(|due| due.saturating_duration_since(Instant::now()))
    }

    /// Whether what poll reported means the reader is gone.
    fn gone(&self, revents: PollFlags) -> std::io::Result<bool> {
        let Some(fd) = self.fd.filter(|_| !revents.is_empty()) else {
            return Ok(false);
        };
        if cfg!(target_vendor = "apple") && self.socket && !revents.contains(PollFlags::ERR) {
            if Self::unread(fd)? {
                return Ok(true);
            }
            self.probe_at.set(Some(Instant::now() + PROBE_INTERVAL));
            return Ok(false);
        }
        Ok(true)
    }

    /// Whether a probed output, once its probe is due, has lost its reader.
    fn probe(&self) -> std::io::Result<bool> {
        match (self.fd, self.probe_at.get()) {
            (Some(fd), Some(due)) if Instant::now() >= due => {
                self.probe_at.set(Some(Instant::now() + PROBE_INTERVAL));
                Self::unread(fd)
            }
            _ => Ok(false),
        }
    }

    /// A zero-length send fails only once the peer cannot read. A send that
    /// would block or was interrupted proves nothing; the next probe asks again.
    fn unread(fd: BorrowedFd<'_>) -> std::io::Result<bool> {
        use rustix::io::Errno;
        match rustix::net::send(fd, &[], rustix::net::SendFlags::empty()) {
            Ok(_) => Ok(false),
            Err(Errno::PIPE | Errno::CONNRESET | Errno::NOTCONN) => Ok(true),
            Err(Errno::AGAIN | Errno::INTR) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn closed(&self) -> std::io::Result<bool> {
        if let (Some(fd), Some(_)) = (self.fd, self.probe_at.get()) {
            return Self::unread(fd);
        }
        let Some(output) = self.poll() else {
            return Ok(false);
        };
        let mut ready = [output];
        loop {
            match poll(&mut ready, Some(&Timespec::default())) {
                Ok(_) => return self.gone(ready[0].revents()),
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
}

fn refuse(
    output: &mut impl Write,
    errors: &mut impl Write,
    stdout: &Output,
    failure: &Failure,
    json: bool,
) -> std::io::Result<i32> {
    if stdout.closed()? {
        return Ok(0);
    }
    if json {
        output.write_all(projection::failure_json(failure).as_bytes())?;
        output.flush()?;
    } else {
        // Losing the diagnostic cannot turn a refusal into success.
        let _ = errors.write_all(failure_human(failure).as_bytes());
        let _ = errors.flush();
    }
    Ok(match failure.code.as_str() {
        "subscriber-limit" | "ambiguous-session" => 1,
        code if endpoint::failure_status(code) == 3 => 3,
        _ => 4,
    })
}

fn lost(error: impl std::fmt::Display) -> Error {
    Error::unreachable(format!("event connection lost: {error}"))
}
fn invalid(detail: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, detail)
}

fn read_message(
    stream: &mut std::os::unix::net::UnixStream,
    stdout: &Output,
    deadline: Option<Instant>,
) -> std::io::Result<wire::SubscriptionMessage> {
    let mut bytes = vec![0; wire::HEADER_BYTES];
    let mut filled = 0;
    loop {
        if filled == bytes.len() {
            if filled > wire::HEADER_BYTES {
                break;
            }
            let length =
                wire::declared_message_len(&bytes).map_err(|error| invalid(&error.to_string()))?;
            if length == filled {
                break;
            }
            bytes.resize(length, 0);
        }
        let input = PollFd::new(&*stream, PollFlags::IN);
        let output = stdout.poll();
        let count = if output.is_some() { 2 } else { 1 };
        let mut fds = [input.clone(), output.unwrap_or(input)];
        let wait = deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .into_iter()
            .chain(stdout.probe_after())
            .min();
        // Rounded up, so a wait that times out has reached its deadline.
        let timeout = wait.map(|wait| {
            let millis = i32::try_from(wait.as_nanos().div_ceil(1_000_000)).unwrap_or(i32::MAX);
            Timespec {
                tv_sec: i64::from(millis / 1000),
                tv_nsec: i64::from(millis % 1000) * 1_000_000,
            }
        });
        let result = match poll(&mut fds[..count], timeout.as_ref()) {
            Ok(result) => result,
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        };
        // Due even while input keeps arriving.
        if stdout.probe()? {
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        if result == 0 {
            if deadline.is_none_or(|deadline| Instant::now() < deadline) {
                continue;
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "subscription handshake timed out",
            ));
        }
        if count == 2 && stdout.gone(fds[1].revents())? {
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
        }
        if fds[0].revents().is_empty() {
            continue;
        }
        match stream.read(&mut bytes[filled..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "no complete end message",
                ));
            }
            Ok(count) => filled += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    wire::decode_subscription_message(&bytes).map_err(|error| invalid(&error.to_string()))
}

fn consume(
    stream: &mut std::os::unix::net::UnixStream,
    output: &mut impl Write,
    errors: &mut impl Write,
    stdout: &Output,
    json: bool,
) -> std::io::Result<i32> {
    use wire::SubscriptionMessage as Message;
    let deadline = Some(Instant::now() + endpoint::CONTROL_TIMEOUT);
    let subscribed = match read_message(stream, stdout, deadline)? {
        Message::Subscribed(value) => value,
        Message::Failure(error) => return refuse(output, errors, stdout, &error, json),
        _ => return Err(invalid("expected SUBSCRIBED")),
    };
    bound_io(stream, stream.set_read_timeout(None))?;
    let mut seq = subscribed.seq;
    let tabs = match read_tabs(stream, stdout, subscribed.snapshot.as_ref())? {
        TabRead::Complete(tabs) => tabs,
        TabRead::End(end) => return write_end(output, end, json),
    };
    let state = state_json(subscribed.snapshot.as_ref(), &tabs);
    let mut text = if json {
        format!(
            "{{\"type\":\"snapshot\",\"stream\":\"{}\",\"seq\":{seq},\"resumed\":{},\"generation\":\"{}\",\"window\":",
            json_escape(&subscribed.stream),
            subscribed.snapshot.is_none(),
            json_escape(&subscribed.generation)
        )
    } else {
        counts(
            "snapshot",
            seq,
            &subscribed.stream,
            subscribed.snapshot.as_ref(),
            &tabs,
        )
    };
    if json {
        projection::json_option(&mut text, subscribed.window.as_deref());
        text.push_str(",\"caller\":");
        if let Some(caller) = &subscribed.caller {
            text.push_str(&format!(
                "{{\"terminal\":\"{}\",\"tab\":\"{}\",{}}}",
                json_escape(&caller.terminal),
                json_escape(&caller.tab),
                projection::target_id_json(&caller.id)
            ));
        } else {
            text.push_str("null");
        }
        text.push(',');
        text.push_str(&state);
        text.push('}');
    }
    writeln!(output, "{text}")?;
    output.flush()?;
    loop {
        let message = read_message(stream, stdout, None)?;
        let text = match message {
            Message::Event(event) if event.seq > seq => {
                seq = event.seq;
                event_line(&event, json)
            }
            Message::Gap(gap) if gap.seq >= seq => {
                seq = gap.seq;
                let tabs = match read_tabs(stream, stdout, Some(&gap.snapshot))? {
                    TabRead::Complete(tabs) => tabs,
                    TabRead::End(end) => return write_end(output, end, json),
                };
                if json {
                    format!(
                        "{{\"type\":\"gap\",\"seq\":{seq},\"reason\":\"overflow\",\"snapshot\":{{{}}}}}",
                        state_json(Some(&gap.snapshot), &tabs)
                    )
                } else {
                    counts("gap", seq, &subscribed.stream, Some(&gap.snapshot), &tabs)
                }
            }
            Message::End(end) => return write_end(output, end, json),
            _ => return Err(invalid("unexpected subscription message or sequence")),
        };
        writeln!(output, "{text}")?;
        output.flush()?;
    }
}

fn write_end(output: &mut impl Write, end: wire::End, json: bool) -> std::io::Result<i32> {
    let (reason, status) = match &end {
        wire::End::Ended { .. } => ("ended", 0),
        wire::End::TargetsGone => ("targets-gone", 0),
        wire::End::TooSlow => ("too-slow", 1),
    };
    let mut text = if json {
        format!("{{\"type\":\"end\",\"reason\":\"{reason}\"")
    } else {
        format!("end {reason}")
    };
    if json {
        if let wire::End::Ended { generation } = end {
            text.push_str(&format!(",\"generation\":\"{}\"", json_escape(&generation)));
        }
        text.push('}');
    }
    writeln!(output, "{text}")?;
    output.flush()?;
    Ok(status)
}

enum TabRead {
    Complete(Vec<wire::Tabs>),
    End(wire::End),
}

fn read_tabs(
    stream: &mut std::os::unix::net::UnixStream,
    stdout: &Output,
    snapshot: Option<&wire::WindowSnapshot>,
) -> std::io::Result<TabRead> {
    let mut groups: Vec<wire::Tabs> = Vec::new();
    let mut terminals = 0;
    for _ in 0..snapshot.map_or(0, |snapshot| snapshot.tab_messages) {
        let group = match read_message(stream, stdout, None)? {
            wire::SubscriptionMessage::Tabs(group) => group,
            wire::SubscriptionMessage::End(end) => return Ok(TabRead::End(end)),
            _ => return Err(invalid("expected TABS")),
        };
        terminals += group
            .tabs
            .iter()
            .map(|tab| {
                tab.columns
                    .iter()
                    .map(|column| column.panes.len())
                    .sum::<usize>()
                    + tab.popups.len()
            })
            .sum::<usize>();
        if terminals > wire::MAX_SESSIONS
            || groups.iter().any(|old| old.workspace == group.workspace)
            || !snapshot
                .unwrap()
                .workspaces
                .iter()
                .any(|entry| entry.workspace.id == group.workspace)
        {
            return Err(invalid("invalid snapshot tab groups"));
        }
        groups.push(group);
    }
    Ok(TabRead::Complete(groups))
}

fn counts(
    kind: &str,
    seq: u64,
    stream: &str,
    snapshot: Option<&wire::WindowSnapshot>,
    tabs: &[wire::Tabs],
) -> String {
    let tab_count: usize = tabs.iter().map(|group| group.tabs.len()).sum();
    let terminals: usize = tabs
        .iter()
        .flat_map(|group| &group.tabs)
        .map(|tab| {
            tab.columns
                .iter()
                .map(|column| column.panes.len())
                .sum::<usize>()
                + tab.popups.len()
        })
        .sum();
    format!(
        "{seq} {kind} {stream} sessions={} tabs={tab_count} terminals={terminals}",
        snapshot.map_or(0, |state| state.workspaces.len())
    )
}

fn array<T>(items: &[T], render: impl Fn(&mut String, &T)) -> String {
    let mut output = String::from("[");
    for (index, item) in items.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        render(&mut output, item);
    }
    output.push(']');
    output
}
fn entry_json(output: &mut String, entry: &wire::WorkspaceEntry) {
    projection::json_workspace(output, &entry.workspace, entry.workspace.recency == 0);
    output.pop();
    output.push_str(",\"active_tab\":");
    projection::json_option(output, Some(&entry.active_tab));
    output.push('}');
}
fn queue_json(queue: &[wire::QueueItem]) -> String {
    array(queue, |output, entry| {
        output.push_str(&format!(
            "{{\"session\":\"{}\",\"tab\":\"{}\",\"target\":\"{}\",\"state\":\"{}\"}}",
            json_escape(&entry.workspace),
            json_escape(&entry.tab),
            json_escape(&entry.target),
            projection::attention_name(entry.state)
        ));
    })
}
fn state_json(snapshot: Option<&wire::WindowSnapshot>, tabs: &[wire::Tabs]) -> String {
    let Some(snapshot) = snapshot else {
        return "\"sessions\":null,\"tabs\":null,\"attention_queue\":null,\"busy\":null".into();
    };
    let mut busy = String::new();
    projection::json_busy(
        &mut busy,
        snapshot
            .busy
            .as_ref()
            .map(|terminals| wire::BusyTerminals {
                terminals: terminals.clone(),
                omitted: 0,
            })
            .as_ref(),
        true,
    );
    let tabs = array(
        &tabs
            .iter()
            .flat_map(|group| group.tabs.iter().map(move |tab| (&group.workspace, tab)))
            .collect::<Vec<_>>(),
        |output, (session, tab)| {
            projection::json_tab(output, tab);
            output.pop();
            output.push_str(",\"session\":");
            projection::json_option(output, Some(session));
            output.push('}');
        },
    );
    format!(
        "\"sessions\":{},\"tabs\":{tabs},\"attention_queue\":{},\"busy\":{busy}",
        array(&snapshot.workspaces, entry_json),
        queue_json(&snapshot.attention_queue)
    )
}

fn event_line(event: &wire::Event, json: bool) -> String {
    use projection::{attention_name, human_attention, json_attention, json_option};
    use wire::{Change, PaneChange as P, PopupChange as U, TabChange as T, WorkspaceChange as W};
    let (kind, name) = match &event.change {
        Change::Workspace { change, .. } => (
            "session",
            match change {
                W::Created => "session-created",
                W::Renamed => "session-renamed",
                W::Closed => "session-closed",
                W::Shown => "session-shown",
                W::Registered => "session-registered",
                W::RegistrationFailed => "session-registration-failed",
            },
        ),
        Change::Tab { change, .. } => (
            "tab",
            match change {
                T::Created => "tab-created",
                T::Moved => "tab-moved",
                T::Directory => "tab-directory",
                T::Closed => "tab-closed",
                T::Activated => "tab-activated",
            },
        ),
        Change::Pane { change, .. } => (
            "pane",
            match change {
                P::Created => "pane-created",
                P::Moved => "pane-moved",
                P::Renamed => "pane-renamed",
                P::Ended => "pane-ended",
                P::Closed => "pane-closed",
                P::Selected => "pane-selected",
            },
        ),
        Change::Popup { change, .. } => (
            "popup",
            match change {
                U::Created => "popup-created",
                U::Shown => "popup-shown",
                U::Hidden => "popup-hidden",
                U::Closed => "popup-closed",
            },
        ),
        Change::Attention { .. } => ("attention", "attention"),
        Change::Queue(_) => ("queue", "queue"),
        Change::Busy { .. } => ("busy", "busy"),
    };
    let mut output = if json {
        format!(
            "{{\"type\":\"event\",\"seq\":{},\"kind\":\"{kind}\",\"event\":\"{name}\"",
            event.seq
        )
    } else {
        format!("{} {name}", event.seq)
    };
    match &event.change {
        Change::Workspace { workspace, .. } => {
            if json {
                output.push_str(",\"session\":");
                entry_json(&mut output, workspace);
            } else {
                output.push_str(&format!(" session={}", workspace.workspace.id));
            }
        }
        Change::Tab {
            workspace,
            tab,
            tabs,
            ..
        } => {
            if json {
                output.push_str(&format!(
                    ",\"session\":\"{}\",\"tab\":",
                    json_escape(workspace)
                ));
                projection::json_tab(&mut output, tab);
                if let Some(tabs) = tabs {
                    output.push_str(",\"tabs\":");
                    output.push_str(&array(tabs, |output, id| json_option(output, Some(id))));
                }
            } else {
                output.push_str(&format!(" session={workspace} tab={}", tab.id));
            }
        }
        Change::Pane {
            workspace,
            tab,
            pane,
            column,
            expanded,
            columns,
            ..
        } => {
            if json {
                output.push_str(&format!(
                    ",\"session\":\"{}\",\"tab\":\"{}\",\"pane\":",
                    json_escape(workspace),
                    json_escape(tab)
                ));
                projection::json_pane(&mut output, pane, *column, *expanded);
                output.push_str(",\"columns\":");
                output.push_str(&array(columns, |output, column| {
                    output.push_str("{\"panes\":");
                    output.push_str(&array(&column.panes, |output, pane| {
                        json_option(output, Some(pane))
                    }));
                    output.push_str(",\"expanded\":");
                    json_option(output, Some(&column.expanded));
                    output.push('}');
                }));
            } else {
                output.push_str(&format!(
                    " session={workspace} tab={tab} pane={} terminal={}",
                    pane.id, pane.session
                ));
            }
        }
        Change::Popup {
            workspace,
            tab,
            popup,
            chosen,
            ..
        } => {
            if json {
                output.push_str(&format!(
                    ",\"session\":\"{}\",\"tab\":\"{}\",\"popup\":",
                    json_escape(workspace),
                    json_escape(tab)
                ));
                projection::json_popup(&mut output, popup, *chosen);
            } else {
                output.push_str(&format!(
                    " session={workspace} tab={tab} popup={} terminal={}",
                    popup.id, popup.session
                ));
            }
        }
        Change::Attention {
            target,
            previous,
            attention,
        } => {
            target_fields(&mut output, target, json);
            if json {
                output.push_str(",\"previous\":");
                json_option(&mut output, previous.map(attention_name));
                output.push_str(",\"attention\":");
                json_attention(&mut output, attention.as_ref());
            } else {
                output.push(' ');
                output.push_str(&human_attention(attention.as_ref()));
            }
        }
        Change::Queue(queue) => {
            if json {
                output.push_str(",\"attention_queue\":");
                output.push_str(&queue_json(queue));
            } else {
                output.push_str(&format!(" targets={}", queue.len()));
            }
        }
        Change::Busy { target, busy, name } => {
            target_fields(&mut output, target, json);
            if json {
                output.push_str(&format!(",\"busy\":{busy}"));
                if *busy {
                    output.push_str(",\"name\":");
                    json_option(&mut output, name.as_deref());
                }
            } else {
                output.push_str(&format!(" busy={busy}"));
                if let Some(name) = name {
                    output.push_str(&format!(" name={}", name.escape_debug()));
                }
            }
        }
    }
    if json {
        output.push('}');
    }
    output
}
fn target_fields(output: &mut String, target: &wire::TerminalTarget, json: bool) {
    if json {
        output.push_str(&format!(
            ",\"session\":\"{}\",\"tab\":\"{}\",{},\"terminal\":\"{}\"",
            json_escape(&target.workspace),
            json_escape(&target.tab),
            projection::target_id_json(&target.id),
            json_escape(&target.terminal)
        ));
    } else {
        let (noun, id) = match &target.id {
            wire::TargetId::Pane(id) => ("pane", id),
            wire::TargetId::Popup(id) => ("popup", id),
        };
        output.push_str(&format!(
            " session={} tab={} {noun}={id} terminal={}",
            target.workspace, target.tab, target.terminal
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }
    #[test]
    fn filters_are_distinct_and_resume_is_strict() {
        let options = parse(&args(&[
            "events",
            "--session",
            "main",
            "--session",
            "main",
            "--tab",
            "t2",
            "--tab",
            "t2",
            "--kind",
            "attention",
            "--kind",
            "attention",
            "--state",
            "needs-input",
            "--state",
            "needs-input",
            "--target",
            "p2",
            "--target",
            "p2",
            "--after",
            "stream:9",
            "--json",
        ]))
        .unwrap();
        assert_eq!(options.filter.workspaces, ["main"]);
        assert_eq!(options.filter.tabs, ["t2"]);
        assert_eq!(options.filter.targets, ["p2"]);
        assert_eq!(options.filter.kinds, [EventKind::Attention]);
        assert_eq!(options.filter.states, [AttentionState::NeedsInput]);
        assert_eq!(options.after.unwrap().seq, 9);
        for words in [
            vec!["--after", "s:+1"],
            vec!["--after", "s:1:2"],
            vec!["--after", ":1"],
            vec!["--after", "s:18446744073709551616"],
            vec!["--after", "s:"],
            vec!["--kind", "workspace"],
            vec!["--workspace", "s1"],
            vec!["--generation", "g1"],
            vec!["--state", "needs_input"],
            vec!["--tab"],
            vec!["--json", "--json"],
        ] {
            let mut command = vec!["events"];
            command.extend(words);
            assert_eq!(
                parse(&args(&command)).err().unwrap().status,
                2,
                "{command:?}"
            );
        }
    }
    fn run(messages: &[wire::SubscriptionMessage], json: bool) -> std::io::Result<(i32, String)> {
        let (mut reader, mut writer) = std::os::unix::net::UnixStream::pair()?;
        let frames = messages
            .iter()
            .map(|message| wire::encode_subscription_message(message).unwrap())
            .collect::<Vec<_>>();
        let sender = thread::spawn(move || {
            for frame in frames {
                if writer.write_all(&frame).is_err() {
                    break;
                }
            }
        });
        let mut output = Vec::new();
        let result = consume(
            &mut reader,
            &mut output,
            &mut Vec::new(),
            &Output::default(),
            json,
        );
        drop(reader);
        sender.join().unwrap();
        result.map(|status| (status, String::from_utf8(output).unwrap()))
    }
    fn subscribed(snapshot: Option<wire::WindowSnapshot>) -> wire::SubscriptionMessage {
        wire::SubscriptionMessage::Subscribed(wire::Subscribed {
            stream: "stream".into(),
            seq: 0,
            generation: "g1".into(),
            window: None,
            caller: None,
            snapshot,
        })
    }
    #[test]
    fn stream_joins_tabs_handles_gaps_resume_ends_and_truncation() {
        use super::super::super::events::tests::{pane, tab};
        use wire::SubscriptionMessage as M;
        let entry = wire::WorkspaceEntry {
            workspace: wire::WorkspaceInfo {
                id: "s1".into(),
                persistent_id: Some("0123456789abcdef0123456789abcdef0000000000000001".into()),
                registration_error: None,
                name: Some("a\"b".into()),
                tab_count: 1,
                attention: None,
                recency: 0,
            },
            active_tab: "t1".into(),
        };
        let mut tabs = wire::Tabs {
            workspace: "s1".into(),
            tabs: vec![tab("t1", vec![vec![pane("p1", "terminal-1")]], vec![])],
        };
        tabs.tabs[0].columns[0].panes[0].tag = Some("work \"🦀\"".into());
        let snapshot = wire::WindowSnapshot {
            workspaces: vec![entry],
            attention_queue: vec![],
            busy: Some(vec![]),
            tab_messages: 1,
        };
        let messages = [
            subscribed(Some(snapshot.clone())),
            M::Tabs(tabs.clone()),
            M::Gap(wire::Gap {
                seq: 3,
                reason: wire::GapReason::Overflow,
                snapshot,
            }),
            M::Tabs(tabs.clone()),
            M::End(wire::End::TargetsGone),
        ];
        let (status, output) = run(&messages, true).unwrap();
        assert_eq!(status, 0);
        let lines = output
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0]["sessions"][0]["name"], "a\"b");
        assert_eq!(
            lines[0]["sessions"][0]["persistent_id"],
            "0123456789abcdef0123456789abcdef0000000000000001"
        );
        assert_eq!(lines[0]["sessions"][0]["shown"], true);
        assert_eq!(lines[0]["tabs"][0]["session"], "s1");
        assert_eq!(lines[0]["tabs"][0]["panes"][0]["terminal"], "terminal-1");
        assert_eq!(lines[0]["tabs"][0]["panes"][0]["tag"], "work \"🦀\"");
        assert_eq!(lines[1]["snapshot"]["tabs"], lines[0]["tabs"]);
        assert_eq!(lines[0]["busy"]["omitted"], 0);
        assert_eq!(lines[2]["reason"], "targets-gone");
        let (_, human) = run(&messages, false).unwrap();
        assert!(human.contains("0 snapshot stream sessions=1 tabs=1 terminals=1\n"));
        assert!(human.contains("3 gap stream sessions=1 tabs=1 terminals=1\n"));
        assert!(human.ends_with("end targets-gone\n"));
        for (end, status, reason) in [
            (wire::End::TooSlow, 1, "too-slow"),
            (
                wire::End::Ended {
                    generation: "g1".into(),
                },
                0,
                "ended",
            ),
        ] {
            let (code, output) = run(&[subscribed(None), M::End(end)], true).unwrap();
            assert_eq!(code, status);
            assert!(output.contains(&format!("\"reason\":\"{reason}\"")));
            assert!(output.contains("\"resumed\":true"));
            assert!(output.contains("\"tabs\":null"));
        }
        for (code, expected) in [
            ("subscriber-limit", 1),
            ("unknown-caller", 3),
            ("ambiguous-session", 1),
            ("stop-incomplete", 4),
            ("unavailable", 4),
            ("malformed-action", 4),
        ] {
            assert_eq!(
                run(&[M::Failure(failure(code, "refused"))], true)
                    .unwrap()
                    .0,
                expected
            );
        }
        for (end, expected) in [
            (wire::End::TargetsGone, 0),
            (
                wire::End::Ended {
                    generation: "g1".into(),
                },
                0,
            ),
            (wire::End::TooSlow, 1),
        ] {
            let expected_line = match &end {
                wire::End::TargetsGone => {
                    serde_json::json!({"type":"end", "reason":"targets-gone"})
                }
                wire::End::TooSlow => serde_json::json!({"type":"end", "reason":"too-slow"}),
                wire::End::Ended { .. } => {
                    serde_json::json!({"type":"end", "reason":"ended", "generation":"g1"})
                }
            };
            let (code, output) = run(&[messages[0].clone(), M::End(end.clone())], true).unwrap();
            assert_eq!(code, expected);
            let lines = output
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                lines.as_slice(),
                std::slice::from_ref(&expected_line),
                "partial initial snapshot was written"
            );
            let (code, output) =
                run(&[subscribed(None), messages[2].clone(), M::End(end)], true).unwrap();
            assert_eq!(code, expected);
            let lines = output
                .lines()
                .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(lines.len(), 2, "partial gap snapshot was written");
            assert_eq!(lines[0]["type"], "snapshot");
            assert_eq!(lines[0]["resumed"], true);
            assert_eq!(lines[1], expected_line);
        }
        assert!(run(&messages[..1], true).is_err());
        assert!(run(&[subscribed(None)], true).is_err());
        assert!(run(&[M::Tabs(tabs.clone())], true).is_err());
        assert!(run(&[subscribed(None), M::Tabs(tabs)], true).is_err());
    }
    #[test]
    fn attention_json_keeps_untrusted_text_while_human_output_limits_it() {
        use wire::{
            Attention, AttentionKind, AttentionRecord, Change, Event, TargetId, TerminalTarget,
        };
        let event = Event {
            seq: 5,
            change: Change::Attention {
                target: TerminalTarget {
                    workspace: "s2".into(),
                    tab: "t2".into(),
                    id: TargetId::Popup("u2".into()),
                    terminal: "terminal-2".into(),
                },
                previous: Some(AttentionState::Working),
                attention: Some(Attention {
                    state: AttentionState::NeedsInput,
                    record: Some(AttentionRecord {
                        app: "claude".into(),
                        name: "agent\nname".into(),
                        kind: Some(AttentionKind::Question),
                        progress: None,
                        title: "untrusted title".into(),
                        message: "line\nquestion?".into(),
                        changed_at: 7,
                        stale: false,
                        others: 0,
                    }),
                }),
            },
        };
        let line = event_line(&event, true);
        assert!(!line.contains('\n'));
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["attention"]["kind"], "question");
        assert_eq!(value["attention"]["message"], "line\nquestion?");
        assert_eq!(value["previous"], "working");
        assert_eq!(value["pane"], serde_json::Value::Null);
        let human = event_line(&event, false);
        assert!(human.contains("5 attention session=s2 tab=t2 popup=u2 terminal=terminal-2"));
        assert!(human.contains("app=claude"));
        assert!(!human.contains('\n'));
        assert!(!human.contains("untrusted title"));
        assert!(!human.contains("question?"));
    }
    #[test]
    fn outputs_are_watched_per_platform_and_a_gone_reader_closes_them() {
        use std::os::fd::AsRawFd;
        let root = super::super::super::supervisor::temporary_directory();
        let file = std::fs::File::create(root.join("output")).unwrap();
        let output = Output::watch(file.as_fd()).unwrap();
        assert_eq!(output.fd.is_none(), cfg!(target_vendor = "apple"));
        assert!(!output.closed().unwrap());
        std::fs::remove_dir_all(root).unwrap();
        let (reader, writer) = std::io::pipe().unwrap();
        let (peer, socket) = std::os::unix::net::UnixStream::pair().unwrap();
        let outputs =
            [writer.as_fd(), socket.as_fd()].map(|fd| (fd.as_raw_fd(), Output::watch(fd).unwrap()));
        for (fd, output) in &outputs {
            assert_eq!(output.fd.map(|fd| fd.as_raw_fd()), Some(*fd));
            assert!(!output.closed().unwrap());
        }
        drop((reader, peer));
        // A child that a parallel test forks holds both peers until it execs.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        for (_, output) in &outputs {
            while !output.closed().unwrap() {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the gone reader never closed the output"
                );
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
    #[test]
    fn a_reader_that_only_stops_writing_still_receives_the_stream() {
        use std::io::Read;
        use wire::SubscriptionMessage as M;
        let (mut peer, socket) = std::os::unix::net::UnixStream::pair().unwrap();
        let output = Output::watch(socket.as_fd()).unwrap();
        peer.shutdown(std::net::Shutdown::Write).unwrap();
        assert!(!output.closed().unwrap());
        let (mut reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        for message in [subscribed(None), M::End(wire::End::TooSlow)] {
            writer
                .write_all(&wire::encode_subscription_message(&message).unwrap())
                .unwrap();
        }
        let status = consume(&mut reader, &mut &socket, &mut Vec::new(), &output, true).unwrap();
        assert_eq!(status, 1);
        drop(socket);
        let mut received = String::new();
        peer.read_to_string(&mut received).unwrap();
        assert_eq!(received.lines().count(), 2, "{received}");
        assert!(received.ends_with("\"reason\":\"too-slow\"}\n"));
    }
    #[test]
    fn a_reader_that_stops_writing_and_then_closes_ends_the_stream() {
        use std::io::Read;
        use std::sync::atomic::{AtomicBool, Ordering};
        // A frame far longer than what arrives of it in the test.
        let frame = wire::encode_subscription_message(&wire::SubscriptionMessage::Failure(
            failure("unavailable", "x".repeat(4000)),
        ))
        .unwrap();
        for dribbling in [false, true] {
            let (mut peer, socket) = std::os::unix::net::UnixStream::pair().unwrap();
            peer.shutdown(std::net::Shutdown::Write).unwrap();
            let (mut reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
            writer
                .write_all(&wire::encode_subscription_message(&subscribed(None)).unwrap())
                .unwrap();
            let (done, finished) = std::sync::mpsc::channel();
            let stream = thread::spawn(move || {
                let output = Output::watch(socket.as_fd()).unwrap();
                // As Darwin's hangup for that half-close leaves it, so Linux, which
                // would report the full close itself, proves the probe too.
                output.probe_at.set(Some(Instant::now()));
                let result = consume(&mut reader, &mut &socket, &mut Vec::new(), &output, true);
                done.send(stream_result(result).map_err(|error| error.status))
                    .unwrap();
            });
            peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut snapshot = Vec::new();
            let mut byte = [0];
            while !snapshot.ends_with(b"\n") {
                peer.read_exact(&mut byte).unwrap();
                snapshot.push(byte[0]);
            }
            assert!(snapshot.starts_with(b"{\"type\":\"snapshot\""));
            drop(peer);
            // The event socket stays open, quiet or dribbling a frame faster
            // than the probe interval: only the closed reader can end it.
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let dribble = thread::spawn({
                let (frame, stop) = (frame.clone(), stop.clone());
                move || {
                    for byte in frame.iter().filter(|_| dribbling) {
                        if stop.load(Ordering::Relaxed) || writer.write_all(&[*byte]).is_err() {
                            break;
                        }
                        thread::sleep(Duration::from_millis(20));
                    }
                    writer
                }
            });
            let ended = finished.recv_timeout(Duration::from_secs(2));
            stop.store(true, Ordering::Relaxed);
            drop(dribble.join().unwrap());
            assert_eq!(ended.unwrap(), Ok(0), "dribbling={dribbling}");
            stream.join().unwrap();
        }
    }
    #[test]
    fn refusals_use_stderr_and_truncated_ends_are_connection_loss() {
        use wire::SubscriptionMessage as M;
        let (mut reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        writer
            .write_all(
                &wire::encode_subscription_message(&M::Failure(failure(
                    "subscriber-limit",
                    "full",
                )))
                .unwrap(),
            )
            .unwrap();
        let (mut output, mut errors) = (Vec::new(), Vec::new());
        assert_eq!(
            consume(
                &mut reader,
                &mut output,
                &mut errors,
                &Output::default(),
                false
            )
            .unwrap(),
            1
        );
        assert!(output.is_empty());
        assert!(
            String::from_utf8(errors)
                .unwrap()
                .contains("subscriber-limit")
        );
        let end = wire::encode_subscription_message(&M::End(wire::End::TooSlow)).unwrap();
        for cut in [0, wire::HEADER_BYTES / 2, end.len() - 1, end.len()] {
            let (mut reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
            writer
                .write_all(&wire::encode_subscription_message(&subscribed(None)).unwrap())
                .unwrap();
            writer.write_all(&end[..cut]).unwrap();
            drop(writer);
            let result = stream_result(consume(
                &mut reader,
                &mut Vec::new(),
                &mut Vec::new(),
                &Output::default(),
                true,
            ));
            if cut == end.len() {
                assert_eq!(result.unwrap(), 1);
            } else {
                assert_eq!(result.unwrap_err().status, 4, "cut={cut}");
            }
        }
    }
}
