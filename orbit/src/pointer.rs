//! Left-pointer sequences and host selection (ORB-C9). Bend decides each
//! sequence's route and answers; this module carries out a host phase through
//! libghostty's tracked gesture APIs, freezes the copied text, keeps the
//! selection through compatible PTY output, and clears gesture state, as
//! the old Orbit does.

use crate::errno;
use libghostty_vt::{
    Terminal,
    fmt::Format,
    selection::{
        FormatOptions,
        gesture::{
            Autoscroll, AutoscrollTickEvent, Behavior, Behaviors, DragEvent, Geometry, Gesture,
            PressEvent, ReleaseEvent,
        },
    },
    terminal::{Point, PointCoordinate},
};
use orbit_protocol::session::{self, SelectionAction, SelectionPosition, SurfaceSize};
use std::{io, time::Duration};

/// The old Orbit's click repetition interval.
const CLICK_REPEAT_INTERVAL: Duration = Duration::from_millis(500);

/// What delimits a space-delimited span: ASCII space and tab, besides
/// unwritten cells and hard line breaks.
const SPAN_BOUNDARIES: [char; 2] = [' ', '\t'];

/// libghostty's gesture and its events. The old Orbit's click ladder selects
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
                    .and_then(|press| {
                        press.apply(&mut gestures.gesture, terminal, grid_ref.clone())
                    })
            }
            _ => {
                let drag = if gestures.is_span(terminal).map_err(|_| PhaseFailed)? {
                    &mut gestures.span_drag
                } else {
                    &mut gestures.drag
                };
                drag.set_position(x, y).and_then(|drag| {
                    drag.apply(
                        &mut gestures.gesture,
                        terminal,
                        grid_ref.clone(),
                        geometry(size),
                    )
                })
            }
        }
        .map_err(|_| PhaseFailed)?;
        terminal
            .set_selection(selected.as_ref())
            .map_err(|_| PhaseFailed)?;
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

    /// Extends the held host gesture one row toward older history from a
    /// top-row position, as the old Orbit does: a drag to the top edge
    /// enters upward autoscroll, and one autoscroll tick moves the viewport
    /// and the tracked selection. Reports whether a selection is presented
    /// after it; the gesture and selection are cleared if it fails.
    pub(crate) fn tick(
        &mut self,
        terminal: &Terminal<'static, 'static>,
        position: SelectionPosition,
        size: SurfaceSize,
    ) -> Result<bool, PhaseFailed> {
        let ticked = self.apply_tick(terminal, position, size);
        if ticked.is_err() {
            let _ = self.clear(terminal);
        }
        ticked
    }

    fn apply_tick(
        &mut self,
        terminal: &Terminal<'static, 'static>,
        position: SelectionPosition,
        size: SurfaceSize,
    ) -> Result<bool, PhaseFailed> {
        let point = viewport_point(position, size)
            .filter(|point| point.y == 0)
            .ok_or(PhaseFailed)?;
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
        drag.set_position(x, 0.0)
            .and_then(|drag| drag.apply(&mut gestures.gesture, terminal, grid_ref, geometry(size)))
            .map_err(|_| PhaseFailed)?;
        if gestures
            .gesture
            .autoscroll(terminal)
            .map_err(|_| PhaseFailed)?
            != Autoscroll::Up
        {
            return Err(PhaseFailed);
        }
        let selected = autoscroll
            .set_position(x, 0.0)
            .and_then(|autoscroll| {
                autoscroll.apply(&mut gestures.gesture, terminal, point, geometry(size))
            })
            .map_err(|_| PhaseFailed)?;
        terminal
            .set_selection(selected.as_ref())
            .map_err(|_| PhaseFailed)?;
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

/// The viewport cell under a surface position, or None outside the grid.
pub(crate) fn viewport_point(
    position: SelectionPosition,
    size: SurfaceSize,
) -> Option<PointCoordinate> {
    let x = f64::from(position.x) - f64::from(size.padding_left);
    let y = f64::from(position.y) - f64::from(size.padding_top);
    let grid_width = f64::from(size.cols) * f64::from(size.cell_width);
    let grid_height = f64::from(size.rows) * f64::from(size.cell_height);
    (x >= 0.0 && x < grid_width && y >= 0.0 && y < grid_height).then_some(PointCoordinate {
        x: (x / f64::from(size.cell_width)) as u16,
        y: (y / f64::from(size.cell_height)) as u32,
    })
}

/// The selection as plain unwrapped, trimmed text, or None if it is empty to
/// libghostty or longer than one maximum copy.
fn format_selection(terminal: &Terminal<'static, 'static>) -> Option<String> {
    let options = || {
        FormatOptions::new()
            .with_emit_format(Format::Plain)
            .with_unwrap(true)
            .with_trim(true)
    };
    let required = match terminal.format_selection_buf(options(), &mut []) {
        Ok(Some(written)) => written,
        Err(libghostty_vt::error::Error::OutOfSpace { required }) => required,
        _ => return None,
    };
    if required > session::MAX_COPY_BYTES {
        return None;
    }
    let mut bytes = vec![0; required];
    let written = terminal
        .format_selection_buf(options(), &mut bytes)
        .ok()??;
    bytes.truncate(written);
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use libghostty_vt::TerminalOptions;

    fn terminal(cols: u16, rows: u16) -> Terminal<'static, 'static> {
        let mut terminal = Terminal::new(TerminalOptions {
            cols,
            rows,
            max_scrollback: 0,
        })
        .unwrap();
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
        let line = "prefix alpha:beta/gamma suffix";
        for (time_ns, expected) in [
            (1_000_000_000, None),
            (1_100_000_000, Some("beta/gamma")),
            (1_200_000_000, Some("alpha:beta/gamma")),
            (1_300_000_000, Some(line)),
            (1_400_000_000, Some(line)),
            // A late press after the saturated sequence starts over.
            (2_000_000_000, None),
            (2_100_000_000, Some("beta/gamma")),
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
