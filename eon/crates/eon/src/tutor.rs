//! Printed lessons in the caller's terminal; workspace actions remain the learner's choice.
mod state;

use serde_json::{Value, json};
use std::{
    env,
    ffi::OsString,
    fs,
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub(crate) fn initial_picker_header() -> u8 {
    let (Ok(mut notice), Some(marker)) = (
        env::var("EON_FILES_NOTICE"),
        env::var_os("EON_QUICK_MARKER").map(PathBuf::from),
    ) else {
        return 2;
    };
    if !env::var("EON_FILES_START").is_ok_and(|start| start == "quick")
        || !env::var("EON_QUICK_TOKEN").is_ok_and(|token| token == "initial")
        || !initial_picker_ready(&marker)
    {
        return 2;
    }
    if state::claim_discovery() {
        if !notice.is_empty() {
            notice.push('\n');
        }
        notice.push_str("New to Eon? Run eon tutor.");
    }
    u8::from(io::stdout().lock().write_all(notice.as_bytes()).is_err())
}

fn initial_picker_ready(marker: &Path) -> bool {
    if !marker.is_absolute() {
        return false;
    }
    let Ok(file) = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(marker)
    else {
        return false;
    };
    let initial = b"initial";
    let mut token = Vec::new();
    file.metadata().is_ok_and(|metadata| metadata.is_file())
        && file
            .take((initial.len() + 1) as u64)
            .read_to_end(&mut token)
            .is_ok()
        && token == initial
}

const HELP: &str = "\
eon tutor [list|N|next|--reset] [--json]

Print lessons in your own terminal.
  tutor          Welcome and current lesson
  tutor list     Show all lessons and marks
  tutor N        Select lesson 1 through 10
  tutor next     Continue without verification
  tutor --reset  Reset only lesson progress
  tutor --help   Show this help
JSON: lessons/list/next/reset, not help.
Read means presented, not verified.
Progress is saved in Eon's state directory.
Only tutorial progress is saved.
";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Current,
    List,
    Lesson(usize),
    Next,
    Reset,
}

