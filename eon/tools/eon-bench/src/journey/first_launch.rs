//! The first pending tab uses the real Files quick search to start its first shell.

use std::collections::HashMap;

use super::{Action, Cli, Journey, Probe, Result, Step, Value, error, fs, object, path_text};
use crate::picture::{Picture, Rect};
use std::time::{Duration, Instant};

const PICKER_LIMIT: Duration = Duration::from_secs(45);
const QUERY_LIMIT: Duration = Duration::from_secs(10);
const SHELL_LIMIT: Duration = Duration::from_secs(10);
const PROJECT: &str = "journey-cobalt";
const DECOY: &str = "journey-linen";
const QUERY: &str = "cobalt";
const SHELL_MARK: &str = "EON journey shell";
const QUICK_PROMPT: &str = "Quick search >";
const NON_BACKGROUND_FLOOR: f64 = 0.002;
const MARKER_PIXELS: usize = 64;

// shortcut: default fullscreen layout at 1280x800, revisit if the text areas move.
const PICKER_ROWS: [Rect; 3] = [
    Rect {
        x: 316,
        y: 84,
        w: 144,
        h: 17,
    },
    Rect {
        x: 335,
        y: 102,
        w: 260,
        h: 16,
    },
    Rect {
        x: 335,
        y: 120,
        w: 260,
        h: 17,
    },
];
const PICKER_FOREGROUND_FLOOR: usize = 80;
const PANE_INSET: Rect = Rect {
    x: 320,
    y: 112,
    w: 800,
    h: 360,
};

pub(super) const STEPS: &[Step] = &[
    Step("launch", Action::Cli(Cli::Launch(&[]))),
    Step("picker-ready", Action::Wait(PICKER_LIMIT, picker)),
    Step("picker", Action::Capture(picker_pixels)),
    Step("type-query", Action::Send(popup_target, Some(QUERY))),
    Step("query-filtered", Action::Wait(QUERY_LIMIT, filtered)),
    Step("accept-directory", Action::Send(popup_target, None)),
    Step("first-shell", Action::Wait(SHELL_LIMIT, shell)),
    Step("pane", Action::Capture(pane_pixels)),
];

/// In bundle mode, after each named step, the processes that must be live in
/// the jail, by executable name, while every candidate process's origin is
/// audited (crate::bundle::origin).
pub(super) const BUNDLE_AUDITS: &[(&str, &[&str])] = &[
    ("picker-ready", &["yazi", "fzf"]),
    (
        "first-shell",
        &[".eon-wrapped", crate::bundle::VENUS, "yazelix-orbit", "nu"],
    ),
];

fn popup_target(journey: &Journey) -> Option<&str> {
    journey.popup.as_deref()
}

pub(super) fn prepare(journey: &Journey) -> Result<Value> {
    let home = &journey.launch.out.join("home");
    let project = home.join(PROJECT);
    let decoy = home.join(DECOY);
    for dir in [&project, &decoy] {
        fs::create_dir(dir).map_err(error)?;
    }
    let nu_config = home.join(".config/nushell");
    fs::create_dir_all(&nu_config).map_err(error)?;
    fs::write(
        nu_config.join("config.nu"),
        format!(
            r#"
pwd | save --raw --force ($env.HOME | path join "shell-cwd")
print "{SHELL_MARK}"
let stripe = "                                "
let ink = (ansi -e "48;2;255;0;255m")
for _ in 0..2 {{ print $"($ink)($stripe)(ansi reset)" }}
"#
        ),
    )
    .map_err(error)?;
    let zoxide = journey.product_root.join("libexec/eon/bin/zoxide");
    journey.call(
        &zoxide,
        &["add", "--", path_text(&project)?, path_text(&decoy)?],
        Instant::now() + super::CALL_LIMIT,
    )?;
    Ok(object([
        ("project", path_text(&project)?.into()),
        ("decoy", path_text(&decoy)?.into()),
    ]))
}

fn active_first_tab(snapshot: &Value) -> Option<&Value> {
    let first = snapshot.get("tabs")?.as_array()?.first()?;
    let id = first.get("id")?.as_str()?;
    (snapshot.get("active_tab")?.as_str()? == id).then_some(first)
}

