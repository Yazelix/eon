//! Orbit's `serve` command line (consumer compatibility):
//! `serve [SOCKET] [--management-v10 SESSION_ID RUN_ID COMPONENT_GENERATION]
//! [--ansi-palette-v1 RGB,...] [--default-colors-v1 FG,BG] [-- COMMAND ...]`.

use crate::management::Launch;
use libghostty_vt::color::Rgb as RgbColor;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
};

const ANSI_PALETTE_ARGUMENT: &str = "--ansi-palette-v1";
const DEFAULT_COLORS_ARGUMENT: &str = "--default-colors-v1";
const MANAGEMENT_ARGUMENT: &str = "--management-v10";
const USAGE: &str = "usage: yazelix-orbit serve [SOCKET] [--management-v10 SESSION_ID RUN_ID COMPONENT_GENERATION] [--ansi-palette-v1 RGB,...] [--default-colors-v1 FG,BG] [-- COMMAND ...]";

/// One session to serve: its socket pathname, its management identities, the
/// terminal's starting colors, and the PTY child's command line.
#[derive(Debug, PartialEq)]
pub(crate) struct Serve {
    pub(crate) socket: PathBuf,
    pub(crate) management: Option<Launch>,
    pub(crate) palette: Option<[RgbColor; 16]>,
    pub(crate) default_colors: Option<[RgbColor; 2]>,
    pub(crate) command: Vec<OsString>,
}

/// Parses the arguments after the program name: a
/// missing socket is the private default, and a missing command is `$SHELL`,
/// or `/bin/sh`.
pub(crate) fn parse(arguments: &[OsString]) -> Result<Serve, String> {
    let Some((first, arguments)) = arguments.split_first() else {
        return Err(USAGE.into());
    };
    if first != "serve" {
        return Err(USAGE.into());
    }
    let separator = arguments
        .iter()
        .position(|argument| argument == "--")
        .unwrap_or(arguments.len());
    let (mut prefix, suffix) = arguments.split_at(separator);
    let (mut management, mut palette, mut default_colors) = (None, None, None);
    let mut command = Vec::new();
    while let Some((argument, rest)) = prefix.split_first() {
        prefix = rest;
        match launch_option(argument) {
            Some(MANAGEMENT_ARGUMENT) => {
                if management.is_some() || rest.iter().take(3).any(|id| id == MANAGEMENT_ARGUMENT) {
                    return Err("duplicate --management-v10 argument".into());
                }
                let (values, remaining) = launch_values(rest)
                    .ok_or("--management-v10 requires Session, run, and component identities")?;
                management = Some(Launch::new(values.each_ref().map(OsString::as_os_str))?);
                prefix = remaining;
            }
            Some(option) => {
                let is_palette = option == ANSI_PALETTE_ARGUMENT;
                if (is_palette && palette.is_some()) || (!is_palette && default_colors.is_some()) {
                    return Err(format!("duplicate {option} argument"));
                }
                let ([value], remaining) =
                    launch_values(rest).ok_or_else(|| format!("{option} requires a value"))?;
                let value = value
                    .to_str()
                    .ok_or_else(|| format!("{option} requires a UTF-8 value"))?;
                if is_palette {
                    palette = Some(parse_colors(value, option)?);
                } else {
                    default_colors = Some(parse_colors(value, option)?);
                }
                prefix = remaining;
            }
            None => command.push(argument.clone()),
        }
    }
    if (palette.is_some() || default_colors.is_some()) && command.len() > 1 {
        return Err("unexpected argument before the command separator".into());
    }
    let socket = if command.is_empty() {
        default_socket_path()?
    } else {
        PathBuf::from(command.remove(0))
    };
    // A legacy positional command keeps a literal separator in its argv.
    let child = if command.is_empty() {
        suffix.get(1..).unwrap_or_default()
    } else {
        suffix
    };
    command.extend_from_slice(child);
    if command.is_empty() {
        command.push(env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into()));
    }
    Ok(Serve {
        socket,
        management,
        palette,
        default_colors,
        command,
    })
}

fn launch_option(argument: &OsStr) -> Option<&'static str> {
    [
        MANAGEMENT_ARGUMENT,
        ANSI_PALETTE_ARGUMENT,
        DEFAULT_COLORS_ARGUMENT,
    ]
    .into_iter()
    .find(|option| argument == *option)
}

fn launch_values<const N: usize>(arguments: &[OsString]) -> Option<(&[OsString; N], &[OsString])> {
    let (values, remaining) = arguments.split_first_chunk()?;
    values
        .iter()
        .all(|value| launch_option(value).is_none())
        .then_some((values, remaining))
}

