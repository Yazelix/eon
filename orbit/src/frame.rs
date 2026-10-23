//! Canonical frames and vertical previews read from libghostty.

use crate::errno;
use libghostty_vt::{
    Error as GhosttyError, RenderState, Terminal,
    render::{CellIterator, CursorVisualStyle as GhosttyCursorShape, RowIterator},
    screen::{CellContentTag, CellWide as GhosttyCellWidth, GridRef, Screen as GhosttyScreen},
    style::{RgbColor, Style, StyleColor as GhosttyStyleColor, Underline as GhosttyUnderline},
    terminal::{Point, PointCoordinate},
};
use orbit_protocol::{
    Capabilities, Cell as ProtocolCell, CellStyle, CellWidth, Colors, Cursor, CursorShape,
    CursorViewport, Dimensions, Frame, FrameSize, MAX_CELLS, MAX_FRAME_BYTES, Rgb, Row,
    Screen as ProtocolScreen, ScrollPosition, StyleColor, Underline,
    session::{PreviewOutcome, VerticalDirection},
};
use std::io::{self};

pub(crate) const SCROLLBACK_BUDGET_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct FrameExtractor {
    state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
}

impl FrameExtractor {
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            state: RenderState::new().map_err(|_| errno(libc::EIO))?,
            rows: RowIterator::new().map_err(|_| errno(libc::EIO))?,
            cells: CellIterator::new().map_err(|_| errno(libc::EIO))?,
        })
    }

    pub(crate) fn frame(
        &mut self,
        revision: u64,
        terminal: &Terminal<'static, '_>,
    ) -> io::Result<Frame> {
        let snapshot = self.state.update(terminal).map_err(|_| errno(libc::EIO))?;
        let cols = snapshot.cols().map_err(|_| errno(libc::EIO))?;
        let rows = snapshot.rows().map_err(|_| errno(libc::EIO))?;
        if usize::from(cols) * usize::from(rows) > MAX_CELLS {
            return Err(errno(libc::EOVERFLOW));
        }

        let colors = snapshot.colors().map_err(|_| errno(libc::EIO))?;
        let screen = match terminal.active_screen().map_err(|_| errno(libc::EIO))? {
            GhosttyScreen::Primary => ProtocolScreen::Primary,
            GhosttyScreen::Alternate => ProtocolScreen::Alternate,
        };
        let scrollbar = terminal.scrollbar().map_err(|_| errno(libc::EIO))?;
        if scrollbar.len != u64::from(rows) {
            return Err(errno(libc::EPROTO));
        }
        let history_rows = scrollbar
            .total
            .checked_sub(scrollbar.len)
            .ok_or_else(|| errno(libc::EPROTO))?;
        let cursor = Cursor {
            visible: snapshot.cursor_visible().map_err(|_| errno(libc::EIO))?,
            blinking: snapshot.cursor_blinking().map_err(|_| errno(libc::EIO))?,
            password_input: snapshot
                .cursor_password_input()
                .map_err(|_| errno(libc::EIO))?,
            shape: cursor_shape(
                snapshot
                    .cursor_visual_style()
                    .map_err(|_| errno(libc::EIO))?,
            )?,
            viewport: snapshot
                .cursor_viewport()
                .map_err(|_| errno(libc::EIO))?
                .map(|cursor| CursorViewport {
                    x: cursor.x,
                    y: cursor.y,
                    at_wide_tail: cursor.at_wide_tail,
                }),
        };
        let (title, working_directory) = title_and_directory(terminal);
        let mut size = FrameSize::new(
            title,
            working_directory,
            colors.cursor.is_some(),
            cursor.viewport.is_some(),
        )
        .map_err(|_| errno(libc::EOVERFLOW))?;
        let title = title.to_owned();
        let working_directory = working_directory.to_owned();
        let mut row_iter = self.rows.update(&snapshot).map_err(|_| errno(libc::EIO))?;
        let mut text = String::new();
        let mut frame_rows = Vec::with_capacity(usize::from(rows));
        let mut link_bytes = 0;
        for y in 0..rows {
            size.add_row().map_err(|_| errno(libc::EOVERFLOW))?;
            let row = row_iter.next().ok_or_else(|| errno(libc::EPROTO))?;
            let raw_row = row.raw_row().map_err(|_| errno(libc::EIO))?;
            let mut cell_iter = self.cells.update(row).map_err(|_| errno(libc::EIO))?;
            let mut frame_cells = Vec::with_capacity(usize::from(cols));
            for x in 0..cols {
                let cell = cell_iter.next().ok_or_else(|| errno(libc::EPROTO))?;
                let raw_cell = cell.raw_cell().map_err(|_| errno(libc::EIO))?;
                let mut style = cell.style().map_err(|_| errno(libc::EIO))?;
                if let Some(background) = cell.bg_color().map_err(|_| errno(libc::EIO))? {
                    style.bg_color = GhosttyStyleColor::Rgb(background);
                }
                let grapheme_count = cell.graphemes_len().map_err(|_| errno(libc::EIO))?;
                if grapheme_count > MAX_FRAME_BYTES / size_of::<char>() {
                    return Err(errno(libc::EOVERFLOW));
                }
                text.clear();
                cell.graphemes_utf8(&mut text)
                    .map_err(|_| errno(libc::EOVERFLOW))?;
                size.add_cell(&text, "")
                    .map_err(|_| errno(libc::EOVERFLOW))?;
                // The cell reserved an empty link, so a link adds only its
                // bytes. Later text only shrinks the room for links, so once
                // the links read pass the budget, none after them can fit.
                let hyperlink = if size.bytes() + link_bytes <= MAX_FRAME_BYTES
                    && raw_cell.has_hyperlink().map_err(|_| errno(libc::EIO))?
                {
                    hyperlink(terminal, x, y)?
                } else {
                    String::new()
                };
                link_bytes += hyperlink.len();
                frame_cells.push(ProtocolCell {
                    width: cell_width(raw_cell.wide().map_err(|_| errno(libc::EIO))?),
                    style: cell_style(
                        style,
                        cell.is_selected().map_err(|_| errno(libc::EIO))?,
                        raw_cell.is_protected().map_err(|_| errno(libc::EIO))?,
                    )?,
                    text: text.clone(),
                    hyperlink,
                });
            }
            if cell_iter.next().is_some() {
                return Err(errno(libc::EPROTO));
            }
            frame_rows.push(Row {
                wrapped: raw_row.is_wrapped().map_err(|_| errno(libc::EIO))?,
                wrap_continuation: raw_row
                    .is_wrap_continuation()
                    .map_err(|_| errno(libc::EIO))?,
                kitty_virtual_placeholder: raw_row
                    .has_kitty_virtual_placeholder()
                    .map_err(|_| errno(libc::EIO))?,
                cells: frame_cells,
            });
        }
        if row_iter.next().is_some() {
            return Err(errno(libc::EPROTO));
        }
        // Every cell's text is budgeted; links follow in reading order, and
        // from the first that does not fit, each cell keeps its text without
        // a link. A program's many links never end its terminal (ORB-C6).
        link_bytes = 0;
        for cell in frame_rows.iter_mut().flat_map(|row| &mut row.cells) {
            link_bytes += cell.hyperlink.len();
            if size.bytes() + link_bytes > MAX_FRAME_BYTES {
                cell.hyperlink.clear();
            }
        }
        Ok(Frame {
            revision,
            dimensions: Dimensions { cols, rows },
            screen,
            title,
            working_directory,
            capabilities: Capabilities {
                hyperlinks: true,
                kitty_graphics: false,
            },
            colors: Colors {
                background: rgb(colors.background),
                foreground: rgb(colors.foreground),
                cursor: colors.cursor.map(rgb),
                palette: colors.palette.map(rgb),
            },
            cursor,
            scroll_position: ScrollPosition {
                rows_from_live: history_rows
                    .checked_sub(scrollbar.offset)
                    .ok_or_else(|| errno(libc::EPROTO))?,
                history_rows,
            },
            rows: frame_rows,
        })
    }
}