fn pending_files(tab: &Value) -> Option<(&str, &str)> {
    if tab.get("pending")?.as_bool()? && tab.get("panes")?.as_array()?.is_empty() {
        let popup = tab.get("selected_popup")?.as_str()?;
        let listed = tab.get("popups")?.as_array()?.iter().any(|item| {
            item.get("id").and_then(Value::as_str) == Some(popup)
                && item.get("entry").and_then(Value::as_str) == Some("files")
        });
        if listed {
            return Some((tab.get("id")?.as_str()?, popup));
        }
    }
    None
}

fn picker(journey: &mut Journey, snapshot: &Value, deadline: Instant) -> Result<Probe> {
    let Some((tab, popup)) = active_first_tab(snapshot).and_then(pending_files) else {
        return Ok(Probe::Pending(snapshot.clone()));
    };
    if journey.generation.is_none()
        || !snapshot
            .get("running")
            .and_then(|running| running.get("others"))
            .and_then(Value::as_array)
            .is_some_and(<[Value]>::is_empty)
    {
        return Ok(Probe::Pending(snapshot.clone()));
    }
    let screen = journey.call_json(&["pane", "read", popup, "--json"], deadline)?;
    let text = screen
        .get("text")
        .and_then(Value::as_str)
        .ok_or("picker read has no text")?;
    let found = [QUICK_PROMPT, PROJECT, DECOY]
        .into_iter()
        .all(|wanted| text.contains(wanted));
    let observation = object([("session", snapshot.clone()), ("screen", screen)]);
    if found {
        journey.tab = Some(tab.into());
        journey.popup = Some(popup.into());
        return Ok(Probe::Ready(observation));
    }
    Ok(Probe::Pending(observation))
}

fn filtered(journey: &mut Journey, snapshot: &Value, deadline: Instant) -> Result<Probe> {
    let Some((tab, popup)) = active_first_tab(snapshot).and_then(pending_files) else {
        return Ok(Probe::Pending(snapshot.clone()));
    };
    if (Some(tab), Some(popup)) != (journey.tab.as_deref(), journey.popup.as_deref()) {
        return Ok(Probe::Pending(snapshot.clone()));
    }
    let screen = journey.call_json(&["pane", "read", popup, "--json"], deadline)?;
    let text = screen
        .get("text")
        .and_then(Value::as_str)
        .ok_or("picker read has no text")?;
    let matches = text.contains(&format!("{QUICK_PROMPT} {QUERY}"))
        && text.contains(PROJECT)
        && !text.contains(DECOY);
    Ok(if matches {
        Probe::Ready(screen)
    } else {
        Probe::Pending(screen)
    })
}

fn shell(journey: &mut Journey, snapshot: &Value, deadline: Instant) -> Result<Probe> {
    let home = journey.launch.out.join("home");
    let project = home.join(PROJECT);
    let expected = path_text(&project)?;
    let Some(tab) = active_first_tab(snapshot) else {
        return Ok(Probe::Pending(snapshot.clone()));
    };
    if tab.get("id").and_then(Value::as_str) != journey.tab.as_deref()
        || tab.get("pending").and_then(Value::as_bool) != Some(false)
        || tab.get("directory").and_then(Value::as_str) != Some(expected)
        || tab.get("selected_popup") != Some(&Value::Null)
    {
        return Ok(Probe::Pending(snapshot.clone()));
    }
    let Some([pane]) = tab.get("panes").and_then(Value::as_array) else {
        return Ok(Probe::Pending(snapshot.clone()));
    };
    let pane_id = pane
        .get("id")
        .and_then(Value::as_str)
        .ok_or("pane has no ID")?;
    if pane.get("live").and_then(Value::as_bool) != Some(true)
        || tab.get("selected_pane").and_then(Value::as_str) != Some(pane_id)
    {
        return Ok(Probe::Pending(snapshot.clone()));
    }
    let screen = journey.call_json(&["pane", "read", pane_id, "--json"], deadline)?;
    let text = screen
        .get("text")
        .and_then(Value::as_str)
        .ok_or("shell read has no text")?;
    if !text.contains(SHELL_MARK) {
        return Ok(Probe::Pending(object([
            ("session", snapshot.clone()),
            ("screen", screen),
        ])));
    }
    let actual = fs::read_to_string(home.join("shell-cwd")).map_err(error)?;
    if actual != expected {
        return Err(format!("shell cwd is {actual:?}, expected {expected:?}"));
    }
    Ok(Probe::Ready(object([
        ("session", snapshot.clone()),
        ("pane", pane_id.into()),
        ("shell_cwd", actual.into()),
        ("screen", screen),
    ])))
}

