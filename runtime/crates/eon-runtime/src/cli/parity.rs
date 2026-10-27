//! Agent parity (AGENTS.md, Humans and Agents): each EONW action has an `eon`
//! command or a named exception. `cli_path` matches without a wildcard, so a
//! new action fails the test build until it is mapped here; give it a sample
//! in the test too.

use super::*;
use eon_workspace_protocol::v23::InvokeIntent;

enum CliPath {
    /// These arguments parse to exactly this action.
    Command(Vec<OsString>),
    /// This `lifecycle_command` sends the action or reads what it returns. It
    /// has no parse step to check, so the test checks only its `eon --help`
    /// entry; the mapping itself is reviewed.
    Lifecycle(&'static str),
    /// No command: why, naming the open bead that adds one.
    Exception(&'static str),
}

fn cli_path(action: &Action) -> CliPath {
    use CliPath::{Command, Exception, Lifecycle};
    let words = |words: &[&str]| words.iter().map(OsString::from).collect::<Vec<_>>();
    let flag = |set: bool, flag: &str| if set { words(&[flag]) } else { Vec::new() };
    let direction = |direction: &Direction| match direction {
        Direction::Left => "left",
        Direction::Right => "right",
        Direction::Up => "up",
        Direction::Down => "down",
    };
    let bytes = |bytes: &Vec<u8>| OsString::from_vec(bytes.clone());
    Command(match action {
        Action::Workspace(action) => match action {
            WorkspaceAction::Inspect => words(&["workspace"]),
            WorkspaceAction::InspectRuntime | WorkspaceAction::InspectPresentation => {
                return Lifecycle("generations");
            }
            WorkspaceAction::Present { .. } => return Lifecycle("attach"),
            WorkspaceAction::CreateTab => words(&["tab", "create"]),
            WorkspaceAction::NewWorkspace { name, focus } => [
                words(&["workspace", "new"]),
                flag(!focus, "--no-focus"),
                name.iter()
                    .flat_map(|name| words(&["--name", name]))
                    .collect(),
            ]
            .concat(),
            WorkspaceAction::SwitchWorkspace { workspace } => {
                words(&["workspace", "switch", "--", workspace])
            }
            WorkspaceAction::RenameWorkspace { workspace, name } => [
                words(&["workspace", "rename", "--"]),
                workspace.iter().map(OsString::from).collect(),
                words(&[name]),
            ]
            .concat(),
            WorkspaceAction::CloseWorkspace { workspace } => {
                words(&["workspace", "close", "--", workspace])
            }
            WorkspaceAction::CreatePane { focus } => {
                [words(&["pane", "create"]), flag(!focus, "--no-focus")].concat()
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
            WorkspaceAction::Stop { .. } => return Lifecycle("stop"),
            WorkspaceAction::SetTabDirectory { tab, directory } => [
                words(&["tab", "directory", tab, "--"]),
                vec![bytes(directory)],
            ]
            .concat(),
        },
        Action::InvokePopup { .. } | Action::DismissPopup(_) => {
            return Exception("eon-agent-parity-a9g0.1 adds popup commands");
        }
        Action::CommitDirectory { .. } => {
            return Exception(
                "the directory picker completes its own popup; `tab directory` retargets a tab, \
                 and eon-quick-search-parity-23y9.5 makes it pick for a pending tab",
            );
        }
        Action::CreateColumn
        | Action::ToggleSidebar
        | Action::FocusAttention { .. }
        | Action::NextAttention
        | Action::PreviousAttention => {
            return Exception(
                "eon-agent-parity-a9g0.6 adds column, sidebar and attention commands",
            );
        }
        Action::ClosePane { pane } => words(&["pane", "close", pane]),
        Action::RenamePane { pane, name: None } => {
            words(&["pane", "rename", "--clear", "--", pane])
        }
        Action::RenamePane {
            pane,
            name: Some(name),
        } => words(&["pane", "rename", "--", pane, name]),
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
        WorkspaceAction::InspectRuntime,
        WorkspaceAction::InspectPresentation,
        WorkspaceAction::Present { workspace: true },
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
            workspace: text("-w2"),
        },
        WorkspaceAction::RenameWorkspace {
            workspace: None,
            name: text("--json"),
        },
        WorkspaceAction::RenameWorkspace {
            workspace: Some(text("w2")),
            name: String::new(),
        },
        WorkspaceAction::CloseWorkspace {
            workspace: text("-w2"),
        },
        WorkspaceAction::CreatePane { focus: true },
        WorkspaceAction::CreatePane { focus: false },
        WorkspaceAction::FocusId(text("p2")),
        WorkspaceAction::CloseTab { tab: text("t2") },
        WorkspaceAction::Stop {
            generation: text("g1-0123456789abcdef0123456789abcdef"),
        },
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
        Action::DismissPopup(popup.clone()),
        Action::CommitDirectory {
            target: popup,
            directory: b"/tmp".to_vec(),
        },
        Action::CreateColumn,
        Action::ClosePane { pane: text("p3") },
        Action::RenamePane {
            pane: text("p1"),
            name: None,
        },
        Action::RenamePane {
            pane: text("p1"),
            name: Some(text("-Review")),
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
        Action::ToggleSidebar,
        Action::FocusAttention {
            tab: text("t1"),
            target: text("p1"),
        },
        Action::NextAttention,
        Action::PreviousAttention,
    ]);
    let help = help_text("0", false);
    for action in samples {
        match cli_path(&action) {
            // The parse `execute` runs: its first-word gate, target flags, then the action.
            CliPath::Command(arguments) => {
                assert!(control_command(&arguments[0]), "{arguments:?}");
                let parsed = parse_workspace_target_arguments(&arguments)
                    .and_then(|(arguments, _)| parse_control_arguments(&arguments));
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
            CliPath::Exception(reason) => {
                assert!(reason.contains("eon-"), "{action:?}: {reason}");
            }
        }
    }
}
