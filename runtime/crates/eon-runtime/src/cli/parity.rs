//! Agent parity (AGENTS.md, Humans and Agents): each EONW action has an `eon`
//! command or a named exception. `cli_path` matches without a wildcard, so a
//! new action fails the test build until it is mapped here; give it a sample
//! in the test too. The Reserved Stop, outside the versioned actions, is
//! `eon window stop` and `eon quit`.

use super::*;
use eon_workspace_protocol::v43::{Filter, InvokeIntent, TerminalInput};

enum CliPath {
    /// These arguments parse to exactly this action.
    Command(Vec<OsString>),
    /// This `lifecycle_command` sends the action or reads what it returns. It
    /// has no parse step to check, so the test checks only its `eon --help`
    /// entry; the mapping itself is reviewed.
    Lifecycle(&'static str),
    /// No command: why, naming the open bead that adds one.
    Exception(&'static str),
    /// These arguments parse to a `pane send` whose one request is exactly
    /// this action.
    Send(Vec<OsString>),
    /// These arguments parse to a `popup hide` of this action's popup; the
    /// command reads its owning tab from the session.
    Hide(Vec<OsString>),
    /// These arguments parse to the `sidebar toggle` that sends this action.
    Sidebar(Vec<OsString>),
}

fn cli_path(action: &Action) -> CliPath {
    use CliPath::{Command, Exception, Hide, Lifecycle, Send, Sidebar};
    let words = |words: &[&str]| words.iter().map(OsString::from).collect::<Vec<_>>();
    let flag = |set: bool, flag: &str| if set { words(&[flag]) } else { Vec::new() };
    let direction = |direction: &Direction| match direction {
        Direction::Left => "left",
        Direction::Right => "right",
        Direction::Up => "up",
        Direction::Down => "down",
    };
    let bytes = |bytes: &Vec<u8>| OsString::from_vec(bytes.clone());
    let target = |workspace: &SessionTarget| match workspace {
        SessionTarget::Selector(selector) => words(&["--", selector]),
        SessionTarget::PersistentId(id) => words(&["--id", id]),
    };
    Command(match action {
        Action::Workspace(action) => match action {
            WorkspaceAction::Inspect => words(&["session"]),
            WorkspaceAction::InspectWorkspace {
                workspace: SessionTarget::Selector(_),
            } => {
                // Resolve the selector with session ls, then inspect its persistent ID.
                return Lifecycle("session ls");
            }
            WorkspaceAction::InspectWorkspace { workspace } => {
                [words(&["session"]), target(workspace)].concat()
            }
            WorkspaceAction::InspectRuntime | WorkspaceAction::InspectPresentation => {
                return Lifecycle("window ls");
            }
            WorkspaceAction::Present { .. } => return Lifecycle("window attach"),
            WorkspaceAction::ReloadPresentation { .. } => return Lifecycle("window reload"),
            WorkspaceAction::CreateTab => words(&["tab", "create"]),
            WorkspaceAction::NewWorkspace { name, focus } => [
                words(&["session", "new"]),
                flag(!focus, "--no-focus"),
                name.iter()
                    .flat_map(|name| words(&["--name", name]))
                    .collect(),
            ]
            .concat(),
            WorkspaceAction::SwitchWorkspace { workspace } => {
                [words(&["session", "switch"]), target(workspace)].concat()
            }
            WorkspaceAction::RenameWorkspace { workspace, name } => [
                words(&["session", "rename"]),
                workspace.as_ref().map_or_else(|| words(&["--"]), &target),
                flag(
                    matches!(workspace, Some(SessionTarget::PersistentId(_))),
                    "--",
                ),
                words(&[name]),
            ]
            .concat(),
            WorkspaceAction::CloseWorkspace { workspace } => {
                [words(&["session", "close"]), target(workspace)].concat()
            }
            WorkspaceAction::CreatePane { focus } => {
                [words(&["pane", "create"]), flag(!focus, "--no-focus")].concat()
            }
            WorkspaceAction::FocusId(tab) if tab_number(tab).is_some() => {
                words(&["tab", "switch", tab])
            }
            WorkspaceAction::FocusId(id) => words(&["focus", id]),
            WorkspaceAction::Focus(to) => words(&["focus", direction(to)]),
            WorkspaceAction::Move(to @ (Direction::Left | Direction::Right)) => {
                words(&["tab", "move", direction(to)])
            }
            WorkspaceAction::Move(to @ (Direction::Up | Direction::Down)) => {
                words(&["pane", "move", direction(to)])
            }
            WorkspaceAction::CloseTab { tab } => words(&["tab", "close", tab]),
            WorkspaceAction::SetTabDirectory { tab, directory } => [
                words(&["tab", "directory", tab, "--"]),
                vec![bytes(directory)],
            ]
            .concat(),
        },
        Action::InvokePopup { .. } => {
            return Exception("eon-agent-parity-a9g0.1 adds popup commands");
        }
        Action::DismissPopup(target) => return Hide(words(&["popup", "hide", &target.instance])),
        Action::ToggleSidebar => return Sidebar(words(&["sidebar", "toggle"])),
        Action::SetSidebar { open } => {
            return Sidebar(words(&["sidebar", if *open { "show" } else { "hide" }]));
        }
        Action::CreateColumn
        | Action::FocusAttention { .. }
        | Action::NextAttention
        | Action::PreviousAttention => {
            return Exception("eon-agent-parity-a9g0.6 adds column and attention commands");
        }
        Action::Subscribe { .. } => return Lifecycle("events"),
        Action::ReadTerminal { target } => words(&["pane", "read", target]),
        Action::ReadTerminalHistory { target } => words(&["pane", "history", target]),
        Action::EditTerminalHistory { target } => words(&["pane", "history", target, "--edit"]),
        Action::SendTerminal {
            target,
            input,
            force,
        } => {
            let mut arguments = words(&["pane", "send", target]);
            if *force {
                arguments.push("--force".into());
            }
            arguments.extend(match input {
                TerminalInput::Paste(text) => words(&["--", text]),
                TerminalInput::Enter => words(&["--enter"]),
            });
            return Send(arguments);
        }
        Action::ClosePane { pane } => words(&["pane", "close", pane]),
        Action::CloseColumn { pane, .. } => words(&["column", "close", pane]),
        Action::RenamePane { pane, name, tag } => {
            let mut arguments = words(&["pane", "rename"]);
            if let Some(tag) = tag {
                arguments.extend(words(&["--tag", tag]));
            }
            if name.is_none() {
                arguments.push("--clear".into());
            }
            arguments.extend(words(&["--", pane]));
            if let Some(name) = name {
                arguments.push(name.into());
            }
            arguments
        }
        Action::MovePaneSideways(side) => words(&[
            "pane",
            "move",
            match side {
                Side::Left => "left",
                Side::Right => "right",
            },
        ]),
        Action::Edit { file } => [words(&["edit", "--"]), vec![bytes(file)]].concat(),
        Action::CreateCommandPane { argv, focus } => [
            words(&["pane", "create"]),
            flag(!focus, "--no-focus"),
            words(&["--"]),
            argv.iter().map(bytes).collect(),
        ]
        .concat(),
        Action::LaunchTab {
            directory,
            argv,
            focus,
        } => [
            words(&["tab", "create"]),
            flag(!focus, "--no-focus"),
            words(&["--dir"]),
            vec![bytes(directory)],
            flag(!argv.is_empty(), "--"),
            argv.iter().map(bytes).collect(),
        ]
        .concat(),
    })
}

#[test]
fn every_eonw_action_has_a_cli_path_or_a_named_exception() {
    let text = |value: &str| value.to_owned();
    let popup = PopupTarget {
        tab: text("t1"),
        instance: text("u1"),
    };
    let mut samples = vec![
        WorkspaceAction::Inspect,
        WorkspaceAction::InspectWorkspace {
            workspace: SessionTarget::Selector(text("notes")),
        },
        WorkspaceAction::InspectWorkspace {
            workspace: SessionTarget::PersistentId(text(
                "0123456789abcdef0123456789abcdef0000000000000001",
            )),
        },
        WorkspaceAction::SwitchWorkspace {
            workspace: SessionTarget::PersistentId(text(
                "0123456789abcdef0123456789abcdef0000000000000001",
            )),
        },
        WorkspaceAction::RenameWorkspace {
            workspace: Some(SessionTarget::PersistentId(text(
                "0123456789abcdef0123456789abcdef0000000000000001",
            ))),
            name: text("--json"),
        },
        WorkspaceAction::CloseWorkspace {
            workspace: SessionTarget::PersistentId(text(
                "0123456789abcdef0123456789abcdef0000000000000001",
            )),
        },
        WorkspaceAction::InspectRuntime,
        WorkspaceAction::InspectPresentation,
        WorkspaceAction::Present { workspace: true },
        WorkspaceAction::ReloadPresentation {
            venus: "/nix/store/fixture/bin/venus".into(),
            eonw: eon_workspace_protocol::v43::VERSION,
            orbs: 14,
            orbf: 3,
        },
        WorkspaceAction::CreateTab,
        WorkspaceAction::NewWorkspace {
            name: None,
            focus: true,
        },
        WorkspaceAction::NewWorkspace {
            name: Some(text("-notes")),
            focus: false,
        },
        WorkspaceAction::SwitchWorkspace {
            workspace: SessionTarget::Selector(text("-s2")),
        },
        WorkspaceAction::RenameWorkspace {
            workspace: None,
            name: text("--json"),
        },
        WorkspaceAction::RenameWorkspace {
            workspace: Some(SessionTarget::Selector(text("s2"))),
            name: String::new(),
        },
        WorkspaceAction::CloseWorkspace {
            workspace: SessionTarget::Selector(text("-s2")),
        },
        WorkspaceAction::CreatePane { focus: true },
        WorkspaceAction::CreatePane { focus: false },
        WorkspaceAction::FocusId(text("p2")),
        WorkspaceAction::FocusId(text("t2")),
        WorkspaceAction::CloseTab { tab: text("t2") },
        WorkspaceAction::SetTabDirectory {
            tab: text("t1"),
            directory: b"/tmp/a\xff".to_vec(),
        },
    ]
    .into_iter()
    .chain(
        [
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ]
        .into_iter()
        .flat_map(|to| [WorkspaceAction::Focus(to), WorkspaceAction::Move(to)]),
    )
    .map(Action::Workspace)
    .collect::<Vec<_>>();
    samples.extend([
        Action::InvokePopup {
            tab: text("t1"),
            entry: text("lazygit"),
            expected_instance: None,
            intent: InvokeIntent::Toggle,
        },
        Action::DismissPopup(popup),
        Action::CreateColumn,
        Action::ClosePane { pane: text("p3") },
        Action::CloseColumn {
            pane: text("p3"),
            consent: None,
        },
        Action::RenamePane {
            pane: text("p1"),
            name: None,
            tag: None,
        },
        Action::RenamePane {
            pane: text("p1"),
            name: Some(text("-Review")),
            tag: None,
        },
        Action::RenamePane {
            pane: text("p1"),
            name: Some(text("--clear")),
            tag: Some(text("--json")),
        },
        Action::RenamePane {
            pane: text("p1"),
            name: Some(text("")),
            tag: Some(text("")),
        },
        Action::MovePaneSideways(Side::Left),
        Action::MovePaneSideways(Side::Right),
        Action::Edit {
            file: b"/tmp/notes\xff.md".to_vec(),
        },
        Action::CreateCommandPane {
            argv: vec![b"echo".to_vec(), b"--window".to_vec(), b"\xff".to_vec()],
            focus: false,
        },
        Action::LaunchTab {
            directory: b"/tmp/project-\xff".to_vec(),
            argv: vec![b"echo".to_vec(), b"--window".to_vec(), b"\xff".to_vec()],
            focus: false,
        },
        Action::LaunchTab {
            directory: b"/tmp/project".to_vec(),
            argv: vec![],
            focus: true,
        },
        Action::ToggleSidebar,
        Action::SetSidebar { open: true },
        Action::SetSidebar { open: false },
        Action::FocusAttention {
            tab: text("t1"),
            target: text("p1"),
        },
        Action::NextAttention,
        Action::PreviousAttention,
        Action::ReadTerminal { target: text("p1") },
        Action::ReadTerminalHistory { target: text("u1") },
        Action::EditTerminalHistory { target: text("p1") },
        Action::SendTerminal {
            target: text("u1"),
            force: false,
            input: TerminalInput::Paste(text("echo hi")),
        },
        Action::SendTerminal {
            target: text("p1"),
            force: true,
            input: TerminalInput::Enter,
        },
        Action::SendTerminal {
            target: text("p1"),
            force: true,
            input: TerminalInput::Paste(text("hi")),
        },
        Action::Subscribe {
            filter: Filter::default(),
            after: None,
        },
    ]);
    let help = help_text("0", false);
    for action in samples {
        match cli_path(&action) {
            // The parse `execute` runs: its first-word gate, target flags, then the action.
            CliPath::Command(arguments) => {
                assert!(control_command(&arguments[0]), "{arguments:?}");
                let parsed =
                    parse_workspace_target_arguments(&arguments).and_then(|(arguments, _)| {
                        match parse_close_arguments(&arguments)? {
                            Some((action, json, _yes)) => Ok((action, json)),
                            None => parse_control_arguments(&arguments),
                        }
                    });
                assert_eq!(
                    parsed.map_err(|error| error.message),
                    Ok((action, false)),
                    "{arguments:?}"
                );
            }
            CliPath::Lifecycle(command) => assert!(
                help.lines()
                    .any(|line| line.trim_start().starts_with(&format!("{command} "))),
                "{command} is not in eon --help"
            ),
            CliPath::Send(arguments) => {
                assert!(control_command(&arguments[0]), "{arguments:?}");
                let send = parse_workspace_target_arguments(&arguments)
                    .and_then(|(arguments, _)| parse_pane_send_arguments(&arguments))
                    .map_err(|error| error.message)
                    .unwrap()
                    .unwrap();
                let input = match (send.text, send.enter) {
                    (SendText::Operand(text), false) => {
                        TerminalInput::Paste(text.into_string().unwrap())
                    }
                    (SendText::None, true) => TerminalInput::Enter,
                    other => panic!("{arguments:?} is not one request: {other:?}"),
                };
                assert!(!send.json, "{arguments:?}");
                assert_eq!(
                    Action::SendTerminal {
                        target: send.target,
                        force: send.force,
                        input
                    },
                    action,
                    "{arguments:?}"
                );
            }
            CliPath::Hide(arguments) => {
                assert!(control_command(&arguments[0]), "{arguments:?}");
                let (popup, json) = parse_workspace_target_arguments(&arguments)
                    .and_then(|(arguments, _)| parse_popup_hide_arguments(&arguments))
                    .map_err(|error| error.message)
                    .unwrap()
                    .unwrap();
                assert!(!json, "{arguments:?}");
                let Action::DismissPopup(target) = &action else {
                    panic!("{action:?}");
                };
                assert_eq!(popup, target.instance, "{arguments:?}");
            }
            CliPath::Sidebar(arguments) => {
                assert!(control_command(&arguments[0]), "{arguments:?}");
                let parsed = parse_workspace_target_arguments(&arguments)
                    .and_then(|(arguments, _)| parse_sidebar_arguments(&arguments))
                    .map_err(|error| error.message);
                let verb = match action {
                    Action::ToggleSidebar => SidebarVerb::Toggle,
                    Action::SetSidebar { open: true } => SidebarVerb::Show,
                    Action::SetSidebar { open: false } => SidebarVerb::Hide,
                    _ => panic!("{action:?}"),
                };
                assert_eq!(parsed, Ok(Some((verb, false))), "{arguments:?}");
            }
            CliPath::Exception(reason) => {
                assert!(reason.contains("eon-"), "{action:?}: {reason}");
            }
        }
    }
}
