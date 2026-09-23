use kinestra::{Error, Recorder, Result, Size};
use serde_json::Value;
use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
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

fn record(r: &mut Recorder) -> Result<()> {
    let root = env::current_dir()?;
    if !root.join("components/eon-alpha-v3.json").is_file() {
        return Err(Error::Invalid(
            "run the recorder from the Eon repository root".into(),
        ));
    }
    let eon = PathBuf::from(env::var_os("EON_BIN").expect("Nix supplies EON_BIN"));
    let zoxide = env::var_os("ZOXIDE_BIN").expect("Nix supplies ZOXIDE_BIN");
    let work = r.work().to_path_buf();
    let home = work.join("home");
    let demo = home.join("demo");
    fs::create_dir_all(demo.join("src"))?;
    fs::create_dir_all(home.join(".config/nushell"))?;
    fs::write(home.join(".config/nushell/env.nu"), "")?;
    fs::write(
        home.join(".config/nushell/config.nu"),
        "let marker = ($env.HOME | path join '.eon-demo-first-pane')\nif ($marker | path exists) {\n  ^yazi\n} else {\n  'ready' | save $marker\n  ls\n}\n",
    )?;
    fs::write(
        demo.join("README.md"),
        "# Eon demo\nA durable terminal workspace.\n",
    )?;
    fs::write(
        demo.join("notes.md"),
        "Sessions keep running when the window closes.\n",
    )?;
    fs::write(
        demo.join("src/main.rs"),
        "fn main() { println!(\"hello from Eon\"); }\n",
    )?;
    let mut history = isolated(&zoxide, &home);
    history.arg("add").arg(&demo);
    r.exec(&mut history)?;

    r.wayland_display(Size::new(1280, 720)?)?;
    let mut stop = isolated(&eon, &home);
    stop.args(["stop", "all", "--json"]);
    r.on_exit(stop);
    let mut launch = isolated(&eon, &home);
    launch.current_dir(&demo);
    r.launch("eon", &mut launch)?;

    let video = work.join("eon-demo.mp4");
    let poster = work.join("eon-demo.png");
    r.record(&video, |r| {
        r.sleep(Duration::from_secs(1))?;
        r.type_text("demo", Duration::from_millis(70))?;
        r.key("Return", Duration::from_millis(300))?;
        wait_for(r, &eon, &home, "first terminal", |state| {
            state["tabs"][0]["panes"]
                .as_array()
                .is_some_and(|panes| !panes.is_empty())
        })?;
        r.sleep(Duration::from_secs(1))?;
        let mut create_pane = isolated(&eon, &home);
        create_pane.args(["pane", "create", "--json"]);
        r.output(&mut create_pane)?;
        wait_for(r, &eon, &home, "second pane", |state| {
            state["tabs"][0]["panes"]
                .as_array()
                .is_some_and(|panes| panes.len() >= 2)
        })?;
        r.sleep(Duration::from_secs(2))?;
        let mut focus = isolated(&eon, &home);
        focus.args(["focus", "p1", "--json"]);
        r.output(&mut focus)?;
        wait_for(r, &eon, &home, "first pane selection", |state| {
            state["tabs"][0]["selected_pane"] == "p1"
        })?;
        r.sleep(Duration::from_secs(1))?;
        let mut focus = isolated(&eon, &home);
        focus.args(["focus", "p2", "--json"]);
        r.output(&mut focus)?;
        wait_for(r, &eon, &home, "second pane selection", |state| {
            state["tabs"][0]["selected_pane"] == "p2"
        })?;
        r.sleep(Duration::from_secs(1))?;
        r.snapshot(&poster)?;
        r.sleep(Duration::from_secs(1))
    })?;

    let assets = root.join("assets/demo");
    fs::create_dir_all(&assets)?;
    for (source, name) in [(&video, "eon-demo.mp4"), (&poster, "eon-demo.png")] {
        let temporary = assets.join(format!(".{name}.tmp"));
        fs::copy(source, &temporary)?;
        fs::rename(temporary, assets.join(name))?;
    }
    Ok(())
}

fn main() -> ExitCode {
    kinestra::run(record)
}
