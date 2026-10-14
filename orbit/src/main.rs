mod attachment;
mod diagnostic;
mod interaction;
mod management;
mod platform;
mod presentation;
mod runtime;

use libghostty_vt::style::RgbColor;
use std::{env, error::Error, path::PathBuf};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

const ANSI_PALETTE_ARGUMENT: &str = "--ansi-palette-v1";

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("orbit: {error}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<i32> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("serve") => {
            let mut arguments: Vec<String> = arguments.collect();
            let management = management::take_launch(&mut arguments)?;
            let ansi_palette = take_ansi_palette(&mut arguments)?;
            let socket = if arguments.first().is_some_and(|value| value != "--") {
                PathBuf::from(arguments.remove(0))
            } else {
                platform::default_socket_path()?
            };
            if arguments.first().is_some_and(|value| value == "--") {
                arguments.remove(0);
            }
            if arguments.is_empty() {
                arguments.push(env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()));
            }
            runtime::run(&socket, &arguments, ansi_palette, management)
        }
        Some("client") => {
            let socket = arguments
                .next()
                .map(PathBuf::from)
                .map_or_else(|| platform::default_socket_path(), Ok)?;
            diagnostic::run(&socket)
        }
        _ => Err("usage: yazelix-orbit serve [SOCKET] [--management-v1 SESSION_ID RUN_ID COMPONENT_GENERATION] [--ansi-palette-v1 RGB,...] [-- COMMAND ...] | client [SOCKET]".into()),
    }
}

fn take_ansi_palette(arguments: &mut Vec<String>) -> Result<Option<[RgbColor; 16]>> {
    let launch_end = arguments
        .iter()
        .position(|argument| argument == "--")
        .unwrap_or(arguments.len());
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
        .ok_or("missing --ansi-palette-v1 value")?;
    let colors = parse_ansi_palette(value)?;
    arguments.drain(index..=index + 1);
    Ok(Some(colors))
}

fn parse_ansi_palette(value: &str) -> Result<[RgbColor; 16]> {
    let mut entries = value.split(',');
    let mut colors = [RgbColor::default(); 16];
    for color in &mut colors {
        let entry = entries
            .next()
            .ok_or("ANSI palette must contain exactly 16 colors")?;
        if entry.len() != 6 || !entry.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("ANSI palette colors must be six hexadecimal digits".into());
        }
        *color = RgbColor {
            r: u8::from_str_radix(&entry[..2], 16)?,
            g: u8::from_str_radix(&entry[2..4], 16)?,
            b: u8::from_str_radix(&entry[4..], 16)?,
        };
    }
    if entries.next().is_some() {
        return Err("ANSI palette must contain exactly 16 colors".into());
    }
    Ok(colors)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_ANSI_PALETTE: &str = "000102,101112,202122,303132,404142,505152,606162,707172,808182,909192,a0a1a2,b0b1b2,c0c1c2,d0d1d2,e0e1e2,f0f1f2";

    #[test]
    fn ansi_palette_is_bounded_and_resets_to_supplied_defaults() -> Result {
        let mut arguments = vec![
            "/tmp/orbit.sock".into(),
            ANSI_PALETTE_ARGUMENT.into(),
            TEST_ANSI_PALETTE.into(),
            "--".into(),
            "/bin/sh".into(),
        ];
        let colors = take_ansi_palette(&mut arguments)?.expect("palette option");
        assert_eq!(
            arguments,
            ["/tmp/orbit.sock", "--", "/bin/sh"].map(String::from)
        );

        for value in [
            "",
            "000000",
            "00000g",
            "000000,",
            TEST_ANSI_PALETTE.trim_end_matches(",f0f1f2"),
        ] {
            let mut arguments = vec![ANSI_PALETTE_ARGUMENT.into(), value.into()];
            assert!(
                take_ansi_palette(&mut arguments).is_err(),
                "accepted {value:?}"
            );
        }
        assert!(take_ansi_palette(&mut vec![ANSI_PALETTE_ARGUMENT.into()]).is_err());
        assert!(
            take_ansi_palette(&mut vec![
                "/tmp/orbit.sock".into(),
                "trailing".into(),
                ANSI_PALETTE_ARGUMENT.into(),
                TEST_ANSI_PALETTE.into(),
                "--".into(),
            ])
            .is_err()
        );
        let mut duplicate = vec![
            ANSI_PALETTE_ARGUMENT.into(),
            TEST_ANSI_PALETTE.into(),
            ANSI_PALETTE_ARGUMENT.into(),
            TEST_ANSI_PALETTE.into(),
        ];
        assert!(take_ansi_palette(&mut duplicate).is_err());
        let mut child_arguments = vec![
            "--".into(),
            "--ansi-palette-v1".into(),
            "child-value".into(),
        ];
        assert!(take_ansi_palette(&mut child_arguments)?.is_none());

        let mut terminal = runtime::tests::terminal()?;
        let original = terminal.color_palette()?;
        runtime::install_ansi_palette(&mut terminal, Some(colors))?;
        assert_eq!(&terminal.default_color_palette()?.0[..16], &colors);
        assert_eq!(&terminal.color_palette()?.0[..16], &colors);
        assert_eq!(&terminal.color_palette()?.0[16..], &original.0[16..]);

        terminal.vt_write(b"\x1b]4;1;rgb:01/02/03\x1b\\");
        assert_ne!(terminal.color_palette()?.0[1], colors[1]);
        assert_eq!(terminal.default_color_palette()?.0[1], colors[1]);
        terminal.vt_write(b"\x1b]104;1\x1b\\");
        assert_eq!(terminal.color_palette()?.0[1], colors[1]);

        terminal.vt_write(b"\x1b]4;0;rgb:01/02/03;1;rgb:04/05/06\x1b\\");
        let overridden = terminal.color_palette()?;
        assert_ne!(overridden.0[0], colors[0]);
        assert_ne!(overridden.0[1], colors[1]);
        terminal.vt_write(b"\x1b]104\x1b\\");
        assert_eq!(&terminal.color_palette()?.0[..16], &colors);
        Ok(())
    }
}
