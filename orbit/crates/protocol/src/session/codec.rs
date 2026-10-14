use super::*;
use crate::{
    MAX_CELLS, MIN_FRAME_BYTES, decode_canonical_rows, decode_frame, encode_canonical_rows,
    encode_frame,
};
use std::str;

pub(super) const CLIENT_HELLO: u8 = 1;
const CLIENT_KEY: u8 = 2;
const CLIENT_MOUSE: u8 = 3;
const CLIENT_FOCUS: u8 = 4;
pub(super) const CLIENT_PASTE: u8 = 5;
const CLIENT_RESIZE: u8 = 6;
const CLIENT_SELECTION: u8 = 7;
const CLIENT_PREVIEW_VERTICAL: u8 = 8;
const CLIENT_OBSERVE_METADATA: u8 = 9;
const CLIENT_SCROLL_VERTICAL: u8 = 10;
const CLIENT_RETURN_TO_LIVE: u8 = 11;
const SERVER_ATTACHED: u8 = 129;
pub(super) const SERVER_BUSY: u8 = 130;
pub(super) const SERVER_FRAME: u8 = 132;
const SERVER_ACCEPTED: u8 = 133;
pub(super) const SERVER_FAILURE: u8 = 134;
const SERVER_EXITED: u8 = 135;
pub(super) const SERVER_COPIED_TEXT: u8 = 136;
pub(super) const SERVER_CLIPBOARD_WRITE: u8 = 137;
const SERVER_WHEEL_TERMINAL_ROUTED: u8 = 138;
const SERVER_WHEEL_VIEWPORT_UP: u8 = 139;
const SERVER_WHEEL_VIEWPORT_STILL: u8 = 140;
const SERVER_WHEEL_VIEWPORT_DOWN: u8 = 141;
const SERVER_VERTICAL_PREVIEW: u8 = 142;
const SERVER_OBSERVING_METADATA: u8 = 143;
const SERVER_METADATA: u8 = 144;
const SERVER_SCROLL_TERMINAL_OWNED: u8 = 145;
pub(super) const SERVER_SCROLL_VIEWPORT: u8 = 146;
const SERVER_SELECTION_FINISHED: u8 = 147;

const SELECTION_BEGIN: u8 = 0;
const SELECTION_UPDATE: u8 = 1;
const SELECTION_FINISH: u8 = 2;
const SELECTION_COPY: u8 = 3;
const SELECTION_CANCEL: u8 = 4;
const SELECTION_AUTOSCROLL_UP: u8 = 5;

/// Returns the complete client-message length once a valid header is available.
pub fn client_message_len(bytes: &[u8]) -> Result<Option<usize>> {
    message_len(bytes, client_payload_limits)
}

/// Returns the complete server-message length once a valid header is available.
pub fn server_message_len(bytes: &[u8]) -> Result<Option<usize>> {
    message_len(bytes, server_payload_limits)
}

