use kinestra::{Error, Recorder, Result, Size};
use serde_json::Value;
use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::{Duration, Instant},
};

fn isolated(program: impl AsRef<OsStr>, home: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("HOME", home)
        .env("USER", "demo")
        .env("LOGNAME", "demo")
        .env("LANG", "C.UTF-8")
        .env(
            "PATH",
            env::var_os("EON_DEMO_PATH").expect("Nix supplies EON_DEMO_PATH"),
        )
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_STATE_HOME", home.join(".local/state"))
        .env("XDG_CACHE_HOME", home.join(".cache"));
    command
}

fn wait_for(
    r: &mut Recorder,
    eon: &Path,
    home: &Path,
    label: &str,
    ready: impl Fn(&Value) -> bool,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let mut command = isolated(eon, home);
        command.args(["workspace", "--json"]);
        let workspace = r.output(&mut command)?;
        if serde_json::from_str::<Value>(&workspace).is_ok_and(|state| ready(&state)) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(Error::Timeout(format!("waiting for {label}")));
        }
        r.sleep(Duration::from_millis(200))?;
    }
}

fn action(r: &mut Recorder, eon: &Path, home: &Path, args: &[&str]) -> Result<()> {
    let mut command = isolated(eon, home);
    command.args(args).arg("--json");
    r.output(&mut command)?;
    Ok(())
}

