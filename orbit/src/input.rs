//! Client input encoded with libghostty's encoders.

use crate::errno;
use libghostty_vt::{
    Terminal, focus,
    key::{
        Action as GhosttyKeyAction, Encoder as KeyEncoder, Event as GhosttyKeyEvent, Key,
        Mods as GhosttyModifiers,
    },
    mouse::{
        Action as GhosttyMouseAction, Button as GhosttyMouseButton, Encoder as MouseEncoder,
        EncoderSize as MouseEncoderSize, Event as GhosttyMouseEvent, Position as MousePosition,
    },
    paste,
    terminal::Mode,
};
use orbit_protocol::session::{self, ClientMessage, VerticalDirection};
use std::io;

pub(crate) fn encode_input(
    terminal: &Terminal<'static, '_>,
    surface: session::SurfaceSize,
    request: ClientMessage,
) -> io::Result<Vec<u8>> {
    match request {
        ClientMessage::Key(input) => {
            let key = Key::try_from(u32::from(input.key.raw())).map_err(|_| errno(libc::EINVAL))?;
            let unshifted = input
                .unshifted_codepoint
                .or((input.key == session::PhysicalKey::SPACE).then_some(' '));
            let text = input
                .text
                .filter(|_| input.action != session::KeyAction::Release || unshifted.is_some());
            let mut event = GhosttyKeyEvent::new().map_err(|_| errno(libc::EIO))?;
            event
                .set_action(match input.action {
                    session::KeyAction::Press => GhosttyKeyAction::Press,
                    session::KeyAction::Release => GhosttyKeyAction::Release,
                    session::KeyAction::Repeat => GhosttyKeyAction::Repeat,
                })
                .set_key(key)
                .set_mods(ghostty_modifiers(input.modifiers)?)
                .set_consumed_mods(ghostty_modifiers(input.consumed_modifiers)?)
                .set_composing(input.composing)
                .set_utf8(text);
            if let Some(codepoint) = unshifted {
                event.set_unshifted_codepoint(codepoint);
            }
            let mut encoder = KeyEncoder::new().map_err(|_| errno(libc::EIO))?;
            encoder.set_options_from_terminal(terminal);
            let mut output = Vec::new();
            encoder
                .encode_to_vec(&event, &mut output)
                .map_err(|_| errno(libc::EIO))?;
            Ok(output)
        }
        // Mouse input reaches the child only while it tracks the mouse; the
        // encoder writes nothing otherwise.
        ClientMessage::Mouse(input) => {
            let wheel = matches!(
                input.button,
                Some(
                    session::MouseButton::Four
                        | session::MouseButton::Five
                        | session::MouseButton::Six
                        | session::MouseButton::Seven
                )
            );
            let mut event = GhosttyMouseEvent::new().map_err(|_| errno(libc::EIO))?;
            event
                .set_action(match input.action {
                    session::MouseAction::Press => GhosttyMouseAction::Press,
                    session::MouseAction::Release => GhosttyMouseAction::Release,
                    session::MouseAction::Motion => GhosttyMouseAction::Motion,
                })
                .set_button(input.button.map(ghostty_mouse_button))
                .set_mods(ghostty_modifiers(input.modifiers)?)
                .set_position(MousePosition {
                    x: input.x,
                    y: input.y,
                });
            let mut encoder = MouseEncoder::new().map_err(|_| errno(libc::EIO))?;
            encoder
                .set_options_from_terminal(terminal)
                .set_size(MouseEncoderSize {
                    screen_width: surface.screen_width,
                    screen_height: surface.screen_height,
                    cell_width: surface.cell_width,
                    cell_height: surface.cell_height,
                    padding_top: surface.padding_top,
                    padding_bottom: surface.padding_bottom,
                    padding_right: surface.padding_right,
                    padding_left: surface.padding_left,
                })
                .set_any_button_pressed(
                    input.button.is_some()
                        && input.action != session::MouseAction::Release
                        && !wheel,
                );
            let mut output = Vec::new();
            encoder
                .encode_to_vec(&event, &mut output)
                .map_err(|_| errno(libc::EIO))?;
            Ok(output)
        }
        ClientMessage::Focus(event) => {
            if !terminal
                .mode(Mode::FOCUS_EVENT)
                .map_err(|_| errno(libc::EIO))?
            {
                return Ok(Vec::new());
            }
            let event = match event {
                session::FocusEvent::Gained => focus::Event::Gained,
                session::FocusEvent::Lost => focus::Event::Lost,
            };
            let mut output = vec![0; 8];
            let written = event.encode(&mut output).map_err(|_| errno(libc::EIO))?;
            output.truncate(written);
            Ok(output)
        }
        ClientMessage::Paste(mut data) => {
            let bracketed = terminal
                .mode(Mode::BRACKETED_PASTE)
                .map_err(|_| errno(libc::EIO))?;
            let mut output = vec![0; data.len() + 16];
            let written =
                paste::encode(&mut data, bracketed, &mut output).map_err(|_| errno(libc::EIO))?;
            output.truncate(written);
            Ok(output)
        }
        _ => Err(errno(libc::EOPNOTSUPP)),
    }
}

