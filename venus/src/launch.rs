use crate::Result;
#[cfg(target_os = "macos")]
use std::process::Command;
use std::{
    collections::hash_map::{DefaultHasher, RandomState},
    env,
    ffi::OsString,
    fs,
    hash::{BuildHasher, Hash, Hasher},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use yazelix_venus::{BarItem, Color, FontSettings};

const CURSOR_COLOR_PRESETS: [(&str, Color); 8] = [
    ("magma", rgb(0xff3b30)),
    ("solar", rgb(0xffd23f)),
    ("lime", rgb(0xb7f34a)),
    ("forest", rgb(0x35c978)),
    ("ice", rgb(0x7ddcff)),
    ("ocean", rgb(0x5271ff)),
    ("nebula", rgb(0xa970ff)),
    ("bubblegum", rgb(0xff5da2)),
];
const USAGE: &str = "usage: yazelix-venus [--application-id ID] [--no-decorations] [--pane-frames true|false] [--eon-version VERSION] [--bar-items ITEM,...] [--background-opacity VALUE] [--background-blur] [--cursor-trail-color random|preset:NAME|custom:#RRGGBB] [--cursor-effect-v1 none|tail] [--cursor-trail-color-v1 #RRGGBB] [--cursor-trail-duration-v1 0.25..4.0] [--font-family FAMILY] [--font-fallback FAMILY] [--font-size 6..96] [--line-height 1..3] [--columns N] [--rows N] [ORBIT_SOCKET | --workspace EON_WORKSPACE_SOCKET]";

#[derive(Debug)]
pub(super) struct LaunchArguments {
    pub(super) application_id: String,
    pub(super) orbit_socket: Option<PathBuf>,
    pub(super) workspace_socket: Option<PathBuf>,
    pub(super) supervised: bool,
    pub(super) startup_admission: bool,
    pub(super) decorations: bool,
    pub(super) pane_frames: bool,
    pub(super) eon_version: Option<String>,
    pub(super) bar_items: Vec<BarItem>,
    pub(super) background_opacity: f32,
    pub(super) background_blur: bool,
    pub(super) cursor_tail: Option<(Color, f32)>,
    pub(super) fonts: FontSettings,
    pub(super) columns: Option<u16>,
    pub(super) rows: Option<u16>,
}

pub(super) fn launch_arguments(
    arguments: impl IntoIterator<Item = OsString>,
    presentation_control: Option<OsString>,
) -> Result<LaunchArguments> {
    let mut arguments = arguments.into_iter();
    let mut application_id = None;
    let mut orbit_socket = None;
    let mut workspace_socket = None;
    let mut decorations = true;
    let mut pane_frames = None;
    let mut eon_version = None;
    let mut bar_items = None;
    let mut background_opacity = None;
    let mut background_blur = false;
    let mut cursor_effect = None;
    let mut cursor_color_choice = None;
    let mut cursor_trail_color = None;
    let mut cursor_trail_duration = None;
    let mut family = None;
    let mut fallbacks = Vec::new();
    let mut font_size = None;
    let mut line_height = None;
    let mut columns = None;
    let mut rows = None;

    while let Some(argument) = arguments.next() {
        if argument == "--font-family" {
            option_value(&mut family, arguments.next())?;
        } else if argument == "--font-fallback" {
            let mut family = None;
            option_value(&mut family, arguments.next())?;
            if fallbacks.len() == 8 {
                return Err(USAGE.into());
            }
            fallbacks.push(family.unwrap());
        } else if argument == "--font-size" {
            option_value(&mut font_size, arguments.next())?;
        } else if argument == "--line-height" {
            option_value(&mut line_height, arguments.next())?;
        } else if argument == "--columns" {
            option_value(&mut columns, arguments.next())?;
        } else if argument == "--rows" {
            option_value(&mut rows, arguments.next())?;
        } else if argument == "--eon-version" {
            option_value(&mut eon_version, arguments.next())?;
            if !eon_version.as_deref().is_some_and(|value: &str| {
                value.len() <= 128
                    && value.starts_with(|c: char| c.is_ascii_digit())
                    && value
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
            }) {
                return Err(USAGE.into());
            }
        } else if argument == "--bar-items" {
            if bar_items.is_some() {
                return Err(USAGE.into());
            }
            bar_items = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .and_then(|value| parse_bar_items(&value));
            if bar_items.is_none() {
                return Err(USAGE.into());
            }
        } else if argument == "--pane-frames" {
            option_value(&mut pane_frames, arguments.next())?;
        } else if argument == "--application-id" {
            if application_id.is_some() {
                return Err(USAGE.into());
            }
            application_id = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .filter(|value| valid_application_id(value));
            if application_id.is_none() {
                return Err(USAGE.into());
            }
        } else if argument == "--no-decorations" {
            decorations = false;
        } else if argument == "--background-opacity" {
            if background_opacity.is_some() {
                return Err(USAGE.into());
            }
            let Some(value) = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
            else {
                return Err(USAGE.into());
            };
            background_opacity = Some(value);
        } else if argument == "--background-blur" {
            if background_blur {
                return Err(USAGE.into());
            }
            background_blur = true;
        } else if argument == "--cursor-effect-v1" {
            if cursor_effect.is_some() {
                return Err(USAGE.into());
            }
            cursor_effect = match arguments.next().as_deref() {
                Some(value) if value == "none" => Some(false),
                Some(value) if value == "tail" => Some(true),
                _ => return Err(USAGE.into()),
            };
        } else if argument == "--cursor-trail-color" {
            if cursor_color_choice.is_some() {
                return Err(USAGE.into());
            }
            cursor_color_choice = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .and_then(|value| parse_cursor_color_choice(&value));
            if cursor_color_choice.is_none() {
                return Err(USAGE.into());
            }
        } else if argument == "--cursor-trail-color-v1" {
            if cursor_trail_color.is_some() {
                return Err(USAGE.into());
            }
            cursor_trail_color = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .and_then(|value| parse_cursor_color(&value));
            if cursor_trail_color.is_none() {
                return Err(USAGE.into());
            }
        } else if argument == "--cursor-trail-duration-v1" {
            if cursor_trail_duration.is_some() {
                return Err(USAGE.into());
            }
            cursor_trail_duration = arguments
                .next()
                .and_then(|value| value.into_string().ok())
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| value.is_finite() && (0.25..=4.0).contains(value));
            if cursor_trail_duration.is_none() {
                return Err(USAGE.into());
            }
        } else if argument == "--workspace" {
            if workspace_socket.is_some() || orbit_socket.is_some() {
                return Err(USAGE.into());
            }
            workspace_socket = arguments
                .next()
                .filter(|value| !value.as_encoded_bytes().starts_with(b"-"))
                .map(PathBuf::from);
            if workspace_socket.is_none() {
                return Err(USAGE.into());
            }
        } else if argument.as_encoded_bytes().starts_with(b"-") {
            return Err(USAGE.into());
        } else if orbit_socket.is_none() && workspace_socket.is_none() {
            orbit_socket = Some(PathBuf::from(argument));
        } else {
            return Err(USAGE.into());
        }
    }

    if (eon_version.is_some() || bar_items.is_some()) && workspace_socket.is_none() {
        return Err(USAGE.into());
    }
    if orbit_socket.is_none() && workspace_socket.is_none() {
        orbit_socket = Some(default_socket_path()?);
    }
    let endpoint = workspace_socket
        .as_deref()
        .or(orbit_socket.as_deref())
        .expect("launch resolves one endpoint");

    let cursor_tail = match (
        cursor_color_choice,
        cursor_effect,
        cursor_trail_color,
        cursor_trail_duration,
    ) {
        (color, None, None, duration) => Some((
            color
                .flatten()
                .unwrap_or_else(|| endpoint_cursor_color(endpoint)),
            duration.unwrap_or(1.0),
        )),
        (None, Some(false), None, None) => None,
        (None, Some(true), Some(color), Some(duration)) => Some((color, duration)),
        _ => return Err(USAGE.into()),
    };

    let defaults = FontSettings::default();
    let fonts = FontSettings {
        family,
        fallbacks,
        size: font_size.unwrap_or(defaults.size),
        line_height: line_height.unwrap_or(defaults.line_height),
    };
    fonts.validate()?;
    if columns == Some(0)
        || rows == Some(0)
        || u32::from(columns.unwrap_or(1)) * u32::from(rows.unwrap_or(1))
            > orbit_protocol::MAX_CELLS as u32
    {
        return Err(
            "initial terminal columns/rows must be positive and fit Orbit's 100,000-cell limit"
                .into(),
        );
    }

    let startup_admission = presentation_control.as_deref() == Some("stdin-ready-v1".as_ref());
    Ok(LaunchArguments {
        application_id: application_id.unwrap_or_else(|| "eon".into()),
        orbit_socket,
        workspace_socket,
        supervised: startup_admission || presentation_control == Some(OsString::from("stdin")),
        startup_admission,
        decorations,
        pane_frames: pane_frames.unwrap_or(true),
        eon_version,
        bar_items: bar_items.unwrap_or_else(|| BarItem::DEFAULT.into()),
        background_opacity: background_opacity.unwrap_or(1.0),
        background_blur,
        cursor_tail,
        fonts,
        columns,
        rows,
    })
}