enum Line {
    Text(&'static str),
    Action(&'static str, &'static str),
    Popup(&'static str, &'static str),
}

struct Lesson {
    id: &'static str,
    title: &'static str,
    lines: &'static [Line],
}

use Line::{Action, Popup, Text};
const LESSONS: &[Lesson; 10] = &[
    Lesson {
        id: "tabs-directory",
        title: "Tabs and directories",
        lines: &[
            Text("Goal: give each project its own tab."),
            Text("Create a tab and choose a directory."),
            Action("create_tab", "eon tab create"),
            Text("Use the footer to choose a directory."),
            Text("Switch and move tabs; close only a"),
            Text("practice tab when you are finished."),
            Text("eon keys lists move and close bindings."),
        ],
    },
    Lesson {
        id: "panes",
        title: "Panes in a Stack",
        lines: &[
            Text("Goal: work in two panes of one tab."),
            Text("Create a pane in your working Stack."),
            Action("create_pane", "eon pane create"),
            Text("Try eon pane move up or down."),
            Text("eon session lists IDs for pane close."),
            Text("Close only work you choose to end."),
        ],
    },
    Lesson {
        id: "columns",
        title: "Side by side",
        lines: &[
            Text("Goal: use two columns in one tab."),
            Action("create_column", "eon pane create"),
            Text("CLI: from the new pane, run"),
            Text("eon pane move right"),
            Text("Move a practice pane left and right."),
            Text("A column vanishes when its last pane"),
            Text("leaves it. Your terminal keeps running."),
        ],
    },
    Lesson {
        id: "files",
        title: "Files",
        lines: &[
            Text("Goal: keep Files handy for this tab."),
            Popup("files", "eon popup show files"),
            Text("Hide Files, then reopen it."),
            Text("Quick search uses the same Files terminal."),
            Text("Use its footer for navigation keys."),
            Text("Reprint Tutor after returning here."),
        ],
    },
    Lesson {
        id: "git",
        title: "Git",
        lines: &[
            Text("Goal: inspect a project's Git changes."),
            Text("Choose one of your Git projects first."),
            Popup("git", "eon popup show git"),
            Text("Inspect a diff, then hide Git."),
            Text("No staging or commit is required."),
            Text("If Git is unavailable, continue below."),
        ],
    },
    Lesson {
        id: "editor",
        title: "Editor",
        lines: &[
            Text("Goal: use your real configured editor."),
            Popup("editor", "eon popup show editor"),
            Text("Hide and reopen it; keep your buffers."),
            Text("To open a file you choose, run"),
            Text("eon edit -- FILE"),
            Text("No editor? Set [editor] command first,"),
            Text("or continue. Nothing is faked or saved."),
        ],
    },
    Lesson {
        id: "agent",
        title: "Agent",
        lines: &[
            Text("Goal: find your configured Agent."),
            Popup("agent", "eon popup show agent"),
            Text("It may need your installed provider."),
            Text("Hide the popup; hiding does not stop it."),
            Text("Tutor never starts an agent for you."),
            Text("No provider? You can continue below."),
        ],
    },
    Lesson {
        id: "attention",
        title: "Agent attention",
        lines: &[
            Text("Goal: find agents that need attention."),
            Action("toggle_sidebar", "eon sidebar show"),
            Text("Reporting agents show working,"),
            Text("needs-input or finished status."),
            Text("Choose a row to return to that agent."),
            Text("eon keys lists attention bindings."),
            Text("No reporting agent? Continue anyway."),
        ],
    },
    Lesson {
        id: "bar-cli",
        title: "The bar and CLI",
        lines: &[
            Text("Goal: inspect and act through the CLI."),
            Text("Find tabs and + in the top bar."),
            Text("Quota/load chips are readings; clock"),
            Text("and version sit beside bottom hints."),
            Text("Run eon session --json for state."),
            Text("Run eon pane create for a new pane."),
            Text("No quota account is required."),
        ],
    },
    Lesson {
        id: "detach-return",
        title: "Detach and return",
        lines: &[
            Text("Ungraded guide: leave work running."),
            Action("detach", "eon window ls"),
            Text("Run eon window ls for your window ID."),
            Text("From another shell on the same host"),
            Text("and Runtime namespace, run"),
            Text("eon window attach ID"),
            Text("Re-run eon tutor whenever you return."),
        ],
    },
];

fn parse(arguments: &[OsString]) -> Result<Operation, &'static str> {
    let mut operation = None;
    let mut json = false;
    for argument in arguments {
        let word = argument.to_str().ok_or("Invalid tutor argument")?;
        if word == "--json" && !json {
            json = true;
            continue;
        }
        let next = match word {
            "list" => Operation::List,
            "next" => Operation::Next,
            "--reset" => Operation::Reset,
            n if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => Operation::Lesson(
                n.parse::<usize>()
                    .ok()
                    .filter(|n| (1..=10).contains(n))
                    .ok_or("Lesson number must be 1 through 10")?
                    - 1,
            ),
            _ => return Err("Unknown tutor argument; run eon tutor --help"),
        };
        if operation.replace(next).is_some() {
            return Err("Choose only one tutor operation");
        }
    }
    Ok(operation.unwrap_or(Operation::Current))
}

fn keymap() -> Option<Value> {
    let mut command = Command::new(std::env::current_exe().ok()?);
    command.arg("keys").arg("--json");
    // Fixed interactive-read budget: missing state must not hold up a printed lesson.
    let output = crate::update::bounded(command, Duration::from_secs(2)).ok()?;
    if !output.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    (value["bindings"].is_array() && value["popups"].is_array()).then_some(value)
}

fn binding<'a>(keys: Option<&'a Value>, line: &Line) -> Option<&'a str> {
    let (array, field, id) = match line {
        Action(id, _) => ("bindings", "action", *id),
        Popup(id, _) => ("popups", "id", *id),
        Text(_) => return None,
    };
    let keys = keys?;
    if keys["input_locked"] == true {
        return None;
    }
    let display = keys[array]
        .as_array()?
        .iter()
        .find(|row| row[field] == id)?["display"]
        .as_str()?;
    // These are physical-key labels, never arbitrary terminal control sequences.
    (!display.is_empty()
        && display.len() <= 128
        && display.bytes().all(|b| b.is_ascii_graphic() || b == b' '))
    .then_some(display)
}