fn parse_colors<const N: usize>(value: &str, option: &str) -> Result<[RgbColor; N], String> {
    let mut entries = value.split(',');
    let count_error = || format!("{option} must contain exactly {N} colors");
    let mut colors = [RgbColor::default(); N];
    for color in &mut colors {
        let entry = entries.next().ok_or_else(count_error)?;
        if entry.len() != 6 || !entry.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("{option} colors must be six hexadecimal digits"));
        }
        let channel = |range| u8::from_str_radix(&entry[range], 16).map_err(|e| e.to_string());
        *color = RgbColor {
            r: channel(0..2)?,
            g: channel(2..4)?,
            b: channel(4..6)?,
        };
    }
    if entries.next().is_some() {
        return Err(count_error());
    }
    Ok(colors)
}

/// The default socket: `orbit.sock` in a private `yazelix-orbit`
/// directory under `$XDG_RUNTIME_DIR`, or `/tmp/yazelix-orbit-UID`. The
/// listener refuses a live server's pathname, so the two never share one.
fn default_socket_path() -> Result<PathBuf, String> {
    let uid = crate::platform::effective_uid();
    let directory = env::var_os("XDG_RUNTIME_DIR").map_or_else(
        || PathBuf::from(format!("/tmp/yazelix-orbit-{uid}")),
        |root| PathBuf::from(root).join("yazelix-orbit"),
    );
    if !directory.exists() {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
    }
    let metadata = fs::symlink_metadata(&directory).map_err(|error| error.to_string())?;
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o7777 != 0o700 {
        return Err(format!(
            "runtime directory {} is not private",
            directory.display()
        ));
    }
    Ok(directory.join("orbit.sock"))
}