fn record(r: &mut Recorder) -> Result<()> {
    let root = env::current_dir()?;
    if !root.join("components/eon-alpha-v3.json").is_file() {
        return Err(Error::Invalid(
            "run the recorder from the Eon repository root".into(),
        ));
    }
    let eon = PathBuf::from(env::var_os("EON_BIN").expect("Nix supplies EON_BIN"));
    let zoxide = env::var_os("ZOXIDE_BIN").expect("Nix supplies ZOXIDE_BIN");
    let codex = env::split_paths(
        &env::var_os("PATH").ok_or_else(|| Error::Invalid("PATH is missing".into()))?,
    )
    .map(|directory| directory.join("codex"))
    .find(|path| path.is_file())
    .ok_or_else(|| Error::Invalid("install Codex CLI to record the Agent popup".into()))?;
    if !Command::new(&codex)
        .args(["login", "status"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success()
    {
        return Err(Error::Invalid(
            "sign in with `codex login` before recording the demo".into(),
        ));
    }
    let codex_home = env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").expect("the host has a home directory"))
                .join(".codex")
        });
    let work = r.work().to_path_buf();
    let home = work.join("home");
    let demo = home.join("demo");
    fs::create_dir_all(demo.join("src"))?;
    fs::create_dir_all(demo.join("docs"))?;
    fs::create_dir_all(home.join(".config/nushell"))?;
    fs::create_dir_all(home.join(".config/eon"))?;
    fs::write(
        home.join(".config/starship.toml"),
        "[username]\ndisabled = true\n",
    )?;
    fs::write(home.join(".config/nushell/env.nu"), "")?;
    fs::write(
        home.join(".config/nushell/config.nu"),
        r#"let marker = ($env.HOME | path join '.eon-demo-pane-count')
let count = if ($marker | path exists) { open $marker | into int } else { 0 }
($count + 1) | save -f $marker
if $count == 1 {
  ^yazi
} else {
  ls
}
"#,
    )?;
    let agent = [
        codex
            .to_str()
            .ok_or_else(|| Error::Invalid("Codex CLI path must be UTF-8".into()))?,
        "--disable",
        "hooks",
        "--disable",
        "plugins",
        "--no-daemon",
        "--sandbox",
        "read-only",
        "--ask-for-approval",
        "never",
        "The Rust file contains fn main() { println!(\"hello from Eon\"); }. What does it print? Answer in one sentence. Do not use tools.",
    ];
    fs::write(
        home.join(".config/eon/config.toml"),
        format!(
            "[terminal]\nbackground_opacity = 0.72\nbackground_blur = true\n\n[anima]\nenabled = false\n\n[popups.agent]\ncommand = {}\nlabel = \"Codex\"\n",
            serde_json::to_string(&agent).expect("Codex command is JSON-compatible TOML")
        ),
    )?;
    fs::write(
        demo.join("README.md"),
        "# Eon demo\n\nA durable terminal workspace for a small Rust project.\n",
    )?;
    fs::write(
        demo.join("notes.md"),
        "Sessions keep running when the window closes.\n",
    )?;
    fs::write(
        demo.join("docs/guide.md"),
        "# Guide\n\nOpen tabs for project folders.\n",
    )?;
    fs::write(
        demo.join("src/main.rs"),
        "fn main() { println!(\"hello from Eon\"); }\n",
    )?;
    let mut git = isolated("git", &home);
    git.current_dir(&demo).args(["init", "-q", "-b", "main"]);
    r.exec(&mut git)?;
    let mut git = isolated("git", &home);
    git.current_dir(&demo).args(["add", "."]);
    r.exec(&mut git)?;
    let mut git = isolated("git", &home);
    git.current_dir(&demo).args([
        "-c",
        "user.name=Eon Demo",
        "-c",
        "user.email=demo@example.invalid",
        "-c",
        "commit.gpgsign=false",
        "commit",
        "-qm",
        "Start demo project",
    ]);
    r.exec(&mut git)?;
    fs::write(
        demo.join("notes.md"),
        "Sessions keep running when the window closes.\nTabs and panes organize the work.\n",
    )?;
    let mut history = isolated(&zoxide, &home);
    history.arg("add").arg(&demo);
    r.exec(&mut history)?;

    r.wayland_display(Size::new(1280, 720)?)?;
    let mut wallpaper = r.command("swaymsg");
    wallpaper
        .args(["output", "*", "bg"])
        .arg(root.join("assets/demo/wallpaper-frosted.png"))
        .arg("fill");
    r.exec(&mut wallpaper)?;
    let mut stop = isolated(&eon, &home);
    stop.args(["stop", "all", "--json"]);
    r.on_exit(stop);
    let mut launch = isolated(&eon, &home);
    launch.current_dir(&demo).env("CODEX_HOME", &codex_home);
    r.launch("eon", &mut launch)?;

    let video = work.join("eon-demo.mp4");
    let poster = work.join("eon-demo.png");
    let gif = work.join("eon-demo.gif");
    r.record(&video, |r| {
        r.sleep(Duration::from_secs(1))?;
        r.key("Tab", Duration::from_secs(2))?;
        r.key("Tab", Duration::from_secs(2))?;
        r.type_text("demo", Duration::from_millis(70))?;
        r.key("Return", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "first terminal", |state| {
            state["tabs"][0]["panes"]
                .as_array()
                .is_some_and(|panes| !panes.is_empty())
        })?;
        r.sleep(Duration::from_secs(1))?;
        for directory in [demo.join("src"), demo.join("docs")] {
            let mut history = isolated(&zoxide, &home);
            history.arg("add").arg(directory);
            r.exec(&mut history)?;
        }
        action(r, &eon, &home, &["pane", "create"])?;
        wait_for(r, &eon, &home, "second pane", |state| {
            state["tabs"][0]["panes"]
                .as_array()
                .is_some_and(|panes| panes.len() >= 2)
        })?;
        r.sleep(Duration::from_millis(1400))?;
        action(r, &eon, &home, &["pane", "create"])?;
        wait_for(r, &eon, &home, "third pane", |state| {
            state["tabs"][0]["panes"]
                .as_array()
                .is_some_and(|panes| panes.len() == 3)
        })?;
        r.sleep(Duration::from_secs(1))?;
        action(r, &eon, &home, &["pane", "move", "up"])?;
        wait_for(r, &eon, &home, "pane moved up", |state| {
            state["tabs"][0]["panes"][1]["id"] == "p3"
        })?;
        r.sleep(Duration::from_secs(1))?;
        action(r, &eon, &home, &["pane", "move", "down"])?;
        wait_for(r, &eon, &home, "pane moved down", |state| {
            state["tabs"][0]["panes"][2]["id"] == "p3"
        })?;
        r.sleep(Duration::from_secs(1))?;
        r.type_text("exit", Duration::from_millis(70))?;
        r.key("Return", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "third pane exited", |state| {
            state["tabs"][0]["panes"]
                .as_array()
                .is_some_and(|panes| panes.len() == 2)
        })?;
        r.sleep(Duration::from_secs(1))?;

        action(r, &eon, &home, &["tab", "create"])?;
        r.sleep(Duration::from_millis(500))?;
        r.type_text("src", Duration::from_millis(90))?;
        r.key("Return", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "source tab", |state| {
            state["active_tab"] == "t2"
                && state["tabs"][1]["pending"] == false
                && state["tabs"][1]["panes"][0]["id"] == "p4"
        })?;
        r.sleep(Duration::from_secs(1))?;
        action(r, &eon, &home, &["tab", "create"])?;
        r.sleep(Duration::from_millis(500))?;
        r.type_text("docs", Duration::from_millis(90))?;
        r.key("Return", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "docs tab", |state| {
            state["active_tab"] == "t3"
                && state["tabs"][2]["pending"] == false
                && state["tabs"][2]["panes"][0]["id"] == "p5"
        })?;
        r.sleep(Duration::from_secs(1))?;
        action(r, &eon, &home, &["tab", "move", "left"])?;
        wait_for(r, &eon, &home, "tab moved left", |state| {
            state["tabs"][1]["id"] == "t3"
        })?;
        r.sleep(Duration::from_secs(1))?;
        action(r, &eon, &home, &["tab", "move", "right"])?;
        wait_for(r, &eon, &home, "tab moved right", |state| {
            state["tabs"][2]["id"] == "t3"
        })?;
        r.sleep(Duration::from_secs(1))?;

        action(r, &eon, &home, &["focus", "t1"])?;
        wait_for(r, &eon, &home, "demo tab focused", |state| {
            state["active_tab"] == "t1"
        })?;
        action(r, &eon, &home, &["focus", "p1"])?;
        wait_for(r, &eon, &home, "first pane focused", |state| {
            state["tabs"][0]["selected_pane"] == "p1"
        })?;
        r.sleep(Duration::from_secs(1))?;
        r.key("Alt+Shift+j", Duration::from_millis(500))?;
        wait_for(r, &eon, &home, "Git popup", |state| {
            state["tabs"][0]["popups"].as_array().is_some_and(|popups| {
                popups
                    .iter()
                    .any(|popup| popup["entry"] == "git" && popup["chosen"] == true)
            })
        })?;
        r.sleep(Duration::from_millis(700))?;
        r.key("Return", Duration::from_millis(400))?;
        r.sleep(Duration::from_secs(1))?;
        r.snapshot(&poster)?;
        r.key("Alt+Shift+j", Duration::from_millis(500))?;
        wait_for(r, &eon, &home, "Git popup hidden", |state| {
            state["tabs"][0]["selected_popup"].is_null()
        })?;
        r.key("Alt+Shift+l", Duration::from_millis(500))?;
        wait_for(r, &eon, &home, "Agent popup", |state| {
            state["tabs"][0]["popups"].as_array().is_some_and(|popups| {
                popups
                    .iter()
                    .any(|popup| popup["entry"] == "agent" && popup["chosen"] == true)
            })
        })?;
        r.sleep(Duration::from_secs(1))?;
        r.key("Return", Duration::from_millis(500))?;
        r.sleep(Duration::from_secs(12))
    })?;
    r.gif(&video, &gif, 800, 10)?;

    let assets = root.join("assets/demo");
    fs::create_dir_all(&assets)?;
    for (source, name) in [
        (&video, "eon-demo.mp4"),
        (&poster, "eon-demo.png"),
        (&gif, "eon-demo.gif"),
    ] {
        let temporary = assets.join(format!(".{name}.tmp"));
        fs::copy(source, &temporary)?;
        fs::rename(temporary, assets.join(name))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    kinestra::run(record)
}