impl FrameExtractor {
    /// One canonical row from libghostty's screen coordinates, for history beyond the viewport.
    fn row(&self, terminal: &Terminal<'static, '_>, screen_y: u32, cols: u16) -> io::Result<Row> {
        let at = |x| {
            terminal
                .grid_ref(Point::Screen(PointCoordinate { x, y: screen_y }))
                .map_err(|_| errno(libc::EIO))
        };
        let raw_row = at(0)?.row().map_err(|_| errno(libc::EIO))?;
        let palette = terminal.color_palette().map_err(|_| errno(libc::EIO))?;
        let mut chars = vec!['\0'; 8];
        let mut cells = Vec::with_capacity(usize::from(cols));
        for x in 0..cols {
            let reference = at(x)?;
            let raw_cell = reference.cell().map_err(|_| errno(libc::EIO))?;
            let mut style = reference.style().map_err(|_| errno(libc::EIO))?;
            style.bg_color = match raw_cell.content_tag().map_err(|_| errno(libc::EIO))? {
                CellContentTag::BgColorPalette => GhosttyStyleColor::Rgb(
                    palette.get(raw_cell.bg_color_palette().map_err(|_| errno(libc::EIO))?),
                ),
                CellContentTag::BgColorRgb => {
                    GhosttyStyleColor::Rgb(raw_cell.bg_color_rgb().map_err(|_| errno(libc::EIO))?)
                }
                CellContentTag::Codepoint | CellContentTag::CodepointGrapheme => {
                    match style.bg_color {
                        GhosttyStyleColor::Palette(index) => {
                            GhosttyStyleColor::Rgb(palette.get(index))
                        }
                        background => background,
                    }
                }
            };
            let length = match reference.graphemes(&mut chars) {
                Ok(length) => length,
                Err(GhosttyError::OutOfSpace { required })
                    if required <= MAX_FRAME_BYTES / size_of::<char>() =>
                {
                    chars.resize(required, '\0');
                    reference
                        .graphemes(&mut chars)
                        .map_err(|_| errno(libc::EIO))?
                }
                Err(GhosttyError::OutOfSpace { .. }) => return Err(errno(libc::EOVERFLOW)),
                Err(_) => return Err(errno(libc::EIO)),
            };
            let hyperlink = if raw_cell.has_hyperlink().map_err(|_| errno(libc::EIO))? {
                hyperlink_uri(&reference)?
            } else {
                String::new()
            };
            cells.push(ProtocolCell {
                width: cell_width(raw_cell.wide().map_err(|_| errno(libc::EIO))?),
                style: cell_style(
                    style,
                    false,
                    raw_cell.is_protected().map_err(|_| errno(libc::EIO))?,
                )?,
                text: chars[..length].iter().collect(),
                hyperlink,
            });
        }
        Ok(Row {
            wrapped: raw_row.is_wrapped().map_err(|_| errno(libc::EIO))?,
            wrap_continuation: raw_row
                .is_wrap_continuation()
                .map_err(|_| errno(libc::EIO))?,
            kitty_virtual_placeholder: raw_row
                .has_kitty_virtual_placeholder()
                .map_err(|_| errno(libc::EIO))?,
            cells,
        })
    }
}

