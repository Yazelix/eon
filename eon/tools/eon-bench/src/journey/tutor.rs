//! Installed tutor output and workspace actions, using the normal managed shell.

use std::{os::unix::net::UnixDatagram, path::Path, time::Duration};

use super::{CALL_LIMIT, Journey, Probe, Receipt, Result, Value, error, fs, object, path_text};
use crate::{environment, launcher, picture::Picture, response::Collector, sha256::Sha256};

const STARTUP: Duration = Duration::from_secs(45);
const WAIT: Duration = Duration::from_secs(10);
const PROJECT: &str = "tutor-project";
const PRACTICE: &str = "tutor-practice";
const NOTE: &str = "notes.md";
const WORKING: &str = "Companion pane ready";
const PROMPT_MARK: &str = "Tutor prompt ready";
const END: &str = "Last lesson; select any number to revisit.";

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}: {value:?}"))
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing {key}: {value:?}"))
}

fn number(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing {key}: {value:?}"))
}

fn eon(journey: &Journey, args: &[&str]) -> Result<Value> {
    journey.call_json(args, std::time::Instant::now() + CALL_LIMIT)
}

fn active(snapshot: &Value) -> Result<&Value> {
    let id = field(snapshot, "active_tab")?;
    array(snapshot, "tabs")?
        .iter()
        .find(|tab| tab.get("id").and_then(Value::as_str) == Some(id))
        .ok_or_else(|| format!("missing active tab {id}"))
}

fn observe(
    journey: &mut Journey,
    limit: Duration,
    predicate: impl Fn(&Value) -> Result<bool>,
) -> Result<Value> {
    journey.wait(limit, |_, snapshot, _| {
        Ok(if predicate(snapshot)? {
            Probe::Ready(snapshot.clone())
        } else {
            Probe::Pending(snapshot.clone())
        })
    })
}

fn read(journey: &Journey, target: &str, deadline: std::time::Instant) -> Result<Value> {
    journey.call_json(&["pane", "read", target, "--json"], deadline)
}

fn wait_screen(
    journey: &mut Journey,
    target: &str,
    predicate: impl Fn(&Value) -> Result<bool>,
) -> Result<Value> {
    journey.wait(WAIT, |journey, _, deadline| {
        let screen = read(journey, target, deadline)?;
        Ok(if predicate(&screen)? {
            Probe::Ready(screen)
        } else {
            Probe::Pending(screen)
        })
    })
}

fn tool(journey: &Journey, program: &str, args: &[&str]) -> Result<Vec<u8>> {
    journey.tool(
        Path::new(program),
        args,
        std::time::Instant::now() + CALL_LIMIT,
    )
}

fn prepare(journey: &Journey, project: &Path, practice: &Path) -> Result<Value> {
    for directory in [project, practice] {
        fs::create_dir(directory).map_err(error)?;
    }
    let path = path_text(project)?;
    let note = project.join(NOTE);
    fs::write(&note, "# Tutor practice\n").map_err(error)?;
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["add", NOTE],
        &[
            "-c",
            "user.name=Demo",
            "-c",
            "user.email=demo@example.invalid",
            "commit",
            "-qm",
            "Practice file",
        ],
    ] {
        let mut git = vec!["-C", path];
        git.extend(args);
        tool(journey, "git", &git)?;
    }
    fs::write(&note, "# Tutor practice\n\nAn unstaged change.\n").map_err(error)?;
    journey.call(
        &journey.product_root.join("libexec/eon/bin/zoxide"),
        &["add", "--", path, path_text(practice)?],
        std::time::Instant::now() + CALL_LIMIT,
    )?;
    fs::write(
        journey.launch.out.join("config/config.toml"),
        r#"[popups.agent]
label = "Demo"
command = ["/bin/sh", "-c", "printf '\\033]7501;state=blocked:kind=permission:app=demo:title=RGVtbw\\007'; printf 'Demo awaiting input\\n'; exec sleep 180"]
"#,
    ).map_err(error)?;
    Ok(object([
        ("project", path.into()),
        ("practice", path_text(practice)?.into()),
    ]))
}