fn message_len(
    bytes: &[u8],
    payload_limits: fn(u8) -> Result<(usize, usize)>,
) -> Result<Option<usize>> {
    if bytes.len() < HEADER_BYTES {
        return Ok(None);
    }
    if &bytes[..4] != MAGIC {
        return Err(Error::InvalidMagic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != VERSION {
        return Err(Error::UnsupportedVersion { version });
    }
    if bytes[7] != 0 {
        return Err(Error::InvalidFlags { value: bytes[7] });
    }
    let (minimum, maximum) = payload_limits(bytes[6])?;
    let payload = u32::from_le_bytes(bytes[8..12].try_into().expect("fixed header")) as usize;
    validate_bound(payload, maximum)?;
    if payload < minimum {
        return Err(Error::InvalidValue {
            field: "message payload length",
        });
    }
    Ok(Some(HEADER_BYTES + payload))
}

fn client_payload_limits(kind: u8) -> Result<(usize, usize)> {
    match kind {
        CLIENT_HELLO => Ok((0, 0)),
        CLIENT_OBSERVE_METADATA => Ok((0, 0)),
        CLIENT_KEY => Ok((16, 16 + MAX_KEY_TEXT_BYTES)),
        CLIENT_MOUSE => Ok((12, 12)),
        CLIENT_FOCUS => Ok((1, 1)),
        CLIENT_PASTE => Ok((0, MAX_PASTE_BYTES)),
        CLIENT_RESIZE => Ok((36, 36)),
        CLIENT_SELECTION => Ok((1, 27)),
        CLIENT_PREVIEW_VERTICAL => Ok((9, 9)),
        CLIENT_SCROLL_VERTICAL => Ok((10, 10)),
        CLIENT_RETURN_TO_LIVE => Ok((0, 0)),
        value => Err(Error::InvalidTag {
            field: "client message",
            value,
        }),
    }
}

fn server_payload_limits(kind: u8) -> Result<(usize, usize)> {
    match kind {
        SERVER_ATTACHED => Ok((0, 0)),
        SERVER_BUSY | SERVER_ACCEPTED | SERVER_OBSERVING_METADATA => Ok((0, 0)),
        SERVER_METADATA => Ok((16, MAX_METADATA_BYTES)),
        SERVER_EXITED => Ok((4, 4)),
        SERVER_FRAME => Ok((MIN_FRAME_BYTES, MAX_FRAME_BYTES)),
        SERVER_FAILURE => Ok((5, 5 + MAX_FAILURE_BYTES)),
        SERVER_COPIED_TEXT => Ok((1, 1 + MAX_COPY_BYTES)),
        SERVER_CLIPBOARD_WRITE => Ok((2, 1 + MAX_COPY_BYTES)),
        SERVER_WHEEL_TERMINAL_ROUTED => Ok((0, 0)),
        SERVER_WHEEL_VIEWPORT_UP | SERVER_WHEEL_VIEWPORT_STILL | SERVER_WHEEL_VIEWPORT_DOWN => {
            Ok((MIN_FRAME_BYTES, MAX_FRAME_BYTES))
        }
        SERVER_VERTICAL_PREVIEW => Ok((10, MAX_FRAME_BYTES + 15)),
        SERVER_SCROLL_TERMINAL_OWNED => Ok((2, 2)),
        SERVER_SCROLL_VIEWPORT => Ok((MIN_FRAME_BYTES + 13, MAX_PAYLOAD_BYTES)),
        SERVER_SELECTION_FINISHED => Ok((8, 8)),
        value => Err(Error::InvalidTag {
            field: "server message",
            value,
        }),
    }
}

/// Encodes one client message with an ORBS v13 header.
pub fn encode_client_message(message: &ClientMessage) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match message {
        ClientMessage::Hello => CLIENT_HELLO,
        ClientMessage::ObserveMetadata => CLIENT_OBSERVE_METADATA,
        ClientMessage::Key(event) => {
            validate_key(event)?;
            payload.push(key_action_tag(event.action));
            put_u16(&mut payload, event.key.raw());
            put_u16(&mut payload, event.modifiers.bits());
            put_u16(&mut payload, event.consumed_modifiers.bits());
            payload.push(u8::from(event.composing));
            put_optional_text(&mut payload, event.text.as_deref())?;
            put_u32(
                &mut payload,
                event.unshifted_codepoint.map_or(u32::MAX, u32::from),
            );
            CLIENT_KEY
        }
        ClientMessage::Mouse(event) => {
            validate_mouse(event)?;
            payload.push(mouse_action_tag(event.action));
            payload.push(
                event
                    .button
                    .map_or(0, |button| mouse_button_tag(button) + 1),
            );
            put_u16(&mut payload, event.modifiers.bits());
            put_u32(&mut payload, event.x.to_bits());
            put_u32(&mut payload, event.y.to_bits());
            CLIENT_MOUSE
        }
        ClientMessage::Focus(event) => {
            payload.push(match event {
                FocusEvent::Gained => 0,
                FocusEvent::Lost => 1,
            });
            CLIENT_FOCUS
        }
        ClientMessage::Paste(bytes) => {
            validate_bound(bytes.len(), MAX_PASTE_BYTES)?;
            payload.extend_from_slice(bytes);
            CLIENT_PASTE
        }
        ClientMessage::Resize(size) => {
            validate_surface_size(size)?;
            put_u16(&mut payload, size.cols);
            put_u16(&mut payload, size.rows);
            for value in [
                size.screen_width,
                size.screen_height,
                size.cell_width,
                size.cell_height,
                size.padding_top,
                size.padding_bottom,
                size.padding_left,
                size.padding_right,
            ] {
                put_u32(&mut payload, value);
            }
            CLIENT_RESIZE
        }
        ClientMessage::Selection(action) => {
            validate_selection(action)?;
            match action {
                SelectionAction::Begin {
                    frame_revision,
                    position,
                    time_ns,
                    modifiers,
                } => {
                    payload.push(SELECTION_BEGIN);
                    payload.extend_from_slice(&frame_revision.to_le_bytes());
                    payload.extend_from_slice(&time_ns.to_le_bytes());
                    put_u16(&mut payload, modifiers.bits());
                    put_u32(&mut payload, position.x.to_bits());
                    put_u32(&mut payload, position.y.to_bits());
                }
                SelectionAction::Update {
                    position,
                    modifiers,
                } => {
                    payload.push(SELECTION_UPDATE);
                    put_u16(&mut payload, modifiers.bits());
                    put_u32(&mut payload, position.x.to_bits());
                    put_u32(&mut payload, position.y.to_bits());
                }
                SelectionAction::Finish {
                    position,
                    modifiers,
                } => {
                    payload.push(SELECTION_FINISH);
                    put_u16(&mut payload, modifiers.bits());
                    put_u32(&mut payload, position.x.to_bits());
                    put_u32(&mut payload, position.y.to_bits());
                }
                SelectionAction::Copy => payload.push(SELECTION_COPY),
                SelectionAction::Cancel => payload.push(SELECTION_CANCEL),
                SelectionAction::AutoscrollUp { position } => {
                    payload.push(SELECTION_AUTOSCROLL_UP);
                    put_u32(&mut payload, position.x.to_bits());
                    put_u32(&mut payload, position.y.to_bits());
                }
            }
            CLIENT_SELECTION
        }
        ClientMessage::PreviewVertical {
            frame_revision,
            direction,
        } => {
            payload.extend_from_slice(&frame_revision.to_le_bytes());
            payload.push(vertical_direction_tag(*direction));
            CLIENT_PREVIEW_VERTICAL
        }
        ClientMessage::ScrollVertical {
            frame_revision,
            rows,
        } => {
            validate_scroll_rows(*rows)?;
            payload.extend_from_slice(&frame_revision.to_le_bytes());
            put_i16(&mut payload, *rows);
            CLIENT_SCROLL_VERTICAL
        }
        ClientMessage::ReturnToLive => CLIENT_RETURN_TO_LIVE,
    };
    frame_message(kind, payload, client_payload_limits(kind)?.1)
}

/// Decodes exactly one client message.
pub fn decode_client_message(bytes: &[u8]) -> Result<ClientMessage> {
    let (kind, payload) = exact_message(bytes, client_message_len(bytes)?)?;
    let mut reader = Reader::new(payload);
    let message = match kind {
        CLIENT_HELLO => ClientMessage::Hello,
        CLIENT_OBSERVE_METADATA => ClientMessage::ObserveMetadata,
        CLIENT_KEY => {
            let action = decode_key_action(reader.u8()?)?;
            let raw_key = reader.u16()?;
            let key = PhysicalKey::from_raw(raw_key).ok_or(Error::InvalidValue {
                field: "physical key",
            })?;
            let modifiers = checked_modifiers(reader.u16()?)?;
            let consumed_modifiers = checked_modifiers(reader.u16()?)?;
            let composing = decode_bool(reader.u8()?, "composing")?;
            let text = reader.optional_text("key text", MAX_KEY_TEXT_BYTES)?;
            let raw_codepoint = reader.u32()?;
            let unshifted_codepoint = if raw_codepoint == u32::MAX {
                None
            } else {
                Some(char::from_u32(raw_codepoint).ok_or(Error::InvalidValue {
                    field: "unshifted codepoint",
                })?)
            };
            let event = KeyEvent {
                action,
                key,
                modifiers,
                consumed_modifiers,
                composing,
                text,
                unshifted_codepoint,
            };
            validate_key(&event)?;
            ClientMessage::Key(event)
        }
        CLIENT_MOUSE => {
            let action = decode_mouse_action(reader.u8()?)?;
            let button_tag = reader.u8()?;
            let button = if button_tag == 0 {
                None
            } else {
                Some(decode_mouse_button(button_tag - 1)?)
            };
            let modifiers = checked_modifiers(reader.u16()?)?;
            let event = MouseEvent {
                action,
                button,
                modifiers,
                x: f32::from_bits(reader.u32()?),
                y: f32::from_bits(reader.u32()?),
            };
            validate_mouse(&event)?;
            ClientMessage::Mouse(event)
        }
        CLIENT_FOCUS => ClientMessage::Focus(match reader.u8()? {
            0 => FocusEvent::Gained,
            1 => FocusEvent::Lost,
            value => {
                return Err(Error::InvalidTag {
                    field: "focus",
                    value,
                });
            }
        }),
        CLIENT_PASTE => {
            let data = reader.take_remaining();
            validate_bound(data.len(), MAX_PASTE_BYTES)?;
            ClientMessage::Paste(data.to_vec())
        }
        CLIENT_RESIZE => {
            let size = SurfaceSize {
                cols: reader.u16()?,
                rows: reader.u16()?,
                screen_width: reader.u32()?,
                screen_height: reader.u32()?,
                cell_width: reader.u32()?,
                cell_height: reader.u32()?,
                padding_top: reader.u32()?,
                padding_bottom: reader.u32()?,
                padding_left: reader.u32()?,
                padding_right: reader.u32()?,
            };
            validate_surface_size(&size)?;
            ClientMessage::Resize(size)
        }
        CLIENT_SELECTION => {
            let action = match reader.u8()? {
                SELECTION_BEGIN => SelectionAction::Begin {
                    frame_revision: u64::from_le_bytes(reader.array()?),
                    time_ns: u64::from_le_bytes(reader.array()?),
                    modifiers: checked_modifiers(reader.u16()?)?,
                    position: SelectionPosition {
                        x: f32::from_bits(reader.u32()?),
                        y: f32::from_bits(reader.u32()?),
                    },
                },
                SELECTION_UPDATE => SelectionAction::Update {
                    modifiers: checked_modifiers(reader.u16()?)?,
                    position: SelectionPosition {
                        x: f32::from_bits(reader.u32()?),
                        y: f32::from_bits(reader.u32()?),
                    },
                },
                SELECTION_FINISH => SelectionAction::Finish {
                    modifiers: checked_modifiers(reader.u16()?)?,
                    position: SelectionPosition {
                        x: f32::from_bits(reader.u32()?),
                        y: f32::from_bits(reader.u32()?),
                    },
                },
                SELECTION_COPY => SelectionAction::Copy,
                SELECTION_CANCEL => SelectionAction::Cancel,
                SELECTION_AUTOSCROLL_UP => SelectionAction::AutoscrollUp {
                    position: SelectionPosition {
                        x: f32::from_bits(reader.u32()?),
                        y: f32::from_bits(reader.u32()?),
                    },
                },
                value => {
                    return Err(Error::InvalidTag {
                        field: "selection action",
                        value,
                    });
                }
            };
            validate_selection(&action)?;
            ClientMessage::Selection(action)
        }
        CLIENT_PREVIEW_VERTICAL => ClientMessage::PreviewVertical {
            frame_revision: u64::from_le_bytes(reader.array()?),
            direction: decode_vertical_direction(reader.u8()?)?,
        },
        CLIENT_SCROLL_VERTICAL => {
            let frame_revision = u64::from_le_bytes(reader.array()?);
            let rows = reader.i16()?;
            validate_scroll_rows(rows)?;
            ClientMessage::ScrollVertical {
                frame_revision,
                rows,
            }
        }
        CLIENT_RETURN_TO_LIVE => ClientMessage::ReturnToLive,
        value => {
            return Err(Error::InvalidTag {
                field: "client message",
                value,
            });
        }
    };
    reader.finish()?;
    Ok(message)
}

/// Encodes one server message with an ORBS v13 header.
pub fn encode_server_message(message: &ServerMessage) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let kind = match message {
        ServerMessage::Attached => SERVER_ATTACHED,
        ServerMessage::ObservingMetadata => SERVER_OBSERVING_METADATA,
        ServerMessage::Metadata(metadata) => {
            payload.extend_from_slice(&metadata.revision.to_le_bytes());
            put_bounded_bytes(&mut payload, metadata.title.as_bytes(), MAX_METADATA_BYTES)?;
            put_bounded_bytes(
                &mut payload,
                metadata.working_directory.as_bytes(),
                MAX_METADATA_BYTES,
            )?;
            validate_bound(payload.len(), MAX_METADATA_BYTES)?;
            SERVER_METADATA
        }
        ServerMessage::Busy => SERVER_BUSY,
        ServerMessage::Frame(frame) => {
            payload = encode_frame(frame)?;
            SERVER_FRAME
        }
        ServerMessage::Accepted => SERVER_ACCEPTED,
        ServerMessage::SelectionFinished { frame_revision } => {
            payload.extend_from_slice(&frame_revision.to_le_bytes());
            SERVER_SELECTION_FINISHED
        }
        ServerMessage::Failure(failure) => {
            payload.push(failure_code_tag(failure.code));
            put_bounded_bytes(&mut payload, failure.detail.as_bytes(), MAX_FAILURE_BYTES)?;
            SERVER_FAILURE
        }
        ServerMessage::Exited { code } => {
            payload.extend_from_slice(&code.to_le_bytes());
            SERVER_EXITED
        }
        ServerMessage::CopiedText { location, text } => {
            validate_bound(text.len(), MAX_COPY_BYTES)?;
            payload.push(clipboard_location_tag(*location));
            payload.extend_from_slice(text.as_bytes());
            SERVER_COPIED_TEXT
        }
        ServerMessage::ClipboardWrite { location, text } => {
            validate_clipboard_text(text)?;
            payload.push(clipboard_location_tag(*location));
            payload.extend_from_slice(text.as_bytes());
            SERVER_CLIPBOARD_WRITE
        }
        ServerMessage::VerticalPreview(preview) => {
            payload.extend_from_slice(&preview.frame_revision.to_le_bytes());
            payload.push(vertical_direction_tag(preview.direction));
            match &preview.outcome {
                PreviewOutcome::TerminalRouted => payload.push(0),
                next @ PreviewOutcome::Viewport { .. } => {
                    payload.push(1);
                    encode_preview_window(&mut payload, next)?;
                }
            }
            SERVER_VERTICAL_PREVIEW
        }
        ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted) => SERVER_WHEEL_TERMINAL_ROUTED,
        ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows,
            frame,
        }) => {
            payload = encode_frame(frame)?;
            match applied_rows {
                -1 => SERVER_WHEEL_VIEWPORT_UP,
                0 => SERVER_WHEEL_VIEWPORT_STILL,
                1 => SERVER_WHEEL_VIEWPORT_DOWN,
                _ => {
                    return Err(Error::InvalidValue {
                        field: "applied wheel rows",
                    });
                }
            }
        }
        ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned { requested_rows }) => {
            validate_scroll_rows(*requested_rows)?;
            put_i16(&mut payload, *requested_rows);
            SERVER_SCROLL_TERMINAL_OWNED
        }
        ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
            requested_rows,
            applied_rows,
            frame,
            next,
        }) => {
            validate_applied_scroll_rows(*requested_rows, *applied_rows)?;
            put_i16(&mut payload, *requested_rows);
            put_i16(&mut payload, *applied_rows);
            let frame = encode_frame(frame)?;
            put_u32(
                &mut payload,
                u32::try_from(frame.len()).expect("frame bound fits u32"),
            );
            payload.extend_from_slice(&frame);
            encode_preview_window(&mut payload, next)?;
            SERVER_SCROLL_VIEWPORT
        }
    };
    frame_message(kind, payload, server_payload_limits(kind)?.1)
}

