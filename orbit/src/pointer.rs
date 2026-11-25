//! Left-pointer sequences and host selection (ORB-C9). Bend decides each
//! sequence's route and answers; this module carries out a host phase through
//! libghostty's tracked gesture APIs, freezes the copied text, keeps the
//! selection through compatible PTY output, and clears gesture state.

use crate::errno;
use libghostty_vt::{
    Terminal,
    fmt::{Format, Formatter},
    point::{Coordinate, Point},
    selection::gesture::{
        Autoscroll, AutoscrollTickEvent, Behavior, Behaviors, DragEvent, Geometry, Gesture,
        PressEvent, ReleaseEvent,
    },
};
use orbit_protocol::session::{
    self, SelectionAction, SelectionPosition, SurfaceSize, VerticalDirection,
};
use std::{io, time::Duration};

/// The click repetition interval.
const CLICK_REPEAT_INTERVAL: Duration = Duration::from_millis(500);

/// What delimits a space-delimited span: ASCII space and tab, besides
/// unwritten cells and hard line breaks.
const SPAN_BOUNDARIES: [char; 2] = [' ', '\t'];

/// libghostty's gesture and its events. The click ladder selects
/// cells, words, space-delimited spans and logical lines: libghostty counts
/// at most three clicks, so the third press uses word events with only space
/// and tab as boundaries, and a fourth or later press its native line
/// behavior.
struct Gestures {
    gesture: Gesture<'static>,
    press: PressEvent<'static>,
    drag: DragEvent<'static>,
    release: ReleaseEvent<'static>,
    span_press: PressEvent<'static>,
    span_drag: DragEvent<'static>,
    autoscroll: AutoscrollTickEvent<'static>,
    span_autoscroll: AutoscrollTickEvent<'static>,
}

impl Gestures {
    fn new() -> libghostty_vt::error::Result<Self> {
        let mut span_press = PressEvent::new()?;
        span_press
            .set_behaviors(&Behaviors::new().with_triple_click_behavior(Behavior::Word))?
            .set_word_boundary_codepoints(&SPAN_BOUNDARIES)?;
        let mut span_drag = DragEvent::new()?;
        span_drag.set_word_boundary_codepoints(&SPAN_BOUNDARIES)?;
        let mut span_autoscroll = AutoscrollTickEvent::new()?;
        span_autoscroll.set_word_boundary_codepoints(&SPAN_BOUNDARIES)?;
        Ok(Self {
            gesture: Gesture::new()?,
            press: PressEvent::new()?,
            drag: DragEvent::new()?,
            release: ReleaseEvent::new()?,
            span_press,
            span_drag,
            autoscroll: AutoscrollTickEvent::new()?,
            span_autoscroll,
        })
    }

    fn is_span(&self, terminal: &Terminal<'static, 'static>) -> libghostty_vt::error::Result<bool> {
        Ok(self.gesture.click_count(terminal)? == 3
            && self.gesture.behavior(terminal)? == Behavior::Word)
    }
}

/// libghostty's gesture, the text the last finished host sequence froze,
/// and whether PTY output left the gesture without an anchor since it was
/// last cleared. Bend's session holds the route and whether a selection is
/// presented or a copy exists.
#[derive(Default)]
pub(crate) struct Pointer {
    gestures: Option<Gestures>,
    pub(crate) copied: String,
    pub(crate) unanchored: bool,
}

/// Why a host phase failed; the sequence is cleared either way.
#[derive(Debug)]
pub(crate) struct PhaseFailed;

impl Pointer {
    /// Applies one host phase at its position, which must lie in the
    /// viewport, and reports whether a selection is presented after it.
    /// Finish releases the gesture and freezes the selected text, at most one
    /// maximum copy, for both clipboards.
    pub(crate) fn host_phase(
        &mut self,
        action: SelectionAction,
        terminal: &Terminal<'static, 'static>,
        size: SurfaceSize,
    ) -> Result<bool, PhaseFailed> {
        let applied = self.apply_host_phase(action, terminal, size);
        if applied.is_err() {
            let _ = self.clear(terminal);
        }
        applied
    }

