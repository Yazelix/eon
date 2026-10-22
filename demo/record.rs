use kinestra::{Error, Recorder, Result, Size};
use serde_json::Value;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::unix::process::CommandExt,
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

struct Demo {
    root: PathBuf,
    eon: PathBuf,
    zoxide: OsString,
    codex: PathBuf,
    codex_home: PathBuf,
    work: PathBuf,
    home: PathBuf,
    demo: PathBuf,
}

/// Builds the isolated home, demo project and Zoxide history every recipe starts from.
fn prepare(r: &mut Recorder) -> Result<Demo> {
    let root = env::current_dir()?;
    if !root.join("components/eon-v4.json").is_file() {
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
    // Nested one level down so Yazi's parent column shows only this folder, not recorder files.
    let home = work.join("home/home");
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
    Ok(Demo {
        root,
        eon,
        zoxide,
        codex,
        codex_home,
        work,
        home,
        demo,
    })
}

fn record(r: &mut Recorder) -> Result<()> {
    let Demo {
        root,
        eon,
        zoxide,
        codex_home,
        work,
        home,
        demo,
        ..
    } = prepare(r)?;
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

    publish(
        &root.join("assets/demo"),
        [&video, &poster, &gif].map(PathBuf::as_path),
    )
}

/// Copies finished files into place through temporary names, so no partial asset is visible.
fn publish<'a>(assets: &Path, files: impl IntoIterator<Item = &'a Path>) -> Result<()> {
    fs::create_dir_all(assets)?;
    for source in files {
        let name = source
            .file_name()
            .ok_or_else(|| Error::Invalid("asset has no file name".into()))?
            .to_string_lossy();
        let temporary = assets.join(format!(".{name}.tmp"));
        fs::copy(source, &temporary)?;
        fs::rename(temporary, assets.join(&*name))?;
    }
    Ok(())
}

fn has_app(node: &Value, app_id: &str) -> bool {
    node["app_id"] == app_id
        || ["nodes", "floating_nodes"].iter().any(|key| {
            node[*key]
                .as_array()
                .is_some_and(|children| children.iter().any(|child| has_app(child, app_id)))
        })
}

/// Runs an Eon command the way a user does and waits for its window. Plain `eon` and `eonbase`
/// run the supervisor itself, which outlives its window. Kinestra's `launch` would signal that
/// process group, and with it the terminals, before presenting the reattach, and would report
/// `eon attach`, which asks the running supervisor for a window and exits, as a crash. A supervisor's `stop` is registered as soon as it
/// starts, with a kill of its process group behind it in case the stop fails; cleanup runs in
/// reverse registration order.
fn present(
    r: &mut Recorder,
    mut command: Command,
    app_id: &str,
    log: &Path,
    stop: Option<Command>,
) -> Result<()> {
    for (key, value) in r.command("true").get_envs() {
        match value {
            Some(value) => command.env(key, value),
            None => command.env_remove(key),
        };
    }
    let child = command
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(fs::File::create(log)?)
        .stderr(fs::File::create(log.with_extension("err"))?)
        .spawn()?;
    if let Some(stop) = stop {
        let mut kill = Command::new("sh");
        kill.args([
            "-c",
            &format!("kill -KILL -{} 2>/dev/null; true", child.id()),
        ]);
        r.on_exit(kill);
        r.on_exit(stop);
    }
    wait_window(r, app_id, true)
}

/// Snapshots only while the window is open, so a crashed client fails the take instead of
/// publishing an empty frame.
fn shoot(r: &mut Recorder, app_id: &str, output: &Path) -> Result<()> {
    wait_window(r, app_id, true)?;
    r.snapshot(output)
}

fn wait_window(r: &mut Recorder, app_id: &str, shown: bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let tree = r.output(r.command("swaymsg").args(["-t", "get_tree"]))?;
        if serde_json::from_str::<Value>(&tree).is_ok_and(|tree| has_app(&tree, app_id) == shown) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let state = if shown { "open" } else { "close" };
            return Err(Error::Timeout(format!(
                "waiting for window {app_id} to {state}"
            )));
        }
        r.sleep(Duration::from_millis(100))?;
    }
}

/// Asks the window to close as a user would, then waits until it is gone.
fn close_window(r: &mut Recorder, app_id: &str) -> Result<()> {
    r.exec(
        r.command("swaymsg")
            .arg(format!("[app_id=\"^{app_id}$\"]"))
            .arg("kill"),
    )?;
    wait_window(r, app_id, false)
}