/// Decodes exactly one server message.
pub fn decode_server_message(bytes: &[u8]) -> Result<ServerMessage> {
    let (kind, payload) = exact_message(bytes, server_message_len(bytes)?)?;
    if matches!(
        kind,
        SERVER_FRAME
            | SERVER_WHEEL_VIEWPORT_UP
            | SERVER_WHEEL_VIEWPORT_STILL
            | SERVER_WHEEL_VIEWPORT_DOWN
    ) {
        let frame = Box::new(decode_frame(payload)?);
        return Ok(if kind == SERVER_FRAME {
            ServerMessage::Frame(frame)
        } else {
            ServerMessage::WheelOutcome(WheelOutcome::Viewport {
                applied_rows: match kind {
                    SERVER_WHEEL_VIEWPORT_UP => -1,
                    SERVER_WHEEL_VIEWPORT_STILL => 0,
                    SERVER_WHEEL_VIEWPORT_DOWN => 1,
                    _ => unreachable!("frame-bearing kind was matched"),
                },
                frame,
            })
        });
    }
    if kind == SERVER_SCROLL_VIEWPORT {
        let mut reader = Reader::new(payload);
        let requested_rows = reader.i16()?;
        let applied_rows = reader.i16()?;
        validate_applied_scroll_rows(requested_rows, applied_rows)?;
        let frame_length = reader.u32()? as usize;
        validate_bound(frame_length, MAX_FRAME_BYTES)?;
        let frame = Box::new(decode_frame(reader.take(frame_length)?)?);
        let next = decode_preview_window(&mut reader)?;
        reader.finish()?;
        return Ok(ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
            requested_rows,
            applied_rows,
            frame,
            next,
        }));
    }
    let mut reader = Reader::new(payload);
    let message = match kind {
        SERVER_ATTACHED => ServerMessage::Attached,
        SERVER_OBSERVING_METADATA => ServerMessage::ObservingMetadata,
        SERVER_METADATA => ServerMessage::Metadata(Metadata {
            revision: u64::from_le_bytes(reader.array()?),
            title: reader.bounded_text("metadata title", MAX_METADATA_BYTES)?,
            working_directory: reader
                .bounded_text("metadata working directory", MAX_METADATA_BYTES)?,
        }),
        SERVER_BUSY => ServerMessage::Busy,
        SERVER_ACCEPTED => ServerMessage::Accepted,
        SERVER_SELECTION_FINISHED => ServerMessage::SelectionFinished {
            frame_revision: u64::from_le_bytes(reader.array()?),
        },
        SERVER_FAILURE => {
            let code = decode_failure_code(reader.u8()?)?;
            let detail = reader.bounded_text("failure detail", MAX_FAILURE_BYTES)?;
            ServerMessage::Failure(Failure { code, detail })
        }
        SERVER_EXITED => ServerMessage::Exited {
            code: i32::from_le_bytes(reader.array()?),
        },
        SERVER_COPIED_TEXT => {
            let location = decode_clipboard_location(reader.u8()?)?;
            let text = str::from_utf8(reader.take_remaining())
                .map_err(|_| Error::InvalidUtf8 {
                    field: "copied text",
                })?
                .to_owned();
            ServerMessage::CopiedText { location, text }
        }
        SERVER_CLIPBOARD_WRITE => {
            let location = decode_clipboard_location(reader.u8()?)?;
            let text = str::from_utf8(reader.take_remaining()).map_err(|_| Error::InvalidUtf8 {
                field: "clipboard text",
            })?;
            validate_clipboard_text(text)?;
            ServerMessage::ClipboardWrite {
                location,
                text: text.to_owned(),
            }
        }
        SERVER_WHEEL_TERMINAL_ROUTED => ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted),
        SERVER_SCROLL_TERMINAL_OWNED => {
            let requested_rows = reader.i16()?;
            validate_scroll_rows(requested_rows)?;
            ServerMessage::ScrollOutcome(ScrollOutcome::TerminalOwned { requested_rows })
        }
        SERVER_VERTICAL_PREVIEW => {
            let frame_revision = u64::from_le_bytes(reader.array()?);
            let direction = decode_vertical_direction(reader.u8()?)?;
            let outcome = match reader.u8()? {
                0 => PreviewOutcome::TerminalRouted,
                1 => decode_preview_window(&mut reader)?,
                value => {
                    return Err(Error::InvalidTag {
                        field: "preview outcome",
                        value,
                    });
                }
            };
            ServerMessage::VerticalPreview(VerticalPreview {
                frame_revision,
                direction,
                outcome,
            })
        }
        value => {
            return Err(Error::InvalidTag {
                field: "server message",
                value,
            });
        }
    };
    reader.finish()?;
    Ok(message)
}