fn picker_pixels(picture: &Picture) -> Result<(bool, Value)> {
    let mut rows = Vec::new();
    for bounds in PICKER_ROWS {
        let pixels = picture.crop(bounds)?;
        let (background, _) = dominant_color(&pixels)?;
        rows.push(
            pixels
                .chunks_exact(3)
                .filter(|pixel| {
                    pixel
                        .iter()
                        .zip(background)
                        .any(|(&channel, &bg)| channel.abs_diff(bg) >= 128)
                })
                .count(),
        );
    }
    Ok((
        rows.iter().all(|&count| count >= PICKER_FOREGROUND_FLOOR),
        object([
            (
                "foreground_by_row",
                Value::Array(
                    rows.into_iter()
                        .map(|count| (count as u64).into())
                        .collect(),
                ),
            ),
            ("foreground_floor", (PICKER_FOREGROUND_FLOOR as u64).into()),
        ]),
    ))
}

fn pane_pixels(picture: &Picture) -> Result<(bool, Value)> {
    let inset = PANE_INSET;
    let pixels = picture.crop(inset)?;
    let marker = pixels
        .chunks_exact(3)
        .filter(|pixel| pixel[0] >= 160 && pixel[1] <= 96 && pixel[2] >= 160)
        .count();
    let total = pixels.len() / 3;
    let (_, background) = dominant_color(&pixels)?;
    let non_background = (total - background) as f64 / total as f64;
    Ok((
        non_background > NON_BACKGROUND_FLOOR && marker >= MARKER_PIXELS,
        object([
            (
                "inset",
                Value::Array(
                    [inset.x, inset.y, inset.w, inset.h]
                        .map(|n| u64::from(n).into())
                        .to_vec(),
                ),
            ),
            ("non_background_share", non_background.into()),
            ("floor", NON_BACKGROUND_FLOOR.into()),
            ("marker_pixels", (marker as u64).into()),
        ]),
    ))
}

fn dominant_color(pixels: &[u8]) -> Result<(&[u8], usize)> {
    let mut histogram = HashMap::new();
    for pixel in pixels.chunks_exact(3) {
        *histogram.entry(pixel).or_insert(0usize) += 1;
    }
    histogram
        .into_iter()
        .max_by_key(|&(_, count)| count)
        .ok_or("empty image inset".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_requires_foreground_in_the_prompt_and_both_results() {
        let mut picture = Picture {
            width: 1280,
            height: 800,
            pixels: vec![0; 1280 * 800 * 3],
        };
        let selected = PICKER_ROWS[1];
        for y in selected.y..selected.y + selected.h {
            let start = (y * 1280 + selected.x) as usize * 3;
            picture.pixels[start..start + selected.w as usize * 3].fill(160);
        }
        assert!(!picker_pixels(&picture).unwrap().0);
        let colors = [[0, 96, 255], [0, 0, 0], [230, 235, 240]];
        for (index, (row, color)) in PICKER_ROWS.iter().zip(colors).enumerate() {
            let start = (row.y * 1280 + row.x) as usize * 3;
            for pixel in
                picture.pixels[start..start + (PICKER_FOREGROUND_FLOOR + 1) * 3].chunks_exact_mut(3)
            {
                pixel.copy_from_slice(&color);
            }
            assert_eq!(picker_pixels(&picture).unwrap().0, index == 2);
        }
    }

    #[test]
    fn frame_requires_non_blank_pane_and_shell_color() {
        let mut picture = Picture {
            width: 1280,
            height: 800,
            pixels: vec![32; 1280 * 800 * 3],
        };
        assert!(!pane_pixels(&picture).unwrap().0);
        assert!(!picker_pixels(&picture).unwrap().0);
        picture.pixels[..1280 * 30 * 3].fill(255);
        assert!(!pane_pixels(&picture).unwrap().0);

        for y in 125..135 {
            for x in 330..420 {
                let pos = (y * 1280 + x) * 3;
                picture.pixels[pos..pos + 3].copy_from_slice(&[0, 255, 255]);
            }
        }
        assert!(!pane_pixels(&picture).unwrap().0);
        for y in 125..135 {
            for x in 330..420 {
                let pos = (y * 1280 + x) * 3;
                picture.pixels[pos..pos + 3].copy_from_slice(&[255, 0, 255]);
            }
        }
        let (passed, evidence) = pane_pixels(&picture).unwrap();
        assert!(passed, "{evidence:?}");
    }
}