    fn apply_host_phase(
        &mut self,
        action: SelectionAction,
        terminal: &Terminal<'static, 'static>,
        size: SurfaceSize,
    ) -> Result<bool, PhaseFailed> {
        let position = phase_position(action).ok_or(PhaseFailed)?;
        let point = viewport_point(position, size).ok_or(PhaseFailed)?;
        let grid_ref = terminal
            .grid_ref(Point::Viewport(point))
            .map_err(|_| PhaseFailed)?;
        let gestures = match &mut self.gestures {
            Some(gestures) => gestures,
            None => self
                .gestures
                .insert(Gestures::new().map_err(|_| PhaseFailed)?),
        };
        let (x, y) = (f64::from(position.x), f64::from(position.y));
        let selected = match action {
            SelectionAction::Begin { time_ns, .. } => {
                // A press after two clicks begins a span; libghostty still
                // decides whether it repeats them.
                let press = if matches!(gestures.gesture.click_count(terminal), Ok(2)) {
                    &mut gestures.span_press
                } else {
                    &mut gestures.press
                };
                press
                    .set_position(x, y)
                    .and_then(|press| press.set_time(Duration::from_nanos(time_ns)))
                    .and_then(|press| press.set_repeat_distance(f64::from(size.cell_width)))
                    .and_then(|press| press.set_repeat_interval(CLICK_REPEAT_INTERVAL))
                    .and_then(|press| press.apply(&mut gestures.gesture, terminal, grid_ref))
            }
            _ => {
                let drag = if gestures.is_span(terminal).map_err(|_| PhaseFailed)? {
                    &mut gestures.span_drag
                } else {
                    &mut gestures.drag
                };
                drag.set_position(x, y).and_then(|drag| {
                    drag.apply(&mut gestures.gesture, terminal, grid_ref, geometry(size))
                })
            }
        }
        .map_err(|_| PhaseFailed)?;
        terminal.set_selection(selected).map_err(|_| PhaseFailed)?;
        let presented = selected.is_some();
        if let SelectionAction::Finish { .. } = action {
            gestures
                .release
                .apply(&mut gestures.gesture, terminal, Some(grid_ref))
                .map_err(|_| PhaseFailed)?;
            if presented {
                self.copied = format_selection(terminal).ok_or(PhaseFailed)?;
            }
        }
        Ok(presented)
    }

    /// Clears the host selection and resets the gesture.
    pub(crate) fn clear(&mut self, terminal: &Terminal<'static, 'static>) -> io::Result<()> {
        terminal.set_selection(None).map_err(|_| errno(libc::EIO))?;
        if let Some(gestures) = &mut self.gestures {
            gestures.gesture.reset(terminal);
        }
        self.unanchored = false;
        Ok(())
    }

    /// Whether the held gesture's anchor is still valid.
    pub(crate) fn anchored(&self, terminal: &Terminal<'static, 'static>) -> io::Result<bool> {
        match &self.gestures {
            Some(gestures) => Ok(gestures
                .gesture
                .anchor(terminal)
                .map_err(|_| errno(libc::EIO))?
                .is_some()),
            None => Ok(false),
        }
    }

    /// Moves the viewport and extends the held gesture in the requested
    /// direction. Returns whether it presents a selection after the tick.
    pub(crate) fn tick(
        &mut self,
        terminal: &Terminal<'static, 'static>,
        direction: VerticalDirection,
        position: SelectionPosition,
        size: SurfaceSize,
    ) -> Result<bool, PhaseFailed> {
        let ticked = self.apply_tick(terminal, direction, position, size);
        if ticked.is_err() {
            let _ = self.clear(terminal);
        }
        ticked
    }

    fn apply_tick(
        &mut self,
        terminal: &Terminal<'static, 'static>,
        direction: VerticalDirection,
        position: SelectionPosition,
        size: SurfaceSize,
    ) -> Result<bool, PhaseFailed> {
        let point = selection_tick_point(direction, position, size).ok_or(PhaseFailed)?;
        let gestures = self.gestures.as_mut().ok_or(PhaseFailed)?;
        let grid_ref = terminal
            .grid_ref(Point::Viewport(point))
            .map_err(|_| PhaseFailed)?;
        let span = gestures.is_span(terminal).map_err(|_| PhaseFailed)?;
        let (drag, autoscroll) = if span {
            (&mut gestures.span_drag, &mut gestures.span_autoscroll)
        } else {
            (&mut gestures.drag, &mut gestures.autoscroll)
        };
        let x = f64::from(position.x);
        // libghostty activates at the surface edge, beyond any grid padding.
        let (y, expected) = match direction {
            VerticalDirection::Up => (0.0, Autoscroll::Up),
            VerticalDirection::Down => (f64::from(size.screen_height) + 1.0, Autoscroll::Down),
        };
        drag.set_position(x, y)
            .and_then(|drag| drag.apply(&mut gestures.gesture, terminal, grid_ref, geometry(size)))
            .map_err(|_| PhaseFailed)?;
        if gestures
            .gesture
            .autoscroll(terminal)
            .map_err(|_| PhaseFailed)?
            != expected
        {
            return Err(PhaseFailed);
        }
        let selected = autoscroll
            .set_position(x, y)
            .and_then(|autoscroll| {
                autoscroll.apply(&mut gestures.gesture, terminal, point, geometry(size))
            })
            .map_err(|_| PhaseFailed)?;
        terminal.set_selection(selected).map_err(|_| PhaseFailed)?;
        Ok(selected.is_some())
    }