fn start(journey: &mut Journey, project: &Path) -> Result<((Value, u32), Value)> {
    let project = path_text(project)?;
    journey
        .launch
        .start_eon_with(&["--dir", project, "--no-anima"])?;
    let session = observe(journey, STARTUP, |snapshot| {
        let tab = active(snapshot)?;
        Ok(
            tab.get("directory").and_then(Value::as_str) == Some(project)
                && tab.get("pending").and_then(Value::as_bool) == Some(false)
                && array(tab, "panes")?.iter().any(live),
        )
    })?;
    let outputs = sway(journey, "get_outputs")?;
    journey.launch.output_size.check_outputs(&outputs)?;
    let window = wait_window(journey, true)?;
    let pid = number(&window, "pid")?.try_into().map_err(error)?;
    journey.launch.presentation(pid)?;
    let evidence = object([
        ("session", session.clone()),
        ("outputs", outputs),
        ("window", window),
        (
            "compositor",
            String::from_utf8(tool(journey, "sway", &["--version"])?)
                .map_err(error)?
                .trim()
                .into(),
        ),
    ]);
    Ok(((session, pid), evidence))
}

fn live(pane: &Value) -> bool {
    pane.get("live").and_then(Value::as_bool) == Some(true)
}

fn pick_tab(journey: &mut Journey, caller_tab: &str, directory: &Path) -> Result<Value> {
    let created = eon(journey, &["tab", "create", "--json"])?;
    let tab = field(created.get("created").ok_or("no new tab")?, "tab")?;
    let pending = observe(journey, WAIT, |snapshot| {
        Ok(field(snapshot, "active_tab")? == tab
            && active(snapshot)?.get("pending").and_then(Value::as_bool) == Some(true)
            && active(snapshot)?
                .get("selected_popup")
                .and_then(Value::as_str)
                .is_some())
    })?;
    let picker = field(active(&pending)?, "selected_popup")?;
    wait_screen(journey, picker, |screen| {
        Ok(field(screen, "text")?.contains(PRACTICE))
    })?;
    journey.send(picker, Some(PRACTICE), true)?;
    let path = path_text(directory)?;
    let picked = observe(journey, WAIT, |snapshot| {
        let current = active(snapshot)?;
        Ok(field(current, "id")? == tab
            && current.get("pending").and_then(Value::as_bool) == Some(false)
            && current.get("directory").and_then(Value::as_str) == Some(path)
            && array(current, "panes")?.iter().any(live))
    })?;
    let moved = eon(journey, &["tab", "move", "left", "--json"])?;
    if array(&moved, "tabs")?
        .first()
        .and_then(|tab| tab.get("id"))
        .and_then(Value::as_str)
        != Some(tab)
    {
        return Err("practice tab did not move left".into());
    }
    eon(journey, &["tab", "switch", caller_tab, "--json"])?;
    let closed = eon(journey, &["tab", "close", tab, "--yes", "--json"])?;
    if field(&closed, "active_tab")? != caller_tab || array(&closed, "tabs")?.len() != 1 {
        return Err("closing the practice tab did not restore the caller".into());
    }
    Ok(object([("picked", picked), ("closed", closed)]))
}

fn companion(journey: &mut Journey, caller: &str) -> Result<(String, Value)> {
    let created = eon(journey, &["pane", "create", "--json"])?;
    let pane = field(created.get("created").ok_or("no new pane")?, "pane")?;
    let panes = array(active(&created)?, "panes")?;
    if panes.len() != 2 || number(&panes[0], "column")? != number(&panes[1], "column")? {
        return Err("new pane did not start in the caller's Stack".into());
    }
    let moved = eon(journey, &["pane", "move", "up", "--json"])?;
    if field(
        array(active(&moved)?, "panes")?.first().ok_or("no panes")?,
        "id",
    )? != pane
    {
        return Err("new pane did not move up in the Stack".into());
    }
    eon(journey, &["pane", "move", "right", "--json"])?;
    wait_screen(journey, pane, |screen| {
        Ok(field(screen, "text")?.contains(PROJECT))
    })?;
    journey.send(pane, Some("print 'Companion pane ready'"), true)?;
    wait_screen(journey, pane, |screen| {
        Ok(nonempty_lines(field(screen, "text")?).contains(&WORKING))
    })?;
    let focused = eon(journey, &["focus", caller, "--json"])?;
    Ok((pane.into(), focused))
}