fn option_value<T: std::str::FromStr>(slot: &mut Option<T>, value: Option<OsString>) -> Result {
    if slot.is_some() {
        return Err(USAGE.into());
    }
    *slot = value
        .and_then(|value| value.into_string().ok())
        .and_then(|value| value.parse().ok());
    if slot.is_none() {
        return Err(USAGE.into());
    }
    Ok(())
}

/// Parses comma-separated item names in their left-to-right order. An empty list shows no
/// item; an unknown, repeated or empty name is invalid.
fn parse_bar_items(list: &str) -> Option<Vec<BarItem>> {
    let mut items = Vec::new();
    for name in list.split(',').filter(|_| !list.is_empty()) {
        let item = match name {
            "codex_quota" => BarItem::CodexQuota,
            "claude_quota" => BarItem::ClaudeQuota,
            "cpu" => BarItem::Cpu,
            "ram" => BarItem::Ram,
            "clock" => BarItem::Clock,
            "version" => BarItem::Version,
            _ => return None,
        };
        if items.contains(&item) {
            return None;
        }
        items.push(item);
    }
    Some(items)
}

fn valid_application_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
}

fn parse_cursor_color(value: &str) -> Option<Color> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(rgb(u32::from_str_radix(hex, 16).ok()?))
}

const fn rgb(value: u32) -> Color {
    Color {
        r: (value >> 16) as u8,
        g: (value >> 8) as u8,
        b: value as u8,
    }
}