fn active_tab(state: &Value) -> Option<&Value> {
    state["tabs"]
        .as_array()?
        .iter()
        .find(|tab| tab["id"] == state["active_tab"])
}

fn popup_chosen(entry: &'static str) -> impl Fn(&Value) -> bool {
    move |state| {
        active_tab(state)
            .and_then(|tab| tab["popups"].as_array())
            .is_some_and(|popups| {
                popups
                    .iter()
                    .any(|popup| popup["entry"] == entry && popup["chosen"] == true)
            })
    }
}

fn popup_hidden(state: &Value) -> bool {
    active_tab(state).is_some_and(|tab| tab["selected_popup"].is_null())
}

/// Records one short clip per feature for eonterm.com, with the live Codex and Claude quota chips.
fn features(r: &mut Recorder) -> Result<()> {
    let Demo {
        root,
        eon,
        zoxide,
        codex,
        codex_home,
        work,
        home,
        demo,
    } = prepare(r)?;
    let eonbase = env::var_os("EONBASE_BIN").expect("Nix supplies EONBASE_BIN");
    let claude_config = env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env::var_os("HOME").expect("the host has a home directory"))
                .join(".claude")
        });
    if !claude_config.join(".credentials.json").is_file() {
        return Err(Error::Invalid(
            "sign in to Claude Code before recording the quota chips".into(),
        ));
    }
    // Quota reads the host logins in place: PATH gains a directory exposing only `codex`.
    let bin = work.join("bin");
    fs::create_dir_all(&bin)?;
    std::os::unix::fs::symlink(&codex, bin.join("codex"))?;
    let path = env::join_paths(
        env::split_paths(&env::var_os("EON_DEMO_PATH").expect("Nix supplies EON_DEMO_PATH"))
            .chain([bin]),
    )
    .map_err(|error| Error::Invalid(error.to_string()))?;
    let eon_command = |args: &[&str]| {
        let mut command = isolated(&eon, &home);
        command
            .current_dir(&demo)
            .env("PATH", &path)
            .env("CODEX_HOME", &codex_home)
            .env("CLAUDE_CONFIG_DIR", &claude_config)
            .args(args);
        command
    };

    r.wayland_display(Size::new(1280, 720)?)?;
    let mut wallpaper = r.command("swaymsg");
    wallpaper
        .args(["output", "*", "bg"])
        .arg(root.join("assets/demo/wallpaper-frosted.png"))
        .arg("fill");
    r.exec(&mut wallpaper)?;
    // The takes are keyboard-driven, so the idle pointer would only cover content.
    r.exec(r.command("swaymsg").args(["seat", "*", "hide_cursor", "1"]))?;
    let mut stop = isolated(&eon, &home);
    stop.args(["stop", "all", "--json"]);
    present(
        r,
        eon_command(&[]),
        "eon",
        &work.join("eon.log"),
        Some(stop),
    )?;
    // The first quota read lands during the tab setup. Codex reads again a minute after its last
    // read and Claude five minutes after. Workspace JSON omits quota, so check the stills and
    // re-record.
    r.sleep(Duration::from_secs(4))?;

    let out = work.join("features");
    fs::create_dir_all(&out)?;
    let file = |name: &str| out.join(name);
    let mut assets = Vec::new();

    let active_panes = |count: usize| {
        move |state: &Value| {
            active_tab(state)
                .and_then(|tab| tab["panes"].as_array())
                .is_some_and(|panes| panes.len() == count)
        }
    };
    let focus_demo_tab = |r: &mut Recorder| {
        action(r, &eon, &home, &["focus", "t1"])?;
        wait_for(r, &eon, &home, "demo tab focused", |state| {
            state["active_tab"] == "t1"
        })
    };
    for directory in ["docs", "src"] {
        let mut history = isolated(&zoxide, &home);
        history.arg("add").arg(demo.join(directory));
        r.exec(&mut history)?;
    }
    // Clips read better with several tabs, so open demo and docs before the first take.
    r.type_text("demo", Duration::from_millis(80))?;
    r.key("Return", Duration::from_millis(300))?;
    wait_for(r, &eon, &home, "demo tab", active_panes(1))?;
    r.key("Alt+Shift+t", Duration::from_millis(600))?;
    r.type_text("docs", Duration::from_millis(80))?;
    r.key("Return", Duration::from_millis(300))?;
    wait_for(r, &eon, &home, "docs tab", |state| {
        state["active_tab"] == "t2" && state["tabs"][1]["pending"] == false
    })?;
    focus_demo_tab(r)?;
    r.sleep(Duration::from_secs(1))?;

    let clip = file("directory-picker.mp4");
    let poster = file("directory-picker.png");
    r.record(&clip, |r| {
        r.sleep(Duration::from_millis(600))?;
        r.key("Alt+Shift+t", Duration::from_millis(1000))?;
        r.key("Tab", Duration::from_millis(1200))?;
        shoot(r, "eon", &poster)?;
        r.sleep(Duration::from_millis(800))?;
        r.key("Tab", Duration::from_millis(800))?;
        r.type_text("src", Duration::from_millis(110))?;
        r.key("Return", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "source tab", |state| {
            state["active_tab"] == "t3" && state["tabs"][2]["pending"] == false
        })?;
        r.sleep(Duration::from_millis(1500))
    })?;
    assets.extend([clip, poster]);

    focus_demo_tab(r)?;
    let clip = file("tabs-panes.mp4");
    let poster = file("tabs-panes.png");
    r.record(&clip, |r| {
        r.sleep(Duration::from_millis(600))?;
        r.key("Alt+m", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "second pane", active_panes(2))?;
        r.sleep(Duration::from_millis(1200))?;
        r.key("Alt+m", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "third pane", active_panes(3))?;
        r.sleep(Duration::from_millis(900))?;
        r.key("Ctrl+Alt+k", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "pane moved up", |state| {
            active_tab(state).is_some_and(|tab| tab["panes"][1]["id"] == "p5")
        })?;
        r.sleep(Duration::from_millis(500))?;
        shoot(r, "eon", &poster)?;
        r.sleep(Duration::from_millis(400))?;
        r.key("Ctrl+Alt+Shift+l", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "tab moved right", |state| {
            state["tabs"][1]["id"] == "t1"
        })?;
        r.sleep(Duration::from_millis(900))?;
        r.key("Alt+h", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "previous tab focused", |state| {
            state["active_tab"] == "t2"
        })?;
        r.sleep(Duration::from_millis(900))?;
        r.key("Alt+l", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "demo tab focused", |state| {
            state["active_tab"] == "t1"
        })?;
        r.sleep(Duration::from_millis(1000))
    })?;
    // The top row alone shows tabs, the quota chips and the running version.
    let bar = file("eon-bar.png");
    r.exec(
        r.command("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(&poster)
            .args(["-vf", "crop=1280:32:0:0", "-y"])
            .arg(&bar),
    )?;
    assets.extend([clip, poster, bar]);

    // A ticking counter shows the terminal kept running while no window existed.
    // One wtype call for this whole command lost letters on the isolated display,
    // while single characters arrive intact.
    for character in
        "1..999 | each {|i| print $\"still running ($i)\"; sleep 1sec } | ignore".chars()
    {
        r.type_text(&character.to_string(), Duration::from_millis(10))?;
    }
    r.key("Return", Duration::from_secs(3))?;
    let clip = file("detach-reattach.mp4");
    let poster = file("detach-reattach.png");
    r.record(&clip, |r| {
        r.sleep(Duration::from_millis(1500))?;
        close_window(r, "eon")?;
        // With the window gone, the supervisor still reporting every pane shows the terminals
        // outlived it.
        wait_for(
            r,
            &eon,
            &home,
            "terminals without a window",
            active_panes(3),
        )?;
        r.sleep(Duration::from_secs(2))?;
        present(
            r,
            eon_command(&["attach"]),
            "eon",
            &work.join("attach.log"),
            None,
        )?;
        r.sleep(Duration::from_millis(2500))?;
        shoot(r, "eon", &poster)?;
        r.sleep(Duration::from_millis(500))
    })?;
    assets.extend([clip, poster]);

    let clip = file("git-popup.mp4");
    let poster = file("git-popup.png");
    r.record(&clip, |r| {
        r.sleep(Duration::from_millis(600))?;
        r.key("Alt+Shift+j", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "Git popup", popup_chosen("git"))?;
        r.sleep(Duration::from_millis(700))?;
        r.key("Return", Duration::from_millis(400))?;
        r.sleep(Duration::from_millis(1500))?;
        shoot(r, "eon", &poster)?;
        r.sleep(Duration::from_millis(1200))?;
        r.key("Alt+Shift+j", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "Git popup hidden", popup_hidden)?;
        r.sleep(Duration::from_millis(800))
    })?;
    assets.extend([clip, poster]);

    let clip = file("agent-popup.mp4");
    let poster = file("agent-popup.png");
    r.record(&clip, |r| {
        r.sleep(Duration::from_millis(600))?;
        r.key("Alt+Shift+l", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "Agent popup", popup_chosen("agent"))?;
        r.sleep(Duration::from_secs(1))?;
        r.key("Return", Duration::from_millis(500))?;
        r.sleep(Duration::from_secs(6))?;
        shoot(r, "eon", &poster)?;
        r.sleep(Duration::from_millis(800))
    })?;
    assets.extend([clip, poster]);
    r.key("Alt+Shift+l", Duration::from_millis(300))?;
    wait_for(r, &eon, &home, "Agent popup hidden", popup_hidden)?;

    let clip = file("anima.mp4");
    let poster = file("anima.png");
    r.record(&clip, |r| {
        r.sleep(Duration::from_millis(600))?;
        r.key("Alt+Shift+a", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "Anima popup", popup_chosen("anima"))?;
        r.sleep(Duration::from_secs(2))?;
        shoot(r, "eon", &poster)?;
        r.sleep(Duration::from_secs(1))?;
        r.key("Alt+Shift+a", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "Anima dismissed", popup_hidden)?;
        r.sleep(Duration::from_millis(600))
    })?;
    assets.extend([clip, poster]);

    let shortcuts = file("shortcuts.png");
    r.key("Alt+slash", Duration::from_millis(1200))?;
    shoot(r, "eon", &shortcuts)?;
    r.key("Escape", Duration::from_millis(600))?;
    assets.push(shortcuts);

    close_window(r, "eon")?;
    let still = file("eonbase.png");
    // eonbase stops by generation, so name the one its terminal will start in.
    let mut generations = isolated(&eonbase, &home);
    generations.args(["generations", "--json"]);
    let listing = serde_json::from_str::<Value>(&r.output(&mut generations)?)
        .map_err(|error| Error::Invalid(error.to_string()))?;
    let generation = listing["generations"]
        .as_array()
        .and_then(|records| records.iter().find(|record| record["kind"] == "current"))
        .and_then(|record| record["id"].as_str())
        .ok_or_else(|| Error::Invalid("eonbase reported no current generation".into()))?;
    let mut terminal = isolated(&eonbase, &home);
    terminal
        .current_dir(&demo)
        .arg("--")
        .arg(&eon)
        .args(["anima", "accelerando", "--big"]);
    let mut stop = isolated(&eonbase, &home);
    stop.args(["stop", generation, "--json"]);
    present(
        r,
        terminal,
        "eonbase",
        &work.join("eonbase.log"),
        Some(stop),
    )?;
    r.sleep(Duration::from_secs(3))?;
    shoot(r, "eonbase", &still)?;
    close_window(r, "eonbase")?;
    assets.push(still);

    // Capture stays near-lossless; the published clips and stills are encoded once for the web.
    let mut published = Vec::new();
    for asset in &assets {
        let video = asset.extension() == Some(OsStr::new("mp4"));
        let web = asset.with_extension(if video { "web.mp4" } else { "webp" });
        let mut encode = r.command("ffmpeg");
        encode
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(asset);
        if video {
            encode
                .args(["-an", "-c:v", "libx264", "-crf", "28", "-preset", "slow"])
                .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"]);
        } else {
            encode.args(["-c:v", "libwebp", "-quality", "85"]);
        }
        r.exec(encode.arg("-y").arg(&web))?;
        if video {
            fs::rename(&web, asset)?;
            published.push(asset.clone());
        } else {
            published.push(web);
        }
    }
    publish(
        &root.join("assets/demo/features"),
        published.iter().map(PathBuf::as_path),
    )
}

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None => kinestra::run(record),
        Some("features") => kinestra::run(features),
        Some(other) => {
            eprintln!("unknown recipe {other:?}; run with no argument or `features`");
            ExitCode::FAILURE
        }
    }
}