/// The terminal's title and working directory, each empty when libghostty
/// refuses to read it, as it does invalid UTF-8: an OSC 7 directory can be,
/// and so is a title cut inside a character at libghostty's 1024 bytes. A
/// program's bad metadata never ends its terminal.
pub(crate) fn title_and_directory<'a>(terminal: &'a Terminal<'static, '_>) -> (&'a str, &'a str) {
    (
        terminal.title().unwrap_or_default(),
        terminal.pwd().unwrap_or_default(),
    )
}

/// Up to one viewport of rows beyond the viewport, nearest first, bounded like a frame.
pub(crate) fn viewport_preview(
    terminal: &Terminal<'static, '_>,
    extractor: &FrameExtractor,
    direction: VerticalDirection,
) -> io::Result<PreviewOutcome> {
    let scrollbar = terminal.scrollbar().map_err(|_| errno(libc::EIO))?;
    let cols = terminal.cols().map_err(|_| errno(libc::EIO))?;
    let window = u64::from(terminal.rows().map_err(|_| errno(libc::EIO))?);
    let below = scrollbar.offset.saturating_add(scrollbar.len);
    let mut size = FrameSize::new("", "", false, false).map_err(|_| errno(libc::EOVERFLOW))?;
    let mut rows = Vec::new();
    let mut complete = true;
    for index in 0..window {
        let target = match direction {
            VerticalDirection::Up => scrollbar.offset.checked_sub(index + 1),
            VerticalDirection::Down => below.checked_add(index).filter(|y| *y < scrollbar.total),
        };
        let Some(target) = target else {
            break;
        };
        let row = extractor.row(
            terminal,
            u32::try_from(target).map_err(|_| errno(libc::EOVERFLOW))?,
            cols,
        )?;
        let mut next = size;
        let sized = next.add_row().and_then(|()| {
            row.cells
                .iter()
                .try_for_each(|cell| next.add_cell(&cell.text, &cell.hyperlink))
        });
        if sized.is_err() {
            complete = false;
            break;
        }
        size = next;
        rows.push(row);
    }
    let edge_reached = complete
        && match direction {
            VerticalDirection::Up => scrollbar.offset <= window,
            VerticalDirection::Down => below.saturating_add(window) >= scrollbar.total,
        };
    Ok(PreviewOutcome::Viewport {
        cols,
        edge_reached,
        rows,
    })
}