fn frame_message(kind: u8, payload: Vec<u8>, maximum: usize) -> Result<Vec<u8>> {
    validate_bound(payload.len(), maximum)?;
    let mut framed = Vec::with_capacity(HEADER_BYTES + payload.len());
    framed.extend_from_slice(MAGIC);
    put_u16(&mut framed, VERSION);
    framed.push(kind);
    framed.push(0);
    put_u32(
        &mut framed,
        u32::try_from(payload.len()).expect("payload bound fits u32"),
    );
    framed.extend_from_slice(&payload);
    Ok(framed)
}

fn exact_message(bytes: &[u8], length: Option<usize>) -> Result<(u8, &[u8])> {
    let Some(length) = length else {
        return Err(Error::Truncated);
    };
    if bytes.len() < length {
        return Err(Error::Truncated);
    }
    if bytes.len() > length {
        return Err(Error::TrailingBytes {
            count: bytes.len() - length,
        });
    }
    Ok((bytes[6], &bytes[HEADER_BYTES..]))
}

fn validate_bound(size: usize, maximum: usize) -> Result<()> {
    if size > maximum {
        Err(Error::PayloadTooLarge { size, maximum })
    } else {
        Ok(())
    }
}

fn validate_clipboard_text(text: &str) -> Result<()> {
    validate_bound(text.len(), MAX_COPY_BYTES)?;
    if text.is_empty() || text.contains('\0') {
        Err(Error::InvalidValue {
            field: "clipboard text",
        })
    } else {
        Ok(())
    }
}