/// `Some(None)` is `random`, which resolves like an omitted choice.
fn parse_cursor_color_choice(value: &str) -> Option<Option<Color>> {
    if value == "random" {
        return Some(None);
    }
    value
        .strip_prefix("preset:")
        .and_then(|name| {
            CURSOR_COLOR_PRESETS
                .iter()
                .find(|(preset, _)| *preset == name)
                .map(|(_, color)| *color)
        })
        .or_else(|| value.strip_prefix("custom:").and_then(parse_cursor_color))
        .map(Some)
}

/// Every launch against one live endpoint reuses its preset, so a reattached window keeps
/// its cursor color; a recreated socket picks afresh. A socket that does not exist yet
/// falls back to one choice for this process.
fn endpoint_cursor_color(endpoint: &Path) -> Color {
    let entropy = fs::metadata(endpoint).map_or_else(
        |_| RandomState::new().hash_one("venus cursor"),
        |metadata| {
            let mut hasher = DefaultHasher::new();
            (
                metadata.dev(),
                metadata.ino(),
                metadata.ctime(),
                metadata.ctime_nsec(),
            )
                .hash(&mut hasher);
            hasher.finish()
        },
    );
    cursor_preset_from_entropy(entropy)
}

fn cursor_preset_from_entropy(entropy: u64) -> Color {
    CURSOR_COLOR_PRESETS[entropy as usize % CURSOR_COLOR_PRESETS.len()].1
}

