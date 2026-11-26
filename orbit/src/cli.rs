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
    let mut arguments = arguments.to_vec();
    let management = take_management(&mut arguments)?;
    let palette = take_colors(&mut arguments, ANSI_PALETTE_ARGUMENT)?;
    let default_colors = take_colors(&mut arguments, DEFAULT_COLORS_ARGUMENT)?;
    if (palette.is_some() || default_colors.is_some()) && launch_end(&arguments) > 1 {
        return Err("unexpected argument before the command separator".into());
    }
    let socket = if arguments.first().is_some_and(|value| value != "--") {
        PathBuf::from(arguments.remove(0))
    } else {
        default_socket_path()?
    };
    if arguments.first().is_some_and(|value| value == "--") {
        arguments.remove(0);
    }
    if arguments.is_empty() {
        arguments.push(env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into()));
    }
    Ok(Serve {
        socket,
        management,
        palette,
        default_colors,
        command: arguments,
    })
}

/// Where the command separator is, or the end; options stop there.
fn launch_end(arguments: &[OsString]) -> usize {
    arguments
        .iter()
        .position(|argument| argument == "--")
        .unwrap_or(arguments.len())
}

fn take_management(arguments: &mut Vec<OsString>) -> Result<Option<Launch>, String> {
    let launch_end = launch_end(arguments);
    let mut positions = arguments[..launch_end]
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| (argument == MANAGEMENT_ARGUMENT).then_some(index));
    let Some(index) = positions.next() else {
        return Ok(None);
    };
    if positions.next().is_some() {
        return Err("duplicate --management-v10 argument".into());
    }
    let values = arguments
        .get(index + 1..index + 4)
        .filter(|_| index + 4 <= launch_end)
        .ok_or("--management-v10 requires Session, run, and component identities")?;
    let launch = Launch::new([&values[0], &values[1], &values[2]].map(OsString::as_os_str))?;
    arguments.drain(index..index + 4);
    Ok(Some(launch))
}

fn take_colors<const N: usize>(
    arguments: &mut Vec<OsString>,
    option: &str,
) -> Result<Option<[RgbColor; N]>, String> {
    let launch_end = launch_end(arguments);
    let mut positions = arguments[..launch_end]
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| (argument == option).then_some(index));
    let Some(index) = positions.next() else {
        return Ok(None);
    };
    if positions.next().is_some() {
        return Err(format!("duplicate {option} argument"));
    }
    let value = arguments
        .get(index + 1)
        .filter(|_| index + 1 < launch_end)
        .and_then(|value| value.to_str())
        .ok_or_else(|| format!("{option} requires a UTF-8 value"))?;
    let colors = parse_colors(value, option)?;
    arguments.drain(index..=index + 1);
    Ok(Some(colors))
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
    fn default_colors_are_a_pair_of_hex_colors_before_the_child_separator() {
        use std::os::unix::ffi::OsStringExt;

        let pair = "012345,aBcDeF";
        let serve = parse(&args(&[
            "serve",
            "/tmp/orbit.sock",
            DEFAULT_COLORS_ARGUMENT,
            pair,
            ANSI_PALETTE_ARGUMENT,
            PALETTE,
            MANAGEMENT_ARGUMENT,
            "session-1",
            "run-1",
            "component-1",
            "--",
            "child",
            DEFAULT_COLORS_ARGUMENT,
            "child-option",
        ]))
        .unwrap();
        assert_eq!(
            serve.default_colors,
            Some([
                RgbColor {
                    r: 1,
                    g: 0x23,
                    b: 0x45
                },
                RgbColor {
                    r: 0xab,
                    g: 0xcd,
                    b: 0xef
                },
            ])
        );
        assert!(serve.palette.is_some() && serve.management.is_some());
        assert_eq!(
            serve.command,
            args(&["child", DEFAULT_COLORS_ARGUMENT, "child-option"])
        );
        let bare = parse(&args(&["serve", "/tmp/orbit.sock", "child", "value"])).unwrap();
        assert_eq!(bare.default_colors, None);
        assert_eq!(bare.command, args(&["child", "value"]));

        for invalid in ["", "012345", "012345,abcdef,000000", "012345,abcdeg"] {
            assert!(parse(&args(&["serve", DEFAULT_COLORS_ARGUMENT, invalid])).is_err());
        }
        let mut invalid_bytes = args(&["serve", DEFAULT_COLORS_ARGUMENT]);
        assert!(parse(&invalid_bytes).is_err());
        invalid_bytes.push(OsString::from_vec(vec![0xff]));
        assert!(parse(&invalid_bytes).is_err());
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