fn validate_scroll_rows(rows: i16) -> Result<()> {
    if rows == 0 || !(-MAX_SCROLL_ROWS..=MAX_SCROLL_ROWS).contains(&rows) {
        Err(Error::InvalidValue {
            field: "vertical scroll rows",
        })
    } else {
        Ok(())
    }
}

fn validate_applied_scroll_rows(requested: i16, applied: i16) -> Result<()> {
    validate_scroll_rows(requested)?;
    if applied == 0
        || requested.is_negative() == applied.is_negative()
            && applied.unsigned_abs() <= requested.unsigned_abs()
    {
        Ok(())
    } else {
        Err(Error::InvalidValue {
            field: "applied scroll rows",
        })
    }
}

fn encode_preview_window(target: &mut Vec<u8>, next: &PreviewOutcome) -> Result<()> {
    let PreviewOutcome::Viewport {
        cols,
        edge_reached,
        rows,
    } = next
    else {
        return Err(Error::InvalidValue {
            field: "next vertical preview",
        });
    };
    if rows.is_empty() && !edge_reached {
        return Err(Error::InvalidValue {
            field: "preview edge rows",
        });
    }
    if *cols == 0 {
        return Err(Error::InvalidValue {
            field: "preview columns",
        });
    }
    put_u16(target, *cols);
    target.push(u8::from(*edge_reached));
    put_u16(
        target,
        u16::try_from(rows.len()).map_err(|_| Error::InvalidValue {
            field: "preview row count",
        })?,
    );
    target.extend_from_slice(&encode_canonical_rows(rows, *cols)?);
    Ok(())
}

