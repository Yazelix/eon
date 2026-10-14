use crate::Result;
use libghostty_vt::{
    Error as GhosttyError, RenderState, Terminal,
    render::{CellIterator, CursorVisualStyle as GhosttyCursorShape, RowIterator},
    screen::{CellContentTag, CellWide as GhosttyCellWidth, Screen},
    style::{RgbColor, Style, StyleColor as GhosttyStyleColor, Underline as GhosttyUnderline},
    terminal::{Point, PointCoordinate},
};
use orbit_protocol::{
    Capabilities, Cell as ProtocolCell, CellStyle, CellWidth, Colors, Cursor, CursorShape,
    CursorViewport, Dimensions, Frame, FrameSize, MAX_CELLS, MAX_FRAME_BYTES, Rgb, Row,
    Screen as ProtocolScreen, ScrollPosition, StyleColor, Underline,
};

pub(crate) struct Extractor {
    state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
}

impl Extractor {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self {
            state: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
        })
    }

    pub(crate) fn frame(
        &mut self,
        revision: u64,
        terminal: &Terminal<'static, '_>,
    ) -> Result<Frame> {
        let Self { state, rows, cells } = self;
        let snapshot = state.update(terminal)?;
        let cols = snapshot.cols()?;
        let row_count = snapshot.rows()?;
        if usize::from(cols) * usize::from(row_count) > MAX_CELLS {
            return Err("presentation dimensions exceed the cell bound".into());
        }

        let colors = snapshot.colors()?;
        let screen = match terminal.active_screen()? {
            Screen::Primary => ProtocolScreen::Primary,
            Screen::Alternate => ProtocolScreen::Alternate,
        };
        let scrollbar = terminal.scrollbar()?;
        if scrollbar.len != u64::from(row_count) {
            return Err("scrollbar length differs from presentation rows".into());
        }
        let history_rows = scrollbar
            .total
            .checked_sub(scrollbar.len)
            .ok_or("scrollbar total is smaller than its viewport")?;
        let scroll_position = ScrollPosition {
            rows_from_live: history_rows
                .checked_sub(scrollbar.offset)
                .ok_or("scrollbar offset exceeds retained history")?,
            history_rows,
        };
        let cursor = Cursor {
            visible: snapshot.cursor_visible()?,
            blinking: snapshot.cursor_blinking()?,
            password_input: snapshot.cursor_password_input()?,
            shape: cursor_shape(snapshot.cursor_visual_style()?)?,
            viewport: snapshot.cursor_viewport()?.map(|cursor| CursorViewport {
                x: cursor.x,
                y: cursor.y,
                at_wide_tail: cursor.at_wide_tail,
            }),
        };
        let title = terminal.title()?;
        let working_directory = terminal.pwd()?;
        let mut frame_size = FrameSize::new(
            title,
            working_directory,
            colors.cursor.is_some(),
            cursor.viewport.is_some(),
        )?;
        let title = title.to_owned();
        let working_directory = working_directory.to_owned();
        let mut row_iter = rows.update(&snapshot)?;
        let mut graphemes = String::new();
        let mut frame_rows = Vec::with_capacity(usize::from(row_count));
        for y in 0..row_count {
            frame_size.add_row()?;
            let row = row_iter.next().ok_or("render snapshot omitted a row")?;
            let raw_row = row.raw_row()?;
            let mut cell_iter = cells.update(row)?;
            let mut frame_cells = Vec::with_capacity(usize::from(cols));
            for x in 0..cols {
                let cell = cell_iter.next().ok_or("render snapshot omitted a cell")?;
                let raw_cell = cell.raw_cell()?;
                let mut style = cell.style()?;
                if let Some(background) = cell.bg_color()? {
                    style.bg_color = GhosttyStyleColor::Rgb(background);
                }

                let grapheme_count = cell.graphemes_len()?;
                if grapheme_count > MAX_FRAME_BYTES / size_of::<char>() {
                    return Err("cell grapheme exceeds the frame bound".into());
                }
                graphemes.clear();
                cell.graphemes_utf8(&mut graphemes)?;

                let hyperlink = if raw_cell.has_hyperlink()? {
                    let reference = terminal
                        .grid_ref(Point::Viewport(PointCoordinate { x, y: u32::from(y) }))?;
                    hyperlink(&reference)?
                } else {
                    String::new()
                };
                frame_size.add_cell(&graphemes, &hyperlink)?;
                frame_cells.push(ProtocolCell {
                    width: cell_width(raw_cell.wide()?)?,
                    style: cell_style(style, cell.is_selected()?, raw_cell.is_protected()?)?,
                    text: graphemes.clone(),
                    hyperlink,
                });
            }
            if cell_iter.next().is_some() {
                return Err("render snapshot added an unexpected cell".into());
            }
            frame_rows.push(Row {
                wrapped: raw_row.is_wrapped()?,
                wrap_continuation: raw_row.is_wrap_continuation()?,
                kitty_virtual_placeholder: raw_row.has_kitty_virtual_placeholder()?,
                cells: frame_cells,
            });
        }
        if row_iter.next().is_some() {
            return Err("render snapshot added an unexpected row".into());
        }
        Ok(Frame {
            revision,
            dimensions: Dimensions {
                cols,
                rows: row_count,
            },
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
            scroll_position,
            rows: frame_rows,
        })
    }

    pub(crate) fn row(
        &self,
        terminal: &Terminal<'static, '_>,
        screen_y: u32,
        cols: u16,
    ) -> Result<Row> {
        let first = terminal.grid_ref(Point::Screen(PointCoordinate { x: 0, y: screen_y }))?;
        let raw_row = first.row()?;
        let palette = terminal.color_palette()?;
        let mut frame_size = FrameSize::new("", "", false, false)?;
        frame_size.add_row()?;
        let mut chars = vec!['\0'; 8];
        let mut text = String::new();
        let mut cells = Vec::with_capacity(usize::from(cols));
        for x in 0..cols {
            let reference = terminal.grid_ref(Point::Screen(PointCoordinate { x, y: screen_y }))?;
            let raw_cell = reference.cell()?;
            let mut style = reference.style()?;
            style.bg_color = match raw_cell.content_tag()? {
                CellContentTag::BgColorPalette => {
                    GhosttyStyleColor::Rgb(palette.get(raw_cell.bg_color_palette()?))
                }
                CellContentTag::BgColorRgb => GhosttyStyleColor::Rgb(raw_cell.bg_color_rgb()?),
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
                    reference.graphemes(&mut chars)?
                }
                Err(GhosttyError::OutOfSpace { .. }) => {
                    return Err("cell grapheme exceeds the frame bound".into());
                }
                Err(error) => return Err(error.into()),
            };
            text.clear();
            text.extend(chars[..length].iter());
            let hyperlink = if raw_cell.has_hyperlink()? {
                hyperlink(&reference)?
            } else {
                String::new()
            };
            frame_size.add_cell(&text, &hyperlink)?;
            cells.push(ProtocolCell {
                width: cell_width(raw_cell.wide()?)?,
                style: cell_style(style, false, raw_cell.is_protected()?)?,
                text: text.clone(),
                hyperlink,
            });
        }
        Ok(Row {
            wrapped: raw_row.is_wrapped()?,
            wrap_continuation: raw_row.is_wrap_continuation()?,
            kitty_virtual_placeholder: raw_row.has_kitty_virtual_placeholder()?,
            cells,
        })
    }
}

