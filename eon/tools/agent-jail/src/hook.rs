//! The jail hook and the dcg shim, run by the harnesses as PreToolUse hooks
//! (through `/bin/sh -c`, so in the inner layer).
//!
//! Shell commands always start through a wrapper: Claude runs Bash through
//! `CLAUDE_CODE_SHELL`, and Codex 0.170's only shell tool (`exec_command`)
//! runs `[shell, -c, cmd]` with the shell found by name, and every shell in
//! the jail is a wrapper. What runs inside the harness process are its file
//! tools, so the hook confines their paths to the workspace with no link in
//! any component. It fails closed: unknown tools, unknown shapes and
//! malformed input are denied (exit 2 with a reason on stderr, which both
//! harnesses treat as a block).

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::spec::HookCheck;

/// Exit status that blocks the tool call in both harnesses.
pub const DENY_EXIT: i32 = 2;

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
}

/// Decides one PreToolUse input. `root` prefixes filesystem checks (tests).
pub fn decide(harness: &str, input: &str, root: &Path) -> Decision {
    decide_with(harness, input, root, true)
}

/// [`decide`], with the link rule as a switch: only the probe's links
/// control turns it off, to show what the kernel seal alone does when a
/// race has beaten the hook.
pub fn decide_with(harness: &str, input: &str, root: &Path, refuse_links: bool) -> Decision {
    let Ok(json) = serde_json::from_str::<Value>(input) else {
        return Decision::Deny("the hook input is not JSON".into());
    };
    let (Some(tool), Some(tool_input)) = (
        json.get("tool_name").and_then(Value::as_str),
        json.get("tool_input"),
    ) else {
        return Decision::Deny("the hook input has no tool_name or tool_input".into());
    };
    let cwd = json
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or(crate::WORKSPACE);
    let field = |name: &str| tool_input.get(name).and_then(Value::as_str);
    match (harness, tool) {
        (_, "Bash") => match field("command") {
            None => Decision::Deny("Bash input has no command string".into()),
            // Codex applies a patch written as a shell command in-process,
            // in the outer layer, from a directory the command may change in
            // any spelling; the apply_patch tool is the only patch route.
            Some(command) if harness == "codex" && command.contains("*** Begin Patch") => {
                Decision::Deny(
                    "patches go through the apply_patch tool, not a shell command".into(),
                )
            }
            // A background command would outlive this check and could swap
            // workspace links under later file-tool checks.
            Some(_)
                if !matches!(
                    tool_input.get("run_in_background"),
                    None | Some(Value::Null) | Some(Value::Bool(false))
                ) =>
            {
                Decision::Deny("background shell commands are not allowed in the bench jail".into())
            }
            Some(_) => Decision::Allow,
        },
        ("codex", "apply_patch") => {
            let base = match tool_input.get("workdir") {
                None | Some(Value::Null) => cwd.to_owned(),
                Some(Value::String(workdir)) => {
                    match check_path_with(cwd, workdir, root, refuse_links) {
                        Decision::Allow => Path::new(cwd).join(workdir).display().to_string(),
                        deny => return deny,
                    }
                }
                Some(_) => return Decision::Deny("apply_patch workdir is not a string".into()),
            };
            match field("command")
                .or_else(|| field("input"))
                .or_else(|| field("patch"))
            {
                Some(patch) => check_patch(patch, &base, root, refuse_links),
                None => Decision::Deny("apply_patch input has no patch text".into()),
            }
        }
        ("codex", "update_plan") => Decision::Allow,
        // Claude tools with no filesystem, process or network reach of their
        // own. Subagents (Agent, Task) and background tools stay denied.
        (
            "claude",
            "ToolSearch" | "TodoWrite" | "TaskCreate" | "TaskUpdate" | "TaskGet" | "TaskList",
        ) => Decision::Allow,
        ("claude", "Glob")
            if field("pattern").is_some_and(|pattern| {
                pattern.starts_with('/')
                    || pattern.starts_with('~')
                    || pattern.split('/').any(|part| part == "..")
            }) =>
        {
            Decision::Deny("Glob patterns must be relative to the workspace".into())
        }
        (
            "claude",
            "Read" | "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "NotebookRead" | "LS"
            | "Grep" | "Glob",
        ) => {
            let paths: Vec<&str> = ["file_path", "path", "notebook_path"]
                .into_iter()
                .filter_map(field)
                .collect();
            if paths.is_empty() && !matches!(tool, "Grep" | "Glob") {
                return Decision::Deny(format!("{tool} input has no path"));
            }
            for path in paths {
                if let Decision::Deny(reason) = check_path_with(cwd, path, root, refuse_links) {
                    return Decision::Deny(reason);
                }
            }
            check_path_with(cwd, ".", root, refuse_links)
        }
        _ => Decision::Deny(format!("the tool {tool} is not allowed in the bench jail")),
    }
}

