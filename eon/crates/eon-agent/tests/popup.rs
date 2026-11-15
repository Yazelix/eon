#![cfg(target_os = "linux")]

use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rota"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", root)
        .env("CLAUDE_CONFIG_DIR", root.join("inherited-store"))
        .env("ROTA_CONFIG", root.join("agent.toml"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn config(root: &Path, program: &str, extra: &str) {
    fs::write(
        root.join("agent.toml"),
        format!(
            "state_file = '{0}/state'\nlog_file = '{0}/decisions'\n\
             [[backends]]\nname = 'provider'\nroles = ['implement']\n\
             guard = ['true']\nallow_unknown_quota = true\n{extra}\n\
             [backends.commands]\nmedium = ['{program}', '{{prompt}}']\n\
             high = ['{program}', '{{prompt}}']\n",
            root.display()
        ),
    )
    .unwrap();
}

#[test]
fn popup_dry_run_composes_resume_without_a_prompt_argument() {
    let root = tempfile::tempdir().unwrap();
    for (program, command) in [
        ("codex", "codex resume --no-daemon"),
        ("claude", "claude --resume"),
        ("grok", "grok"),
        ("opencode", "opencode"),
        ("pi", "pi"),
        ("/fictional/codex", "/fictional/codex resume --no-daemon"),
    ] {
        config(root.path(), program, "");
        let output = run(root.path(), &["launch", "--popup", "--dry-run"]);
        assert!(output.status.success(), "{program}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("pick: provider\ncommand: {command}\n")
        );
        assert!(!root.path().join("state").exists());
        assert!(!root.path().join("decisions").exists());
    }
}

#[test]
fn popup_mode_validates_custom_commands_and_preserves_ordinary_launch() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    config(root, "custom", "");
    let output = run(root, &["launch", "--popup", "--dry-run"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("no popup_command")
    );
    fs::write(root.join("brief"), "ordinary prompt").unwrap();
    let ordinary = run(root, &["launch", "--dry-run", "brief"]);
    assert!(ordinary.status.success(), "{ordinary:?}");
    for extra in [
        "popup_command = []",
        "popup_command = ['', '--resume']",
        "popup_command = ['custom', '{prompt}']",
        "popup_command = ['custom', 'prefix{prompt}']",
        "popup_command = [\"custom\", \"\\u0000\"]",
    ] {
        config(root, "custom", extra);
        assert_eq!(
            run(root, &["launch", "--popup", "--dry-run"]).status.code(),
            Some(2)
        );
    }
    config(root, "custom", "popup_command = ['printf', '{cwd}']");
    let output = run(root, &["launch", "--popup", "--dry-run"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "pick: provider\ncommand: printf {}\n",
            fs::canonicalize(root).unwrap().display()
        )
    );
    for args in [
        vec!["launch"],
        vec!["launch", "--popup", "brief"],
        vec!["launch", "--popup", "--role", "review"],
        vec!["launch", "--level", "medium", "--popup"],
        vec!["launch", "--popup", "--popup"],
    ] {
        assert_eq!(run(root, &args).status.code(), Some(2), "{args:?}");
    }
    // Real exec gets only the configured argument, while recording admission.
    let output = run(root, &["launch", "--popup"]);
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "pick: provider\n{}",
            fs::canonicalize(root).unwrap().display()
        )
    );
    assert_eq!(
        fs::read_to_string(root.join("state")).unwrap(),
        "provider\n"
    );
    let decision: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("decisions")).unwrap()).unwrap();
    assert_eq!(decision["decision"], "pick");
    assert_eq!(
        decision["folder"],
        fs::canonicalize(root).unwrap().to_str().unwrap()
    );
}

#[test]
fn popup_refusal_and_busy_lock_preserve_exit_codes_and_rotation() {
    use fs2::FileExt;
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    config(root, "custom", "popup_command = ['true']");
    let file = root.join("agent.toml");
    let available = fs::read_to_string(&file).unwrap();
    fs::write(
        &file,
        available.replace("guard = ['true']", "guard = ['false']"),
    )
    .unwrap();
    let refused = run(root, &["launch", "--popup"]);
    assert_eq!(refused.status.code(), Some(3), "{refused:?}");
    assert!(
        String::from_utf8(refused.stderr)
            .unwrap()
            .contains("no backend available, refusing")
    );
    assert!(!root.join("state").exists());
    let decision = fs::read_to_string(root.join("decisions")).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&decision).unwrap()["decision"],
        "REFUSAL"
    );
    fs::write(&file, available).unwrap();
    let lock = fs::File::create(root.join("state.lock")).unwrap();
    lock.lock_exclusive().unwrap();
    let busy = run(root, &["launch", "--popup"]);
    assert_eq!(busy.status.code(), Some(3), "{busy:?}");
    assert!(
        String::from_utf8(busy.stderr)
            .unwrap()
            .contains("another rota command")
    );
    assert_eq!(
        fs::read_to_string(root.join("decisions")).unwrap(),
        decision
    );
    assert!(!root.join("state").exists());
    drop(lock);
    config(
        root,
        "custom",
        "popup_command = ['/fictional/missing-agent']",
    );
    let failed = run(root, &["launch", "--popup"]);
    assert_eq!(failed.status.code(), Some(1), "{failed:?}");
    assert!(
        String::from_utf8(failed.stderr)
            .unwrap()
            .contains("could not exec provider")
    );
}