    /// Whether PTY output must be checked against the gesture, which holds
    /// any host sequence or selection once a press has counted a click, and
    /// if so whether the terminal tracked the mouse before it.
    pub(crate) fn watch(&self, terminal: &Terminal<'static, 'static>) -> io::Result<Option<bool>> {
        let failed = |_| errno(libc::EIO);
        let clicked = match &self.gestures {
            Some(gestures) => gestures.gesture.click_count(terminal).map_err(failed)? != 0,
            None => false,
        };
        if clicked {
            terminal.is_mouse_tracking().map(Some).map_err(failed)
        } else {
            Ok(None)
        }
    }

    /// After watched PTY output, notes output that newly enabled mouse
    /// tracking or left the gesture without a valid anchor, for Bend to
    /// decide on; compatible output keeps the gesture, its click count and
    /// the tracked selection as content moves.
    pub(crate) fn after_output(
        &mut self,
        terminal: &Terminal<'static, 'static>,
        tracked_before: bool,
    ) -> io::Result<()> {
        let failed = |_| errno(libc::EIO);
        let anchored = match &self.gestures {
            Some(gestures) => gestures.gesture.anchor(terminal).map_err(failed)?.is_some(),
            None => false,
        };
        if !anchored || !tracked_before && terminal.is_mouse_tracking().map_err(failed)? {
            self.unanchored = true;
        }
        Ok(())
    }
}

pub(crate) fn phase_position(action: SelectionAction) -> Option<SelectionPosition> {
    match action {
        SelectionAction::Begin { position, .. }
        | SelectionAction::Update { position, .. }
        | SelectionAction::Finish { position, .. } => Some(position),
        _ => None,
    }
}

fn geometry(size: SurfaceSize) -> Geometry {
    Geometry {
        columns: u32::from(size.cols),
        cell_width: size.cell_width,
        padding_left: size.padding_left,
        screen_height: size.screen_height,
    }
}

pub(crate) fn selection_tick_point(
    direction: VerticalDirection,
    position: SelectionPosition,
    size: SurfaceSize,
) -> Option<Coordinate> {
    let point = viewport_point(position, size)?;
    let row = match direction {
        VerticalDirection::Up => 0,
        VerticalDirection::Down => u32::from(size.rows) - 1,
    };
    (point.y == row).then_some(point)
}

/// The viewport cell under a surface position, or None outside the grid.
pub(crate) fn viewport_point(position: SelectionPosition, size: SurfaceSize) -> Option<Coordinate> {
    let x = f64::from(position.x) - f64::from(size.padding_left);
    let y = f64::from(position.y) - f64::from(size.padding_top);
    let grid_width = f64::from(size.cols) * f64::from(size.cell_width);
    let grid_height = f64::from(size.rows) * f64::from(size.cell_height);
    (x >= 0.0 && x < grid_width && y >= 0.0 && y < grid_height).then_some(Coordinate {
        x: (x / f64::from(size.cell_width)) as u16,
        y: (y / f64::from(size.cell_height)) as u32,
    })
}