fn hyperlink(reference: &libghostty_vt::screen::GridRef<'_>) -> Result<String> {
    let mut bytes = vec![0; 256];
    let length = match reference.hyperlink_uri(&mut bytes) {
        Ok(length) => length,
        Err(GhosttyError::OutOfSpace { required }) if required <= MAX_FRAME_BYTES => {
            bytes.resize(required, 0);
            reference.hyperlink_uri(&mut bytes)?
        }
        Err(GhosttyError::OutOfSpace { .. }) => {
            return Err("hyperlink exceeds the frame bound".into());
        }
        Err(error) => return Err(error.into()),
    };
    bytes.truncate(length);
    Ok(String::from_utf8(bytes)?)
}

fn rgb(color: RgbColor) -> Rgb {
    Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}

fn cursor_shape(shape: GhosttyCursorShape) -> Result<CursorShape> {
    match shape {
        GhosttyCursorShape::Bar => Ok(CursorShape::Bar),
        GhosttyCursorShape::Block => Ok(CursorShape::Block),
        GhosttyCursorShape::Underline => Ok(CursorShape::Underline),
        GhosttyCursorShape::BlockHollow => Ok(CursorShape::BlockHollow),
        _ => Err("libghostty returned an unsupported cursor shape".into()),
    }
}

fn cell_width(width: GhosttyCellWidth) -> Result<CellWidth> {
    match width {
        GhosttyCellWidth::Narrow => Ok(CellWidth::Narrow),
        GhosttyCellWidth::Wide => Ok(CellWidth::Wide),
        GhosttyCellWidth::SpacerTail => Ok(CellWidth::SpacerTail),
        GhosttyCellWidth::SpacerHead => Ok(CellWidth::SpacerHead),
    }
}

