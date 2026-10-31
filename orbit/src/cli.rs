//! Orbit's `serve` command line (consumer compatibility):
//! `serve [SOCKET] [--management-v4 SESSION_ID RUN_ID COMPONENT_GENERATION]
//! [--ansi-palette-v1 RGB,...] [-- COMMAND ...]`.

use crate::management::Launch;
use libghostty_vt::style::RgbColor;
use std::{
    env,
    ffi::{OsStr, OsString},
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
};

const ANSI_PALETTE_ARGUMENT: &str = "--ansi-palette-v1";
const MANAGEMENT_ARGUMENT: &str = "--management-v4";
const USAGE: &str = "usage: yazelix-orbit serve [SOCKET] [--management-v4 SESSION_ID RUN_ID COMPONENT_GENERATION] [--ansi-palette-v1 RGB,...] [-- COMMAND ...]";

/// One session to serve: its socket pathname, its management identities, the
/// terminal's default ANSI colors, and the PTY child's command line.
#[derive(Debug, PartialEq)]
pub(crate) struct Serve {
    pub(crate) socket: PathBuf,
    pub(crate) management: Option<Launch>,
    pub(crate) palette: Option<[RgbColor; 16]>,
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
    let palette = take_ansi_palette(&mut arguments)?;
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
        return Err("duplicate --management-v4 argument".into());
    }
    let values = arguments
        .get(index + 1..index + 4)
        .filter(|_| index + 4 <= launch_end)
        .ok_or("--management-v4 requires Session, run, and component identities")?;
    let launch = Launch::new([&values[0], &values[1], &values[2]].map(OsString::as_os_str))?;
    arguments.drain(index..index + 4);
    Ok(Some(launch))
}

fn take_ansi_palette(arguments: &mut Vec<OsString>) -> Result<Option<[RgbColor; 16]>, String> {
    let launch_end = launch_end(arguments);
    let mut positions = arguments[..launch_end]
        .iter()
        .enumerate()
        .filter_map(|(index, argument)| (argument == ANSI_PALETTE_ARGUMENT).then_some(index));
    let Some(index) = positions.next() else {
        return Ok(None);
    };
    if positions.next().is_some() {
        return Err("duplicate --ansi-palette-v1 argument".into());
    }
    if launch_end > 3 {
        return Err("unexpected argument before the command separator".into());
    }
    let value = arguments
        .get(index + 1)
        .filter(|_| index + 1 < launch_end)
        .and_then(|value| value.to_str())
        .ok_or("missing --ansi-palette-v1 value")?;
    let colors = parse_ansi_palette(value)?;
    arguments.drain(index..=index + 1);
    Ok(Some(colors))
}

fn parse_ansi_palette(value: &str) -> Result<[RgbColor; 16], String> {
    let mut entries = value.split(',');
    let mut colors = [RgbColor::default(); 16];
    for color in &mut colors {
        let entry = entries
            .next()
            .ok_or("ANSI palette must contain exactly 16 colors")?;
        if entry.len() != 6 || !entry.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("ANSI palette colors must be six hexadecimal digits".into());
        }
        let channel = |range| u8::from_str_radix(&entry[range], 16).map_err(|e| e.to_string());
        *color = RgbColor {
            r: channel(0..2)?,
            g: channel(2..4)?,
            b: channel(4..6)?,
        };
    }
    if entries.next().is_some() {
        return Err("ANSI palette must contain exactly 16 colors".into());
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
        assert!(parse_ansi_palette(PALETTE).is_ok());
        for value in [
            "",
            "000000",
            "00000g",
            "000000,",
            PALETTE.trim_end_matches(",f0f1f2"),
            &format!("{PALETTE},000000"),
        ] {
            assert!(parse_ansi_palette(value).is_err(), "accepted {value:?}");
        }
    }
}