fn blocked(lesson: &Lesson, keys: Option<&Value>) -> Option<String> {
    let entries = keys?["popups"].as_array()?;
    lesson.lines.iter().find_map(|line| match line {
        Popup(id, _) if !entries.iter().any(|entry| entry["id"] == *id) => {
            Some(format!("{} disabled or unavailable.", lesson.title))
        }
        _ => None,
    })
}

fn wrap(lines: &mut Vec<String>, text: &str) {
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + word.len() + 1 > 44 {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        // All lesson copy and admitted physical chords are ASCII, one byte per cell.
        for byte in word.bytes() {
            if current.len() == 44 {
                lines.push(std::mem::take(&mut current));
            }
            current.push(char::from(byte));
        }
    }
    lines.push(current);
}

fn render(index: usize, progress: &state::Progress, keys: Option<&Value>) -> Vec<String> {
    let lesson = &LESSONS[index];
    let mut lines = Vec::new();
    wrap(&mut lines, "Welcome to Eon. Learn in your own pane.");
    wrap(&mut lines, &format!("{}/10: {}", index + 1, lesson.title));
    lines.push(String::new());
    for line in lesson.lines {
        match line {
            Text(text) => wrap(&mut lines, text),
            Action(_, command) | Popup(_, command) => {
                if let Some(key) = binding(keys, line) {
                    wrap(&mut lines, &format!("Key: {key}"));
                } else if let Popup(id, _) = line {
                    wrap(
                        &mut lines,
                        &format!("Find entry {id} in eon session --json."),
                    );
                    wrap(&mut lines, "Use its uN popup ID below.");
                    wrap(&mut lines, "Existing: eon focus uN");
                    wrap(&mut lines, &format!("Absent: {command}"));
                    wrap(&mut lines, "Hide: eon popup hide uN");
                } else {
                    wrap(&mut lines, &format!("CLI: {command}"));
                }
            }
        }
    }
    if let Some(reason) = blocked(lesson, keys) {
        wrap(&mut lines, &reason);
    }
    if keys.is_none() {
        wrap(&mut lines, "Keys unavailable; CLI steps still work.");
    }
    lines.push(String::new());
    wrap(
        &mut lines,
        if progress.verified[index] {
            "Previously verified."
        } else if index == 9 {
            "Ungraded; no verification recorded."
        } else {
            "Read is not verification."
        },
    );
    wrap(&mut lines, "eon tutor next  |  eon tutor list");
    lines
}

pub(crate) fn run(arguments: &[OsString]) -> u8 {
    let json_output = arguments.iter().any(|a| a == "--json");
    let result = if arguments.len() == 1 && matches!(arguments[0].to_str(), Some("--help" | "-h")) {
        Ok(HELP.to_owned())
    } else {
        execute(arguments, json_output)
    };
    match result {
        Ok(output) => match io::stdout().lock().write_all(output.as_bytes()) {
            Ok(()) => 0,
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => 0,
            Err(error) => {
                eprintln!("eon tutor: {error}");
                1
            }
        },
        Err((status, message)) => {
            if json_output {
                let error = json!({"schema_version": 1, "error": {
                    "code": if status == 2 { "invalid-arguments" } else { "progress-unavailable" },
                    "message": message,
                }});
                let _ = writeln!(io::stdout().lock(), "{error}");
            } else {
                eprintln!("eon tutor: {message}");
            }
            status
        }
    }
}