fn popup(journey: &mut Journey, entry: &str, path: &Path, marker: &str) -> Result<Value> {
    let shown = if entry == "editor" {
        eon(journey, &["edit", "--json", "--", path_text(path)?])?
    } else {
        eon(journey, &["popup", "show", entry, "--json"])?
    };
    let id = field(active(&shown)?, "selected_popup")?;
    let screen = wait_screen(journey, id, |screen| {
        Ok(field(screen, "text")?.contains(marker))
    })?;
    let hidden = eon(journey, &["popup", "hide", id, "--json"])?;
    let tab = active(&hidden)?;
    if tab.get("selected_popup") != Some(&Value::Null)
        || !array(tab, "popups")?
            .iter()
            .any(|popup| popup.get("id").and_then(Value::as_str) == Some(id))
    {
        return Err(format!("{entry} was not retained when hidden"));
    }
    Ok(object([
        ("shown", shown),
        ("screen", screen),
        ("hidden", hidden),
    ]))
}

fn sway(journey: &Journey, request: &str) -> Result<Value> {
    let bytes = tool(journey, "swaymsg", &["-t", request, "-r"])?;
    Value::parse(std::str::from_utf8(&bytes).map_err(error)?)
}

fn eon_windows<'a>(node: &'a Value, windows: &mut Vec<&'a Value>) {
    if node.get("app_id").and_then(Value::as_str) == Some("eon") {
        windows.push(node);
    }
    for key in ["nodes", "floating_nodes"] {
        for child in node.get(key).and_then(Value::as_array).unwrap_or(&[]) {
            eon_windows(child, windows);
        }
    }
}