fn hyperlink(terminal: &Terminal<'static, '_>, x: u16, y: u16) -> io::Result<String> {
    let reference = terminal
        .grid_ref(Point::Viewport(PointCoordinate { x, y: u32::from(y) }))
        .map_err(|_| errno(libc::EIO))?;
    hyperlink_uri(&reference)
}

/// The cell's hyperlink URI, empty when it is not valid UTF-8: libghostty
/// stores OSC 8 bytes unchecked, and a program's bad link never ends its
/// terminal. A lossy decode would link somewhere the program never named.
fn hyperlink_uri(reference: &GridRef<'_>) -> io::Result<String> {
    let mut bytes = vec![0; 256];
    let length = match reference.hyperlink_uri(&mut bytes) {
        Ok(length) => length,
        Err(GhosttyError::OutOfSpace { required }) if required <= MAX_FRAME_BYTES => {
            bytes.resize(required, 0);
            reference
                .hyperlink_uri(&mut bytes)
                .map_err(|_| errno(libc::EIO))?
        }
        Err(GhosttyError::OutOfSpace { .. }) => return Err(errno(libc::EOVERFLOW)),
        Err(_) => return Err(errno(libc::EIO)),
    };
    bytes.truncate(length);
    Ok(String::from_utf8(bytes).unwrap_or_default())
}

fn rgb(color: RgbColor) -> Rgb {
    Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}

fn cursor_shape(shape: GhosttyCursorShape) -> io::Result<CursorShape> {
    match shape {
        GhosttyCursorShape::Bar => Ok(CursorShape::Bar),
        GhosttyCursorShape::Block => Ok(CursorShape::Block),
        GhosttyCursorShape::Underline => Ok(CursorShape::Underline),
        GhosttyCursorShape::BlockHollow => Ok(CursorShape::BlockHollow),
        _ => Err(errno(libc::EPROTO)),
    }
}

fn cell_width(width: GhosttyCellWidth) -> CellWidth {
    match width {
        GhosttyCellWidth::Narrow => CellWidth::Narrow,
        GhosttyCellWidth::Wide => CellWidth::Wide,
        GhosttyCellWidth::SpacerTail => CellWidth::SpacerTail,
        GhosttyCellWidth::SpacerHead => CellWidth::SpacerHead,
    }
}

fn style_color(color: GhosttyStyleColor) -> StyleColor {
    match color {
        GhosttyStyleColor::None => StyleColor::None,
        GhosttyStyleColor::Palette(index) => StyleColor::Palette(index.0),
        GhosttyStyleColor::Rgb(color) => StyleColor::Rgb(rgb(color)),
    }
}