fn style_color(color: GhosttyStyleColor) -> StyleColor {
    match color {
        GhosttyStyleColor::None => StyleColor::None,
        GhosttyStyleColor::Palette(index) => StyleColor::Palette(index.0),
        GhosttyStyleColor::Rgb(color) => StyleColor::Rgb(rgb(color)),
    }
}

fn underline(underline: GhosttyUnderline) -> Result<Underline> {
    match underline {
        GhosttyUnderline::None => Ok(Underline::None),
        GhosttyUnderline::Single => Ok(Underline::Single),
        GhosttyUnderline::Double => Ok(Underline::Double),
        GhosttyUnderline::Curly => Ok(Underline::Curly),
        GhosttyUnderline::Dotted => Ok(Underline::Dotted),
        GhosttyUnderline::Dashed => Ok(Underline::Dashed),
        _ => Err("libghostty returned an unsupported underline style".into()),
    }
}

fn cell_style(style: Style, selected: bool, protected: bool) -> Result<CellStyle> {
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
    use crate::{diagnostic::read_message, runtime};
    use libghostty_vt::{TerminalOptions, terminal::ScrollViewport};
    use orbit_protocol::session::{
        self, ClientMessage, ServerMessage, decode_server_message, encode_client_message,
    };
    use std::{
        fs,
        io::{BufReader, Read, Write},
        os::unix::{fs::PermissionsExt, net::UnixStream},
        path::{Path, PathBuf},
        sync::{
            Mutex, MutexGuard,
            atomic::{AtomicU64, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);
    // ponytail: serialize three process-heavy PTY tests; split only if their runtime matters.
    static REAL_PTY_TEST: Mutex<()> = Mutex::new(());

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> TestResult<Self> {
            let path = std::env::temp_dir().join(format!(
                "orbit-presentation-{}-{}",
                std::process::id(),
                NEXT_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path)?;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
            Ok(Self(path))
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    struct Server {
        thread: Option<thread::JoinHandle<()>>,
        stop: PathBuf,
        _serial: MutexGuard<'static, ()>,
    }

    impl Server {
        fn start(socket: PathBuf, command: Vec<String>, stop: PathBuf) -> Self {
            let serial = REAL_PTY_TEST
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let thread = thread::spawn(move || {
                assert_eq!(runtime::run(&socket, &command, None, None).unwrap(), 0);
            });
            Self {
                thread: Some(thread),
                stop,
                _serial: serial,
            }
        }

        fn finish(mut self) -> TestResult {
            fs::write(&self.stop, b"stop")?;
            self.join()
        }

        fn join(&mut self) -> TestResult {
            self.thread
                .take()
                .expect("server thread exists")
                .join()
                .map_err(|_| "presentation server panicked".into())
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            if self.thread.is_some() {
                let _ = fs::write(&self.stop, b"stop");
                let _ = self.join();
            }
        }
    }

    fn read_frame(reader: &mut BufReader<UnixStream>) -> TestResult<Option<Frame>> {
        loop {
            let mut header = [0; session::HEADER_BYTES];
            reader.read_exact(&mut header)?;
            let length = session::server_message_len(&header)?
                .expect("complete session header declares its length");
            let mut framed = Vec::with_capacity(length);
            framed.extend_from_slice(&header);
            framed.resize(length, 0);
            reader.read_exact(&mut framed[session::HEADER_BYTES..])?;
            match decode_server_message(&framed)? {
                ServerMessage::Attached | ServerMessage::Accepted => {}
                ServerMessage::Frame(frame) => return Ok(Some(*frame)),
                ServerMessage::Busy => return Ok(None),
                message => return Err(format!("unexpected server message: {message:?}").into()),
            }
        }
    }

    fn expect_no_message(reader: &mut BufReader<UnixStream>, detail: &str) -> TestResult {
        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(100)))?;
        match read_message(reader) {
            Err(error)
                if error.downcast_ref::<std::io::Error>().is_some_and(|error| {
                    matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    )
                }) =>
            {
                Ok(())
            }
            result => Err(format!("{detail}: {result:?}").into()),
        }
    }

    fn cell(frame: &Frame, x: usize, y: usize) -> &ProtocolCell {
        &frame.rows[y].cells[x]
    }

    fn cells(frame: &Frame) -> impl Iterator<Item = &ProtocolCell> {
        frame.rows.iter().flat_map(|row| &row.cells)
    }

    fn attach(socket: &Path) -> TestResult<(BufReader<UnixStream>, Frame)> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let hello = encode_client_message(&ClientMessage::Hello)?;
        loop {
            match UnixStream::connect(socket) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                    if let Err(error) = (&stream).write_all(&hello) {
                        if Instant::now() >= deadline {
                            return Err(error.into());
                        }
                        thread::yield_now();
                        continue;
                    }
                    let mut reader = BufReader::new(stream);
                    if let Some(frame) = read_frame(&mut reader)? {
                        return Ok((reader, frame));
                    }
                }
                Err(error)
                    if Instant::now() < deadline
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                        ) => {}
                Err(error) => return Err(error.into()),
            }
            if Instant::now() >= deadline {
                return Err("server did not accept a presentation client".into());
            }
            thread::yield_now();
        }
    }

    fn wait_file(path: &Path) -> TestResult {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            if Instant::now() >= deadline {
                return Err(format!("timed out waiting for {}", path.display()).into());
            }
            thread::yield_now();
        }
        Ok(())
    }

    #[test]
    fn conformance_c6_direct_rows_match_rich_canonical_frame_rows() -> TestResult {
        let mut terminal = Terminal::new(TerminalOptions {
            cols: 12,
            rows: 4,
            max_scrollback: 100,
        })?;
        terminal.vt_write(
            "\x1b]8;;https://example.test\x1b\\\x1b[1\"q\x1b[1;38;2;12;34;56;48;5;17mAe\u{301}界1234567界wrap-me-long\x1b[0m\x1b[0\"q\x1b]8;;\x1b\\\r\n\x1b[48;2;5;6;7m\x1b[2K\x1b[0m\r\nplain\r\ntail-1\r\ntail-2\r\ntail-3"
                .as_bytes(),
        );
        terminal.scroll_viewport(ScrollViewport::Top);

        let mut extractor = Extractor::new()?;
        let frame = extractor.frame(1, &terminal)?;
        frame.encode()?;
        for (y, expected) in frame.rows.iter().enumerate() {
            assert_eq!(
                extractor.row(&terminal, u32::try_from(y)?, frame.dimensions.cols)?,
                *expected
            );
        }
        let mut cells = frame.rows.iter().flat_map(|row| &row.cells);
        assert!(frame.rows.iter().any(|row| row.wrapped));
        assert!(
            cells
                .clone()
                .any(|cell| cell.hyperlink == "https://example.test")
        );
        assert!(cells.clone().any(|cell| cell.style.protected));
        assert!(cells.clone().any(|cell| {
            cell.style.foreground
                == StyleColor::Rgb(Rgb {
                    r: 12,
                    g: 34,
                    b: 56,
                })
        }));
        assert!(
            cells.clone().any(|cell| {
                cell.style.background == StyleColor::Rgb(Rgb { r: 0, g: 0, b: 95 })
            })
        );
        assert!(
            cells
                .clone()
                .any(|cell| { cell.style.background == StyleColor::Rgb(Rgb { r: 5, g: 6, b: 7 }) })
        );
        assert!(cells.clone().any(|cell| cell.text == "e\u{301}"));
        assert!(cells.clone().any(|cell| cell.width == CellWidth::Wide));
        assert!(
            cells
                .clone()
                .any(|cell| cell.width == CellWidth::SpacerTail)
        );
        assert!(cells.any(|cell| cell.width == CellWidth::SpacerHead));

        terminal.scroll_viewport(ScrollViewport::Bottom);
        extractor.frame(2, &terminal)?.encode()?;
        terminal.resize(9, 4, 8, 16)?;
        terminal.scroll_viewport(ScrollViewport::Top);
        extractor.frame(3, &terminal)?.encode()?;
        Ok(())
    }

    #[test]
    fn real_pty_clipboard_write_precedes_its_frame() -> TestResult {
        let directory = TestDir::new()?;
        let socket = directory.0.join("orbit.sock");
        let emit = directory.0.join("emit");
        let emitted = directory.0.join("emitted");
        let stop = directory.0.join("stop");
        let script = format!(
            "while [ ! -e '{emit}' ]; do sleep 0.01; done; printf '\\033]52;c;emVsbGlqIGNvcHk=\\033\\\\after'; : > '{emitted}'; while [ ! -e '{stop}' ]; do sleep 0.01; done",
            emit = emit.display(),
            emitted = emitted.display(),
            stop = stop.display(),
        );
        let server = Server::start(
            socket.clone(),
            vec!["/bin/sh".into(), "-c".into(), script],
            stop,
        );

        let (mut reader, initial) = attach(&socket)?;
        fs::write(&emit, b"emit")?;
        wait_file(&emitted)?;

        assert_eq!(
            read_message(&mut reader)?,
            Some(ServerMessage::ClipboardWrite {
                location: session::ClipboardLocation::Standard,
                text: "zellij copy".into(),
            })
        );
        let Some(ServerMessage::Frame(frame)) = read_message(&mut reader)? else {
            return Err("clipboard write was not followed by its frame".into());
        };
        assert!(frame.revision > initial.revision);

        server.finish()?;
        assert_eq!(
            read_message(&mut reader)?,
            Some(ServerMessage::Exited { code: 0 })
        );
        Ok(())
    }

    #[test]
    fn real_pty_synchronized_output_holds_split_large_update() -> TestResult {
        let directory = TestDir::new()?;
        let socket = directory.0.join("orbit.sock");
        let begin = directory.0.join("begin");
        let parsed = directory.0.join("parsed");
        let payload = directory.0.join("payload");
        let emitted = directory.0.join("emitted");
        let end = directory.0.join("end");
        let timeout = directory.0.join("timeout");
        let timeout_held = directory.0.join("timeout-held");
        let stop = directory.0.join("stop");
        let server = Server::start(
            socket.clone(),
            vec![
                "/bin/sh".into(),
                "-c".into(),
                include_str!("../tests/fixtures/synchronized-output.sh").into(),
                "synchronized-output".into(),
                begin.display().to_string(),
                parsed.display().to_string(),
                payload.display().to_string(),
                emitted.display().to_string(),
                end.display().to_string(),
                timeout.display().to_string(),
                timeout_held.display().to_string(),
                stop.display().to_string(),
            ],
            stop,
        );

        let (mut reader, initial) = attach(&socket)?;
        fs::write(&begin, b"begin")?;
        wait_file(&parsed)?;
        expect_no_message(
            &mut reader,
            "synchronized output published an intermediate message",
        )?;

        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(2)))?;
        fs::write(&payload, b"payload")?;
        wait_file(&emitted)?;
        assert_eq!(
            read_message(&mut reader)?,
            Some(ServerMessage::ClipboardWrite {
                location: session::ClipboardLocation::Standard,
                text: "synced copy".into(),
            })
        );
        let scroll = encode_client_message(&ClientMessage::ScrollVertical {
            frame_revision: initial.revision,
            rows: -6,
        })?;
        reader.get_mut().write_all(&scroll)?;
        expect_no_message(&mut reader, "synchronized scroll was not held")?;
        let second = encode_client_message(&ClientMessage::PreviewVertical {
            frame_revision: initial.revision,
            direction: session::VerticalDirection::Up,
        })?;
        reader.get_mut().write_all(&second)?;
        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(2)))?;
        assert!(matches!(
            read_message(&mut reader)?,
            Some(ServerMessage::Failure(session::Failure {
                code: session::FailureCode::InvalidInput,
                ..
            }))
        ));

        fs::write(&end, b"end")?;
        let released = read_message(&mut reader)?;
        let Some(ServerMessage::ScrollOutcome(session::ScrollOutcome::Viewport {
            requested_rows: -6,
            applied_rows: -6,
            frame,
            ..
        })) = released
        else {
            return Err("synchronized output did not finish with one scroll outcome".into());
        };
        assert_eq!(frame.revision, initial.revision + 2);
        assert_eq!(frame.title, "sync-final");
        assert!(cells(&frame).any(|cell| cell.text == "X"));

        fs::write(&timeout, b"timeout")?;
        wait_file(&timeout_held)?;
        let preview = encode_client_message(&ClientMessage::PreviewVertical {
            frame_revision: frame.revision,
            direction: session::VerticalDirection::Up,
        })?;
        reader.get_mut().write_all(&preview)?;
        expect_no_message(&mut reader, "watchdog preview was not held")?;
        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_secs(2)))?;
        let Some(ServerMessage::Frame(timeout_frame)) = read_message(&mut reader)? else {
            return Err("watchdog did not release one complete frame".into());
        };
        assert_eq!(timeout_frame.revision, frame.revision + 1);
        assert_eq!(timeout_frame.title, "sync-timeout");
        assert!(matches!(
            read_message(&mut reader)?,
            Some(ServerMessage::VerticalPreview(session::VerticalPreview {
                frame_revision,
                direction: session::VerticalDirection::Up,
                ..
            })) if frame_revision == timeout_frame.revision
        ));

        server.finish()?;
        assert_eq!(
            read_message(&mut reader)?,
            Some(ServerMessage::Exited { code: 0 })
        );
        Ok(())
    }

    #[test]
    fn conformance_c4_real_pty_reattach_converges_through_complete_ordered_frames() -> TestResult {
        let directory = TestDir::new()?;
        let socket = directory.0.join("orbit.sock");
        let ready = directory.0.join("ready");
        let enrich = directory.0.join("enrich");
        let release = directory.0.join("release");
        let flooding = directory.0.join("flooding");
        let finished = directory.0.join("finished");
        let stop = directory.0.join("stop");
        let primary = "P".repeat(80);
        let server = Server::start(
            socket.clone(),
            vec![
                "/bin/sh".into(),
                "-c".into(),
                include_str!("../tests/fixtures/presentation-conformance.sh").into(),
                "presentation-conformance".into(),
                ready.display().to_string(),
                enrich.display().to_string(),
                release.display().to_string(),
                flooding.display().to_string(),
                finished.display().to_string(),
                stop.display().to_string(),
                primary,
            ],
            stop,
        );

        wait_file(&ready)?;
        let (mut first_reader, mut rich) = attach(&socket)?;
        fs::write(&enrich, b"enrich")?;
        while rich.title != "rich"
            || rich.cursor.viewport.map(|cursor| (cursor.x, cursor.y)) != Some((4, 2))
        {
            let next =
                read_frame(&mut first_reader)?.ok_or("client became busy after attachment")?;
            assert!(next.revision > rich.revision);
            rich = next;
        }
        first_reader
            .get_mut()
            .write_all(&encode_client_message(&ClientMessage::Resize(
                session::SurfaceSize {
                    rows: 30,
                    screen_height: 480,
                    ..runtime::INITIAL_SIZE
                },
            ))?)?;
        while (rich.dimensions.cols, rich.dimensions.rows) != (80, 30) {
            let next =
                read_frame(&mut first_reader)?.ok_or("client became busy after attachment")?;
            assert!(next.revision > rich.revision);
            rich = next;
        }
        assert_eq!(
            (rich.dimensions.cols, rich.dimensions.rows, rich.screen),
            (80, 30, ProtocolScreen::Alternate)
        );
        assert_eq!(rich.title, "rich");
        assert_eq!(rich.working_directory, "file:///tmp/orbit");
        assert_eq!(
            rich.capabilities,
            Capabilities {
                hyperlinks: true,
                kitty_graphics: false,
            }
        );
        assert_eq!(rich.colors.background, Rgb { r: 4, g: 5, b: 6 });
        assert_eq!(rich.colors.foreground, Rgb { r: 1, g: 2, b: 3 });
        assert_eq!(rich.colors.cursor, Some(Rgb { r: 7, g: 8, b: 9 }));
        assert_eq!(
            rich.colors.palette[17],
            Rgb {
                r: 10,
                g: 11,
                b: 12
            }
        );
        assert_eq!(
            rich.cursor,
            Cursor {
                visible: false,
                blinking: false,
                password_input: false,
                shape: CursorShape::Underline,
                viewport: Some(CursorViewport {
                    x: 4,
                    y: 2,
                    at_wide_tail: false,
                }),
            }
        );
        let linked = cell(&rich, 0, 0);
        assert_eq!(linked.text, "A");
        assert_eq!(linked.hyperlink, "https://example.test");
        assert_eq!(
            linked.style,
            CellStyle {
                foreground: StyleColor::Rgb(Rgb {
                    r: 12,
                    g: 34,
                    b: 56,
                }),
                background: StyleColor::Rgb(Rgb {
                    r: 10,
                    g: 11,
                    b: 12,
                }),
                underline_color: StyleColor::Rgb(Rgb { r: 7, g: 8, b: 9 }),
                bold: true,
                italic: true,
                faint: true,
                blink: true,
                inverse: true,
                invisible: true,
                strikethrough: true,
                overline: true,
                selected: false,
                protected: true,
                underline: Underline::Curly,
            }
        );
        assert_eq!(cell(&rich, 1, 0).text, "e\u{301}");
        assert_eq!(cell(&rich, 2, 0).text, "界");
        assert_eq!(cell(&rich, 2, 0).width, CellWidth::Wide);
        assert_eq!(cell(&rich, 3, 0).width, CellWidth::SpacerTail);
        assert_eq!(
            cell(&rich, 0, 3).style.background,
            StyleColor::Rgb(Rgb { r: 5, g: 6, b: 7 })
        );
        drop(first_reader);

        let (slow_client, reattached_rich) = attach(&socket)?;
        assert_eq!(reattached_rich, rich);
        fs::write(&release, b"go")?;
        wait_file(&flooding)?;
        drop(slow_client);
        let (mut interrupt_client, _) = attach(&socket)?;
        interrupt_client
            .get_mut()
            .write_all(&encode_client_message(&ClientMessage::Key(
                session::KeyEvent {
                    action: session::KeyAction::Press,
                    key: session::PhysicalKey::C,
                    modifiers: session::Modifiers::CTRL,
                    consumed_modifiers: session::Modifiers::empty(),
                    composing: false,
                    text: None,
                    unshifted_codepoint: Some('c'),
                },
            ))?)?;
        wait_file(&finished)?;
        drop(interrupt_client);
        let (mut second_reader, mut final_frame) = attach(&socket)?;
        let initial_revision = final_frame.revision;
        while final_frame.title != "final" {
            let next =
                read_frame(&mut second_reader)?.ok_or("client became busy after attachment")?;
            assert!(next.revision > final_frame.revision);
            final_frame = next;
        }
        assert!(initial_revision > rich.revision);
        assert_eq!(
            (
                final_frame.dimensions.cols,
                final_frame.dimensions.rows,
                final_frame.screen,
            ),
            (80, 30, ProtocolScreen::Primary)
        );
        assert_eq!(final_frame.title, "final");
        assert_eq!(final_frame.working_directory, "file:///tmp/orbit");
        assert_eq!(cell(&final_frame, 0, 0).text, "P");
        assert_eq!(cell(&final_frame, 79, 0).text, "P");
        assert_eq!(cell(&final_frame, 0, 1).text, "X");
        assert!(final_frame.rows[0].wrapped);
        assert!(cells(&final_frame).any(|cell| cell.text == "S"));
        assert!(cells(&final_frame).any(|cell| cell.text == "é"));

        server.finish()
    }
}