/// The selection as plain unwrapped, trimmed text, or None if it is empty to
/// libghostty or longer than one maximum copy.
pub(crate) fn format_selection(terminal: &Terminal<'static, 'static>) -> Option<String> {
    let selection = terminal.selection().ok()??;
    let mut formatter = Formatter::new(terminal).ok()?;
    formatter
        .set_selection(Some(selection))
        .ok()?
        .set_format(Format::Plain)
        .ok()?
        .set_unwrap(true)
        .ok()?
        .set_trim(true)
        .ok()?;
    let required = formatter.format_len().ok()?;
    if required > session::MAX_COPY_BYTES {
        return None;
    }
    let mut bytes = vec![0; required];
    let written = formatter.format_buf(&mut bytes).ok()?;
    bytes.truncate(written);
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameExtractor;

    fn terminal(cols: u16, rows: u16) -> Terminal<'static, 'static> {
        let mut terminal = Terminal::new(cols, rows).unwrap();
        terminal.set_scrollback_max_bytes(Some(4096)).unwrap();
        terminal.resize(cols, rows, 10, 18).unwrap();
        terminal
    }

    fn size(cols: u16, rows: u16) -> SurfaceSize {
        SurfaceSize {
            cols,
            rows,
            screen_width: u32::from(cols) * 10,
            screen_height: u32::from(rows) * 18,
            cell_width: 10,
            cell_height: 18,
            padding_top: 0,
            padding_bottom: 0,
            padding_left: 0,
            padding_right: 0,
        }
    }

    fn at(col: u16, row: u16) -> SelectionPosition {
        SelectionPosition {
            x: f32::from(col) * 10.0 + 5.0,
            y: f32::from(row) * 18.0 + 9.0,
        }
    }

    fn click(
        pointer: &mut Pointer,
        terminal: &mut Terminal<'static, 'static>,
        size: SurfaceSize,
        position: SelectionPosition,
        time_ns: u64,
        between: &[u8],
    ) -> Option<String> {
        let modifiers = session::Modifiers::empty();
        pointer
            .host_phase(
                SelectionAction::Begin {
                    frame_revision: 0,
                    position,
                    time_ns,
                    modifiers,
                },
                terminal,
                size,
            )
            .unwrap();
        // Output between press and release passes through the watch.
        let watched = pointer.watch(terminal).unwrap();
        terminal.vt_write(between);
        if let Some(tracked) = watched {
            pointer.after_output(terminal, tracked).unwrap();
        }
        assert!(!pointer.unanchored);
        pointer
            .host_phase(
                SelectionAction::Finish {
                    position,
                    modifiers,
                },
                terminal,
                size,
            )
            .unwrap()
            .then(|| pointer.copied.clone())
    }

    #[test]
    fn the_click_ladder_selects_cells_words_spans_and_lines_across_wraps() {
        let mut terminal = terminal(16, 4);
        let size = size(16, 4);
        terminal.vt_write(b"prefix alpha:beta/gamma suffix");
        let mut pointer = Pointer::default();
        let mut extractor = FrameExtractor::new(false).unwrap();
        let line = "prefix alpha:beta/gamma suffix";
        for (time_ns, expected, highlighted) in [
            (1_000_000_000, None, 0..0),
            (1_100_000_000, Some("beta/gamma"), 13..23),
            (1_200_000_000, Some("alpha:beta/gamma"), 7..23),
            (1_300_000_000, Some(line), 0..30),
            (1_400_000_000, Some(line), 0..30),
            // A late press after the saturated sequence starts over.
            (2_000_000_000, None, 0..0),
            (2_100_000_000, Some("beta/gamma"), 13..23),
        ] {
            let copied = click(
                &mut pointer,
                &mut terminal,
                size,
                at(14, 0),
                time_ns,
                b"\x1b[4;1Hlive",
            );
            assert_eq!(copied.as_deref(), expected, "time {time_ns}");
            // Exactly the selected text's cells, counted across the wrap, are highlighted.
            let frame = extractor.frame(0, &terminal).unwrap();
            assert!(frame.rows[0].wrapped);
            for (index, cell) in frame.rows.iter().flat_map(|row| &row.cells).enumerate() {
                assert_eq!(
                    cell.style.selected,
                    highlighted.contains(&index),
                    "cell {index}, time {time_ns}"
                );
            }
        }
    }

    /// Spans end only at ASCII space and tab runs, unwritten cells and hard
    /// line breaks: other punctuation, a non-breaking space and combining
    /// marks stay inside, and a click on a space run selects it, trimmed to
    /// nothing in the copy (ORB-C9).
    #[test]
    fn spans_end_only_at_ascii_space_tab_and_unwritten_cells() {
        let mut terminal = terminal(12, 4);
        let size = size(12, 4);
        let mut pointer = Pointer::default();
        for (text, column, expected) in [
            ("a::b,,c", 1, Some("a::b,,c")),
            ("a  b", 1, Some("")),
            ("a\tb", 8, Some("b")),
            ("a\tb", 2, None),
            ("a\u{a0}b", 1, Some("a\u{a0}b")),
            ("a;b,e\u{301}", 0, Some("a;b,e\u{301}")),
        ] {
            pointer.clear(&terminal).unwrap();
            terminal.vt_write(b"\x1b[2J\x1b[H");
            terminal.vt_write(text.as_bytes());
            let mut copied = None;
            for time_ns in [1_000_000_000, 1_100_000_000, 1_200_000_000] {
                copied = click(
                    &mut pointer,
                    &mut terminal,
                    size,
                    at(column, 0),
                    time_ns,
                    b"",
                );
            }
            assert_eq!(copied.as_deref(), expected, "{text:?}");
        }
    }

    /// A drag back to the press cell clears the selection it had made, and
    /// its highlight (ORB-C9).
    #[test]
    fn a_drag_back_to_the_press_cell_clears_the_selection() {
        let mut terminal = terminal(12, 4);
        let size = size(12, 4);
        terminal.vt_write(b"alpha beta");
        let mut pointer = Pointer::default();
        let mut extractor = FrameExtractor::new(false).unwrap();
        let modifiers = session::Modifiers::empty();
        // Whether the phase presents a selection, and how many cells it highlights.
        let mut phase = |action| {
            let presented = pointer.host_phase(action, &terminal, size).unwrap();
            let frame = extractor.frame(0, &terminal).unwrap();
            let cells = frame.rows.iter().flat_map(|row| &row.cells);
            (presented, cells.filter(|cell| cell.style.selected).count())
        };
        let begin = SelectionAction::Begin {
            frame_revision: 0,
            position: at(0, 0),
            time_ns: 1,
            modifiers,
        };
        assert_eq!(phase(begin), (false, 0));
        let away = SelectionAction::Update {
            position: at(5, 0),
            modifiers,
        };
        assert_eq!(phase(away), (true, 5));
        let back = SelectionAction::Update {
            position: at(0, 0),
            modifiers,
        };
        assert_eq!(phase(back), (false, 0));
        let finish = SelectionAction::Finish {
            position: at(0, 0),
            modifiers,
        };
        assert_eq!(phase(finish), (false, 0));
    }

    /// A held gesture survives output that scrolls its text into history and
    /// its release copies from there (ORB-C9).
    #[test]
    fn a_held_gesture_survives_output_that_scrolls_it_into_history() {
        let mut terminal = terminal(12, 4);
        let size = size(12, 4);
        terminal.vt_write(b"alpha beta\r\nsecond\r\nthird\r\nfourth");
        let mut pointer = Pointer::default();
        let modifiers = session::Modifiers::empty();
        pointer
            .host_phase(
                SelectionAction::Begin {
                    frame_revision: 0,
                    position: at(0, 0),
                    time_ns: 1,
                    modifiers,
                },
                &terminal,
                size,
            )
            .unwrap();
        pointer
            .host_phase(
                SelectionAction::Update {
                    position: at(5, 0),
                    modifiers,
                },
                &terminal,
                size,
            )
            .unwrap();
        let tracked = pointer.watch(&terminal).unwrap().unwrap();
        terminal.vt_write(b"\r\nlive");
        pointer.after_output(&terminal, tracked).unwrap();
        assert!(!pointer.unanchored);
        assert_eq!(terminal.scrollbar().unwrap().total, 5);
        assert_eq!(format_selection(&terminal).as_deref(), Some("alpha"));
        // Row 0 now shows "second".
        assert!(
            pointer
                .host_phase(
                    SelectionAction::Finish {
                        position: at(6, 0),
                        modifiers,
                    },
                    &terminal,
                    size,
                )
                .unwrap()
        );
        assert_eq!(pointer.copied, "alpha beta\nsecond");
    }

    #[test]
    fn upward_ticks_extend_cells_spans_and_lines_with_unicode_copy() {
        assert_tick_copy(VerticalDirection::Up);
    }

    #[test]
    fn downward_tick_extends_a_held_selection() {
        assert_tick_copy(VerticalDirection::Down);
    }

    fn padded_size(rows: u16) -> SurfaceSize {
        let size = size(12, rows);
        SurfaceSize {
            screen_height: size.screen_height + 10,
            padding_top: 3,
            padding_bottom: 7,
            ..size
        }
    }

    fn assert_tick_copy(direction: VerticalDirection) {
        use libghostty_vt::terminal::ScrollViewport;
        let lines: Vec<_> = (0..16).map(|row| format!("L{row:02}:part 界")).collect();
        let up = direction == VerticalDirection::Up;
        let (mut press, mut edge) = if up {
            (at(0, 2), at(5, 0))
        } else {
            (
                at(1, 1),
                SelectionPosition {
                    x: 97.0,
                    ..at(9, 3)
                },
            )
        };
        let size = padded_size(4);
        press.y += size.padding_top as f32;
        edge.y += size.padding_top as f32;
        for clicks in [1, 3, 4] {
            let mut terminal = terminal(12, 4);
            terminal.vt_write(lines.join("\r\n").as_bytes());
            if !up {
                terminal.scroll_viewport(ScrollViewport::Top).unwrap();
            }
            let mut pointer = Pointer::default();
            let modifiers = session::Modifiers::empty();
            for click in 1..=clicks {
                let begin = SelectionAction::Begin {
                    frame_revision: 0,
                    position: press,
                    time_ns: click * 100_000_000,
                    modifiers,
                };
                pointer.host_phase(begin, &terminal, size).unwrap();
                if click < clicks {
                    let finish = SelectionAction::Finish {
                        position: press,
                        modifiers,
                    };
                    pointer.host_phase(finish, &terminal, size).unwrap();
                }
            }
            let update = SelectionAction::Update {
                position: edge,
                modifiers,
            };
            pointer.host_phase(update, &terminal, size).unwrap();
            for moved in 1..=13 {
                assert!(pointer.tick(&terminal, direction, edge, size).unwrap());
                let expected = if up {
                    12_u64.saturating_sub(moved)
                } else {
                    moved.min(12)
                };
                assert_eq!(terminal.scrollbar().unwrap().offset, expected);
            }
            let finish = SelectionAction::Finish {
                position: edge,
                modifiers,
            };
            assert!(pointer.host_phase(finish, &terminal, size).unwrap());
            let expected = if up {
                let first = if clicks == 1 { "art 界" } else { &lines[0] };
                let mut expected = format!("{first}\n{}", lines[1..14].join("\n"));
                match clicks {
                    3 => expected.push_str("\nL14:part"),
                    4 => expected.push_str("\nL14:part 界"),
                    _ => {}
                }
                expected
            } else {
                let first = if clicks == 1 {
                    &lines[1][1..]
                } else {
                    &lines[1]
                };
                format!("{first}\n{}", lines[2..].join("\n"))
            };
            assert_eq!(pointer.copied, expected, "{direction:?}, {clicks} clicks");
        }
    }

    #[test]
    fn a_single_row_can_tick_in_both_directions() {
        let mut terminal = terminal(12, 1);
        let size = padded_size(1);
        terminal.vt_write(b"old\r\nlive");
        let position = SelectionPosition { x: 5.0, y: 12.0 };
        let mut pointer = Pointer::default();
        pointer
            .host_phase(
                SelectionAction::Begin {
                    frame_revision: 0,
                    position,
                    time_ns: 0,
                    modifiers: session::Modifiers::empty(),
                },
                &terminal,
                size,
            )
            .unwrap();

        for (direction, offset) in [(VerticalDirection::Up, 0), (VerticalDirection::Down, 1)] {
            pointer.tick(&terminal, direction, position, size).unwrap();
            assert_eq!(terminal.scrollbar().unwrap().offset, offset);
        }
    }

    #[test]
    fn output_that_enables_mouse_tracking_unanchors_the_host_selection() {
        let mut terminal = terminal(16, 4);
        let size = size(16, 4);
        terminal.vt_write(b"hello world");
        let mut pointer = Pointer::default();
        let modifiers = session::Modifiers::empty();
        pointer
            .host_phase(
                SelectionAction::Begin {
                    frame_revision: 0,
                    position: at(0, 0),
                    time_ns: 1,
                    modifiers,
                },
                &terminal,
                size,
            )
            .unwrap();
        let presented = pointer
            .host_phase(
                SelectionAction::Update {
                    position: at(4, 0),
                    modifiers,
                },
                &terminal,
                size,
            )
            .unwrap();
        assert!(presented);
        let tracked = pointer.watch(&terminal).unwrap().unwrap();
        terminal.vt_write(b"\x1b[?1000h");
        pointer.after_output(&terminal, tracked).unwrap();
        assert!(pointer.unanchored);
        pointer.clear(&terminal).unwrap();
        assert!(!pointer.unanchored);
        assert_eq!(pointer.watch(&terminal).unwrap(), None);
    }
}