/// Every file an apply_patch body names must pass [`check_path`].
fn check_patch(patch: &str, cwd: &str, root: &Path, refuse_links: bool) -> Decision {
    let headers = [
        "*** Add File:",
        "*** Update File:",
        "*** Delete File:",
        "*** Move to:",
    ];
    let mut seen = 0;
    for line in patch.lines() {
        let line = line.trim();
        if let Some(path) = headers.iter().find_map(|header| line.strip_prefix(header)) {
            seen += 1;
            if let Decision::Deny(reason) = check_path_with(cwd, path.trim(), root, refuse_links) {
                return Decision::Deny(reason);
            }
        }
    }
    if seen == 0 {
        return Decision::Deny("the patch names no file".into());
    }
    Decision::Allow
}

/// `path` (relative to `cwd`) must stay inside the workspace with no `..`
/// and no symbolic link in any existing component.
pub fn check_path(cwd: &str, path: &str, root: &Path) -> Decision {
    check_path_with(cwd, path, root, true)
}

fn check_path_with(cwd: &str, path: &str, root: &Path, refuse_links: bool) -> Decision {
    if path.is_empty() || path.starts_with('~') {
        return Decision::Deny(format!(
            "{path:?}: empty and ~ paths are not allowed in the bench jail"
        ));
    }
    if !cwd.starts_with('/') {
        return Decision::Deny(format!("{cwd:?}: the working directory must be absolute"));
    }
    let joined = Path::new(cwd).join(path);
    let mut normal = PathBuf::from("/");
    for component in joined.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => normal.push(name),
            Component::ParentDir | Component::Prefix(_) => {
                return Decision::Deny(format!(
                    "{path}: paths with .. are not allowed in the bench jail"
                ));
            }
        }
    }
    let workspace = Path::new(crate::WORKSPACE);
    let Ok(inside) = normal.strip_prefix(workspace) else {
        return Decision::Deny(format!(
            "{path}: only paths inside {} are allowed",
            crate::WORKSPACE
        ));
    };
    let mut current = root.join(workspace.strip_prefix("/").unwrap_or(workspace));
    for component in inside.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() && refuse_links => {
                return Decision::Deny(format!(
                    "{path}: symbolic links are not followed in the bench jail"
                ));
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => break,
            Err(err) => return Decision::Deny(format!("{path}: cannot check for links: {err}")),
        }
    }
    Decision::Allow
}

/// `agent-jail jail-hook <harness>`: reads stdin, prints a deny reason on
/// stderr and returns the exit status.
pub fn run_jail_hook(harness: &str) -> i32 {
    use std::io::Read;
    let mut input = String::new();
    let decision = match std::io::stdin().take(4 << 20).read_to_string(&mut input) {
        Ok(_) => decide(harness, &input, Path::new("/")),
        Err(err) => Decision::Deny(format!("cannot read the hook input: {err}")),
    };
    respond(decision)
}

/// Prints a deny reason on stderr and returns the hook's exit status.
pub fn respond(decision: Decision) -> i32 {
    match decision {
        Decision::Allow => 0,
        Decision::Deny(reason) => {
            eprintln!("agent-jail: {reason}");
            DENY_EXIT
        }
    }
}

/// `agent-jail dcg-hook <harness>`: runs the pinned dcg with the jail's
/// read-only config and a clean environment.
pub fn run_dcg_hook(dcg: &str) -> i32 {
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new(dcg)
        .env_clear()
        .env("DCG_CONFIG", format!("{}/dcg/config.toml", crate::JAIL_DIR))
        .env("DCG_FAIL_CLOSED", "1")
        .env("DCG_NO_UPDATE_CHECK", "1")
        .env("DCG_NO_SELF_HEAL", "1")
        .env("DCG_HISTORY_DISABLED", "1")
        .env("DCG_NON_INTERACTIVE", "1")
        .env("DCG_NO_COLOR", "1")
        .env("HOME", crate::HOME)
        .env("PATH", crate::BIN_DIR)
        .exec();
    eprintln!("agent-jail: cannot run dcg: {err}");
    DENY_EXIT
}