fn pinned_config(root: &Path) -> String {
    format!(
        "state_file = '{root}/state'\nlog_file = '{root}/decisions'\n\
         fallback = 'main'\npopup_backend = 'main'\n\
         [[backends]]\nname = 'secondary'\nroles = ['implement']\n\
         guard = ['sh', '-c', 'touch {root}/secondary-checked; test ! -f {root}/deny-secondary']\n\
         allow_unknown_quota = true\nprepare = [['touch', '{root}/secondary-prepared']]\n\
         [backends.env]\nCLAUDE_CONFIG_DIR = '{root}/.sclaude'\n\
         [backends.commands]\nmedium = ['{root}/claude', '{{prompt}}']\n\
         high = ['{root}/claude', '{{prompt}}']\n\
         [[backends]]\nname = 'main'\nroles = []\nunset_env = ['CLAUDE_CONFIG_DIR']\n\
         guard = ['sh', '-c', 'touch {root}/main-checked; test ! -f {root}/deny-main']\n\
         allow_unknown_quota = true\nprepare = [['touch', '{root}/main-prepared']]\n\
         [backends.commands]\nmedium = ['{root}/claude', '{{prompt}}']\n\
         high = ['{root}/claude', '{{prompt}}']\n",
        root = root.display(),
    )
}

#[test]
fn popup_pin_keeps_the_main_resume_store_and_existing_rotation() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    fs::create_dir_all(root.join(".claude/projects")).unwrap();
    let transcript = root.join(".claude/projects/session.jsonl");
    fs::write(&transcript, "synthetic session\n").unwrap();
    fs::write(
        root.join("claude"),
        "#!/bin/sh\nstore=${CLAUDE_CONFIG_DIR:-$HOME/.claude}\n\
         printf 'argv=%s\\nstore=%s\\n' \"$*\" \"$store\"\n\
         if [ -f \"$store/projects/session.jsonl\" ]; then cat \"$store/projects/session.jsonl\"; fi\n",
    )
    .unwrap();
    fs::set_permissions(root.join("claude"), fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(root.join("agent.toml"), pinned_config(root)).unwrap();
    let dry = run(root, &["launch", "--popup", "--dry-run"]);
    assert!(dry.status.success(), "{dry:?}");
    let dry_text = String::from_utf8(dry.stdout).unwrap();
    assert!(dry_text.starts_with("pick: main\ncommand: env -u CLAUDE_CONFIG_DIR "));
    assert!(dry_text.ends_with("/claude --resume\n"));
    assert!(!root.join("state").exists());
    assert!(!root.join("decisions").exists());
    assert!(!root.join("main-prepared").exists());
    assert!(!root.join("secondary-checked").exists());

    let popup = run(root, &["launch", "--popup"]);
    assert!(popup.status.success(), "{popup:?}");
    assert_eq!(
        String::from_utf8(popup.stdout).unwrap(),
        format!(
            "pick: main\nargv=--resume\nstore={}/.claude\nsynthetic session\n",
            root.display()
        )
    );
    assert!(
        String::from_utf8(popup.stderr)
            .unwrap()
            .contains("FALLBACK main")
    );
    assert_eq!(fs::read_to_string(root.join("state")).unwrap(), "main\n");
    assert!(root.join("main-prepared").exists());
    assert!(!root.join("secondary-checked").exists());
    assert!(!root.join("secondary-prepared").exists());
    let decision: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(root.join("decisions")).unwrap()).unwrap();
    assert_eq!(decision["pick"], "main");
    assert_eq!(decision["decision"], "FALLBACK");

    fs::write(root.join("brief"), "ordinary prompt").unwrap();
    let ordinary = run(root, &["launch", "brief"]);
    assert!(ordinary.status.success(), "{ordinary:?}");
    assert_eq!(
        String::from_utf8(ordinary.stdout).unwrap(),
        format!(
            "pick: secondary\nargv=ordinary prompt\nstore={}/.sclaude\n",
            root.display()
        )
    );
    assert_eq!(
        fs::read_to_string(root.join("state")).unwrap(),
        "secondary\n"
    );
    let explicit = run(
        root,
        &["launch", "--popup", "--only", "secondary", "--dry-run"],
    );
    assert!(explicit.status.success(), "{explicit:?}");
    assert!(
        String::from_utf8(explicit.stdout)
            .unwrap()
            .starts_with("pick: secondary\n")
    );
    assert_eq!(
        fs::read_to_string(transcript).unwrap(),
        "synthetic session\n"
    );
}

#[test]
fn popup_pin_refuses_without_rescue_from_an_available_backend_or_fallback() {
    for pin in ["main", "secondary"] {
        for disabled in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let root = root.path();
            let mut config = pinned_config(root).replace(
                "popup_backend = 'main'",
                &format!("popup_backend = '{pin}'"),
            );
            if disabled {
                config = config.replace(
                    &format!("name = '{pin}'"),
                    &format!("name = '{pin}'\nenabled = false"),
                );
            } else {
                fs::write(root.join(format!("deny-{pin}")), "").unwrap();
            }
            fs::write(root.join("agent.toml"), config).unwrap();
            fs::write(root.join("state"), "previous\n").unwrap();
            let output = run(root, &["launch", "--popup"]);
            assert_eq!(
                output.status.code(),
                Some(3),
                "{pin}, disabled={disabled}: {output:?}"
            );
            assert_eq!(
                fs::read_to_string(root.join("state")).unwrap(),
                "previous\n"
            );
            let other = if pin == "main" { "secondary" } else { "main" };
            assert!(!root.join(format!("{other}-checked")).exists());
            assert!(!root.join("main-prepared").exists());
            assert!(!root.join("secondary-prepared").exists());
            let record: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(root.join("decisions")).unwrap()).unwrap();
            assert_eq!(record["decision"], "REFUSAL");
            assert!(record["pick"].is_null());
        }
    }
}