fn default_socket_path() -> Result<PathBuf> {
    if let Some(root) = env::var_os("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(root).join("yazelix-orbit/orbit.sock"));
    }
    #[cfg(target_os = "linux")]
    let uid = fs::metadata("/proc/self")?.uid();
    #[cfg(target_os = "macos")]
    let uid = {
        let output = Command::new("/usr/bin/id").arg("-u").output()?;
        if !output.status.success() {
            return Err(format!("/usr/bin/id -u failed with {}", output.status).into());
        }
        std::str::from_utf8(&output.stdout)?.trim().parse::<u32>()?
    };
    Ok(PathBuf::from(format!(
        "/tmp/yazelix-orbit-{uid}/orbit.sock"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_items_are_an_ordered_workspace_choice() {
        let parse = |items: &[&str]| {
            launch_arguments(
                items
                    .iter()
                    .copied()
                    .chain(["--workspace", "eon.sock"])
                    .map(OsString::from),
                None,
            )
        };
        assert_eq!(parse(&[]).unwrap().bar_items, BarItem::DEFAULT);
        assert_eq!(
            parse(&["--bar-items", "clock,claude_quota,cpu"])
                .unwrap()
                .bar_items,
            [BarItem::Clock, BarItem::ClaudeQuota, BarItem::Cpu]
        );
        assert!(parse(&["--bar-items", ""]).unwrap().bar_items.is_empty());
        for items in [
            vec!["--bar-items", "gpu"],
            vec!["--bar-items", "CPU"],
            vec!["--bar-items", "cpu,cpu"],
            vec!["--bar-items", "cpu,"],
            vec!["--bar-items", "cpu, ram"],
            vec!["--bar-items", "cpu", "--bar-items", "ram"],
        ] {
            assert!(parse(&items).is_err(), "accepted {items:?}");
        }
        for arguments in [
            &["--bar-items", "cpu", "orbit.sock"][..],
            &["--workspace", "eon.sock", "--bar-items"],
        ] {
            assert!(
                launch_arguments(arguments.iter().map(OsString::from), None).is_err(),
                "accepted {arguments:?}"
            );
        }
    }

    #[test]
    fn eon_version_is_bounded_workspace_identity() {
        let parse = |version: &str| {
            launch_arguments(
                ["--eon-version", version, "--workspace", "eon.sock"].map(OsString::from),
                None,
            )
        };
        assert_eq!(
            parse("9.8.7-alpha.2+proof").unwrap().eon_version.as_deref(),
            Some("9.8.7-alpha.2+proof")
        );
        for version in [
            "",
            "bogus",
            "0.1.0\nwrong",
            "0.1.0 wrong",
            "0.1.0/other",
            &"1".repeat(129),
        ] {
            assert!(parse(version).is_err(), "accepted {version:?}");
        }
        assert!(
            launch_arguments(
                ["--eon-version", "0.1.0", "orbit.sock"].map(OsString::from),
                None
            )
            .is_err()
        );
        assert!(
            launch_arguments(
                [
                    "--eon-version",
                    "0.1.0",
                    "--eon-version",
                    "9.0.0",
                    "--workspace",
                    "eon.sock"
                ]
                .map(OsString::from),
                None
            )
            .is_err()
        );
    }

    #[test]
    fn pane_frame_option_is_a_strict_single_boolean() {
        use std::os::unix::ffi::OsStringExt;
        let parse = |args: &[&str]| launch_arguments(args.iter().map(OsString::from), None);
        for value in ["true", "false"] {
            assert!(parse(&["--pane-frames", value, "--workspace", "eon.sock"]).is_ok());
        }
        for args in [
            vec!["--pane-frames"],
            vec!["--pane-frames", ""],
            vec!["--pane-frames", "1"],
            vec!["--pane-frames", "TRUE"],
            vec!["--pane-frames", "false", "--pane-frames", "true"],
        ] {
            assert!(parse(&args).is_err(), "accepted {args:?}");
        }
        assert!(
            launch_arguments(
                ["--pane-frames".into(), OsString::from_vec(vec![0xff])],
                None
            )
            .is_err()
        );
    }

    #[test]
    fn random_cursor_preset_is_stable_per_live_endpoint() {
        let root = env::temp_dir().join(format!("venus-cursor-endpoint-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let socket = root.join("eon.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let socket = socket.to_str().unwrap();
        let color = |arguments: &[&str]| {
            launch_arguments(arguments.iter().map(OsString::from), None)
                .unwrap()
                .cursor_tail
                .unwrap()
                .0
        };
        let first = color(&["--workspace", socket]);
        for _ in 0..8 {
            assert_eq!(color(&["--workspace", socket]), first);
            assert_eq!(
                color(&["--cursor-trail-color", "random", "--workspace", socket]),
                first
            );
            assert_eq!(color(&[socket]), first);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn startup_admission_keeps_presentation_control_enabled() {
        let arguments = launch_arguments(
            [OsString::from("orbit.sock")],
            Some("stdin-ready-v1".into()),
        )
        .unwrap();
        assert!(arguments.supervised);
    }

    #[test]
    fn typography_options_are_bounded_and_composable() {
        let parse =
            |arguments: &[&str]| launch_arguments(arguments.iter().map(OsString::from), None);
        assert!(
            parse(&[
                "--font-family",
                "DejaVu Sans Mono",
                "--font-fallback",
                "DejaVu Sans",
                "--font-size",
                "20",
                "--line-height",
                "1.5",
                "--columns",
                "100",
                "--rows",
                "30",
                "--workspace",
                "eon.sock"
            ])
            .is_ok()
        );
        for args in [
            vec!["--font-size", "NaN"],
            vec!["--font-size", "5.9"],
            vec!["--font-size", "96.1"],
            vec!["--line-height", "inf"],
            vec!["--line-height", "0.9"],
            vec!["--line-height", "3.1"],
            vec!["--font-family", ""],
            vec!["--font-family", " Font"],
            vec!["--font-fallback", "x\n"],
            vec!["--font-size", "16", "--font-size", "20"],
            vec!["--columns", "0"],
            vec!["--rows", "65536"],
            vec!["--rows", "2", "--rows", "3"],
            vec!["--columns", "1000", "--rows", "1000"],
        ] {
            assert!(parse(&args).is_err(), "accepted {args:?}");
        }
        let too_many = ["--font-fallback", "DejaVu Sans"].repeat(9);
        assert!(parse(&too_many).is_err());
    }

    #[test]
    fn arguments_are_complete_bounded_and_defaulted() {
        let parse =
            |arguments: &[&str]| launch_arguments(arguments.iter().map(OsString::from), None);

        let default = parse(&[]).unwrap();
        assert_eq!(default.application_id, "eon");
        assert!(default.orbit_socket.is_some());
        assert!(default.decorations && default.workspace_socket.is_none());
        assert!(!default.supervised);
        assert_eq!(default.background_opacity, 1.0);
        assert!(!default.background_blur);
        let (default_color, default_duration) = default.cursor_tail.unwrap();
        assert_eq!(default_duration, 1.0);
        assert!(
            CURSOR_COLOR_PRESETS
                .iter()
                .any(|(_, color)| *color == default_color)
        );

        let presets = [
            ("magma", rgb(0xff3b30)),
            ("solar", rgb(0xffd23f)),
            ("lime", rgb(0xb7f34a)),
            ("forest", rgb(0x35c978)),
            ("ice", rgb(0x7ddcff)),
            ("ocean", rgb(0x5271ff)),
            ("nebula", rgb(0xa970ff)),
            ("bubblegum", rgb(0xff5da2)),
        ];
        for (index, (name, color)) in presets.into_iter().enumerate() {
            assert_eq!(cursor_preset_from_entropy(index as u64), color);
            assert_eq!(
                parse(&["--cursor-trail-color", &format!("preset:{name}")])
                    .unwrap()
                    .cursor_tail,
                Some((color, default_duration))
            );
        }
        let random = parse(&["--cursor-trail-color", "random"])
            .unwrap()
            .cursor_tail
            .unwrap();
        assert_eq!(random.1, default_duration);
        assert!(
            CURSOR_COLOR_PRESETS
                .iter()
                .any(|(_, color)| *color == random.0)
        );
        assert_eq!(
            parse(&["--cursor-trail-color", "custom:#12aBcF"])
                .unwrap()
                .cursor_tail,
            Some((
                Color {
                    r: 0x12,
                    g: 0xab,
                    b: 0xcf,
                },
                default_duration,
            ))
        );
        assert_eq!(
            parse(&["--cursor-trail-duration-v1", "0.25"])
                .unwrap()
                .cursor_tail
                .unwrap()
                .1,
            0.25
        );
        assert_eq!(
            parse(&[
                "--cursor-trail-color",
                "preset:ice",
                "--cursor-trail-duration-v1",
                "2.5",
            ])
            .unwrap()
            .cursor_tail,
            Some((rgb(0x7ddcff), 2.5))
        );

        let supervised =
            launch_arguments(Vec::<OsString>::new(), Some(OsString::from("stdin"))).unwrap();
        assert!(supervised.supervised);
        let other_control =
            launch_arguments(Vec::<OsString>::new(), Some(OsString::from("STDIN"))).unwrap();
        assert!(!other_control.supervised);

        for value in ["0", "0.88", "1"] {
            let parsed = parse(&[
                "--no-decorations",
                "--background-opacity",
                value,
                "--background-blur",
                "--workspace",
                "eon.sock",
            ])
            .unwrap();
            assert!(!parsed.decorations);
            assert_eq!(parsed.background_opacity, value.parse::<f32>().unwrap());
            assert!(parsed.background_blur);
            assert_eq!(parsed.orbit_socket, None);
            assert_eq!(parsed.workspace_socket, Some(PathBuf::from("eon.sock")));
        }

        let tail = parse(&[
            "--cursor-effect-v1",
            "tail",
            "--cursor-trail-color-v1",
            "#12aBcF",
            "--cursor-trail-duration-v1",
            "2.5",
            "orbit.sock",
        ])
        .unwrap();
        assert_eq!(
            tail.cursor_tail,
            Some((
                Color {
                    r: 0x12,
                    g: 0xab,
                    b: 0xcf,
                },
                2.5,
            ))
        );
        assert_eq!(tail.orbit_socket, Some(PathBuf::from("orbit.sock")));
        assert_eq!(
            parse(&["--cursor-effect-v1", "none"]).unwrap().cursor_tail,
            None
        );
        assert_eq!(
            parse(&["--application-id", "eonova"])
                .unwrap()
                .application_id,
            "eonova"
        );

        for invalid in [
            &["--unknown"][..],
            &["one", "two", "three"][..],
            &["--background-opacity"][..],
            &["--background-opacity", "bad"][..],
            &["--background-opacity", "NaN"][..],
            &["--background-opacity", "inf"][..],
            &["--background-opacity", "-0.01"][..],
            &["--background-opacity", "1.01"][..],
            &["--background-opacity", "0.5", "--background-opacity", "0.6"][..],
            &["--background-blur", "--background-blur"][..],
            &["--application-id"][..],
            &["--application-id", ""][..],
            &["--application-id", "bad/id"][..],
            &["--application-id", "eon", "--application-id", "eonova"][..],
            &["--cursor-effect-v1"][..],
            &["--cursor-effect-v1", "warp"][..],
            &["--cursor-effect-v1", "tail"][..],
            &["--cursor-trail-color"][..],
            &["--cursor-trail-color", "ocean"][..],
            &["--cursor-trail-color", "preset:volt"][..],
            &["--cursor-trail-color", "custom:123456"][..],
            &["--cursor-trail-color", "custom:#12345g"][..],
            &[
                "--cursor-trail-color",
                "random",
                "--cursor-trail-color",
                "preset:ice",
            ][..],
            &[
                "--cursor-trail-color",
                "preset:ice",
                "--cursor-effect-v1",
                "none",
            ][..],
            &[
                "--cursor-trail-color",
                "preset:ice",
                "--cursor-effect-v1",
                "tail",
                "--cursor-trail-color-v1",
                "#123456",
                "--cursor-trail-duration-v1",
                "1",
            ][..],
            &[
                "--cursor-effect-v1",
                "tail",
                "--cursor-trail-color-v1",
                "#123456",
            ][..],
            &[
                "--cursor-effect-v1",
                "none",
                "--cursor-trail-color-v1",
                "#123456",
                "--cursor-trail-duration-v1",
                "1",
            ][..],
            &["--cursor-trail-color-v1", "#123456"][..],
            &["--cursor-trail-color-v1", "#aéabc"][..],
            &["--cursor-trail-color-v1", "123456"][..],
            &["--cursor-trail-color-v1", "#12345g"][..],
            &[
                "--cursor-effect-v1",
                "none",
                "--cursor-trail-duration-v1",
                "1",
            ][..],
            &[
                "--cursor-effect-v1",
                "tail",
                "--cursor-trail-color-v1",
                "#123456",
                "--cursor-trail-duration-v1",
                "0.24",
            ][..],
            &[
                "--cursor-effect-v1",
                "tail",
                "--cursor-trail-color-v1",
                "#123456",
                "--cursor-trail-duration-v1",
                "4.01",
            ][..],
            &[
                "--cursor-effect-v1",
                "tail",
                "--cursor-trail-color-v1",
                "#123456",
                "--cursor-trail-duration-v1",
                "NaN",
            ][..],
            &[
                "--cursor-effect-v1",
                "tail",
                "--cursor-effect-v1",
                "tail",
                "--cursor-trail-color-v1",
                "#123456",
                "--cursor-trail-duration-v1",
                "1",
            ][..],
            &[
                "--cursor-effect-v1",
                "tail",
                "--cursor-trail-color-v1",
                "#123456",
                "--cursor-trail-color-v1",
                "#abcdef",
                "--cursor-trail-duration-v1",
                "1",
            ][..],
            &[
                "--cursor-effect-v1",
                "tail",
                "--cursor-trail-color-v1",
                "#123456",
                "--cursor-trail-duration-v1",
                "1",
                "--cursor-trail-duration-v1",
                "2",
            ][..],
            &["orbit.sock", "eon.sock"][..],
            &["orbit.sock", "--workspace", "eon.sock"][..],
            &["--workspace"][..],
            &["--workspace", "one", "--workspace", "two"][..],
        ] {
            assert_eq!(parse(invalid).unwrap_err().to_string(), USAGE);
        }
    }
}