fn wait_window(journey: &mut Journey, visible: bool) -> Result<Value> {
    journey.wait(WAIT, |journey, _, _| {
        let tree = sway(journey, "get_tree")?;
        let mut windows = Vec::new();
        eon_windows(&tree, &mut windows);
        if visible {
            if let [window] = windows.as_slice()
                && window.get("focused").and_then(Value::as_bool) == Some(true)
                && number(window, "fullscreen_mode")? > 0
                && window
                    .get("rect")
                    .is_some_and(|rect| journey.launch.output_size.matches(rect))
            {
                return Ok(Probe::Ready((*window).clone()));
            }
        } else if windows.is_empty() {
            return Ok(Probe::Ready(Value::Null));
        }
        Ok(Probe::Pending(tree))
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Grid {
    rows: u64,
    columns: u64,
}

impl Grid {
    fn read(screen: &Value) -> Result<Self> {
        Ok(Self {
            rows: number(screen, "rows")?,
            columns: number(screen, "columns")?,
        })
    }
}

fn layout(snapshot: &Value, caller: &str, companion: &str, sidebar: &str) -> Result<bool> {
    let tab = active(snapshot)?;
    let panes = array(tab, "panes")?;
    Ok(
        snapshot.get("sidebar").and_then(Value::as_str) == Some(sidebar)
            && tab.get("selected_popup") == Some(&Value::Null)
            && tab.get("selected_pane").and_then(Value::as_str) == Some(caller)
            && panes.len() == 2
            && panes.iter().all(|pane| {
                live(pane) && pane.get("expanded").and_then(Value::as_bool) == Some(true)
            })
            && panes
                .iter()
                .any(|pane| pane.get("id").and_then(Value::as_str) == Some(companion))
            && number(&panes[0], "column")? != number(&panes[1], "column")?,
    )
}

fn panes(
    journey: &mut Journey,
    caller: &str,
    companion: &str,
    sidebar: &str,
    closed: Option<Grid>,
) -> Result<([Value; 2], Value)> {
    if sidebar == "open" {
        eon(journey, &["sidebar", "show", "--json"])?;
    }
    let evidence = journey.wait(WAIT, |journey, snapshot, deadline| {
        if !layout(snapshot, caller, companion, sidebar)? {
            return Ok(Probe::Pending(snapshot.clone()));
        }
        let session = [
            read(journey, caller, deadline)?,
            read(journey, companion, deadline)?,
        ];
        let [left, right] = [Grid::read(&session[0])?, Grid::read(&session[1])?];
        let settled = left.columns.abs_diff(right.columns) <= 1
            && left.rows == right.rows
            && closed.is_none_or(|old| left.columns < old.columns && left.rows == old.rows);
        let evidence = Value::Array(session.into_iter().collect());
        Ok(if settled {
            Probe::Ready(evidence)
        } else {
            Probe::Pending(evidence)
        })
    })?;
    let screens: [Value; 2] = evidence
        .as_array()
        .ok_or("no pane grids")?
        .to_vec()
        .try_into()
        .map_err(|_| "expected two pane grids")?;
    Ok((screens, evidence))
}

fn nonempty_lines(screen: &str) -> Vec<&str> {
    let mut lines: Vec<_> = screen.split('\n').collect();
    while lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

fn prompt(journey: &mut Journey, caller: &str) -> Result<[String; 2]> {
    journey.send(caller, Some("print 'Tutor prompt ready'"), true)?;
    let screen = wait_screen(journey, caller, |screen| {
        let lines = nonempty_lines(field(screen, "text")?);
        Ok(
            matches!(lines.as_slice(), [.., PROMPT_MARK, header, indicator]
            if header.contains(PROJECT) && !indicator.is_empty()),
        )
    })?;
    let rows = nonempty_lines(field(&screen, "text")?);
    Ok([rows[rows.len() - 2].into(), rows[rows.len() - 1].into()])
}

fn full_card(screen: &str, command: &str, lesson: &[&str], prompt: &[String; 2]) -> bool {
    let lines = nonempty_lines(screen);
    let Some(start) = lines.len().checked_sub(lesson.len() + 3) else {
        return false;
    };
    let echo = format!("{} {command}", prompt[1]);
    lines[start] == echo
        && &lines[start + 1..start + 1 + lesson.len()] == lesson
        && (lines[lines.len() - 2] == prompt[0]
            || lines[lines.len() - 2]
                .strip_prefix(&prompt[0])
                .and_then(|suffix| suffix.strip_prefix(" took "))
                .is_some_and(|duration| !duration.is_empty()))
        && lines[lines.len() - 1] == prompt[1]
}

fn check_layout_pixels(picture: &Picture) -> Result<(bool, Value)> {
    // Default chrome. Use interior regions to exclude dividers, card outlines and the cursor.
    let (w, h) = (picture.width, picture.height);
    if w < 600 || h < 300 {
        return Err("output is too small for the default chrome regions".into());
    }
    let middle = (w + 288) / 2;
    let regions = [
        ("bar", 288, w - 10, 12, 38),
        ("hints", 16, w - 16, h - 27, h - 5),
        ("agents", 18, 270, 68, h / 3),
        ("tutor", 315, middle - 12, 115, h - 48),
        ("companion", middle + 22, w - 28, 115, h - 48),
    ];
    let evidence: Vec<_> = regions
        .into_iter()
        .map(|(name, x0, x1, y0, y1)| {
            let ink = (y0..y1)
                .map(|y| {
                    (x0..x1)
                        .filter(|&x| {
                            picture
                                .pixel(x, y)
                                .is_some_and(|p| p.iter().any(|channel| *channel >= 140))
                        })
                        .count()
                })
                .sum::<usize>();
            (name, ink)
        })
        .collect();
    Ok((
        evidence.iter().all(|&(_, ink)| ink >= 40),
        object(
            evidence
                .into_iter()
                .map(|(name, ink)| (name, (ink as u64).into())),
        ),
    ))
}

fn generation(snapshot: &Value) -> Result<&str> {
    let window = snapshot
        .get("running")
        .and_then(|running| running.get("this"))
        .ok_or("no running window")?;
    field(window, "generation")
}

fn demo_waiting(snapshot: &Value) -> Result<bool> {
    Ok(array(active(snapshot)?, "popups")?.iter().any(|popup| {
        popup.get("entry").and_then(Value::as_str) == Some("agent")
            && popup.get("attention").is_some_and(|attention| {
                field(attention, "app").ok() == Some("demo")
                    && field(attention, "state").ok() == Some("needs-input")
            })
    }))
}

type TerminalIdentity<'a> = (&'a str, &'a str, Option<&'a str>);

fn terminal_identities(snapshot: &Value) -> Result<Vec<TerminalIdentity<'_>>> {
    let tab = active(snapshot)?;
    array(tab, "panes")?
        .iter()
        .chain(array(tab, "popups")?)
        .map(|terminal| {
            Ok((
                field(terminal, "id")?,
                field(terminal, "terminal")?,
                terminal.get("entry").and_then(Value::as_str),
            ))
        })
        .collect()
}

fn reattach(
    journey: &mut Journey,
    collector: &Collector,
    caller: &str,
    companion: &str,
) -> Result<Value> {
    let before = eon(journey, &["session", "--json"])?;
    let deadline = std::time::Instant::now() + CALL_LIMIT;
    let original = [
        read(journey, caller, deadline)?,
        read(journey, companion, deadline)?,
    ];
    let expected_generation = generation(&before)?;
    let expected_session = field(&before, "persistent_id")?;
    let expected_terminals = terminal_identities(&before)?;
    if !demo_waiting(&before)? {
        return Err("Demo attention disappeared before reattach".into());
    }
    let presented = wait_window(journey, true)?;
    let pid = number(&presented, "pid")?.try_into().map_err(error)?;
    let start = journey.launch.end_presentation(pid)?;
    wait_window(journey, false)?;
    observe(journey, WAIT, |_| Ok(!launcher::live_identity(pid, &start)))?;
    let started = environment::monotonic_ns();
    journey.call(
        &journey.launch.eon,
        &["window", "attach"],
        std::time::Instant::now() + CALL_LIMIT,
    )?;
    let attached = wait_window(journey, true)?;
    let new_pid = number(&attached, "pid")?.try_into().map_err(error)?;
    if new_pid == pid {
        return Err("reattach did not start a replacement Eon client".into());
    }
    journey.launch.presentation(new_pid)?;
    let restored = journey.wait(WAIT, |journey, snapshot, deadline| {
        if !layout(snapshot, caller, companion, "open")?
            || generation(snapshot)? != expected_generation
            || field(snapshot, "persistent_id")? != expected_session
            || terminal_identities(snapshot)? != expected_terminals
            || !demo_waiting(snapshot)?
        {
            return Ok(Probe::Pending(snapshot.clone()));
        }
        let screens = [
            read(journey, caller, deadline)?,
            read(journey, companion, deadline)?,
        ];
        let mut same = true;
        for (screen, previous) in screens.iter().zip(&original) {
            same &= Grid::read(screen)? == Grid::read(previous)?
                && field(screen, "screen")? == "primary"
                && number(screen, "omitted_rows")? == 0
                && field(screen, "text")? == field(previous, "text")?;
        }
        let evidence = object([
            ("session", snapshot.clone()),
            ("screens", Value::Array(screens.into_iter().collect())),
        ]);
        Ok(if same {
            Probe::Ready(evidence)
        } else {
            Probe::Pending(evidence)
        })
    })?;
    let screen = &array(&restored, "screens")?[0];
    let revision = collector.wait_presented(new_pid, started, number(screen, "revision")?, WAIT)?;
    Ok(object([
        ("window", attached),
        ("restored", restored),
        ("presented_revision", revision.into()),
    ]))
}

pub(super) fn run(journey: &mut Journey, receipt: &mut Receipt<'_>) -> Result {
    let home = journey.launch.out.join("home");
    let project = home.join(PROJECT);
    let practice = home.join(PRACTICE);
    receipt.record("seed-home", "setup", || {
        prepare(journey, &project, &practice)
    })?;
    let trace = Path::new(&journey.launch.env["XDG_RUNTIME_DIR"]).join("tutor-trace");
    let mut collector = Collector::start(
        UnixDatagram::bind(&trace).map_err(error)?,
        fs::File::create(journey.launch.out.join("tutor-trace.tsv")).map_err(error)?,
    )?;
    journey
        .launch
        .env
        .insert("EON_BENCH_TRACE".into(), path_text(&trace)?.into());
    let (startup, presenter) = receipt.produce("start", "setup", || start(journey, &project))?;
    let tab = active(&startup)?;
    let caller = field(tab, "selected_pane")?;
    receipt.record("tab-and-picker", "check", || {
        pick_tab(journey, field(tab, "id")?, &practice)
    })?;
    receipt.record("hide-sidebar", "cli", || {
        eon(journey, &["sidebar", "hide", "--json"])
    })?;
    let companion = receipt.produce("panes-and-columns", "check", || companion(journey, caller))?;
    for (entry, marker) in [
        ("files", NOTE),
        ("git", NOTE),
        ("editor", NOTE),
        ("agent", "Demo awaiting input"),
    ] {
        receipt.record(&format!("popup-{entry}"), "check", || {
            popup(journey, entry, &project.join(NOTE), marker)
        })?;
    }
    let closed = receipt.produce("sidebar-closed", "check", || {
        panes(journey, caller, &companion, "closed", None)
    })?;
    let opened = receipt.produce("sidebar-open", "check", || {
        let panes = panes(
            journey,
            caller,
            &companion,
            "open",
            Some(Grid::read(&closed[0])?),
        )?;
        let attention = observe(journey, WAIT, demo_waiting)?;
        Ok((
            panes.0,
            object([("screens", panes.1), ("session", attention)]),
        ))
    })?;
    let dimensions = [Grid::read(&opened[0])?, Grid::read(&opened[1])?];
    let prompt = receipt.produce("packaged-prompt", "check", || {
        let prompt = prompt(journey, caller)?;
        let evidence = Value::Array(prompt.iter().map(|line| line.as_str().into()).collect());
        Ok((prompt, evidence))
    })?;
    let catalog = receipt.produce("catalog", "cli", || {
        let catalog = eon(journey, &["tutor", "list", "--json"])?;
        let mut hash = Sha256::new();
        hash.update(
            array(&catalog, "lessons")
                .map(|lessons| Value::Array(lessons.to_vec()))?
                .to_pretty()
                .as_bytes(),
        );
        let evidence = object([("sha256", hash.hex().into()), ("catalog", catalog.clone())]);
        Ok((catalog, evidence))
    })?;
    let lessons = array(&catalog, "lessons")?;
    if lessons.is_empty() || number(&catalog, "current_lesson")? != 1 {
        return Err("tutor did not start from a fresh catalog".into());
    }
    let mut capture = false;
    for (index, lesson) in lessons.iter().chain(lessons.last()).enumerate() {
        let id = field(lesson, "id")?;
        let mut lines = array(lesson, "text")?
            .iter()
            .map(|line| line.as_str().ok_or("lesson contains non-text"))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if lines.contains(&"Keys unavailable; CLI steps still work.") {
            return Err("installed tutor catalog lacked active shortcuts".into());
        }
        let command = match index {
            0 => "eon tutor",
            index if index == lessons.len() => "eon tutor next # at end",
            _ => "eon tutor next",
        };
        let at_end = index + 1 >= lessons.len();
        if at_end {
            lines.push(END);
        }
        if lines.len() as u64 + 3 > dimensions[0].rows
            || lines
                .iter()
                .any(|line| !line.is_ascii() || line.len() as u64 > dimensions[0].columns)
        {
            return Err(format!(
                "lesson {id} exceeds the live cell grid {:?}",
                dimensions[0]
            ));
        }
        let step = if index == lessons.len() {
            "next-at-end"
        } else {
            id
        };
        receipt.record(step, "fit", || {
            let before = read(journey, caller, std::time::Instant::now() + CALL_LIMIT)?;
            let revision = number(&before, "revision")?;
            let started = environment::monotonic_ns();
            journey.send(caller, Some(command), true)?;
            let screen = journey.wait(WAIT, |journey, snapshot, deadline| {
                if !layout(snapshot, caller, &companion, "open")? {
                    return Err("tutor fit layout changed".into());
                }
                let screen = read(journey, caller, deadline)?;
                Ok(
                    if field(&screen, "screen")? == "primary"
                        && number(&screen, "omitted_rows")? == 0
                        && number(&screen, "revision")? > revision
                        && Grid::read(&screen)? == dimensions[0]
                        && full_card(field(&screen, "text")?, command, &lines, &prompt)
                    {
                        let other = read(journey, &companion, deadline)?;
                        if Grid::read(&other)? != dimensions[1]
                            || !nonempty_lines(field(&other, "text")?).contains(&WORKING)
                        {
                            return Err("companion pane changed during the fit check".into());
                        }
                        Probe::Ready(screen)
                    } else {
                        Probe::Pending(screen)
                    },
                )
            })?;
            let presented =
                collector.wait_presented(presenter, started, number(&screen, "revision")?, WAIT)?;
            Ok(object([
                ("screen", screen),
                ("presented_revision", presented.into()),
                ("card_rows", (lines.len() as u64).into()),
                (
                    "card_columns",
                    (lines
                        .iter()
                        .map(|line| line.len())
                        .max()
                        .unwrap_or_default() as u64)
                        .into(),
                ),
            ]))
        })?;
        if id == "attention" {
            receipt.record("agents", "capture", || {
                journey.capture("agents", check_layout_pixels)
            })?;
            capture = true;
        }
    }
    if !capture {
        return Err("no Agent attention lesson was captured".into());
    }
    receipt.record("progress", "check", || {
        let final_state = eon(journey, &["tutor", "list", "--json"])?;
        if number(&final_state, "current_lesson")? != lessons.len() as u64
            || array(&final_state, "lessons")?.len() != lessons.len()
            || !array(&final_state, "lessons")?.iter().all(|lesson| {
                lesson.get("read").and_then(Value::as_bool) == Some(true)
                    && lesson.get("verified").and_then(Value::as_bool) == Some(false)
            })
        {
            return Err(format!(
                "tutor next did not leave all lessons read and unverified: {final_state:?}"
            ));
        }
        Ok(final_state)
    })?;
    receipt.record("detach-and-return", "check", || {
        reattach(journey, &collector, caller, &companion)
    })?;
    collector.finish()
}

#[cfg(test)]
mod tests {
    use super::full_card;

    #[test]
    fn fit_requires_the_whole_current_card_and_prompt_in_order() {
        let prompt = ["repo on main".to_owned(), "❯".to_owned()];
        let echo = "❯ eon tutor next";
        let card = ["Welcome", "2/10: Panes", "", "Goal: two panes"];
        let body = card.join("\n");
        let good = format!("{echo}\n{body}\nrepo on main took 2s\n❯\n\n");
        assert!(full_card(&good, "eon tutor next", &card, &prompt));
        for wrong in [
            format!("{body}\nrepo on main\n❯"),
            format!("{echo}\n{body}\n❯"),
            format!("{echo}\n{body}\nrepo on\nmain\n❯"),
            format!("{echo}\n{body}\nrepo on main\n❯\n{echo}"),
            good.replace("Goal: two panes", "Goal: two\npanes"),
        ] {
            assert!(
                !full_card(&wrong, "eon tutor next", &card, &prompt),
                "{wrong}"
            );
        }
    }
}