/// The containment check's fault: Bend fails as soon as the child starts. It
/// is an environment variable, outside Orbit's command line, because no
/// client message can make the proved policy fail.
pub(crate) fn guardian_fault() -> bool {
    env::var_os("ORBIT_GUARDIAN_FAULT").as_deref() == Some(OsStr::new("1"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::ffi::OsStringExt;

    const PALETTE: &str = "000102,101112,202122,303132,404142,505152,606162,707172,808182,909192,a0a1a2,b0b1b2,c0c1c2,d0d1d2,e0e1e2,f0f1f2";

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn serve_takes_the_command_line_arguments() {
        let serve = parse(&args(&[
            "serve",
            "/tmp/orbit.sock",
            ANSI_PALETTE_ARGUMENT,
            PALETTE,
            "--",
            "/bin/sh",
            "-c",
            "exit 3",
        ]))
        .unwrap();
        assert_eq!(serve.socket, PathBuf::from("/tmp/orbit.sock"));
        assert_eq!(serve.command, args(&["/bin/sh", "-c", "exit 3"]));
        let palette = serve.palette.unwrap();
        assert_eq!(
            (palette[0], palette[15]),
            (
                RgbColor { r: 0, g: 1, b: 2 },
                RgbColor {
                    r: 0xf0,
                    g: 0xf1,
                    b: 0xf2
                }
            )
        );

        let serve = parse(&args(&[
            "serve",
            "/tmp/orbit.sock",
            MANAGEMENT_ARGUMENT,
            "session-1",
            "run-1",
            "component-1",
            ANSI_PALETTE_ARGUMENT,
            PALETTE,
            "--",
            "/bin/sh",
        ]))
        .unwrap();
        assert_eq!(
            serve.management,
            Some(Launch {
                session_id: "session-1".into(),
                run_id: "run-1".into(),
                component_generation: "component-1".into(),
            })
        );
        assert!(serve.palette.is_some());
        assert_eq!(serve.socket, PathBuf::from("/tmp/orbit.sock"));

        // The child's own arguments are never options.
        let serve = parse(&args(&[
            "serve",
            "/tmp/orbit.sock",
            "--",
            "child",
            ANSI_PALETTE_ARGUMENT,
            "x",
        ]))
        .unwrap();
        assert_eq!(serve.palette, None);
        assert_eq!(serve.command, args(&["child", ANSI_PALETTE_ARGUMENT, "x"]));

        let child = args(&["child", "value", "--", DEFAULT_COLORS_ARGUMENT]);
        let mut legacy = args(&["serve", "/tmp/orbit.sock"]);
        legacy.extend_from_slice(&child);
        assert_eq!(parse(&legacy).unwrap().command, child);

        for bad in [
            &["client"][..],
            &[],
            &["serve", "/tmp/orbit.sock", MANAGEMENT_ARGUMENT, "s", "r"],
            &["serve", MANAGEMENT_ARGUMENT, "", "r", "c"],
            &[
                "serve",
                MANAGEMENT_ARGUMENT,
                "s",
                "r",
                "c",
                MANAGEMENT_ARGUMENT,
                "s",
                "r",
                "c",
            ],
            // In the Runtime's launch form, which always passes a palette,
            // an earlier management generation's flag is a usage error.
            // Without a palette it would be taken as the command.
            &[
                "serve",
                "/tmp/orbit.sock",
                "--management-v4",
                "s",
                "r",
                "c",
                ANSI_PALETTE_ARGUMENT,
                PALETTE,
                "--",
            ],
            &["serve", ANSI_PALETTE_ARGUMENT],
            &["serve", ANSI_PALETTE_ARGUMENT, "000000"],
            &[
                "serve",
                ANSI_PALETTE_ARGUMENT,
                PALETTE,
                ANSI_PALETTE_ARGUMENT,
                PALETTE,
            ],
            &[
                "serve",
                "/tmp/orbit.sock",
                "extra",
                ANSI_PALETTE_ARGUMENT,
                PALETTE,
                "--",
            ],
        ] {
            assert!(parse(&args(bad)).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn management_requires_three_adjacent_identities() {
        for (option, value) in [
            (ANSI_PALETTE_ARGUMENT, PALETTE),
            (DEFAULT_COLORS_ARGUMENT, "012345,abcdef"),
        ] {
            let mut arguments = args(&["serve", "/tmp/orbit.sock", MANAGEMENT_ARGUMENT, "s"]);
            arguments.extend(args(&[option, value]));
            assert_eq!(
                parse(&arguments).unwrap_err(),
                "--management-v10 requires Session, run, and component identities"
            );
        }
    }

    #[test]
    fn palettes_are_exactly_sixteen_hex_colors() {
        assert!(parse_colors::<16>(PALETTE, ANSI_PALETTE_ARGUMENT).is_ok());
        for value in [
            "",
            "000000",
            "00000g",
            "000000,",
            PALETTE.trim_end_matches(",f0f1f2"),
            &format!("{PALETTE},000000"),
        ] {
            assert!(
                parse_colors::<16>(value, ANSI_PALETTE_ARGUMENT).is_err(),
                "accepted {value:?}"
            );
        }
    }

    #[test]
    fn launch_options_work_in_every_order() {
        let options: [&[&str]; 3] = [
            &[DEFAULT_COLORS_ARGUMENT, "012345,aBcDeF"],
            &[ANSI_PALETTE_ARGUMENT, PALETTE],
            &[MANAGEMENT_ARGUMENT, "session-1", "run-1", "component-1"],
        ];
        let mut command = args(&[
            "child",
            DEFAULT_COLORS_ARGUMENT,
            ANSI_PALETTE_ARGUMENT,
            MANAGEMENT_ARGUMENT,
            "--",
        ]);
        command.push(OsString::from_vec(vec![0xff]));
        let expected = Serve {
            socket: PathBuf::from("/tmp/orbit.sock"),
            management: Some(Launch {
                session_id: "session-1".into(),
                run_id: "run-1".into(),
                component_generation: "component-1".into(),
            }),
            palette: Some(parse_colors(PALETTE, ANSI_PALETTE_ARGUMENT).unwrap()),
            default_colors: Some([
                RgbColor {
                    r: 1,
                    g: 0x23,
                    b: 0x45,
                },
                RgbColor {
                    r: 0xab,
                    g: 0xcd,
                    b: 0xef,
                },
            ]),
            command,
        };
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let mut arguments = args(&["serve", "/tmp/orbit.sock"]);
            arguments.extend(order.into_iter().flat_map(|index| args(options[index])));
            arguments.push("--".into());
            arguments.extend_from_slice(&expected.command);
            assert_eq!(parse(&arguments).unwrap(), expected, "order {order:?}");
        }
    }

    #[test]
    fn color_arguments_distinguish_missing_and_non_utf8_values() {
        for option in [ANSI_PALETTE_ARGUMENT, DEFAULT_COLORS_ARGUMENT] {
            for tail in [&[][..], &["--", "child"]] {
                let mut arguments = args(&["serve", option]);
                arguments.extend(args(tail));
                assert_eq!(
                    parse(&arguments).unwrap_err(),
                    format!("{option} requires a value")
                );
            }
            let mut arguments = args(&["serve", option]);
            arguments.push(OsString::from_vec(vec![0xff]));
            assert_eq!(
                parse(&arguments).unwrap_err(),
                format!("{option} requires a UTF-8 value")
            );
        }
    }

    #[test]
    fn invalid_default_color_pairs_are_refused() {
        let pair = "012345,aBcDeF";
        for invalid in ["", "012345", "012345,abcdef,000000", "012345,abcdeg"] {
            assert!(parse(&args(&["serve", DEFAULT_COLORS_ARGUMENT, invalid])).is_err());
        }
        assert!(
            parse(&args(&[
                "serve",
                DEFAULT_COLORS_ARGUMENT,
                pair,
                DEFAULT_COLORS_ARGUMENT,
                pair
            ]))
            .is_err()
        );
        let mut extra = args(&[
            "serve",
            "/tmp/orbit.sock",
            "extra",
            DEFAULT_COLORS_ARGUMENT,
            pair,
        ]);
        assert!(parse(&extra).is_err());
        extra.extend(args(&[ANSI_PALETTE_ARGUMENT, PALETTE, "--", "child"]));
        assert!(parse(&extra).is_err());
    }
}