fn underline(value: GhosttyUnderline) -> io::Result<Underline> {
    match value {
        GhosttyUnderline::None => Ok(Underline::None),
        GhosttyUnderline::Single => Ok(Underline::Single),
        GhosttyUnderline::Double => Ok(Underline::Double),
        GhosttyUnderline::Curly => Ok(Underline::Curly),
        GhosttyUnderline::Dotted => Ok(Underline::Dotted),
        GhosttyUnderline::Dashed => Ok(Underline::Dashed),
        _ => Err(errno(libc::EPROTO)),
    }
}

fn cell_style(style: Style, selected: bool, protected: bool) -> io::Result<CellStyle> {
    Ok(CellStyle {
        foreground: style_color(style.fg_color),
        background: style_color(style.bg_color),
        underline_color: style_color(style.underline_color),
        bold: style.bold,
        italic: style.italic,
        faint: style.faint,
        blink: style.blink,
        inverse: style.inverse,
        invisible: style.invisible,
        strikethrough: style.strikethrough,
        overline: style.overline,
        selected,
        protected,
        underline: underline(style.underline)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::terminal::ScrollViewport;
    use orbit_protocol::session::{self, ServerMessage, VerticalPreview};

    #[test]
    fn terminal_frame_uses_the_canonical_protocol_owner() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal.vt_write(b"\x1b]8;;https://example.test\x1b\\READY\x1b]8;;\x1b\\");
        let frame = FrameExtractor::new().unwrap().frame(1, &terminal).unwrap();
        assert_eq!(
            (frame.revision, frame.dimensions.cols, frame.dimensions.rows),
            (1, 8, 2)
        );
        assert!(frame.capabilities.hyperlinks);
        assert_eq!(frame.rows[0].cells[0].text, "R");
        assert_eq!(frame.rows[0].cells[0].hyperlink, "https://example.test");
        let encoded =
            session::encode_server_message(&ServerMessage::Frame(Box::new(frame))).unwrap();
        assert!(matches!(
            session::decode_server_message(&encoded).unwrap(),
            ServerMessage::Frame(frame) if frame.revision == 1
        ));
    }

    /// A frame whose links exceed the budget keeps every cell's text, with
    /// links kept in reading order until the first that does not fit
    /// (ORB-C6). Each linked cell carries its URI, so one 2 KB link over
    /// most of a 100x24 screen is enough.
    #[test]
    fn hyperlinks_over_the_frame_budget_keep_the_frame() {
        let mut terminal = Terminal::new(100, 24).unwrap();
        let uri = format!("https://example.test/{}", "x".repeat(2000));
        terminal
            .vt_write(format!("\x1b]8;;{uri}\x1b\\{}\x1b]8;;\x1b\\", "L".repeat(2399)).as_bytes());
        let frame = FrameExtractor::new().unwrap().frame(1, &terminal).unwrap();
        let cells: Vec<_> = frame
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .take(2399)
            .collect();
        assert!(cells.iter().all(|cell| cell.text == "L"));
        let kept = cells
            .iter()
            .take_while(|cell| cell.hyperlink == uri)
            .count();
        assert!(kept > 0 && kept < cells.len());
        assert!(cells[kept..].iter().all(|cell| cell.hyperlink.is_empty()));
        assert!(frame.encode().unwrap().len() + uri.len() > MAX_FRAME_BYTES);
    }

    /// Rows read directly from libghostty's screen equal the canonical frame's
    /// rows, for rich text that keeps protection, spacer-head and wide-cell
    /// width roles, colors and a hyperlink, before and after a resize reflows
    /// it (ORB-C6).
    #[test]
    fn direct_rows_match_rich_canonical_frame_rows() {
        let mut terminal = Terminal::new(12, 4).unwrap();
        terminal.set_scrollback_max_bytes(Some(100)).unwrap();
        terminal.vt_write(
            "\x1b]8;;https://example.test\x1b\\\x1b[1\"q\x1b[1;38;2;12;34;56;48;5;17mAe\u{301}界1234567界wrap-me-long\x1b[0m\x1b[0\"q\x1b]8;;\x1b\\\r\n\x1b[48;2;5;6;7m\x1b[2K\x1b[0m\r\nplain\r\ntail-1\r\ntail-2\r\ntail-3"
                .as_bytes(),
        );
        let mut extractor = FrameExtractor::new().unwrap();
        let mut frame_at_top = |terminal: &mut Terminal<'static, '_>, revision| {
            terminal.scroll_viewport(ScrollViewport::Top);
            let frame = extractor.frame(revision, terminal).unwrap();
            session::encode_server_message(&ServerMessage::Frame(Box::new(frame.clone()))).unwrap();
            for (y, expected) in frame.rows.iter().enumerate() {
                let direct = extractor
                    .row(terminal, y as u32, frame.dimensions.cols)
                    .unwrap();
                assert_eq!(direct, *expected, "row {y} at revision {revision}");
            }
            frame
        };

        let frame = frame_at_top(&mut terminal, 1);
        let cells = || frame.rows.iter().flat_map(|row| &row.cells);
        assert!(frame.rows.iter().any(|row| row.wrapped));
        assert!(cells().any(|cell| cell.hyperlink == "https://example.test"));
        assert!(cells().any(|cell| cell.style.protected));
        assert!(cells().any(|cell| {
            cell.style.foreground
                == StyleColor::Rgb(Rgb {
                    r: 12,
                    g: 34,
                    b: 56,
                })
        }));
        assert!(
            cells().any(|cell| cell.style.background == StyleColor::Rgb(Rgb { r: 0, g: 0, b: 95 }))
        );
        assert!(
            cells().any(|cell| cell.style.background == StyleColor::Rgb(Rgb { r: 5, g: 6, b: 7 }))
        );
        assert!(cells().any(|cell| cell.text == "e\u{301}"));
        assert!(cells().any(|cell| cell.width == CellWidth::Wide));
        assert!(cells().any(|cell| cell.width == CellWidth::SpacerTail));
        assert!(cells().any(|cell| cell.width == CellWidth::SpacerHead));

        // Narrower columns reflow the wrapped rich text; its rows still match
        // and keep their protection and hyperlink.
        terminal.resize(9, 4, 8, 16).unwrap();
        let reflowed = frame_at_top(&mut terminal, 2);
        assert_eq!(reflowed.dimensions.cols, 9);
        let cells = || reflowed.rows.iter().flat_map(|row| &row.cells);
        assert!(reflowed.rows.iter().filter(|row| row.wrapped).count() >= 2);
        assert!(cells().any(|cell| cell.style.protected));
        assert!(cells().any(|cell| cell.hyperlink == "https://example.test"));
    }

    /// A frame's scroll position follows libghostty's viewport through output
    /// below a held viewport, the alternate screen, reflow, byte-budget
    /// eviction, `CSI 3 J` and reset (ORB-C8).
    #[test]
    fn scroll_position_follows_reflow_eviction_and_clears() {
        let mut terminal = Terminal::new(8, 4).unwrap();
        terminal.set_scrollback_max_bytes(Some(64 * 1024)).unwrap();
        let mut extractor = FrameExtractor::new().unwrap();
        let mut frame = |terminal: &Terminal<'static, '_>| extractor.frame(0, terminal).unwrap();
        let position = |rows_from_live, history_rows| ScrollPosition {
            rows_from_live,
            history_rows,
        };
        for line in 0..100 {
            terminal.vt_write(format!("{line:07}\r\n").as_bytes());
        }
        assert_eq!(frame(&terminal).scroll_position, position(0, 97));

        // Output below a held viewport keeps its rows and deepens it.
        terminal.scroll_viewport(ScrollViewport::Delta(-3));
        let held = frame(&terminal);
        assert_eq!(held.scroll_position, position(3, 97));
        terminal.vt_write(b"0000100\r\n0000101\r\n");
        let output = frame(&terminal);
        assert_eq!(output.rows, held.rows);
        assert_eq!(output.scroll_position, position(5, 99));

        // The alternate screen has no history; leaving it restores the primary viewport.
        terminal.vt_write(b"\x1b[?1049hAlternate");
        assert_eq!(frame(&terminal).scroll_position, position(0, 0));
        terminal.vt_write(b"\x1b[?1049l");
        let primary = frame(&terminal);
        assert_eq!(primary.scroll_position, output.scroll_position);
        assert_eq!(primary.rows, output.rows);

        // Narrower columns reflow history into more rows.
        terminal.resize(4, 4, 8, 16).unwrap();
        let reflowed = frame(&terminal).scroll_position;
        assert!(reflowed.history_rows > primary.scroll_position.history_rows);
        assert!(reflowed.rows_from_live > primary.scroll_position.rows_from_live);

        // Output past the byte budget evicts the oldest rows under a viewport
        // held at the top. Wider rows fill libghostty's pages in fewer rows,
        // each of which costs about a millisecond in a debug build.
        terminal.resize(80, 4, 8, 16).unwrap();
        terminal.scroll_viewport(ScrollViewport::Top);
        let oldest = frame(&terminal);
        for line in 102..1_602 {
            terminal.vt_write(format!("{line:07}\r\n").as_bytes());
        }
        let evicted = frame(&terminal);
        assert!(evicted.scroll_position.history_rows < 1_500);
        assert_eq!(
            evicted.scroll_position.rows_from_live,
            evicted.scroll_position.history_rows
        );
        assert_ne!(evicted.rows, oldest.rows);

        // CSI 3 J and reset leave no history.
        terminal.scroll_viewport(ScrollViewport::Bottom);
        assert_eq!(frame(&terminal).scroll_position.rows_from_live, 0);
        terminal.vt_write(b"\x1b[3J");
        assert_eq!(frame(&terminal).scroll_position, position(0, 0));
        terminal.vt_write(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        terminal.scroll_viewport(ScrollViewport::Delta(-1));
        let refilled = frame(&terminal).scroll_position;
        assert!(refilled.history_rows > 0 && refilled.rows_from_live == 1);
        terminal.vt_write(b"\x1bc");
        assert_eq!(frame(&terminal).scroll_position, position(0, 0));
    }

    #[test]
    fn vertical_preview_reads_history_nearest_first() {
        let mut terminal = Terminal::new(8, 2).unwrap();
        terminal
            .set_scrollback_max_bytes(Some(SCROLLBACK_BUDGET_BYTES))
            .unwrap();
        terminal.vt_write(
            b"\x1b]8;;https://example.test\x1b\\one\x1b]8;;\x1b\\\r\n\x1b]8;;http://x/\xff\x1b\\two\x1b]8;;\x1b\\\r\nthree\r\nlongerthan8\r\nlast",
        );
        let extractor = FrameExtractor::new().unwrap();
        let text = |row: &Row| {
            row.cells
                .iter()
                .map(|cell| cell.text.as_str())
                .collect::<String>()
        };
        let up = viewport_preview(&terminal, &extractor, VerticalDirection::Up).unwrap();
        let encoded =
            session::encode_server_message(&ServerMessage::VerticalPreview(VerticalPreview {
                frame_revision: 3,
                direction: VerticalDirection::Up,
                outcome: up,
            }))
            .unwrap();
        let ServerMessage::VerticalPreview(VerticalPreview {
            outcome:
                PreviewOutcome::Viewport {
                    cols: 8,
                    edge_reached: false,
                    rows,
                },
            ..
        }) = session::decode_server_message(&encoded).unwrap()
        else {
            panic!("preview was not a canonical viewport window");
        };
        let rows: Vec<_> = rows.iter().map(|row| (text(row), row.wrapped)).collect();
        assert_eq!(rows, [("longerth".into(), true), ("three".into(), false)]);

        let oldest = extractor.row(&terminal, 0, 8).unwrap();
        assert_eq!(text(&oldest), "one");
        assert_eq!(oldest.cells[0].hyperlink, "https://example.test");
        // A URI that is not UTF-8 is no link, and the text stays.
        let invalid = extractor.row(&terminal, 1, 8).unwrap();
        assert_eq!(
            (text(&invalid).as_str(), invalid.cells[0].hyperlink.as_str()),
            ("two", "")
        );
        assert_eq!(
            viewport_preview(&terminal, &extractor, VerticalDirection::Down).unwrap(),
            PreviewOutcome::Viewport {
                cols: 8,
                edge_reached: true,
                rows: Vec::new(),
            }
        );
    }
}