fn decode_preview_window(reader: &mut Reader<'_>) -> Result<PreviewOutcome> {
    let cols = reader.u16()?;
    let edge_reached = decode_bool(reader.u8()?, "preview edge")?;
    let row_count = reader.u16()?;
    if cols == 0 {
        return Err(Error::InvalidValue {
            field: "preview columns",
        });
    }
    if row_count == 0 && !edge_reached {
        return Err(Error::InvalidValue {
            field: "preview edge rows",
        });
    }
    if usize::from(row_count).saturating_mul(usize::from(cols)) > MAX_CELLS {
        return Err(Error::InvalidValue {
            field: "preview cell count",
        });
    }
    let rows = decode_canonical_rows(reader.take_remaining(), cols, row_count)?;
    Ok(PreviewOutcome::Viewport {
        cols,
        edge_reached,
        rows,
    })
}

fn validate_key(event: &KeyEvent) -> Result<()> {
    checked_modifiers(event.modifiers.bits())?;
    checked_modifiers(event.consumed_modifiers.bits())?;
    if !event.modifiers.contains(event.consumed_modifiers) {
        return Err(Error::ConsumedModifiersNotActive);
    }
    if let Some(text) = &event.text {
        validate_bound(text.len(), MAX_KEY_TEXT_BYTES)?;
        if text.chars().any(|character| {
            matches!(
                character,
                '\0'..='\u{1f}' | '\u{7f}' | '\u{f700}'..='\u{f8ff}'
            )
        }) {
            return Err(Error::InvalidValue { field: "key text" });
        }
    }
    Ok(())
}