fn execute(arguments: &[OsString], json_output: bool) -> Result<String, (u8, String)> {
    let operation = parse(arguments).map_err(|e| (2, e.into()))?;
    let progress = state::progress(operation).map_err(|e| (1, e))?;
    if operation == Operation::List && !json_output {
        let mut lines = vec!["Eon tutor: lessons".to_owned()];
        for (index, lesson) in LESSONS.iter().enumerate() {
            let mark = if progress.verified[index] {
                "verified"
            } else if progress.read[index] {
                "read"
            } else {
                "unread"
            };
            lines.push(format!(
                "{} {:2} {} - {mark}",
                if index == progress.current { ">" } else { " " },
                index + 1,
                lesson.title
            ));
        }
        lines.push("10 is an ungraded guide.".into());
        return Ok(format!("{}\n", lines.join("\n")));
    }
    let keys = keymap();
    if !json_output {
        let mut lines = render(progress.current, &progress, keys.as_ref());
        if operation == Operation::Next && progress.current == 9 {
            wrap(&mut lines, "Last lesson; select any number to revisit.");
        }
        return Ok(format!("{}\n", lines.join("\n")));
    }
    let lessons = LESSONS
        .iter()
        .enumerate()
        .map(|(index, lesson)| {
            json!({
                "id": lesson.id, "number": index + 1, "title": lesson.title,
                "text": render(index, &progress, keys.as_ref()), "read": progress.read[index],
                "verified": progress.verified[index], "checkable": index != 9,
                "blocked_reason": blocked(lesson, keys.as_ref()),
            })
        })
        .collect::<Vec<_>>();
    Ok(format!(
        "{}\n",
        json!({"schema_version": 1, "catalog_version": 1,
            "current_lesson": progress.current + 1, "check_available": false,
            "lessons": lessons, "check_result": null,
        })
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_input_shows_the_cli_step_for_a_bound_chord() {
        let mut keys = json!({
            "bindings": [{"action": "create_pane", "display": "Alt+M", "source": "default"}],
            "popups": [],
            "input_locked": true
        });
        let progress = state::Progress::default();
        let locked = render(1, &progress, Some(&keys)).join("\n");
        assert!(locked.contains("CLI: eon pane create"), "{locked}");
        keys["input_locked"] = false.into();
        let unlocked = render(1, &progress, Some(&keys)).join("\n");
        assert!(unlocked.contains("Key: Alt+M"), "{unlocked}");
    }

    #[test]
    fn released_pane_shortcut_shows_the_cli_step() {
        let keys = json!({
            "bindings": [{"action": "create_pane", "display": null, "source": "released"}],
            "popups": []
        });
        let text = render(1, &state::Progress::default(), Some(&keys)).join("\n");
        assert!(text.contains("CLI: eon pane create"), "{text}");
    }

    #[test]
    fn unbound_popup_instructions_distinguish_retained_and_absent_instances() {
        let keys = json!({"bindings": [], "popups": [
            {"id": "files", "display": null}, {"id": "git", "display": null},
            {"id": "editor", "display": null}, {"id": "agent", "display": null}
        ]});
        let progress = state::Progress::default();
        for map in [None, Some(&keys)] {
            for index in 3..=6 {
                let text = render(index, &progress, map).join("\n");
                assert!(text.contains("eon session --json"), "{text}");
                assert!(text.contains("Existing: eon focus uN"), "{text}");
                assert!(text.contains("Absent: eon popup show"), "{text}");
                assert!(text.contains("Hide: eon popup hide uN"), "{text}");
                assert!(!text.contains("CLI: eon popup show"), "{text}");
            }
        }
    }

    #[test]
    fn active_and_overridden_bindings_and_all_feedback_fit() {
        let mut keys = json!({"bindings": [], "popups": []});
        for lesson in LESSONS {
            for line in lesson.lines {
                match line {
                    Action(id, _) => keys["bindings"].as_array_mut().unwrap().push(
                        json!({"action": id, "display": "Control+Option+Shift+Command+PageDown"}),
                    ),
                    Popup(id, _) => keys["popups"].as_array_mut().unwrap().push(
                        json!({"id": id, "display": "Control+Option+Shift+Command+PageDown"}),
                    ),
                    Text(_) => {}
                }
            }
        }
        let mut progress = state::Progress::default();
        progress.verified[0] = true;
        for data in [
            None,
            Some(keys.clone()),
            Some(json!({"bindings": [], "popups": []})),
        ] {
            for index in 0..10 {
                let lines = render(index, &progress, data.as_ref());
                assert!(lines.len() <= 20, "{}: {lines:?}", index + 1);
                assert!(lines.iter().all(|line| line.is_ascii() && line.len() <= 44));
            }
        }
        assert!(
            render(0, &progress, Some(&keys))
                .join("\n")
                .contains("Key: Control+Option+Shift+Command+PageDown")
        );
        assert!(
            render(9, &progress, Some(&keys))
                .join("\n")
                .contains("eon window ls")
        );
        keys["bindings"][0]["display"] = Value::Null;
        assert!(
            render(0, &progress, Some(&keys))
                .join("\n")
                .contains("CLI: eon tab create")
        );
        keys["bindings"][0]["display"] = json!("\u{1b}[2J");
        assert!(
            render(0, &progress, Some(&keys))
                .join("\n")
                .contains("CLI: eon tab create")
        );
    }
}