/// Reads a hook's verdict from its exit status and stdout.
pub fn denied(status: i32, stdout: &str) -> bool {
    if status == DENY_EXIT {
        return true;
    }
    let Ok(json) = serde_json::from_str::<Value>(stdout.trim()) else {
        return false;
    };
    json.pointer("/hookSpecificOutput/permissionDecision")
        .and_then(Value::as_str)
        == Some("deny")
        || json.get("decision").and_then(Value::as_str) == Some("block")
}

fn input(tool: &str, tool_input: Value) -> String {
    serde_json::json!({
        "hook_event_name": "PreToolUse",
        "cwd": crate::WORKSPACE,
        "tool_name": tool,
        "tool_input": tool_input,
    })
    .to_string()
}

/// The guard's live hook checks (condition 3) for `harness`.
pub fn guard_checks(harness: &str, control: crate::control::Control) -> Vec<HookCheck> {
    let dcg = format!("{}/agent-jail dcg-hook {harness}", crate::JAIL_DIR);
    let jail = control.jail_hook_command(harness);
    let bash = |command: &str| input("Bash", serde_json::json!({ "command": command }));
    let check = |name: &str, command: &str, input: String, deny: bool| HookCheck {
        name: name.into(),
        command: command.into(),
        input,
        deny,
    };
    let auth = if harness == "claude" {
        input(
            "Read",
            serde_json::json!({ "file_path": "/home/bench/.claude/.credentials.json" }),
        )
    } else {
        input(
            "apply_patch",
            serde_json::json!({ "command": "*** Begin Patch\n*** Update File: /home/bench/.codex/auth.json\n*** Move to: /bench/workspace/x\n*** End Patch\n" }),
        )
    };
    vec![
        check("dcg denies rm -rf /", &dcg, bash("rm -rf /"), true),
        check("dcg allows true", &dcg, bash("true"), false),
        check("the jail hook denies a credential path", &jail, auth, true),
        check(
            "the jail hook denies an unknown tool",
            &jail,
            input("web_fetch", serde_json::json!({})),
            true,
        ),
        check(
            "the jail hook allows bash",
            &jail,
            bash("bash -c true"),
            false,
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(command: &str) -> String {
        input("Bash", serde_json::json!({ "command": command }))
    }

    fn patch(body: &str) -> String {
        input("apply_patch", serde_json::json!({ "command": body }))
    }

    fn root() -> PathBuf {
        let root = crate::test_dir("hook");
        let workspace = root.join("bench/workspace");
        std::fs::create_dir_all(workspace.join("src")).unwrap();
        std::fs::write(workspace.join("src/lib.rs"), "x").unwrap();
        std::os::unix::fs::symlink("/home/bench/.codex/auth.json", workspace.join("leak")).unwrap();
        std::os::unix::fs::symlink("/home", workspace.join("homedir")).unwrap();
        std::fs::create_dir_all(workspace.join("sub")).unwrap();
        std::os::unix::fs::symlink("/home/bench/.codex/auth.json", workspace.join("sub/x"))
            .unwrap();
        root
    }

    #[test]
    fn malformed_and_unknown_inputs_are_denied() {
        let root = root();
        for (harness, text) in [
            ("codex", "not json".to_owned()),
            ("codex", "{}".to_owned()),
            ("codex", input("Bash", serde_json::json!({ "cmd": ["ls"] }))),
            (
                "codex",
                input("view_image", serde_json::json!({ "path": "/x" })),
            ),
            ("codex", input("mcp__x__y", serde_json::json!({}))),
            (
                "claude",
                input(
                    "WebFetch",
                    serde_json::json!({ "url": "https://github.com" }),
                ),
            ),
            ("claude", input("Read", serde_json::json!({}))),
        ] {
            assert!(
                matches!(decide(harness, &text, &root), Decision::Deny(_)),
                "{harness} {text}"
            );
        }
    }

    #[test]
    fn shell_commands_are_allowed_because_they_start_through_wrappers() {
        let root = root();
        assert_eq!(
            decide(
                "codex",
                &bash("/nix/store/x/bin/base64 /home/bench/.codex/auth.json"),
                &root
            ),
            Decision::Allow
        );
        assert_eq!(decide("claude", &bash("ls"), &root), Decision::Allow);
        assert_eq!(
            decide("codex", &input("update_plan", serde_json::json!({})), &root),
            Decision::Allow
        );
    }

    #[test]
    fn patches_stay_in_the_workspace_without_links() {
        let root = root();
        let ok = "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-x\n+y\n*** Add File: src/new/file.rs\n+z\n*** End Patch\n";
        assert_eq!(decide("codex", &patch(ok), &root), Decision::Allow);
        for bad in [
            "*** Begin Patch\n*** Update File: /home/bench/.codex/auth.json\n*** Move to: x\n*** End Patch\n",
            "*** Begin Patch\n*** Update File: leak\n*** Move to: copy\n*** End Patch\n",
            "*** Begin Patch\n*** Add File: homedir/bench/x\n+x\n*** End Patch\n",
            "*** Begin Patch\n*** Add File: ../../home/bench/x\n+x\n*** End Patch\n",
            "*** Begin Patch\n*** Update File: src/lib.rs\n*** Move to: /tmp/x\n*** End Patch\n",
            "*** Begin Patch\n*** End Patch\n",
        ] {
            assert!(
                matches!(decide("codex", &patch(bad), &root), Decision::Deny(_)),
                "{bad}"
            );
        }
        // Every shell-form patch is denied, whatever the directory change
        // is spelled as, and even when its paths look harmless.
        for prefix in [
            "",
            "cd /home/bench/.codex && ",
            "cd\t/home/bench/.codex && ",
            "cd\"/home/bench/.codex\" && ",
            "pushd /home/bench/.codex && ",
        ] {
            let command = format!("{prefix}apply_patch <<'EOF'\n{ok}EOF\n");
            assert!(
                matches!(decide("codex", &bash(&command), &root), Decision::Deny(_)),
                "{command}"
            );
        }
        // The tool's workdir moves the base the patch paths resolve against.
        let with_workdir = |workdir: Value| {
            input(
                "apply_patch",
                serde_json::json!({ "command": "*** Begin Patch\n*** Update File: auth.json\n*** End Patch\n", "workdir": workdir }),
            )
        };
        for workdir in [
            serde_json::json!("/home/bench/.codex"),
            serde_json::json!("../../home"),
            serde_json::json!(7),
        ] {
            assert!(
                matches!(
                    decide("codex", &with_workdir(workdir.clone()), &root),
                    Decision::Deny(_)
                ),
                "{workdir}"
            );
        }
        assert_eq!(
            decide("codex", &with_workdir(serde_json::json!("src")), &root),
            Decision::Allow
        );
        // Patch paths resolve against the workdir: `x` is a link in `sub`
        // (and absent from the workspace root).
        let in_sub = input(
            "apply_patch",
            serde_json::json!({
                "command": "*** Begin Patch\n*** Update File: x\n*** End Patch\n",
                "workdir": "sub"
            }),
        );
        assert!(matches!(decide("codex", &in_sub, &root), Decision::Deny(_)));
    }

    #[test]
    fn tilde_empty_and_relative_inputs_fail_closed() {
        let root = root();
        let read = |path: &str| input("Read", serde_json::json!({ "file_path": path }));
        for path in ["~/.claude/.credentials.json", "~", ""] {
            assert!(
                matches!(decide("claude", &read(path), &root), Decision::Deny(_)),
                "{path:?}"
            );
        }
        assert!(matches!(
            check_path("bench/workspace", "x", &root),
            Decision::Deny(_)
        ));
        // An lstat error other than "not found" denies.
        let blocked = root.join("bench/workspace/locked");
        std::fs::create_dir_all(&blocked).unwrap();
        std::fs::write(blocked.join("f"), "x").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let denied = std::fs::symlink_metadata(blocked.join("f")).is_err();
        let verdict = check_path("/bench/workspace", "locked/f", &root);
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o755)).unwrap();
        if denied {
            assert!(matches!(verdict, Decision::Deny(_)));
        }
    }

    #[test]
    fn shipped_claude_hooks_send_every_tool_through_the_jail_hook() {
        let home = crate::manifest::Home::shipped("claude").unwrap();
        let settings: Value =
            serde_json::from_str(&home.source_text("managed-settings.json").unwrap()).unwrap();
        let entries = settings["hooks"]["PreToolUse"].as_array().unwrap();
        let catch_all = entries
            .iter()
            .find(|entry| entry["matcher"] == "*")
            .expect("a catch-all PreToolUse matcher");
        assert!(
            catch_all["hooks"]
                .as_array()
                .unwrap()
                .iter()
                .any(|hook| hook["command"] == "/bench/jail/agent-jail jail-hook claude")
        );
        for entry in entries {
            for hook in entry["hooks"].as_array().unwrap() {
                assert_eq!(hook["onFailure"], "block", "{hook}");
            }
        }
    }

    #[test]
    fn the_links_control_drops_only_the_link_rule() {
        let root = root();
        let read = |path: &str| input("Read", serde_json::json!({ "file_path": path }));
        assert!(matches!(
            decide("claude", &read("leak"), &root),
            Decision::Deny(_)
        ));
        assert_eq!(
            decide_with("claude", &read("leak"), &root, false),
            Decision::Allow
        );
        for path in [
            "/home/bench/.claude/.credentials.json",
            "../../home/x",
            "~/.claude/x",
        ] {
            assert!(
                matches!(
                    decide_with("claude", &read(path), &root, false),
                    Decision::Deny(_)
                ),
                "{path}"
            );
        }
        assert!(matches!(
            decide_with(
                "claude",
                &input("Agent", serde_json::json!({})),
                &root,
                false
            ),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn claude_tools_are_denied_by_default() {
        let root = root();
        for tool in ["ToolSearch", "TodoWrite", "TaskCreate"] {
            assert_eq!(
                decide("claude", &input(tool, serde_json::json!({})), &root),
                Decision::Allow,
                "{tool}"
            );
        }
        for tool in [
            "Monitor",
            "Agent",
            "Task",
            "BashOutput",
            "KillShell",
            "WebSearch",
            "WebFetch",
            "Skill",
            "mcp__x__y",
            "SomethingNew",
        ] {
            assert!(
                matches!(
                    decide("claude", &input(tool, serde_json::json!({})), &root),
                    Decision::Deny(_)
                ),
                "{tool}"
            );
        }
        let glob = |pattern: &str| input("Glob", serde_json::json!({ "pattern": pattern }));
        for pattern in ["/home/bench/**", "~/.claude/*", "../../home/*"] {
            assert!(
                matches!(decide("claude", &glob(pattern), &root), Decision::Deny(_)),
                "{pattern}"
            );
        }
        assert_eq!(
            decide("claude", &glob("src/**/*.rs"), &root),
            Decision::Allow
        );
        let background = input(
            "Bash",
            serde_json::json!({ "command": "sleep 100", "run_in_background": true }),
        );
        assert!(matches!(
            decide("claude", &background, &root),
            Decision::Deny(_)
        ));
        let foreground = input(
            "Bash",
            serde_json::json!({ "command": "ls", "run_in_background": false }),
        );
        assert_eq!(decide("claude", &foreground, &root), Decision::Allow);
    }

    #[test]
    fn claude_file_tools_stay_in_the_workspace_without_links() {
        let root = root();
        let read = |path: &str| input("Read", serde_json::json!({ "file_path": path }));
        assert_eq!(
            decide("claude", &read("/bench/workspace/src/lib.rs"), &root),
            Decision::Allow
        );
        assert_eq!(
            decide("claude", &read("src/lib.rs"), &root),
            Decision::Allow
        );
        for bad in [
            "/home/bench/.claude/.credentials.json",
            "/bench/workspace/leak",
            "leak",
            "/proc/1/environ",
        ] {
            assert!(
                matches!(decide("claude", &read(bad), &root), Decision::Deny(_)),
                "{bad}"
            );
        }
        let grep = input("Grep", serde_json::json!({ "pattern": "x" }));
        assert_eq!(decide("claude", &grep, &root), Decision::Allow);
        let grep_out = input(
            "Grep",
            serde_json::json!({ "pattern": "x", "path": "/home/bench" }),
        );
        assert!(matches!(
            decide("claude", &grep_out, &root),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn verdicts_read_exit_status_and_json() {
        assert!(denied(2, ""));
        assert!(denied(
            0,
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"x"}}"#
        ));
        assert!(denied(0, r#"{"decision":"block","reason":"x"}"#));
        assert!(!denied(0, ""));
        assert!(!denied(1, "garbage"));
    }
}