fn ghostty_modifiers(modifiers: session::Modifiers) -> io::Result<GhosttyModifiers> {
    GhosttyModifiers::from_bits(modifiers.bits()).ok_or_else(|| errno(libc::EINVAL))
}

fn ghostty_mouse_button(button: session::MouseButton) -> GhosttyMouseButton {
    match button {
        session::MouseButton::Unknown => GhosttyMouseButton::Unknown,
        session::MouseButton::Left => GhosttyMouseButton::Left,
        session::MouseButton::Middle => GhosttyMouseButton::Middle,
        session::MouseButton::Right => GhosttyMouseButton::Right,
        session::MouseButton::Four => GhosttyMouseButton::Four,
        session::MouseButton::Five => GhosttyMouseButton::Five,
        session::MouseButton::Six => GhosttyMouseButton::Six,
        session::MouseButton::Seven => GhosttyMouseButton::Seven,
        session::MouseButton::Eight => GhosttyMouseButton::Eight,
        session::MouseButton::Nine => GhosttyMouseButton::Nine,
        session::MouseButton::Ten => GhosttyMouseButton::Ten,
        session::MouseButton::Eleven => GhosttyMouseButton::Eleven,
    }
}

/// The direction of a physical vertical wheel, which Bend decides apart from
/// other mouse input.
pub(crate) fn wheel_direction(button: Option<session::MouseButton>) -> Option<VerticalDirection> {
    match button {
        Some(session::MouseButton::Four) => Some(VerticalDirection::Up),
        Some(session::MouseButton::Five) => Some(VerticalDirection::Down),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::TerminalOptions;

    #[test]
    fn canonical_input_uses_libghostty_encoding() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 8,
            rows: 2,
            max_scrollback: 16,
        })
        .unwrap();
        let surface = session::SurfaceSize {
            cols: 8,
            rows: 2,
            screen_width: 80,
            screen_height: 36,
            cell_width: 10,
            cell_height: 18,
            padding_top: 0,
            padding_bottom: 0,
            padding_left: 0,
            padding_right: 0,
        };
        let key = |key, text, unshifted_codepoint| {
            ClientMessage::Key(session::KeyEvent {
                action: session::KeyAction::Press,
                key,
                modifiers: session::Modifiers::empty(),
                consumed_modifiers: session::Modifiers::empty(),
                composing: false,
                text,
                unshifted_codepoint,
            })
        };
        let mouse = |action, button| {
            ClientMessage::Mouse(session::MouseEvent {
                action,
                button,
                modifiers: session::Modifiers::empty(),
                x: 25.0,
                y: 20.0,
            })
        };
        let encode = |terminal: &Terminal<'static, '_>, request| {
            encode_input(terminal, surface, request).unwrap()
        };
        assert_eq!(
            encode(
                &terminal,
                key(session::PhysicalKey::X, Some("x".into()), Some('x'))
            ),
            b"x"
        );
        assert_eq!(
            encode(&terminal, key(session::PhysicalKey::ENTER, None, None)),
            b"\r"
        );
        assert!(encode(&terminal, ClientMessage::Paste(Vec::new())).is_empty());

        // Without mouse tracking the child hears nothing; with SGR tracking a
        // press, release and wheel name the cell under the pointer.
        let left = Some(session::MouseButton::Left);
        assert!(encode(&terminal, mouse(session::MouseAction::Press, left)).is_empty());
        terminal.vt_write(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            encode(&terminal, mouse(session::MouseAction::Press, left)),
            b"\x1b[<0;3;2M"
        );
        assert_eq!(
            encode(&terminal, mouse(session::MouseAction::Release, left)),
            b"\x1b[<0;3;2m"
        );
        assert_eq!(
            encode(
                &terminal,
                mouse(
                    session::MouseAction::Press,
                    Some(session::MouseButton::Four)
                )
            ),
            b"\x1b[<64;3;2M"
        );
    }

    /// A Kitty-mode release reports a key the encoder can name, and never
    /// falls back to its text for one it cannot (ORB-C5).
    #[test]
    fn kitty_release_never_falls_back_to_text() {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 8,
            rows: 2,
            max_scrollback: 16,
        })
        .unwrap();
        terminal.vt_write(b"\x1b[>3u");
        let surface = session::SurfaceSize {
            cols: 8,
            rows: 2,
            screen_width: 80,
            screen_height: 36,
            cell_width: 10,
            cell_height: 18,
            padding_top: 0,
            padding_bottom: 0,
            padding_left: 0,
            padding_right: 0,
        };
        let release = |key, text: &str| {
            let request = ClientMessage::Key(session::KeyEvent {
                action: session::KeyAction::Release,
                key,
                modifiers: session::Modifiers::empty(),
                consumed_modifiers: session::Modifiers::empty(),
                composing: false,
                text: Some(text.into()),
                unshifted_codepoint: None,
            });
            encode_input(&terminal, surface, request).unwrap()
        };
        assert_eq!(release(session::PhysicalKey::SPACE, " "), b"\x1b[32;1:3u");
        assert!(release(session::PhysicalKey::UNIDENTIFIED, "!").is_empty());
    }
}