fn validate_mouse(event: &MouseEvent) -> Result<()> {
    checked_modifiers(event.modifiers.bits())?;
    if event.button.is_none() && event.action != MouseAction::Motion {
        return Err(Error::InvalidValue {
            field: "mouse button",
        });
    }
    if matches!(
        event.button,
        Some(MouseButton::Four | MouseButton::Five | MouseButton::Six | MouseButton::Seven)
    ) && event.action != MouseAction::Press
    {
        return Err(Error::InvalidValue {
            field: "mouse action",
        });
    }
    validate_surface_position(event.x, event.y)
}

fn validate_selection(action: &SelectionAction) -> Result<()> {
    match action {
        SelectionAction::Begin {
            position,
            modifiers,
            ..
        }
        | SelectionAction::Update {
            position,
            modifiers,
        }
        | SelectionAction::Finish {
            position,
            modifiers,
        } => {
            checked_modifiers(modifiers.bits())?;
            validate_surface_position(position.x, position.y)
        }
        SelectionAction::AutoscrollUp { position } => {
            validate_surface_position(position.x, position.y)
        }
        SelectionAction::Cancel | SelectionAction::Copy => Ok(()),
    }
}

fn validate_surface_position(x: f32, y: f32) -> Result<()> {
    let coordinate_range = 0.0..=f32::from(u16::MAX);
    if coordinate_range.contains(&x) && coordinate_range.contains(&y) {
        Ok(())
    } else {
        Err(Error::InvalidCoordinates)
    }
}

fn validate_surface_size(size: &SurfaceSize) -> Result<()> {
    let max_screen = u32::from(u16::MAX);
    let cells = usize::from(size.cols).checked_mul(usize::from(size.rows));
    let horizontal_padding = size.padding_left.checked_add(size.padding_right);
    let vertical_padding = size.padding_top.checked_add(size.padding_bottom);
    let grid_width = u32::from(size.cols).checked_mul(size.cell_width);
    let grid_height = u32::from(size.rows).checked_mul(size.cell_height);
    let used_width = grid_width.and_then(|grid| horizontal_padding?.checked_add(grid));
    let used_height = grid_height.and_then(|grid| vertical_padding?.checked_add(grid));
    if size.cols == 0
        || size.rows == 0
        || size.screen_width == 0
        || size.screen_height == 0
        || size.screen_width > max_screen
        || size.screen_height > max_screen
        || size.cell_width == 0
        || size.cell_height == 0
        || !matches!(cells, Some(count) if count <= MAX_CELLS)
        || !matches!(horizontal_padding, Some(padding) if padding < size.screen_width)
        || !matches!(vertical_padding, Some(padding) if padding < size.screen_height)
        || !matches!(used_width, Some(used) if used <= size.screen_width)
        || !matches!(used_height, Some(used) if used <= size.screen_height)
    {
        return Err(Error::InvalidSurfaceSize);
    }
    Ok(())
}

fn key_action_tag(action: KeyAction) -> u8 {
    match action {
        KeyAction::Press => 0,
        KeyAction::Release => 1,
        KeyAction::Repeat => 2,
    }
}

fn decode_key_action(value: u8) -> Result<KeyAction> {
    match value {
        0 => Ok(KeyAction::Press),
        1 => Ok(KeyAction::Release),
        2 => Ok(KeyAction::Repeat),
        value => Err(Error::InvalidTag {
            field: "key action",
            value,
        }),
    }
}

pub(super) fn mouse_action_tag(action: MouseAction) -> u8 {
    match action {
        MouseAction::Press => 0,
        MouseAction::Release => 1,
        MouseAction::Motion => 2,
    }
}

fn vertical_direction_tag(direction: VerticalDirection) -> u8 {
    match direction {
        VerticalDirection::Up => 0,
        VerticalDirection::Down => 1,
    }
}

fn decode_vertical_direction(value: u8) -> Result<VerticalDirection> {
    match value {
        0 => Ok(VerticalDirection::Up),
        1 => Ok(VerticalDirection::Down),
        value => Err(Error::InvalidTag {
            field: "vertical direction",
            value,
        }),
    }
}

fn decode_mouse_action(value: u8) -> Result<MouseAction> {
    match value {
        0 => Ok(MouseAction::Press),
        1 => Ok(MouseAction::Release),
        2 => Ok(MouseAction::Motion),
        value => Err(Error::InvalidTag {
            field: "mouse action",
            value,
        }),
    }
}

