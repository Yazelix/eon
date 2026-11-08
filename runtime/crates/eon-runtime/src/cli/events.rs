use super::super::{control as endpoint, supervisor::request_id, workspace as projection};
use super::*;
use eon_workspace_protocol::v38::{self as wire, EventKind, Filter, Resume};

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
        target: WorkspaceTarget::default(),
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
    let result = if stdout_closed(libc::STDOUT_FILENO).map_err(lost)? {
        Ok(0)
    } else {
        execute_inner(inputs, arguments)
    };
    unsafe {
        libc::signal(libc::SIGINT, previous);
    }
    if stdout_closed(libc::STDOUT_FILENO).map_err(lost)? {
        Ok(0)
    } else {
        result
    }
}

fn execute_inner(inputs: &Inputs, arguments: &[OsString]) -> Result<i32, Error> {
    let options = parse(arguments)?;
    let mut output = std::io::stdout().lock();
    let mut errors = std::io::stderr().lock();
    let stdout = Some(libc::STDOUT_FILENO);
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

fn stdout_closed(fd: i32) -> std::io::Result<bool> {
    let mut poll = libc::pollfd {
        fd,
        events: 0,
        revents: 0,
    };
    loop {
        if unsafe { libc::poll(&mut poll, 1, 0) } >= 0 {
            return Ok(poll.revents != 0);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn refuse(
    output: &mut impl Write,
    errors: &mut impl Write,
    stdout: Option<i32>,
    failure: &Failure,
    json: bool,
) -> std::io::Result<i32> {
    if let Some(fd) = stdout
        && stdout_closed(fd)?
    {
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
    stdout: Option<i32>,
    deadline: Option<Instant>,
) -> std::io::Result<wire::SubscriptionMessage> {
    use std::os::fd::AsRawFd;
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
        let mut fds = [
            libc::pollfd {
                fd: stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: stdout.unwrap_or(-1),
                events: 0,
                revents: 0,
            },
        ];
        let timeout = match deadline {
            Some(deadline) => i32::try_from(
                deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis(),
            )
            .unwrap_or(i32::MAX),
            None => -1,
        };
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
        if result < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
        if result == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "subscription handshake timed out",
            ));
        }
        if fds[1].revents != 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
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
    stdout: Option<i32>,
    json: bool,
) -> std::io::Result<i32> {
    use wire::SubscriptionMessage as Message;
    let deadline = Some(Instant::now() + endpoint::CONTROL_TIMEOUT);
    let subscribed = match read_message(stream, stdout, deadline)? {
        Message::Subscribed(value) => value,
        Message::Failure(error) => return refuse(output, errors, stdout, &error, json),
        _ => return Err(invalid("expected SUBSCRIBED")),
    };
    stream.set_read_timeout(None)?;
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
    stdout: Option<i32>,
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
        let result = consume(&mut reader, &mut output, &mut Vec::new(), None, json);
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
                persistent_id: "0123456789abcdef0123456789abcdef0000000000000001".into(),
                name: Some("a\"b".into()),
                tab_count: 1,
                attention: None,
                recency: 0,
            },
            active_tab: "t1".into(),
        };
        let tabs = wire::Tabs {
            workspace: "s1".into(),
            tabs: vec![tab("t1", vec![vec![pane("p1", "terminal-1")]], vec![])],
        };
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
            consume(&mut reader, &mut output, &mut errors, None, false).unwrap(),
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
                None,
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