fn mouse_button_tag(button: MouseButton) -> u8 {
    match button {
        MouseButton::Unknown => 0,
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        MouseButton::Four => 4,
        MouseButton::Five => 5,
        MouseButton::Six => 6,
        MouseButton::Seven => 7,
        MouseButton::Eight => 8,
        MouseButton::Nine => 9,
        MouseButton::Ten => 10,
        MouseButton::Eleven => 11,
    }
}

fn decode_mouse_button(value: u8) -> Result<MouseButton> {
    match value {
        0 => Ok(MouseButton::Unknown),
        1 => Ok(MouseButton::Left),
        2 => Ok(MouseButton::Middle),
        3 => Ok(MouseButton::Right),
        4 => Ok(MouseButton::Four),
        5 => Ok(MouseButton::Five),
        6 => Ok(MouseButton::Six),
        7 => Ok(MouseButton::Seven),
        8 => Ok(MouseButton::Eight),
        9 => Ok(MouseButton::Nine),
        10 => Ok(MouseButton::Ten),
        11 => Ok(MouseButton::Eleven),
        value => Err(Error::InvalidTag {
            field: "mouse button",
            value,
        }),
    }
}

fn failure_code_tag(code: FailureCode) -> u8 {
    match code {
        FailureCode::InvalidInput => 0,
        FailureCode::Protocol => 1,
        FailureCode::Terminal => 2,
    }
}

fn clipboard_location_tag(location: ClipboardLocation) -> u8 {
    match location {
        ClipboardLocation::Standard => 0,
        ClipboardLocation::Selection => 1,
        ClipboardLocation::Primary => 2,
    }
}

fn decode_clipboard_location(value: u8) -> Result<ClipboardLocation> {
    match value {
        0 => Ok(ClipboardLocation::Standard),
        1 => Ok(ClipboardLocation::Selection),
        2 => Ok(ClipboardLocation::Primary),
        value => Err(Error::InvalidTag {
            field: "clipboard location",
            value,
        }),
    }
}

fn decode_failure_code(value: u8) -> Result<FailureCode> {
    match value {
        0 => Ok(FailureCode::InvalidInput),
        1 => Ok(FailureCode::Protocol),
        2 => Ok(FailureCode::Terminal),
        value => Err(Error::InvalidTag {
            field: "failure code",
            value,
        }),
    }
}

fn checked_modifiers(value: u16) -> Result<Modifiers> {
    Modifiers::from_bits(value).ok_or(Error::InvalidValue { field: "modifiers" })
}

fn decode_bool(value: u8, field: &'static str) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(Error::InvalidTag { field, value }),
    }
}

fn put_optional_text(target: &mut Vec<u8>, value: Option<&str>) -> Result<()> {
    match value {
        Some(value) => put_bounded_bytes(target, value.as_bytes(), MAX_KEY_TEXT_BYTES),
        None => {
            put_u32(target, u32::MAX);
            Ok(())
        }
    }
}

fn put_bounded_bytes(target: &mut Vec<u8>, value: &[u8], maximum: usize) -> Result<()> {
    validate_bound(value.len(), maximum)?;
    put_u32(
        target,
        u32::try_from(value.len()).expect("message-specific bound fits u32"),
    );
    target.extend_from_slice(value);
    Ok(())
}

fn put_u16(target: &mut Vec<u8>, value: u16) {
    target.extend_from_slice(&value.to_le_bytes());
}

fn put_i16(target: &mut Vec<u8>, value: i16) {
    target.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(target: &mut Vec<u8>, value: u32) {
    target.extend_from_slice(&value.to_le_bytes());
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn i16(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| Error::Truncated)
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self.offset.checked_add(length).ok_or(Error::Truncated)?;
        let value = self.bytes.get(self.offset..end).ok_or(Error::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn take_remaining(&mut self) -> &'a [u8] {
        let value = &self.bytes[self.offset..];
        self.offset = self.bytes.len();
        value
    }

    fn optional_text(&mut self, field: &'static str, maximum: usize) -> Result<Option<String>> {
        let length = self.u32()?;
        if length == u32::MAX {
            return Ok(None);
        }
        self.text_with_length(field, length as usize, maximum)
            .map(Some)
    }

    fn bounded_text(&mut self, field: &'static str, maximum: usize) -> Result<String> {
        let length = self.u32()? as usize;
        self.text_with_length(field, length, maximum)
    }

    fn text_with_length(
        &mut self,
        field: &'static str,
        length: usize,
        maximum: usize,
    ) -> Result<String> {
        validate_bound(length, maximum)?;
        let bytes = self.take(length)?;
        str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| Error::InvalidUtf8 { field })
    }

    fn finish(self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(Error::TrailingBytes {
                count: self.bytes.len() - self.offset,
            })
        }
    }
}
