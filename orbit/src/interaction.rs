use crate::{
    Result,
    attachment::Client,
    platform::Pty,
    runtime::{Presentation, can_queue_pty_write, next_revision, queue_pty_write},
};
use libghostty_vt::{
    Terminal,
    fmt::Format,
    focus,
    key::{
        Action as GhosttyKeyAction, Encoder as KeyEncoder, Event as GhosttyKeyEvent, Key,
        Mods as GhosttyModifiers,
    },
    mouse::{
        Action as GhosttyMouseAction, Button as GhosttyMouseButton, Encoder as MouseEncoder,
        EncoderSize as MouseEncoderSize, Event as GhosttyMouseEvent, Position as MousePosition,
    },
    paste,
    screen::{GridRef, Screen},
    selection::{
        FormatOptions,
        gesture::{
            Autoscroll, AutoscrollTickEvent, Behavior, Behaviors, DragEvent, Geometry, Gesture,
            PressEvent, ReleaseEvent,
        },
    },
    terminal::{Mode, Point, PointCoordinate, ScrollViewport},
};
use orbit_protocol::{
    Error as ProtocolError, FrameSize,
    session::{
        self, ClientMessage, FailureCode, FocusEvent, KeyAction, KeyEvent, Modifiers, MouseAction,
        MouseButton, PhysicalKey, PreviewOutcome, SelectionAction, SelectionPosition,
        ServerMessage, SurfaceSize, VerticalDirection, VerticalPreview, WheelOutcome,
    },
};
use std::{cell::RefCell, collections::VecDeque, time::Duration};

const CLICK_REPEAT_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Default)]
pub(crate) struct SelectionState {
    gesture: Option<GestureState>,
    route: Option<PointerRoute>,
    copied: Option<String>,
    has_selection: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerRoute {
    Host,
    Terminal,
}

#[derive(Debug)]
struct GestureState {
    gesture: Gesture<'static>,
    press: PressEvent<'static>,
    drag: DragEvent<'static>,
    autoscroll: AutoscrollTickEvent<'static>,
    release: ReleaseEvent<'static>,
    span_press: PressEvent<'static>,
    span_drag: DragEvent<'static>,
    span_autoscroll: AutoscrollTickEvent<'static>,
}

impl GestureState {
    fn new() -> Result<Self> {
        let mut span_press = PressEvent::new()?;
        span_press
            .set_behaviors(&Behaviors::new().with_triple_click_behavior(Behavior::Word))?
            .set_word_boundary_codepoints(&[' ', '\t'])?;
        let mut span_drag = DragEvent::new()?;
        span_drag.set_word_boundary_codepoints(&[' ', '\t'])?;
        let mut span_autoscroll = AutoscrollTickEvent::new()?;
        span_autoscroll.set_word_boundary_codepoints(&[' ', '\t'])?;
        Ok(Self {
            gesture: Gesture::new()?,
            press: PressEvent::new()?,
            drag: DragEvent::new()?,
            autoscroll: AutoscrollTickEvent::new()?,
            release: ReleaseEvent::new()?,
            span_press,
            span_drag,
            span_autoscroll,
        })
    }

    fn is_span(&self, terminal: &Terminal<'_, '_>) -> Result<bool> {
        Ok(self.gesture.click_count(terminal)? == 3
            && self.gesture.behavior(terminal)? == Behavior::Word)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_client_message(
    client: &mut Client,
    message: ClientMessage,
    terminal: &mut Terminal<'static, '_>,
    pty: Option<&Pty>,
    size: &mut SurfaceSize,
    writes: &RefCell<VecDeque<u8>>,
    presentation: &mut Presentation,
    selection: &mut SelectionState,
) -> Result<bool> {
    let Some(pty) = pty else {
        return client.fail(FailureCode::Terminal, "PTY is closed".into());
    };
    if let ClientMessage::PreviewVertical {
        frame_revision,
        direction,
    } = message
    {
        return handle_vertical_preview(client, frame_revision, direction, terminal, presentation);
    }
    if let ClientMessage::ScrollVertical {
        frame_revision,
        rows,
    } = message
    {
        return handle_vertical_scroll(
            client,
            frame_revision,
            rows,
            terminal,
            presentation,
            selection,
        );
    }
    if let ClientMessage::ReturnToLive = message {
        return handle_return_to_live(client, terminal, presentation, selection);
    }
    if let ClientMessage::Selection(action) = message {
        return handle_selection(
            client,
            action,
            terminal,
            *size,
            writes,
            presentation,
            selection,
        );
    }
    let message = match message {
        ClientMessage::Resize(surface) => {
            clear_pointer_sequence(terminal, selection, false)?;
            pty.resize(surface)?;
            terminal.resize(
                surface.cols,
                surface.rows,
                surface.cell_width,
                surface.cell_height,
            )?;
            *size = surface;
            presentation.advance()?;
            if !client.push_message(&ServerMessage::Accepted)? {
                return Ok(false);
            }
            return presentation.publish(Some(client), terminal);
        }
        message => message,
    };
    let return_live = matches!(&message, ClientMessage::Key(_));
    if let ClientMessage::Mouse(input) = &message
        && let Some(direction) = vertical_wheel_direction(input.button)
    {
        return handle_vertical_wheel(
            client,
            *input,
            direction,
            terminal,
            *size,
            writes,
            presentation,
            selection,
        );
    }
    let encoded = match encode_input(terminal, *size, message) {
        Ok(encoded) => encoded,
        Err(error) => {
            return client.fail(FailureCode::Terminal, error.to_string());
        }
    };
    if !queue_pty_write(&mut writes.borrow_mut(), &encoded) {
        let queued = client.fail(FailureCode::Terminal, "PTY input queue is full".into())?;
        client.close_when_flushed();
        return Ok(queued);
    }
    let returned_live =
        if return_live && !encoded.is_empty() && terminal.active_screen()? == Screen::Primary {
            let scrollbar = terminal.scrollbar()?;
            if scrollbar.offset.saturating_add(scrollbar.len) < scrollbar.total {
                clear_selection(terminal, selection, false)?;
                terminal.scroll_viewport(ScrollViewport::Bottom);
                presentation.advance()?;
                true
            } else {
                false
            }
        } else {
            false
        };
    if !client.push_message(&ServerMessage::Accepted)? {
        return Ok(false);
    }
    if returned_live {
        return presentation.publish(Some(client), terminal);
    }
    Ok(true)
}

fn defer_vertical(
    client: &mut Client,
    presentation: &mut Presentation,
    request: ClientMessage,
) -> Result<bool> {
    if presentation.defer_vertical(request) {
        Ok(true)
    } else {
        client.fail(
            FailureCode::InvalidInput,
            "vertical interaction is already deferred".into(),
        )
    }
}

fn vertical_wheel_direction(button: Option<MouseButton>) -> Option<VerticalDirection> {
    match button {
        Some(MouseButton::Four) => Some(VerticalDirection::Up),
        Some(MouseButton::Five) => Some(VerticalDirection::Down),
        _ => None,
    }
}

fn routes_vertical_wheel_to_terminal(terminal: &Terminal<'_, '_>) -> Result<bool> {
    Ok(terminal.is_mouse_tracking()?
        || terminal.active_screen()? == Screen::Alternate && terminal.mode(Mode::ALT_SCROLL)?)
}

fn handle_vertical_preview(
    client: &mut Client,
    frame_revision: u64,
    direction: VerticalDirection,
    terminal: &Terminal<'static, '_>,
    presentation: &mut Presentation,
) -> Result<bool> {
    if frame_revision > presentation.revision {
        return client.fail(
            FailureCode::InvalidInput,
            "preview frame revision is ahead of authority".into(),
        );
    }
    let outcome = if routes_vertical_wheel_to_terminal(terminal)? {
        PreviewOutcome::TerminalRouted
    } else {
        if terminal.mode(Mode::SYNC_OUTPUT)? {
            return defer_vertical(
                client,
                presentation,
                ClientMessage::PreviewVertical {
                    frame_revision,
                    direction,
                },
            );
        }
        match viewport_preview(terminal, presentation, direction) {
            Ok(outcome) => outcome,
            Err(error) => {
                return client.fail(FailureCode::Terminal, error.to_string());
            }
        }
    };
    client.push_message(&ServerMessage::VerticalPreview(VerticalPreview {
        frame_revision: presentation.revision,
        direction,
        outcome,
    }))
}

fn viewport_preview(
    terminal: &Terminal<'static, '_>,
    presentation: &Presentation,
    direction: VerticalDirection,
) -> Result<PreviewOutcome> {
    let scrollbar = terminal.scrollbar()?;
    let cols = terminal.cols()?;
    let window_rows = terminal.rows()?;
    let window = u64::from(window_rows);
    let below = scrollbar.offset.saturating_add(scrollbar.len);
    let mut encoded_size = FrameSize::new("", "", false, false)?;
    let mut rows = Vec::with_capacity(usize::from(window_rows));
    let mut window_complete = true;
    for index in 0..window {
        let target = match direction {
            VerticalDirection::Up => scrollbar.offset.checked_sub(index + 1),
            VerticalDirection::Down => below.checked_add(index).filter(|y| *y < scrollbar.total),
        };
        let Some(target) = target else {
            break;
        };
        let target = u32::try_from(target)
            .map_err(|_| "preview row exceeds the terminal coordinate range")?;
        let row = presentation.extractor.row(terminal, target, cols)?;
        let mut next_size = encoded_size;
        let sized = next_size.add_row().and_then(|()| {
            row.cells
                .iter()
                .try_for_each(|cell| next_size.add_cell(&cell.text, &cell.hyperlink))
        });
        match sized {
            Ok(()) => {
                encoded_size = next_size;
                rows.push(row);
            }
            Err(ProtocolError::FrameTooLarge { .. }) => {
                window_complete = false;
                break;
            }
            Err(error) => return Err(error.into()),
        }
    }
    let edge_reached = match direction {
        VerticalDirection::Up => scrollbar.offset <= window,
        VerticalDirection::Down => below.saturating_add(window) >= scrollbar.total,
    } && window_complete;
    Ok(PreviewOutcome::Viewport {
        cols,
        edge_reached,
        rows,
    })
}

fn handle_vertical_scroll(
    client: &mut Client,
    frame_revision: u64,
    rows: i16,
    terminal: &mut Terminal<'static, '_>,
    presentation: &mut Presentation,
    selection: &mut SelectionState,
) -> Result<bool> {
    if frame_revision > presentation.revision {
        return client.fail(
            FailureCode::InvalidInput,
            "scroll frame revision is ahead of authority".into(),
        );
    }
    if routes_vertical_wheel_to_terminal(terminal)? {
        let outcome = ServerMessage::ScrollOutcome(session::ScrollOutcome::TerminalOwned {
            requested_rows: rows,
        });
        return client.push_message(&outcome);
    }
    if terminal.mode(Mode::SYNC_OUTPUT)? {
        return defer_vertical(
            client,
            presentation,
            ClientMessage::ScrollVertical {
                frame_revision,
                rows,
            },
        );
    }
    if !client.can_push_scroll_outcome() {
        return client.fail(
            FailureCode::Terminal,
            "client output queue cannot admit scroll outcome".into(),
        );
    }
    let next_revision = match next_revision(presentation.revision) {
        Ok(next) => next,
        Err(error) => return client.fail(FailureCode::Terminal, error.to_string()),
    };
    let before = terminal.scrollbar()?.offset;
    clear_selection(terminal, selection, false)?;
    terminal.scroll_viewport(ScrollViewport::Delta(isize::from(rows)));
    let after = terminal.scrollbar()?.offset;
    let applied_rows = i16::try_from(i128::from(after) - i128::from(before))
        .map_err(|_| "terminal applied an invalid scroll delta")?;
    if applied_rows != 0
        && (applied_rows.is_negative() != rows.is_negative()
            || applied_rows.unsigned_abs() > rows.unsigned_abs())
    {
        return Err("terminal applied rows outside the requested scroll distance".into());
    }

    let direction = if rows.is_negative() {
        VerticalDirection::Up
    } else {
        VerticalDirection::Down
    };
    presentation.revision = next_revision;
    presentation.synchronized_until = None;
    let frame = presentation.extractor.frame(next_revision, terminal)?;
    let next = viewport_preview(terminal, presentation, direction)?;
    if !client.push_message(&ServerMessage::ScrollOutcome(
        session::ScrollOutcome::Viewport {
            requested_rows: rows,
            applied_rows,
            frame: Box::new(frame),
            next,
        },
    ))? {
        return Err("scroll outcome admission contradicted its preflight".into());
    }
    Ok(true)
}

fn handle_return_to_live(
    client: &mut Client,
    terminal: &mut Terminal<'static, '_>,
    presentation: &mut Presentation,
    selection: &mut SelectionState,
) -> Result<bool> {
    if terminal.active_screen()? != Screen::Primary || routes_vertical_wheel_to_terminal(terminal)?
    {
        return client.fail(
            FailureCode::InvalidInput,
            "live history is unavailable while the terminal owns scrolling".into(),
        );
    }
    if terminal.mode(Mode::SYNC_OUTPUT)? {
        return defer_vertical(client, presentation, ClientMessage::ReturnToLive);
    }
    if !client.can_push_frame_message() {
        return client.fail(
            FailureCode::Terminal,
            "client output queue cannot admit the live viewport".into(),
        );
    }
    let next = match next_revision(presentation.revision) {
        Ok(next) => next,
        Err(error) => return client.fail(FailureCode::Terminal, error.to_string()),
    };
    clear_selection(terminal, selection, false)?;
    terminal.scroll_viewport(ScrollViewport::Bottom);
    presentation.revision = next;
    presentation.synchronized_until = None;
    presentation.publish(Some(client), terminal)
}

#[allow(clippy::too_many_arguments)]
fn handle_vertical_wheel(
    client: &mut Client,
    input: session::MouseEvent,
    direction: VerticalDirection,
    terminal: &mut Terminal<'static, '_>,
    size: SurfaceSize,
    writes: &RefCell<VecDeque<u8>>,
    presentation: &mut Presentation,
    selection: &mut SelectionState,
) -> Result<bool> {
    if routes_vertical_wheel_to_terminal(terminal)? {
        let routed = if terminal.is_mouse_tracking()? {
            ClientMessage::Mouse(input)
        } else {
            ClientMessage::Key(KeyEvent {
                action: KeyAction::Press,
                key: match direction {
                    VerticalDirection::Up => PhysicalKey::ARROW_UP,
                    VerticalDirection::Down => PhysicalKey::ARROW_DOWN,
                },
                modifiers: Modifiers::empty(),
                consumed_modifiers: Modifiers::empty(),
                composing: false,
                text: None,
                unshifted_codepoint: None,
            })
        };
        let encoded = match encode_input(terminal, size, routed) {
            Ok(encoded) if !encoded.is_empty() => encoded,
            Ok(_) => {
                return client.fail(
                    FailureCode::InvalidInput,
                    "wheel is outside the terminal grid".into(),
                );
            }
            Err(error) => {
                return client.fail(FailureCode::Terminal, error.to_string());
            }
        };
        let outcome = ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted);
        if !client.can_push_message(&outcome)? {
            return client.fail(
                FailureCode::Terminal,
                "client output queue cannot admit wheel outcome".into(),
            );
        }
        if !queue_pty_write(&mut writes.borrow_mut(), &encoded) {
            let queued = client.fail(FailureCode::Terminal, "PTY input queue is full".into())?;
            client.close_when_flushed();
            return Ok(queued);
        }
        if !client.push_message(&outcome)? {
            return Err("wheel outcome admission contradicted its preflight".into());
        }
        return Ok(true);
    }

    if terminal.mode(Mode::SYNC_OUTPUT)? {
        return client.fail(FailureCode::Terminal, "presentation is synchronized".into());
    }
    if !client.can_push_frame_message() {
        return client.fail(
            FailureCode::Terminal,
            "client output queue cannot admit wheel outcome".into(),
        );
    }
    let next = match next_revision(presentation.revision) {
        Ok(next) => next,
        Err(error) => return client.fail(FailureCode::Terminal, error.to_string()),
    };
    let before = terminal.scrollbar()?.offset;
    clear_selection(terminal, selection, false)?;
    terminal.scroll_viewport(ScrollViewport::Delta(match direction {
        VerticalDirection::Up => -1,
        VerticalDirection::Down => 1,
    }));
    let after = terminal.scrollbar()?.offset;
    let applied_rows = i8::try_from(i128::from(after) - i128::from(before))
        .map_err(|_| "terminal applied an invalid wheel delta")?;
    if !matches!(applied_rows, -1..=1) {
        return Err("terminal applied more than one wheel row".into());
    }
    presentation.revision = next;
    presentation.synchronized_until = None;
    let frame = presentation.extractor.frame(next, terminal)?;
    if !client.push_message(&ServerMessage::WheelOutcome(WheelOutcome::Viewport {
        applied_rows,
        frame: Box::new(frame),
    }))? {
        return Err("wheel outcome admission contradicted its preflight".into());
    }
    Ok(true)
}

fn handle_selection(
    client: &mut Client,
    action: SelectionAction,
    terminal: &Terminal<'static, '_>,
    size: SurfaceSize,
    writes: &RefCell<VecDeque<u8>>,
    presentation: &mut Presentation,
    state: &mut SelectionState,
) -> Result<bool> {
    if let SelectionAction::AutoscrollUp { position } = action {
        return handle_selection_autoscroll(client, position, terminal, size, presentation, state);
    }
    if matches!(action, SelectionAction::Copy) {
        return match &state.copied {
            Some(text) => client.push_message(&ServerMessage::CopiedText {
                location: session::ClipboardLocation::Standard,
                text: text.clone(),
            }),
            None => client.fail(
                FailureCode::InvalidInput,
                "no finished selection to copy".into(),
            ),
        };
    }
    if matches!(action, SelectionAction::Cancel) {
        if !client.can_push_result_frame() {
            return client.fail(
                FailureCode::Terminal,
                "client output queue cannot admit selection frame".into(),
            );
        }
        let next = match next_revision(presentation.revision) {
            Ok(next) => next,
            Err(error) => return client.fail(FailureCode::Terminal, error.to_string()),
        };
        clear_pointer_sequence(terminal, state, false)?;
        presentation.revision = next;
        if !client.push_message(&ServerMessage::Accepted)? {
            return Ok(false);
        }
        return presentation.publish(Some(client), terminal);
    }

    let (position, modifiers) = match action {
        SelectionAction::Begin {
            frame_revision,
            position,
            modifiers,
            ..
        } => {
            if frame_revision > presentation.revision {
                return client.fail(
                    FailureCode::InvalidInput,
                    "selection frame revision is ahead of authority".into(),
                );
            }
            if state.route.is_some() {
                return client.fail(
                    FailureCode::InvalidInput,
                    "selection press has an active gesture".into(),
                );
            }
            (position, modifiers)
        }
        SelectionAction::Update {
            position,
            modifiers,
        } => {
            if state.route.is_none() {
                return client.fail(
                    FailureCode::InvalidInput,
                    "selection update has no active selection".into(),
                );
            }
            (position, modifiers)
        }
        SelectionAction::Finish {
            position,
            modifiers,
        } => {
            if state.route.is_none() {
                return client.fail(
                    FailureCode::InvalidInput,
                    "selection finish has no active selection".into(),
                );
            }
            (position, modifiers)
        }
        SelectionAction::AutoscrollUp { .. } | SelectionAction::Cancel | SelectionAction::Copy => {
            unreachable!("handled above")
        }
    };

    let route = match action {
        SelectionAction::Begin { .. } => {
            if terminal.is_mouse_tracking()? && !modifiers.contains(Modifiers::SHIFT) {
                PointerRoute::Terminal
            } else {
                PointerRoute::Host
            }
        }
        _ => state.route.expect("active route was checked"),
    };
    if route == PointerRoute::Terminal {
        return handle_terminal_pointer(
            client,
            action,
            position,
            modifiers,
            terminal,
            size,
            writes,
            presentation,
            state,
        );
    }
    let Some(point) = viewport_point(position, size) else {
        return client.fail(
            FailureCode::InvalidInput,
            "selection position is outside the current viewport".into(),
        );
    };
    let finishing = matches!(action, SelectionAction::Finish { .. });
    if !client.can_push_host_selection_result(finishing) {
        return client.fail(
            FailureCode::Terminal,
            "client output queue cannot admit selection frame".into(),
        );
    }
    let next = match next_revision(presentation.revision) {
        Ok(next) => next,
        Err(error) => return client.fail(FailureCode::Terminal, error.to_string()),
    };

    let grid_ref = match terminal.grid_ref(Point::Viewport(point)) {
        Ok(grid_ref) => grid_ref,
        Err(error) => return client.fail(FailureCode::Terminal, error.to_string()),
    };
    if let Err(error) = apply_selection_gesture(action, grid_ref, terminal, size, state) {
        clear_selection(terminal, state, false)?;
        return client.fail(FailureCode::Terminal, error.to_string());
    }
    client.supersede_pending_frame();
    debug_assert_eq!(
        matches!(action, SelectionAction::Finish { .. }),
        state.route.is_none()
    );
    presentation.revision = next;
    let response = if finishing {
        ServerMessage::SelectionFinished {
            frame_revision: next,
        }
    } else {
        ServerMessage::Accepted
    };
    if !client.push_message(&response)? {
        return Ok(false);
    }
    if !presentation.publish(Some(client), terminal)? {
        return Ok(false);
    }
    if finishing && let Some(text) = &state.copied {
        return client.push_message(&ServerMessage::CopiedText {
            location: session::ClipboardLocation::Selection,
            text: text.clone(),
        });
    }
    Ok(true)
}

fn handle_selection_autoscroll(
    client: &mut Client,
    position: SelectionPosition,
    terminal: &Terminal<'static, '_>,
    size: SurfaceSize,
    presentation: &mut Presentation,
    state: &mut SelectionState,
) -> Result<bool> {
    if state.route != Some(PointerRoute::Host) || terminal.active_screen()? != Screen::Primary {
        return client.fail(
            FailureCode::InvalidInput,
            "selection autoscroll requires an active primary-screen host gesture".into(),
        );
    }
    let Some(point) = viewport_point(position, size).filter(|point| point.y == 0) else {
        return client.fail(
            FailureCode::InvalidInput,
            "selection autoscroll position must be in the top row".into(),
        );
    };
    let Some(gesture) = state.gesture.as_mut() else {
        return client.fail(
            FailureCode::InvalidInput,
            "selection gesture is missing".into(),
        );
    };
    if gesture.gesture.anchor(terminal)?.is_none() {
        clear_selection(terminal, state, false)?;
        return client.fail(
            FailureCode::InvalidInput,
            "selection anchor is no longer valid".into(),
        );
    }
    if terminal.mode(Mode::SYNC_OUTPUT)? {
        return defer_vertical(
            client,
            presentation,
            ClientMessage::Selection(SelectionAction::AutoscrollUp { position }),
        );
    }
    if !client.can_push_scroll_outcome() {
        return client.fail(
            FailureCode::Terminal,
            "client output queue cannot admit selection scroll outcome".into(),
        );
    }
    let next_revision = match next_revision(presentation.revision) {
        Ok(next) => next,
        Err(error) => return client.fail(FailureCode::Terminal, error.to_string()),
    };
    let grid_ref = terminal.grid_ref(Point::Viewport(point))?;
    let before = terminal.scrollbar()?.offset;
    let (drag, autoscroll) = if gesture.is_span(terminal)? {
        (&mut gesture.span_drag, &mut gesture.span_autoscroll)
    } else {
        (&mut gesture.drag, &mut gesture.autoscroll)
    };
    let selected = (|| -> Result<_> {
        drag.set_position(f64::from(position.x), 0.0)?.apply(
            &mut gesture.gesture,
            terminal,
            grid_ref,
            selection_geometry(size),
        )?;
        if gesture.gesture.autoscroll(terminal)? != Autoscroll::Up {
            return Err("selection gesture did not enter upward autoscroll".into());
        }
        Ok(autoscroll.set_position(f64::from(position.x), 0.0)?.apply(
            &mut gesture.gesture,
            terminal,
            point,
            selection_geometry(size),
        )?)
    })();
    let selected = match selected {
        Ok(selected) => selected,
        Err(error) => {
            clear_selection(terminal, state, false)?;
            return client.fail(FailureCode::Terminal, error.to_string());
        }
    };
    terminal.set_selection(selected.as_ref())?;
    state.has_selection = selected.is_some();
    let after = terminal.scrollbar()?.offset;
    let applied_rows = i16::try_from(i128::from(after) - i128::from(before))
        .map_err(|_| "terminal applied an invalid selection scroll delta")?;
    if !matches!(applied_rows, -1..=0) {
        return Err("terminal applied rows outside one upward selection tick".into());
    }
    presentation.revision = next_revision;
    presentation.synchronized_until = None;
    let frame = presentation.extractor.frame(next_revision, terminal)?;
    let next = viewport_preview(terminal, presentation, VerticalDirection::Up)?;
    if !client.push_message(&ServerMessage::ScrollOutcome(
        session::ScrollOutcome::Viewport {
            requested_rows: -1,
            applied_rows,
            frame: Box::new(frame),
            next,
        },
    ))? {
        return Err("selection scroll outcome admission contradicted its preflight".into());
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn handle_terminal_pointer(
    client: &mut Client,
    action: SelectionAction,
    position: SelectionPosition,
    modifiers: Modifiers,
    terminal: &Terminal<'static, '_>,
    size: SurfaceSize,
    writes: &RefCell<VecDeque<u8>>,
    presentation: &mut Presentation,
    state: &mut SelectionState,
) -> Result<bool> {
    let changed = matches!(action, SelectionAction::Begin { .. }) && state.has_selection;
    let response = if matches!(action, SelectionAction::Finish { .. }) {
        ServerMessage::SelectionFinished {
            frame_revision: presentation.revision,
        }
    } else {
        ServerMessage::Accepted
    };
    let admitted = if changed {
        client.can_push_result_frame()
    } else {
        client.can_push_message(&response)?
    };
    if !admitted {
        return client.fail(
            FailureCode::Terminal,
            "client output queue cannot admit pointer result".into(),
        );
    }
    let next = if changed {
        match next_revision(presentation.revision) {
            Ok(next) => Some(next),
            Err(error) => return client.fail(FailureCode::Terminal, error.to_string()),
        }
    } else {
        None
    };
    let input = ClientMessage::Mouse(session::MouseEvent {
        action: match action {
            SelectionAction::Begin { .. } => MouseAction::Press,
            SelectionAction::Update { .. } => MouseAction::Motion,
            SelectionAction::Finish { .. } => MouseAction::Release,
            SelectionAction::AutoscrollUp { .. }
            | SelectionAction::Cancel
            | SelectionAction::Copy => {
                unreachable!("handled above")
            }
        },
        button: Some(MouseButton::Left),
        modifiers,
        x: position.x,
        y: position.y,
    });
    let encoded = match encode_input(terminal, size, input) {
        Ok(encoded) => encoded,
        Err(error) => return client.fail(FailureCode::Terminal, error.to_string()),
    };
    if !can_queue_pty_write(&writes.borrow(), &encoded) {
        let queued = client.fail(FailureCode::Terminal, "PTY input queue is full".into())?;
        client.close_when_flushed();
        return Ok(queued);
    }
    if matches!(action, SelectionAction::Begin { .. }) {
        clear_selection(terminal, state, true)?;
        state.route = Some(PointerRoute::Terminal);
        if let Some(next) = next {
            presentation.revision = next;
        }
    } else if matches!(action, SelectionAction::Finish { .. }) {
        state.route = None;
    }
    if !queue_pty_write(&mut writes.borrow_mut(), &encoded) {
        return Err("PTY input queue admission contradicted its preflight".into());
    }
    if !client.push_message(&response)? {
        return Ok(false);
    }
    if changed {
        return presentation.publish(Some(client), terminal);
    }
    Ok(true)
}

fn apply_selection_gesture(
    action: SelectionAction,
    grid_ref: GridRef<'_>,
    terminal: &Terminal<'_, '_>,
    size: SurfaceSize,
    state: &mut SelectionState,
) -> Result {
    let gesture = match &mut state.gesture {
        Some(gesture) => gesture,
        None => state.gesture.insert(GestureState::new()?),
    };
    let selected = match action {
        SelectionAction::Begin {
            position, time_ns, ..
        } => {
            // Ghostty caps its count at three: only the first third press uses
            // span events; a continuing fourth-or-later press uses native line
            // behavior. Ghostty still owns repeat validation and reset.
            let press = if gesture.gesture.click_count(terminal)? == 2 {
                &mut gesture.span_press
            } else {
                &mut gesture.press
            };
            press
                .set_position(f64::from(position.x), f64::from(position.y))?
                .set_time(Duration::from_nanos(time_ns))?
                .set_repeat_distance(f64::from(size.cell_width))?
                .set_repeat_interval(CLICK_REPEAT_INTERVAL)?
                .apply(&mut gesture.gesture, terminal, grid_ref.clone())?
        }
        SelectionAction::Update { position, .. } | SelectionAction::Finish { position, .. } => {
            let drag = if gesture.is_span(terminal)? {
                &mut gesture.span_drag
            } else {
                &mut gesture.drag
            };
            drag.set_position(f64::from(position.x), f64::from(position.y))?
                .apply(
                    &mut gesture.gesture,
                    terminal,
                    grid_ref.clone(),
                    selection_geometry(size),
                )?
        }
        SelectionAction::AutoscrollUp { .. } | SelectionAction::Cancel | SelectionAction::Copy => {
            unreachable!("handled above")
        }
    };

    terminal.set_selection(selected.as_ref())?;
    state.has_selection = selected.is_some();

    if matches!(action, SelectionAction::Begin { .. }) {
        state.copied = None;
        state.route = Some(PointerRoute::Host);
    } else if matches!(action, SelectionAction::Finish { .. }) {
        gesture
            .release
            .apply(&mut gesture.gesture, terminal, Some(grid_ref))?;
        state.route = None;
        state.copied = state
            .has_selection
            .then(|| format_selection(terminal))
            .transpose()?;
    }
    Ok(())
}

fn selection_geometry(size: SurfaceSize) -> Geometry {
    Geometry {
        columns: u32::from(size.cols),
        cell_width: size.cell_width,
        padding_left: size.padding_left,
        screen_height: size.screen_height,
    }
}

fn viewport_point(position: SelectionPosition, size: SurfaceSize) -> Option<PointCoordinate> {
    let x = f64::from(position.x) - f64::from(size.padding_left);
    let y = f64::from(position.y) - f64::from(size.padding_top);
    let grid_width = f64::from(size.cols) * f64::from(size.cell_width);
    let grid_height = f64::from(size.rows) * f64::from(size.cell_height);
    (x >= 0.0 && x < grid_width && y >= 0.0 && y < grid_height).then_some(PointCoordinate {
        x: (x / f64::from(size.cell_width)) as u16,
        y: (y / f64::from(size.cell_height)) as u32,
    })
}

fn format_selection(terminal: &Terminal<'_, '_>) -> Result<String> {
    let options = || {
        FormatOptions::new()
            .with_emit_format(Format::Plain)
            .with_unwrap(true)
            .with_trim(true)
    };
    let required = match terminal.format_selection_buf(options(), &mut []) {
        Ok(Some(written)) => written,
        Ok(None) => return Err("terminal returned no selection text".into()),
        Err(libghostty_vt::error::Error::OutOfSpace { required }) => required,
        Err(error) => return Err(error.into()),
    };
    if required > session::MAX_COPY_BYTES {
        return Err("selection text exceeds the copy limit".into());
    }
    let mut bytes = vec![0; required];
    let Some(written) = terminal.format_selection_buf(options(), &mut bytes)? else {
        return Err("terminal returned no selection text".into());
    };
    bytes.truncate(written);
    Ok(String::from_utf8(bytes)?)
}

fn clear_selection(
    terminal: &Terminal<'_, '_>,
    state: &mut SelectionState,
    forget_copy: bool,
) -> Result<bool> {
    let changed = state.has_selection;
    if changed {
        terminal.set_selection(None)?;
    }
    if let Some(gesture) = &mut state.gesture {
        gesture.gesture.reset(terminal);
    }
    if state.route == Some(PointerRoute::Host) {
        state.route = None;
    }
    state.has_selection = false;
    if forget_copy {
        state.copied = None;
    }
    Ok(changed)
}

pub(crate) fn clear_pointer_sequence(
    terminal: &Terminal<'_, '_>,
    state: &mut SelectionState,
    forget_copy: bool,
) -> Result<bool> {
    let changed = clear_selection(terminal, state, forget_copy)?;
    state.route = None;
    Ok(changed)
}

pub(crate) fn apply_pty_output(
    terminal: &mut Terminal<'_, '_>,
    state: &mut SelectionState,
    bytes: &[u8],
) -> Result {
    let preserve_host = state.route == Some(PointerRoute::Host)
        || state.has_selection
        || match &state.gesture {
            Some(gesture) => gesture.gesture.click_count(terminal)? != 0,
            None => false,
        };
    let mouse_tracking = preserve_host
        .then(|| terminal.is_mouse_tracking())
        .transpose()?;
    terminal.vt_write(bytes);
    if let Some(mouse_tracking) = mouse_tracking {
        let anchor_valid = match &state.gesture {
            Some(gesture) => gesture.gesture.anchor(terminal)?.is_some(),
            None => false,
        };
        if (!mouse_tracking && terminal.is_mouse_tracking()?) || !anchor_valid {
            clear_selection(terminal, state, false)?;
        }
    }
    Ok(())
}

fn encode_input(
    terminal: &Terminal<'_, '_>,
    size: SurfaceSize,
    input: ClientMessage,
) -> Result<Vec<u8>> {
    match input {
        ClientMessage::Key(input) => {
            let key = Key::try_from(u32::from(input.key.raw()))
                .map_err(|value| format!("unsupported physical key {value}"))?;
            let unshifted_codepoint = input
                .unshifted_codepoint
                .or((input.key == PhysicalKey::SPACE).then_some(' '));
            let text = input
                .text
                .filter(|_| input.action != KeyAction::Release || unshifted_codepoint.is_some());
            let mut event = GhosttyKeyEvent::new()?;
            event
                .set_action(match input.action {
                    KeyAction::Press => GhosttyKeyAction::Press,
                    KeyAction::Release => GhosttyKeyAction::Release,
                    KeyAction::Repeat => GhosttyKeyAction::Repeat,
                })
                .set_key(key)
                .set_mods(ghostty_modifiers(input.modifiers)?)
                .set_consumed_mods(ghostty_modifiers(input.consumed_modifiers)?)
                .set_composing(input.composing)
                .set_utf8(text);
            if let Some(codepoint) = unshifted_codepoint {
                event.set_unshifted_codepoint(codepoint);
            }
            let mut encoder = KeyEncoder::new()?;
            encoder.set_options_from_terminal(terminal);
            let mut output = Vec::new();
            encoder.encode_to_vec(&event, &mut output)?;
            Ok(output)
        }
        ClientMessage::Mouse(input) => {
            let is_wheel = matches!(
                input.button,
                Some(MouseButton::Four | MouseButton::Five | MouseButton::Six | MouseButton::Seven)
            );
            let mut event = GhosttyMouseEvent::new()?;
            event
                .set_action(match input.action {
                    MouseAction::Press => GhosttyMouseAction::Press,
                    MouseAction::Release => GhosttyMouseAction::Release,
                    MouseAction::Motion => GhosttyMouseAction::Motion,
                })
                .set_button(input.button.map(ghostty_mouse_button))
                .set_mods(ghostty_modifiers(input.modifiers)?)
                .set_position(MousePosition {
                    x: input.x,
                    y: input.y,
                });
            let mut encoder = MouseEncoder::new()?;
            encoder
                .set_options_from_terminal(terminal)
                .set_size(mouse_size(size))
                .set_any_button_pressed(
                    input.button.is_some() && input.action != MouseAction::Release && !is_wheel,
                );
            let mut output = Vec::new();
            encoder.encode_to_vec(&event, &mut output)?;
            Ok(output)
        }
        ClientMessage::Focus(event) => {
            if !terminal.mode(Mode::FOCUS_EVENT)? {
                return Ok(Vec::new());
            }
            let event = match event {
                FocusEvent::Gained => focus::Event::Gained,
                FocusEvent::Lost => focus::Event::Lost,
            };
            let mut output = vec![0; 8];
            let written = event.encode(&mut output)?;
            output.truncate(written);
            Ok(output)
        }
        ClientMessage::Paste(mut data) => {
            let bracketed = terminal.mode(Mode::BRACKETED_PASTE)?;
            let mut output = vec![0; data.len() + 16];
            let written = paste::encode(&mut data, bracketed, &mut output)?;
            output.truncate(written);
            Ok(output)
        }
        ClientMessage::Hello
        | ClientMessage::ObserveMetadata
        | ClientMessage::Resize(_)
        | ClientMessage::Selection(_)
        | ClientMessage::PreviewVertical { .. }
        | ClientMessage::ScrollVertical { .. }
        | ClientMessage::ReturnToLive => Err("message is not terminal input".into()),
    }
}

fn mouse_size(size: SurfaceSize) -> MouseEncoderSize {
    MouseEncoderSize {
        screen_width: size.screen_width,
        screen_height: size.screen_height,
        cell_width: size.cell_width,
        cell_height: size.cell_height,
        padding_top: size.padding_top,
        padding_bottom: size.padding_bottom,
        padding_right: size.padding_right,
        padding_left: size.padding_left,
    }
}

fn ghostty_modifiers(modifiers: Modifiers) -> Result<GhosttyModifiers> {
    GhosttyModifiers::from_bits(modifiers.bits()).ok_or_else(|| "invalid key modifiers".into())
}

fn ghostty_mouse_button(button: MouseButton) -> GhosttyMouseButton {
    match button {
        MouseButton::Unknown => GhosttyMouseButton::Unknown,
        MouseButton::Left => GhosttyMouseButton::Left,
        MouseButton::Middle => GhosttyMouseButton::Middle,
        MouseButton::Right => GhosttyMouseButton::Right,
        MouseButton::Four => GhosttyMouseButton::Four,
        MouseButton::Five => GhosttyMouseButton::Five,
        MouseButton::Six => GhosttyMouseButton::Six,
        MouseButton::Seven => GhosttyMouseButton::Seven,
        MouseButton::Eight => GhosttyMouseButton::Eight,
        MouseButton::Nine => GhosttyMouseButton::Nine,
        MouseButton::Ten => GhosttyMouseButton::Ten,
        MouseButton::Eleven => GhosttyMouseButton::Eleven,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        presentation::Extractor,
        runtime::{
            INITIAL_SIZE, MAX_PTY_WRITE_BYTES,
            tests::{attached_client, flush_message, terminal, terminal_with_scrollback, wheel},
        },
    };
    use libghostty_vt::selection::Selection;
    use orbit_protocol::session::Failure;

    fn selection_between<'terminal>(
        terminal: &'terminal Terminal<'_, '_>,
        start: (u16, u32),
        end: (u16, u32),
    ) -> Result<Selection<'terminal>> {
        let point = |(x, y)| terminal.grid_ref(Point::Viewport(PointCoordinate { x, y }));
        Ok(Selection::new(point(start)?, point(end)?, false))
    }

    fn position(size: SurfaceSize, x: u16, y: u16) -> SelectionPosition {
        SelectionPosition {
            x: (size.padding_left + u32::from(x) * size.cell_width) as f32
                + size.cell_width as f32 / 2.0,
            y: (size.padding_top + u32::from(y) * size.cell_height) as f32
                + size.cell_height as f32 / 2.0,
        }
    }

    #[test]
    fn selection_positions_use_authoritative_surface_geometry() {
        let size = SurfaceSize {
            cols: 2,
            rows: 2,
            screen_width: 24,
            screen_height: 40,
            cell_width: 8,
            cell_height: 16,
            padding_top: 4,
            padding_bottom: 4,
            padding_left: 4,
            padding_right: 4,
        };
        for (position, expected) in [
            (
                SelectionPosition { x: 4.0, y: 4.0 },
                Some(PointCoordinate { x: 0, y: 0 }),
            ),
            (
                SelectionPosition { x: 19.99, y: 35.99 },
                Some(PointCoordinate { x: 1, y: 1 }),
            ),
            (SelectionPosition { x: 3.99, y: 4.0 }, None),
            (SelectionPosition { x: 20.0, y: 4.0 }, None),
        ] {
            assert_eq!(viewport_point(position, size), expected);
        }
    }

    #[test]
    fn authoritative_selection_accepts_presented_input_and_freezes_copy() -> Result {
        let mut text = terminal_with_scrollback(100)?;
        text.vt_write(b"A\r\n\r\nZ\r\nB\x1b[3;1H\x1b[X");
        for (start, end, expected) in [
            ((0, 0), (0, 0), "A"),
            ((0, 0), (0, 3), "A\n\n\nB"),
            ((0, 3), (0, 0), "A\n\n\nB"),
            ((0, 1), (0, 1), ""),
            ((0, 2), (0, 2), ""),
        ] {
            let selected = selection_between(&text, start, end)?;
            text.set_selection(Some(&selected))?;
            assert_eq!(format_selection(&text)?, expected);
        }

        for line in 0..30 {
            text.vt_write(format!("history-{line:02}\r\n").as_bytes());
        }
        text.scroll_viewport(ScrollViewport::Top);
        let history = text.scrollbar()?;
        assert_eq!(history.offset, 0);
        assert!(history.offset + history.len < history.total);
        let retained = selection_between(&text, (0, 0), (0, 0))?;
        text.set_selection(Some(&retained))?;
        assert_eq!(format_selection(&text)?, "A");

        let (mut client, mut peer) = attached_client()?;
        let pty = Pty::without_child_for_test()?;
        let mut terminal = terminal()?;
        terminal.vt_write("alpha 界\r\n".as_bytes());
        let mut size = INITIAL_SIZE;
        let writes = RefCell::new(VecDeque::new());
        let mut presentation = Presentation::new()?;
        presentation.revision = 1;
        let mut selection = SelectionState::default();

        macro_rules! reject {
            ($action:expr) => {{
                let unchanged = presentation.revision;
                assert!(handle_client_message(
                    &mut client,
                    ClientMessage::Selection($action),
                    &mut terminal,
                    Some(&pty),
                    &mut size,
                    &writes,
                    &mut presentation,
                    &mut selection,
                )?);
                assert_eq!(presentation.revision, unchanged);
                assert!(selection.route.is_none());
                assert!(!selection.has_selection);
                assert!(selection.copied.is_none());
                assert!(matches!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::Failure(Failure {
                        code: FailureCode::InvalidInput,
                        ..
                    })
                ));
            }};
        }
        macro_rules! select {
            ($action:expr) => {{
                let action = $action;
                let finishing = matches!(&action, SelectionAction::Finish { .. });
                assert!(handle_client_message(
                    &mut client,
                    ClientMessage::Selection(action),
                    &mut terminal,
                    Some(&pty),
                    &mut size,
                    &writes,
                    &mut presentation,
                    &mut selection,
                )?);
                assert_eq!(
                    flush_message(&mut client, &mut peer)?,
                    if finishing {
                        ServerMessage::SelectionFinished {
                            frame_revision: presentation.revision,
                        }
                    } else {
                        ServerMessage::Accepted
                    }
                );
                let ServerMessage::Frame(frame) = flush_message(&mut client, &mut peer)? else {
                    return Err("selection did not publish a frame".into());
                };
                assert_eq!(frame.revision, presentation.revision);
                frame
            }};
        }
        let presented_revision = presentation.revision;
        apply_pty_output(&mut terminal, &mut selection, b"live")?;
        presentation.revision = next_revision(presentation.revision)?;
        select!(session::SelectionAction::Begin {
            frame_revision: presented_revision,
            position: position(size, 0, 0),
            time_ns: 0,
            modifiers: Modifiers::empty(),
        });
        select!(session::SelectionAction::Cancel);
        reject!(session::SelectionAction::Begin {
            frame_revision: next_revision(presentation.revision)?,
            position: position(size, 0, 0),
            time_ns: 0,
            modifiers: Modifiers::empty(),
        });
        reject!(session::SelectionAction::Update {
            position: position(size, 0, 0),
            modifiers: Modifiers::empty(),
        });
        reject!(session::SelectionAction::Begin {
            frame_revision: presentation.revision,
            position: session::SelectionPosition {
                x: size.screen_width as f32,
                y: position(size, 0, 0).y,
            },
            time_ns: 0,
            modifiers: Modifiers::empty(),
        });

        let frame = select!(session::SelectionAction::Begin {
            frame_revision: presentation.revision,
            position: position(size, 0, 0),
            time_ns: 0,
            modifiers: Modifiers::empty(),
        });
        assert!(!frame.rows[0].cells[0].style.selected);
        select!(session::SelectionAction::Cancel);
        reject!(session::SelectionAction::Finish {
            position: position(size, 0, 0),
            modifiers: Modifiers::empty(),
        });

        select!(session::SelectionAction::Begin {
            frame_revision: presentation.revision,
            position: position(size, 0, 0),
            time_ns: 0,
            modifiers: Modifiers::empty(),
        });
        apply_pty_output(&mut terminal, &mut selection, b"!")?;
        assert_eq!(selection.route, Some(PointerRoute::Host));
        assert!(selection.copied.is_none());
        select!(session::SelectionAction::Finish {
            position: position(size, 7, 0),
            modifiers: Modifiers::empty(),
        });
        assert_eq!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::CopiedText {
                location: session::ClipboardLocation::Selection,
                text: "alpha 界".into(),
            }
        );

        select!(session::SelectionAction::Begin {
            frame_revision: presentation.revision,
            position: position(size, 0, 0),
            time_ns: 1_000_000_000,
            modifiers: Modifiers::empty(),
        });
        select!(session::SelectionAction::Update {
            position: position(size, 7, 0),
            modifiers: Modifiers::empty(),
        });
        select!(session::SelectionAction::Finish {
            position: position(size, 7, 0),
            modifiers: Modifiers::empty(),
        });
        assert_eq!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::CopiedText {
                location: session::ClipboardLocation::Selection,
                text: "alpha 界".into(),
            }
        );

        assert!(handle_client_message(
            &mut client,
            ClientMessage::Selection(session::SelectionAction::Copy),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::CopiedText {
                location: session::ClipboardLocation::Standard,
                text: "alpha 界".into(),
            }
        );

        apply_pty_output(&mut terminal, &mut selection, b"later")?;
        assert!(
            Extractor::new()?
                .frame(presentation.revision + 1, &terminal)?
                .rows[0]
                .cells
                .iter()
                .any(|cell| cell.style.selected)
        );
        assert_eq!(selection.copied.as_deref(), Some("alpha 界"));

        let (mut blocked, _) = attached_client()?;
        blocked.fill_output_for_test();
        let stable_revision = presentation.revision;
        assert!(!handle_client_message(
            &mut blocked,
            ClientMessage::Selection(session::SelectionAction::Begin {
                frame_revision: presentation.revision,
                position: position(size, 1, 0),
                time_ns: 2_000_000_000,
                modifiers: Modifiers::empty(),
            }),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(presentation.revision, stable_revision);
        assert_eq!(selection.copied.as_deref(), Some("alpha 界"));

        apply_pty_output(&mut terminal, &mut selection, b"\x1b[?1049h")?;
        assert_eq!(terminal.active_screen()?, Screen::Alternate);
        assert_eq!(selection.copied.as_deref(), Some("alpha 界"));
        assert!(selection.route.is_none());
        assert!(!selection.has_selection);
        Ok(())
    }

    #[test]
    fn conformance_c9_upward_selection_scroll_keeps_one_gesture_and_exact_copy() -> Result {
        for (clicks, anchor_end) in [(1, None), (3, Some("L14:part")), (4, Some("L14:part 界"))] {
            let size = SurfaceSize {
                cols: 12,
                rows: 4,
                screen_width: 12 * INITIAL_SIZE.cell_width + 5,
                screen_height: 4 * INITIAL_SIZE.cell_height + 6,
                padding_left: 5,
                padding_top: 6,
                ..INITIAL_SIZE
            };
            let lines: Vec<_> = (0..16).map(|row| format!("L{row:02}:part 界")).collect();
            let mut terminal = terminal_with_scrollback(4096)?;
            terminal.resize(size.cols, size.rows, size.cell_width, size.cell_height)?;
            terminal.vt_write(lines.join("\r\n").as_bytes());
            let (mut client, mut peer) = attached_client()?;
            let pty = Pty::without_child_for_test()?;
            let mut size = size;
            let writes = RefCell::new(VecDeque::new());
            let mut presentation = Presentation::new()?;
            presentation.revision = 1;
            let mut selection = SelectionState::default();

            macro_rules! send {
                ($action:expr) => {
                    assert!(handle_client_message(
                        &mut client,
                        ClientMessage::Selection($action),
                        &mut terminal,
                        Some(&pty),
                        &mut size,
                        &writes,
                        &mut presentation,
                        &mut selection,
                    )?);
                };
            }
            let live_offset = terminal.scrollbar()?.offset;
            send!(SelectionAction::AutoscrollUp {
                position: position(size, 0, 0),
            });
            assert!(matches!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::Failure(Failure {
                    code: FailureCode::InvalidInput,
                    ..
                })
            ));
            assert_eq!(terminal.scrollbar()?.offset, live_offset);
            assert_eq!(presentation.revision, 1);
            for click in 1..=clicks {
                send!(SelectionAction::Begin {
                    frame_revision: presentation.revision,
                    position: position(size, 0, 2),
                    time_ns: click * 100_000_000,
                    modifiers: Modifiers::empty(),
                });
                assert_eq!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::Accepted
                );
                assert!(matches!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::Frame(_)
                ));
                if click == clicks {
                    break;
                }
                send!(SelectionAction::Finish {
                    position: position(size, 0, 2),
                    modifiers: Modifiers::empty(),
                });
                assert!(matches!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::SelectionFinished { .. }
                ));
                assert!(matches!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::Frame(_)
                ));
                if selection.copied.is_some() {
                    assert!(matches!(
                        flush_message(&mut client, &mut peer)?,
                        ServerMessage::CopiedText { .. }
                    ));
                }
            }
            send!(SelectionAction::Update {
                position: position(size, 0, 0),
                modifiers: Modifiers::empty(),
            });
            assert_eq!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::Accepted
            );
            assert!(matches!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::Frame(_)
            ));

            let stable_offset = terminal.scrollbar()?.offset;
            let stable_revision = presentation.revision;
            send!(SelectionAction::AutoscrollUp {
                position: position(size, 0, 1),
            });
            assert!(matches!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::Failure(Failure {
                    code: FailureCode::InvalidInput,
                    ..
                })
            ));
            assert_eq!(terminal.scrollbar()?.offset, stable_offset);
            assert_eq!(presentation.revision, stable_revision);

            terminal.set_mode(Mode::SYNC_OUTPUT, true)?;
            send!(SelectionAction::AutoscrollUp {
                position: position(size, 0, 0),
            });
            assert_eq!(terminal.scrollbar()?.offset, stable_offset);
            assert_eq!(presentation.revision, stable_revision);
            let deferred = presentation
                .take_deferred_vertical()
                .ok_or("selection tick was not deferred")?;
            terminal.set_mode(Mode::SYNC_OUTPUT, false)?;
            assert!(handle_client_message(
                &mut client,
                deferred,
                &mut terminal,
                Some(&pty),
                &mut size,
                &writes,
                &mut presentation,
                &mut selection,
            )?);
            assert!(matches!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::ScrollOutcome(session::ScrollOutcome::Viewport {
                    requested_rows: -1,
                    applied_rows: -1,
                    ..
                })
            ));

            let mut moved = 0;
            loop {
                send!(SelectionAction::AutoscrollUp {
                    position: position(size, 0, 0),
                });
                let ServerMessage::ScrollOutcome(session::ScrollOutcome::Viewport {
                    applied_rows,
                    frame,
                    next,
                    ..
                }) = flush_message(&mut client, &mut peer)?
                else {
                    return Err("selection scroll did not return an authoritative frame".into());
                };
                assert_eq!(frame.revision, presentation.revision);
                assert_eq!(selection.route, Some(PointerRoute::Host));
                assert!(frame.rows[0].cells.iter().any(|cell| cell.style.selected));
                if applied_rows == 0 {
                    assert!(matches!(
                        next,
                        PreviewOutcome::Viewport {
                            edge_reached: true,
                            ..
                        }
                    ));
                    break;
                }
                assert_eq!(applied_rows, -1);
                moved += 1;
                assert!(moved < 16);
            }
            assert!(moved >= 8, "selection did not cross two viewports");
            assert_eq!(terminal.scrollbar()?.offset, 0);
            send!(SelectionAction::Finish {
                position: position(size, 0, 0),
                modifiers: Modifiers::empty(),
            });
            assert!(matches!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::SelectionFinished { .. }
            ));
            assert!(matches!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::Frame(_)
            ));
            assert_eq!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::CopiedText {
                    location: session::ClipboardLocation::Selection,
                    text: match anchor_end {
                        Some(end) => format!("{}\n{end}", lines[..14].join("\n")),
                        None => lines[..14].join("\n"),
                    },
                }
            );
            assert!(writes.borrow().is_empty());
        }
        Ok(())
    }

    #[test]
    fn authoritative_left_pointer_route_is_pinned_and_shift_selects() -> Result {
        let (mut client, mut peer) = attached_client()?;
        let pty = Pty::without_child_for_test()?;
        let mut terminal = terminal()?;
        terminal.vt_write(b"alpha beta\r\n\x1b[?1000h\x1b[?1006h");
        let mut size = INITIAL_SIZE;
        let writes = RefCell::new(VecDeque::new());
        let mut presentation = Presentation::new()?;
        presentation.revision = 1;
        let mut selection = SelectionState::default();

        macro_rules! send {
            ($action:expr) => {{
                let action = $action;
                let finishing = matches!(&action, SelectionAction::Finish { .. });
                assert!(handle_client_message(
                    &mut client,
                    ClientMessage::Selection(action),
                    &mut terminal,
                    Some(&pty),
                    &mut size,
                    &writes,
                    &mut presentation,
                    &mut selection,
                )?);
                assert_eq!(
                    flush_message(&mut client, &mut peer)?,
                    if finishing {
                        ServerMessage::SelectionFinished {
                            frame_revision: presentation.revision,
                        }
                    } else {
                        ServerMessage::Accepted
                    }
                );
            }};
        }

        let selected = selection_between(&terminal, (0, 0), (0, 0))?;
        terminal.set_selection(Some(&selected))?;
        selection.has_selection = true;
        terminal.set_mode(Mode::SYNC_OUTPUT, true)?;
        let stable_revision = presentation.revision;
        send!(SelectionAction::Begin {
            frame_revision: stable_revision,
            position: position(size, 0, 0),
            time_ns: 1,
            modifiers: Modifiers::empty(),
        });
        assert_eq!(selection.route, Some(PointerRoute::Terminal));
        assert!(handle_client_message(
            &mut client,
            ClientMessage::Selection(SelectionAction::AutoscrollUp {
                position: position(size, 0, 0),
            }),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Failure(Failure {
                code: FailureCode::InvalidInput,
                ..
            })
        ));
        assert_eq!(presentation.revision, stable_revision + 1);
        assert_eq!(
            writes.take().into_iter().collect::<Vec<_>>(),
            b"\x1b[<0;1;1M"
        );
        send!(SelectionAction::Update {
            position: SelectionPosition {
                x: size.screen_width as f32 + 1.0,
                y: position(size, 0, 0).y,
            },
            modifiers: Modifiers::empty(),
        });
        assert_eq!(selection.route, Some(PointerRoute::Terminal));
        assert!(writes.borrow().is_empty());

        apply_pty_output(&mut terminal, &mut selection, b"\x1b[?1000l\x1b[?1006l")?;
        send!(SelectionAction::Update {
            position: position(size, 4, 0),
            modifiers: Modifiers::SHIFT,
        });
        assert_eq!(selection.route, Some(PointerRoute::Terminal));
        assert!(writes.borrow().is_empty());

        apply_pty_output(&mut terminal, &mut selection, b"\x1b[?1000h\x1b[?1006h")?;
        send!(SelectionAction::Finish {
            position: position(size, 4, 0),
            modifiers: Modifiers::empty(),
        });
        assert!(selection.route.is_none());
        assert_eq!(presentation.revision, stable_revision + 1);
        assert_eq!(
            writes.take().into_iter().collect::<Vec<_>>(),
            b"\x1b[<0;5;1m"
        );
        terminal.set_mode(Mode::SYNC_OUTPUT, false)?;
        assert!(presentation.publish(Some(&mut client), &terminal)?);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Frame(frame) if frame.revision == presentation.revision
        ));

        send!(SelectionAction::Begin {
            frame_revision: presentation.revision,
            position: position(size, 0, 0),
            time_ns: 2,
            modifiers: Modifiers::empty(),
        });
        assert_eq!(
            writes.take().into_iter().collect::<Vec<_>>(),
            b"\x1b[<0;1;1M"
        );
        send!(SelectionAction::Cancel);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Frame(_)
        ));
        assert!(selection.route.is_none());
        assert!(writes.borrow().is_empty());

        send!(SelectionAction::Begin {
            frame_revision: presentation.revision,
            position: position(size, 0, 0),
            time_ns: 3,
            modifiers: Modifiers::SHIFT,
        });
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Frame(_)
        ));
        assert_eq!(selection.route, Some(PointerRoute::Host));
        send!(SelectionAction::Update {
            position: position(size, 5, 0),
            modifiers: Modifiers::empty(),
        });
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Frame(_)
        ));
        terminal.set_mode(Mode::SYNC_OUTPUT, true)?;
        send!(SelectionAction::Finish {
            position: position(size, 5, 0),
            modifiers: Modifiers::empty(),
        });
        let required_revision = presentation.revision;
        assert_eq!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::CopiedText {
                location: session::ClipboardLocation::Selection,
                text: "alpha".into(),
            }
        );
        terminal.set_mode(Mode::SYNC_OUTPUT, false)?;
        assert!(presentation.publish(Some(&mut client), &terminal)?);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Frame(frame) if frame.revision == required_revision
        ));
        assert!(writes.borrow().is_empty());

        assert!(handle_client_message(
            &mut client,
            ClientMessage::Selection(SelectionAction::Copy),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::CopiedText {
                location: session::ClipboardLocation::Standard,
                text: "alpha".into(),
            }
        );

        let selected = selection_between(&terminal, (0, 0), (0, 0))?;
        terminal.set_selection(Some(&selected))?;
        selection.has_selection = true;
        let saved_revision = presentation.revision;
        presentation.revision = u64::MAX;
        assert!(handle_client_message(
            &mut client,
            ClientMessage::Selection(SelectionAction::Begin {
                frame_revision: presentation.revision,
                position: position(size, 0, 0),
                time_ns: 4,
                modifiers: Modifiers::empty(),
            }),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Failure(Failure {
                code: FailureCode::Terminal,
                detail,
            }) if detail == "presentation revision exhausted"
        ));
        assert!(writes.borrow().is_empty());
        assert!(selection.has_selection);
        assert!(selection.route.is_none());
        presentation.revision = saved_revision;

        let (mut blocked, _) = attached_client()?;
        blocked.fill_output_for_test();
        assert!(!handle_client_message(
            &mut blocked,
            ClientMessage::Selection(SelectionAction::Begin {
                frame_revision: presentation.revision,
                position: position(size, 0, 0),
                time_ns: 5,
                modifiers: Modifiers::empty(),
            }),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert!(writes.borrow().is_empty());
        assert!(selection.route.is_none());

        writes.borrow_mut().resize(MAX_PTY_WRITE_BYTES, b'x');
        assert!(handle_client_message(
            &mut client,
            ClientMessage::Selection(SelectionAction::Begin {
                frame_revision: presentation.revision,
                position: position(size, 0, 0),
                time_ns: 6,
                modifiers: Modifiers::empty(),
            }),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(writes.borrow().len(), MAX_PTY_WRITE_BYTES);
        assert!(selection.route.is_none());
        assert!(selection.has_selection);
        assert_eq!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Failure(Failure {
                code: FailureCode::Terminal,
                detail: "PTY input queue is full".into(),
            })
        );

        writes.borrow_mut().clear();
        let (mut pressured, mut pressured_peer) = attached_client()?;
        assert!(handle_client_message(
            &mut pressured,
            ClientMessage::Selection(SelectionAction::Begin {
                frame_revision: presentation.revision,
                position: position(size, 0, 0),
                time_ns: 7,
                modifiers: Modifiers::SHIFT,
            }),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(
            flush_message(&mut pressured, &mut pressured_peer)?,
            ServerMessage::Accepted
        );
        assert!(matches!(
            flush_message(&mut pressured, &mut pressured_peer)?,
            ServerMessage::Frame(_)
        ));
        let pressure = ServerMessage::Failure(Failure {
            code: FailureCode::Terminal,
            detail: "x".repeat(session::MAX_FAILURE_BYTES),
        });
        while pressured.can_push_selection_result() {
            assert!(pressured.push_message(&pressure)?);
        }
        assert!(pressured.can_push_result_frame());
        let stable_revision = presentation.revision;
        assert!(handle_client_message(
            &mut pressured,
            ClientMessage::Selection(SelectionAction::Finish {
                position: position(size, 4, 0),
                modifiers: Modifiers::empty(),
            }),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(presentation.revision, stable_revision);
        assert_eq!(selection.route, Some(PointerRoute::Host));
        assert!(selection.has_selection);
        assert!(selection.copied.is_none());
        assert!(writes.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn active_selection_tracks_scrolling_pty_output() -> Result {
        let size = SurfaceSize {
            cols: 12,
            rows: 4,
            screen_width: 12 * INITIAL_SIZE.cell_width,
            screen_height: 4 * INITIAL_SIZE.cell_height,
            ..INITIAL_SIZE
        };
        let mut terminal = terminal_with_scrollback(4096)?;
        terminal.resize(size.cols, size.rows, size.cell_width, size.cell_height)?;
        terminal.vt_write(b"alpha beta\r\nsecond\r\nthird\r\nfourth");
        let mut selection = SelectionState::default();

        macro_rules! gesture {
            ($action:expr) => {{
                let action = $action;
                let position = match action {
                    SelectionAction::Begin { position, .. }
                    | SelectionAction::Update { position, .. }
                    | SelectionAction::Finish { position, .. } => position,
                    SelectionAction::AutoscrollUp { .. }
                    | SelectionAction::Cancel
                    | SelectionAction::Copy => unreachable!(),
                };
                let point = viewport_point(position, size).expect("test position is in bounds");
                let grid_ref = terminal.grid_ref(Point::Viewport(point))?;
                apply_selection_gesture(action, grid_ref, &terminal, size, &mut selection)?;
            }};
        }

        gesture!(SelectionAction::Begin {
            frame_revision: 1,
            position: position(size, 0, 0),
            time_ns: 1,
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Update {
            position: position(size, 5, 0),
            modifiers: Modifiers::empty(),
        });
        apply_pty_output(&mut terminal, &mut selection, b"\r\nlive")?;
        assert_eq!(selection.route, Some(PointerRoute::Host));
        assert_eq!(format_selection(&terminal)?, "alpha");
        gesture!(SelectionAction::Finish {
            position: position(size, 6, 0),
            modifiers: Modifiers::empty(),
        });
        assert_eq!(selection.copied.as_deref(), Some("alpha beta\nsecond"));
        apply_pty_output(&mut terminal, &mut selection, b"\x1b[?1000h\x1b[?1006h")?;
        assert!(!selection.has_selection);
        assert_eq!(selection.copied.as_deref(), Some("alpha beta\nsecond"));
        assert_eq!(
            selection
                .gesture
                .as_ref()
                .expect("selection created a gesture")
                .gesture
                .click_count(&terminal)?,
            0
        );
        Ok(())
    }

    #[test]
    fn authoritative_selection_click_ladder_preserves_wrapped_text_and_copy() -> Result {
        let (mut client, mut peer) = attached_client()?;
        let mut terminal = terminal()?;
        let size = SurfaceSize {
            cols: 16,
            rows: 4,
            screen_width: 16 * INITIAL_SIZE.cell_width,
            screen_height: 4 * INITIAL_SIZE.cell_height,
            ..INITIAL_SIZE
        };
        terminal.resize(size.cols, size.rows, size.cell_width, size.cell_height)?;
        terminal.vt_write(b"prefix alpha:beta/gamma suffix");
        let writes = RefCell::new(VecDeque::new());
        let mut presentation = Presentation::new()?;
        presentation.revision = 1;
        let mut selection = SelectionState::default();

        macro_rules! send {
            ($action:expr) => {{
                let action = $action;
                let finishing = matches!(action, SelectionAction::Finish { .. });
                assert!(handle_selection(
                    &mut client,
                    action,
                    &terminal,
                    size,
                    &writes,
                    &mut presentation,
                    &mut selection,
                )?);
                assert_eq!(
                    flush_message(&mut client, &mut peer)?,
                    if finishing {
                        ServerMessage::SelectionFinished {
                            frame_revision: presentation.revision,
                        }
                    } else {
                        ServerMessage::Accepted
                    }
                );
                let ServerMessage::Frame(frame) = flush_message(&mut client, &mut peer)? else {
                    return Err("selection did not publish a frame".into());
                };
                assert_eq!(frame.revision, presentation.revision);
                frame
            }};
        }

        let click = position(size, 14, 0);
        for (time_ns, expected, selected) in [
            (1_000_000_000, None, 0..0),
            (1_100_000_000, Some("beta/gamma"), 13..23),
            (1_200_000_000, Some("alpha:beta/gamma"), 7..23),
            (1_300_000_000, Some("prefix alpha:beta/gamma suffix"), 0..30),
            (1_400_000_000, Some("prefix alpha:beta/gamma suffix"), 0..30),
            // A late press after the saturated sequence must start over.
            (2_000_000_000, None, 0..0),
            (2_100_000_000, Some("beta/gamma"), 13..23),
            // Reset while the next press would otherwise choose span events.
            (3_000_000_000, None, 0..0),
            (3_100_000_000, Some("beta/gamma"), 13..23),
        ] {
            send!(SelectionAction::Begin {
                frame_revision: presentation.revision,
                position: click,
                time_ns,
                modifiers: Modifiers::empty(),
            });
            // Compatible output between press and release preserves the ladder.
            apply_pty_output(&mut terminal, &mut selection, b"\x1b[4;1Hlive")?;
            let frame = send!(SelectionAction::Finish {
                position: click,
                modifiers: Modifiers::empty(),
            });
            assert!(frame.rows[0].wrapped && frame.rows[1].wrap_continuation);
            for (index, cell) in frame.rows.iter().flat_map(|row| &row.cells).enumerate() {
                assert_eq!(
                    cell.style.selected,
                    selected.contains(&index),
                    "cell {index}, time {time_ns}"
                );
            }
            assert_eq!(selection.copied.as_deref(), expected);
            if let Some(text) = expected {
                assert_eq!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::CopiedText {
                        location: session::ClipboardLocation::Selection,
                        text: text.into(),
                    }
                );
                apply_pty_output(&mut terminal, &mut selection, b"\x1b[4;1Hnext")?;
                assert!(handle_selection(
                    &mut client,
                    SelectionAction::Copy,
                    &terminal,
                    size,
                    &writes,
                    &mut presentation,
                    &mut selection
                )?);
                assert_eq!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::CopiedText {
                        location: session::ClipboardLocation::Standard,
                        text: text.into(),
                    }
                );
            }
            assert!(writes.borrow().is_empty());
        }
        Ok(())
    }

    #[test]
    fn authoritative_selection_uses_libghostty_click_and_drag_granularity() -> Result {
        let mut terminal = terminal()?;
        let size = SurfaceSize {
            cols: 12,
            rows: 4,
            screen_width: 12 * INITIAL_SIZE.cell_width,
            screen_height: 4 * INITIAL_SIZE.cell_height,
            ..INITIAL_SIZE
        };
        terminal.resize(size.cols, size.rows, size.cell_width, size.cell_height)?;
        terminal.vt_write(b"alpha beta\r\nsecond line xx");
        let mut selection = SelectionState::default();

        macro_rules! gesture {
            ($action:expr) => {{
                let action = $action;
                let position = match action {
                    SelectionAction::Begin { position, .. }
                    | SelectionAction::Update { position, .. }
                    | SelectionAction::Finish { position, .. } => position,
                    SelectionAction::AutoscrollUp { .. }
                    | SelectionAction::Cancel
                    | SelectionAction::Copy => {
                        unreachable!("test applies pointer gestures only")
                    }
                };
                let point = viewport_point(position, size).expect("test position is in bounds");
                let grid_ref = terminal.grid_ref(Point::Viewport(point))?;
                apply_selection_gesture(action, grid_ref, &terminal, size, &mut selection)?;
            }};
        }

        gesture!(SelectionAction::Begin {
            frame_revision: 1,
            position: position(size, 0, 0),
            time_ns: 1_000_000_000,
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Update {
            position: position(size, 5, 0),
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Finish {
            position: position(size, 5, 0),
            modifiers: Modifiers::empty(),
        });
        assert_eq!(selection.copied.as_deref(), Some("alpha"));

        gesture!(SelectionAction::Begin {
            frame_revision: 2,
            position: position(size, 1, 0),
            time_ns: 2_000_000_000,
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Finish {
            position: position(size, 1, 0),
            modifiers: Modifiers::empty(),
        });
        apply_pty_output(&mut terminal, &mut selection, b"\x1b[4;1Hlive output")?;
        gesture!(SelectionAction::Begin {
            frame_revision: 3,
            position: position(size, 1, 0),
            time_ns: 2_100_000_000,
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Update {
            position: position(size, 9, 0),
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Finish {
            position: position(size, 9, 0),
            modifiers: Modifiers::empty(),
        });
        assert_eq!(selection.copied.as_deref(), Some("alpha beta"));

        apply_pty_output(&mut terminal, &mut selection, b"\x1b[4;1Hmore output")?;
        gesture!(SelectionAction::Begin {
            frame_revision: 4,
            position: position(size, 1, 0),
            time_ns: 2_200_000_000,
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Update {
            position: position(size, 5, 1),
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Finish {
            position: position(size, 5, 1),
            modifiers: Modifiers::empty(),
        });
        assert_eq!(selection.copied.as_deref(), Some("alpha beta\nsecond"));

        gesture!(SelectionAction::Begin {
            frame_revision: 5,
            position: position(size, 1, 0),
            time_ns: 2_300_000_000,
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Update {
            position: position(size, 5, 1),
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Finish {
            position: position(size, 5, 1),
            modifiers: Modifiers::empty(),
        });
        assert_eq!(
            selection.copied.as_deref(),
            Some("alpha beta\nsecond line xx")
        );

        gesture!(SelectionAction::Begin {
            frame_revision: 5,
            position: position(size, 1, 0),
            time_ns: 2_000_000_000,
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Update {
            position: position(size, 2, 0),
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Finish {
            position: position(size, 2, 0),
            modifiers: Modifiers::empty(),
        });
        assert_eq!(selection.copied.as_deref(), Some("l"));

        gesture!(SelectionAction::Begin {
            frame_revision: 6,
            position: position(size, 0, 0),
            time_ns: 3_000_000_000,
            modifiers: Modifiers::empty(),
        });
        gesture!(SelectionAction::Update {
            position: position(size, 5, 0),
            modifiers: Modifiers::empty(),
        });
        assert!(selection.has_selection);
        gesture!(SelectionAction::Update {
            position: position(size, 0, 0),
            modifiers: Modifiers::empty(),
        });
        assert!(!selection.has_selection);
        gesture!(SelectionAction::Finish {
            position: position(size, 0, 0),
            modifiers: Modifiers::empty(),
        });
        assert!(selection.copied.is_none());

        for (text, column, expected) in [
            ("a::b,,c", 1, Some("a::b,,c")),
            ("a  b", 1, Some("")),
            ("a\tb", 8, Some("b")),
            ("a\tb", 2, None),
            ("a\u{a0}b", 1, Some("a\u{a0}b")),
            ("a;b,e\u{301}", 0, Some("a;b,e\u{301}")),
        ] {
            clear_pointer_sequence(&terminal, &mut selection, true)?;
            terminal.vt_write(b"\x1b[2J\x1b[H");
            terminal.vt_write(text.as_bytes());
            for click in 0..3 {
                gesture!(SelectionAction::Begin {
                    frame_revision: 1,
                    position: position(size, column, 0),
                    time_ns: 1_000_000_000 + click * 100_000_000,
                    modifiers: Modifiers::empty(),
                });
                gesture!(SelectionAction::Finish {
                    position: position(size, column, 0),
                    modifiers: Modifiers::empty(),
                });
            }
            assert_eq!(selection.copied.as_deref(), expected, "{text:?}");
        }
        Ok(())
    }

    #[test]
    fn host_selection_supersedes_unpresented_drag_frames_before_release() -> Result {
        let (mut client, mut peer) = attached_client()?;
        let pty = Pty::without_child_for_test()?;
        let mut terminal = terminal()?;
        terminal.vt_write(b"alpha beta\r\n");
        let mut size = INITIAL_SIZE;
        let writes = RefCell::new(VecDeque::new());
        let mut presentation = Presentation::new()?;
        presentation.revision = 1;
        let mut selection = SelectionState::default();
        let actions = [
            SelectionAction::Begin {
                frame_revision: 1,
                position: position(size, 0, 0),
                time_ns: 1,
                modifiers: Modifiers::empty(),
            },
            SelectionAction::Update {
                position: position(size, 2, 0),
                modifiers: Modifiers::empty(),
            },
            SelectionAction::Update {
                position: position(size, 5, 0),
                modifiers: Modifiers::empty(),
            },
            SelectionAction::Finish {
                position: position(size, 5, 0),
                modifiers: Modifiers::empty(),
            },
        ];

        for action in actions {
            assert!(handle_client_message(
                &mut client,
                ClientMessage::Selection(action),
                &mut terminal,
                Some(&pty),
                &mut size,
                &writes,
                &mut presentation,
                &mut selection,
            )?);
        }

        for _ in 0..3 {
            assert_eq!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::Accepted
            );
        }
        assert_eq!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::SelectionFinished {
                frame_revision: presentation.revision,
            }
        );
        let ServerMessage::Frame(frame) = flush_message(&mut client, &mut peer)? else {
            return Err("selection release did not publish its final frame".into());
        };
        assert_eq!(frame.revision, presentation.revision);
        assert_eq!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::CopiedText {
                location: session::ClipboardLocation::Selection,
                text: "alpha".into(),
            }
        );
        Ok(())
    }

    #[test]
    fn conformance_c8_authoritative_viewport_routes_wheel_and_key_from_terminal_state() -> Result {
        let (mut client, mut peer) = attached_client()?;
        let pty = Pty::without_child_for_test()?;
        let mut terminal = terminal_with_scrollback(100)?;
        for line in 0..32 {
            terminal.vt_write(format!("history-{line:02}\r\n").as_bytes());
        }
        let mut size = INITIAL_SIZE;
        let writes = RefCell::new(VecDeque::new());
        let mut presentation = Presentation::new()?;
        let mut selection = SelectionState::default();
        macro_rules! accept {
            ($message:expr, $frame:expr) => {{
                let previous_revision = presentation.revision;
                assert!(handle_client_message(
                    &mut client,
                    $message,
                    &mut terminal,
                    Some(&pty),
                    &mut size,
                    &writes,
                    &mut presentation,
                    &mut selection,
                )?);
                assert_eq!(
                    presentation.revision,
                    previous_revision + u64::from($frame)
                );
                assert_eq!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::Accepted
                );
                if $frame {
                    assert!(matches!(
                        flush_message(&mut client, &mut peer)?,
                        ServerMessage::Frame(frame) if frame.revision == presentation.revision
                    ));
                }
            }};
        }
        macro_rules! viewport_wheel {
            ($button:expr, $applied:expr) => {{
                let previous_revision = presentation.revision;
                assert!(handle_client_message(
                    &mut client,
                    wheel($button),
                    &mut terminal,
                    Some(&pty),
                    &mut size,
                    &writes,
                    &mut presentation,
                    &mut selection,
                )?);
                assert_eq!(presentation.revision, previous_revision + 1);
                assert!(matches!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::WheelOutcome(WheelOutcome::Viewport {
                        applied_rows,
                        frame,
                    }) if applied_rows == $applied && frame.revision == presentation.revision
                ));
            }};
        }
        macro_rules! terminal_wheel {
            ($button:expr) => {{
                let previous_revision = presentation.revision;
                assert!(handle_client_message(
                    &mut client,
                    wheel($button),
                    &mut terminal,
                    Some(&pty),
                    &mut size,
                    &writes,
                    &mut presentation,
                    &mut selection,
                )?);
                assert_eq!(presentation.revision, previous_revision);
                assert_eq!(
                    flush_message(&mut client, &mut peer)?,
                    ServerMessage::WheelOutcome(WheelOutcome::TerminalRouted)
                );
            }};
        }

        let live = terminal.scrollbar()?;
        assert_eq!(live.offset + live.len, live.total);
        let stable_revision = presentation.revision;
        let stable_offset = live.offset;
        {
            let selected = selection_between(&terminal, (0, 0), (1, 0))?;
            terminal.set_selection(Some(&selected))?;
        }
        selection.route = Some(PointerRoute::Host);
        selection.has_selection = true;
        assert!(handle_client_message(
            &mut client,
            ClientMessage::PreviewVertical {
                frame_revision: stable_revision,
                direction: VerticalDirection::Up,
            },
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        let ServerMessage::VerticalPreview(VerticalPreview {
            frame_revision,
            direction: VerticalDirection::Up,
            outcome:
                PreviewOutcome::Viewport {
                    cols: 80,
                    edge_reached,
                    rows,
                },
        }) = flush_message(&mut client, &mut peer)?
        else {
            return Err("upward preview did not return one row window".into());
        };
        assert_eq!(frame_revision, stable_revision);
        let window = u64::from(INITIAL_SIZE.rows);
        assert_eq!(rows.len(), usize::try_from(stable_offset.min(window))?);
        assert_eq!(edge_reached, stable_offset <= window);
        assert!(
            rows.iter()
                .flat_map(|row| &row.cells)
                .all(|cell| !cell.style.selected)
        );
        assert_eq!(presentation.revision, stable_revision);
        assert_eq!(terminal.scrollbar()?.offset, stable_offset);
        assert!(selection.has_selection);

        assert!(handle_client_message(
            &mut client,
            ClientMessage::PreviewVertical {
                frame_revision: stable_revision,
                direction: VerticalDirection::Down,
            },
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::VerticalPreview(VerticalPreview {
                outcome: PreviewOutcome::Viewport {
                    edge_reached: true,
                    rows,
                    ..
                },
                ..
            }) if rows.is_empty()
        ));

        assert!(handle_client_message(
            &mut client,
            ClientMessage::PreviewVertical {
                frame_revision: stable_revision + 1,
                direction: VerticalDirection::Up,
            },
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Failure(Failure {
                code: FailureCode::InvalidInput,
                ..
            })
        ));
        assert_eq!(presentation.revision, stable_revision);
        assert_eq!(terminal.scrollbar()?.offset, stable_offset);

        viewport_wheel!(MouseButton::Four, -1);
        assert!(!selection.has_selection);
        let scrolled = terminal.scrollbar()?;
        assert_eq!(scrolled.offset + 1, live.offset);

        terminal.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Top);
        viewport_wheel!(MouseButton::Four, 0);
        assert_eq!(terminal.scrollbar()?.offset, 0);

        viewport_wheel!(MouseButton::Five, 1);
        assert_eq!(terminal.scrollbar()?.offset, 1);

        terminal.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Bottom);
        viewport_wheel!(MouseButton::Five, 0);
        let live = terminal.scrollbar()?;
        assert_eq!(live.offset + live.len, live.total);

        terminal.vt_write(b"\x1b[?1000h\x1b[?1006h");
        let tracked_revision = presentation.revision;
        assert!(handle_client_message(
            &mut client,
            ClientMessage::PreviewVertical {
                frame_revision: tracked_revision,
                direction: VerticalDirection::Up,
            },
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::VerticalPreview(VerticalPreview {
                frame_revision: tracked_revision,
                direction: VerticalDirection::Up,
                outcome: PreviewOutcome::TerminalRouted,
            })
        );
        assert!(writes.borrow().is_empty());
        terminal_wheel!(MouseButton::Four);
        assert_eq!(
            writes.take().into_iter().collect::<Vec<_>>(),
            b"\x1b[<64;1;1M"
        );
        accept!(wheel(MouseButton::Six), false);
        assert_eq!(
            writes.take().into_iter().collect::<Vec<_>>(),
            b"\x1b[<66;1;1M"
        );

        terminal.vt_write(b"\x1b[?1000l\x1b[?1006l\x1b[?1049h\x1b[?1007h");
        for (modes, button, expected) in [
            (
                b"\x1b[?1l".as_slice(),
                MouseButton::Four,
                b"\x1b[A".as_slice(),
            ),
            (
                b"\x1b[?1h".as_slice(),
                MouseButton::Five,
                b"\x1bOB".as_slice(),
            ),
        ] {
            terminal.vt_write(modes);
            terminal_wheel!(button);
            assert_eq!(writes.take().into_iter().collect::<Vec<_>>(), expected);
        }

        terminal.vt_write(b"\x1b[?1007l");
        viewport_wheel!(MouseButton::Four, 0);
        assert!(writes.borrow().is_empty());

        terminal.vt_write(b"\x1b[?1049l");
        terminal.vt_write(b"\x1b[?2026h");
        let held_revision = presentation.revision;
        let held_offset = terminal.scrollbar()?.offset;
        assert!(handle_client_message(
            &mut client,
            wheel(MouseButton::Four),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Failure(Failure {
                code: FailureCode::Terminal,
                ..
            })
        ));
        assert_eq!(presentation.revision, held_revision);
        assert_eq!(terminal.scrollbar()?.offset, held_offset);
        terminal.vt_write(b"\x1b[?2026l");
        terminal.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Delta(-1));
        let key = ClientMessage::Key(KeyEvent {
            action: KeyAction::Press,
            key: PhysicalKey::A,
            modifiers: Modifiers::empty(),
            consumed_modifiers: Modifiers::empty(),
            composing: false,
            text: Some("x".into()),
            unshifted_codepoint: Some('x'),
        });
        accept!(key, true);
        assert_eq!(writes.take().into_iter().collect::<Vec<_>>(), b"x");
        let live = terminal.scrollbar()?;
        assert_eq!(live.offset + live.len, live.total);

        terminal.scroll_viewport(libghostty_vt::terminal::ScrollViewport::Delta(-1));
        let before_empty = terminal.scrollbar()?;
        accept!(
            ClientMessage::Key(KeyEvent {
                action: KeyAction::Release,
                key: PhysicalKey::ENTER,
                modifiers: Modifiers::empty(),
                consumed_modifiers: Modifiers::empty(),
                composing: false,
                text: None,
                unshifted_codepoint: None,
            }),
            false
        );
        assert!(writes.borrow().is_empty());
        assert_eq!(terminal.scrollbar()?.offset, before_empty.offset);

        assert!(queue_pty_write(
            &mut writes.borrow_mut(),
            &vec![b'q'; MAX_PTY_WRITE_BYTES],
        ));
        assert!(handle_client_message(
            &mut client,
            ClientMessage::Key(KeyEvent {
                action: KeyAction::Press,
                key: PhysicalKey::A,
                modifiers: Modifiers::empty(),
                consumed_modifiers: Modifiers::empty(),
                composing: false,
                text: Some("rejected".into()),
                unshifted_codepoint: Some('r'),
            }),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(terminal.scrollbar()?.offset, before_empty.offset);
        assert_eq!(presentation.revision, 6);
        assert!(client.is_closing());
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Failure(Failure {
                code: FailureCode::Terminal,
                ..
            })
        ));

        let (mut pressure_client, mut pressure_peer) = attached_client()?;
        terminal.vt_write(b"\x1b[?1000h\x1b[?1006h");
        let pressure_revision = presentation.revision;
        let pressure_offset = terminal.scrollbar()?.offset;
        assert!(handle_client_message(
            &mut pressure_client,
            wheel(MouseButton::Four),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(writes.borrow().len(), MAX_PTY_WRITE_BYTES);
        assert_eq!(presentation.revision, pressure_revision);
        assert_eq!(terminal.scrollbar()?.offset, pressure_offset);
        assert!(pressure_client.is_closing());
        assert!(matches!(
            flush_message(&mut pressure_client, &mut pressure_peer)?,
            ServerMessage::Failure(Failure {
                code: FailureCode::Terminal,
                ..
            })
        ));
        terminal.vt_write(b"\x1b[?1000l\x1b[?1006l");

        let (mut blocked_client, _) = attached_client()?;
        blocked_client.fill_output_for_test();
        let blocked_revision = presentation.revision;
        let blocked_offset = terminal.scrollbar()?.offset;
        assert!(!handle_client_message(
            &mut blocked_client,
            wheel(MouseButton::Four),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(presentation.revision, blocked_revision);
        assert_eq!(terminal.scrollbar()?.offset, blocked_offset);
        let resized = SurfaceSize {
            rows: 25,
            screen_height: 400,
            ..INITIAL_SIZE
        };
        assert!(!handle_client_message(
            &mut blocked_client,
            ClientMessage::Resize(resized),
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(size, resized);
        assert_eq!(presentation.revision, 7);
        Ok(())
    }

    #[test]
    fn conformance_c8_signed_scroll_batch_is_atomic_bounded_and_authoritative() -> Result {
        let (mut client, mut peer) = attached_client()?;
        let pty = Pty::without_child_for_test()?;
        let mut terminal = terminal_with_scrollback(16 * 1024 * 1024)?;
        for line in 0..120 {
            terminal.vt_write(format!("history-{line:03}\r\n").as_bytes());
        }
        let mut size = INITIAL_SIZE;
        let writes = RefCell::new(VecDeque::new());
        let mut presentation = Presentation::new()?;
        let mut selection = SelectionState::default();
        macro_rules! scroll {
            ($rows:expr) => {
                scroll!($rows, presentation.revision)
            };
            ($rows:expr, $frame_revision:expr) => {{
                let frame_revision = $frame_revision;
                assert!(handle_client_message(
                    &mut client,
                    ClientMessage::ScrollVertical {
                        frame_revision,
                        rows: $rows,
                    },
                    &mut terminal,
                    Some(&pty),
                    &mut size,
                    &writes,
                    &mut presentation,
                    &mut selection,
                )?);
                flush_message(&mut client, &mut peer)?
            }};
        }

        let live = terminal.scrollbar()?.offset;
        let selected = selection_between(&terminal, (0, 0), (1, 0))?;
        terminal.set_selection(Some(&selected))?;
        selection.route = Some(PointerRoute::Host);
        selection.has_selection = true;

        let ServerMessage::ScrollOutcome(session::ScrollOutcome::Viewport {
            requested_rows: -32,
            applied_rows: -32,
            frame,
            next,
        }) = scroll!(-32)
        else {
            return Err("signed scroll did not return one viewport result".into());
        };
        assert_eq!(frame.revision, presentation.revision);
        assert_eq!(terminal.scrollbar()?.offset + 32, live);
        assert!(!selection.has_selection);
        assert!(client.output_is_empty());

        assert!(handle_client_message(
            &mut client,
            ClientMessage::PreviewVertical {
                frame_revision: presentation.revision,
                direction: VerticalDirection::Up,
            },
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::VerticalPreview(VerticalPreview {
                direction: VerticalDirection::Up,
                outcome,
                ..
            }) if outcome == next
        ));

        let previous_revision = presentation.revision;
        assert!(matches!(
            scroll!(session::MAX_SCROLL_ROWS),
            ServerMessage::ScrollOutcome(session::ScrollOutcome::Viewport {
                requested_rows: session::MAX_SCROLL_ROWS,
                applied_rows: 32,
                frame,
                next: PreviewOutcome::Viewport {
                    edge_reached: true,
                    rows,
                    ..
                },
            }) if rows.is_empty() && frame.revision == previous_revision + 1
        ));
        assert_eq!(terminal.scrollbar()?.offset, live);

        let previous_revision = presentation.revision;
        let expected_to_top = -i16::try_from(live)?;
        assert!(matches!(
            scroll!(-session::MAX_SCROLL_ROWS),
            ServerMessage::ScrollOutcome(session::ScrollOutcome::Viewport {
                requested_rows,
                applied_rows,
                frame,
                next: PreviewOutcome::Viewport {
                    edge_reached: true,
                    rows,
                    ..
                },
            }) if requested_rows == -session::MAX_SCROLL_ROWS
                && applied_rows == expected_to_top
                && rows.is_empty()
                && frame.revision == previous_revision + 1
        ));
        assert_eq!(terminal.scrollbar()?.offset, 0);

        let presented_revision = presentation.revision;
        terminal.vt_write(b"continuous-one\r\n");
        presentation.advance()?;
        assert!(handle_client_message(
            &mut client,
            ClientMessage::PreviewVertical {
                frame_revision: presented_revision,
                direction: VerticalDirection::Down,
            },
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        let preview_revision = presentation.revision;
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::VerticalPreview(VerticalPreview {
                frame_revision,
                direction: VerticalDirection::Down,
                ..
            }) if frame_revision == preview_revision
        ));

        terminal.vt_write(b"continuous-two\r\n");
        presentation.advance()?;
        let current_revision = presentation.revision;
        assert!(matches!(
            scroll!(1, preview_revision),
            ServerMessage::ScrollOutcome(session::ScrollOutcome::Viewport {
                requested_rows: 1,
                applied_rows: 1,
                frame,
                ..
            }) if frame.revision == current_revision + 1
        ));

        let stable_revision = presentation.revision;
        let stable_offset = terminal.scrollbar()?.offset;
        for message in [
            ClientMessage::PreviewVertical {
                frame_revision: stable_revision + 1,
                direction: VerticalDirection::Down,
            },
            ClientMessage::ScrollVertical {
                frame_revision: stable_revision + 1,
                rows: 1,
            },
        ] {
            assert!(handle_client_message(
                &mut client,
                message,
                &mut terminal,
                Some(&pty),
                &mut size,
                &writes,
                &mut presentation,
                &mut selection,
            )?);
            assert!(matches!(
                flush_message(&mut client, &mut peer)?,
                ServerMessage::Failure(Failure {
                    code: FailureCode::InvalidInput,
                    ..
                })
            ));
            assert_eq!(presentation.revision, stable_revision);
            assert_eq!(terminal.scrollbar()?.offset, stable_offset);
        }

        terminal.vt_write(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            scroll!(-8),
            ServerMessage::ScrollOutcome(session::ScrollOutcome::TerminalOwned {
                requested_rows: -8,
            })
        );
        assert!(writes.borrow().is_empty());
        assert_eq!(presentation.revision, stable_revision);
        assert_eq!(terminal.scrollbar()?.offset, stable_offset);
        terminal.vt_write(b"\x1b[?1000l\x1b[?1006l");

        terminal.vt_write(b"\x1b[?1049h\x1b[?1007h");
        assert_eq!(
            scroll!(8),
            ServerMessage::ScrollOutcome(session::ScrollOutcome::TerminalOwned {
                requested_rows: 8,
            })
        );
        assert!(writes.borrow().is_empty());
        assert_eq!(presentation.revision, stable_revision);
        terminal.vt_write(b"\x1b[?1007l\x1b[?1049l");

        let (mut blocked, _) = attached_client()?;
        blocked.fill_output_for_test();
        assert!(!handle_client_message(
            &mut blocked,
            ClientMessage::ScrollVertical {
                frame_revision: stable_revision,
                rows: -8,
            },
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(presentation.revision, stable_revision);
        assert_eq!(terminal.scrollbar()?.offset, stable_offset);
        Ok(())
    }

    #[test]
    fn conformance_c8_return_to_live_is_one_authoritative_jump_without_pty_input() -> Result {
        let (mut client, mut peer) = attached_client()?;
        let pty = Pty::without_child_for_test()?;
        let mut terminal = terminal_with_scrollback(16 * 1024 * 1024)?;
        for line in 0..1_300 {
            terminal.vt_write(format!("history-{line:04}\r\n").as_bytes());
        }
        terminal.scroll_viewport(ScrollViewport::Top);
        terminal.vt_write(b"continued-output\r\n");
        let mut size = INITIAL_SIZE;
        let writes = RefCell::new(VecDeque::new());
        let mut presentation = Presentation::new()?;
        let mut selection = SelectionState::default();
        macro_rules! jump {
            () => {{
                assert!(handle_client_message(
                    &mut client,
                    ClientMessage::ReturnToLive,
                    &mut terminal,
                    Some(&pty),
                    &mut size,
                    &writes,
                    &mut presentation,
                    &mut selection,
                )?);
                flush_message(&mut client, &mut peer)?
            }};
        }
        assert!(
            presentation
                .extractor
                .frame(0, &terminal)?
                .scroll_position
                .rows_from_live
                > u64::from(session::MAX_SCROLL_ROWS as u16)
        );

        let selected = selection_between(&terminal, (0, 0), (1, 0))?;
        terminal.set_selection(Some(&selected))?;
        selection.has_selection = true;
        assert!(matches!(
            jump!(),
            ServerMessage::Frame(frame)
                if frame.revision == 1 && frame.scroll_position.rows_from_live == 0
        ));
        assert!(!selection.has_selection);
        assert!(writes.borrow().is_empty());

        terminal.scroll_viewport(ScrollViewport::Delta(-8));
        let before = terminal.scrollbar()?.offset;
        terminal.vt_write(b"\x1b[?1000h\x1b[?1006h");
        assert!(matches!(
            jump!(),
            ServerMessage::Failure(Failure {
                code: FailureCode::InvalidInput,
                ..
            })
        ));
        assert_eq!(terminal.scrollbar()?.offset, before);
        assert_eq!(presentation.revision, 1);
        terminal.vt_write(b"\x1b[?1000l\x1b[?1006l");

        terminal.set_mode(Mode::SYNC_OUTPUT, true)?;
        assert!(handle_client_message(
            &mut client,
            ClientMessage::ReturnToLive,
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(terminal.scrollbar()?.offset, before);
        assert!(client.output_is_empty());
        terminal.set_mode(Mode::SYNC_OUTPUT, false)?;
        let deferred = presentation
            .take_deferred_vertical()
            .ok_or("missing deferred jump")?;
        assert!(handle_client_message(
            &mut client,
            deferred,
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert!(matches!(
            flush_message(&mut client, &mut peer)?,
            ServerMessage::Frame(frame)
                if frame.revision == 2 && frame.scroll_position.rows_from_live == 0
        ));
        assert!(matches!(
            jump!(),
            ServerMessage::Frame(frame)
                if frame.revision == 3 && frame.scroll_position.rows_from_live == 0
        ));

        terminal.vt_write(b"\x1b[?1049h");
        assert!(matches!(
            jump!(),
            ServerMessage::Failure(Failure {
                code: FailureCode::InvalidInput,
                ..
            })
        ));
        assert_eq!(presentation.revision, 3);
        terminal.vt_write(b"\x1b[?1049l");

        terminal.scroll_viewport(ScrollViewport::Delta(-8));
        let before_pressure = terminal.scrollbar()?.offset;
        let (mut blocked, _) = attached_client()?;
        blocked.fill_output_for_test();
        assert!(!handle_client_message(
            &mut blocked,
            ClientMessage::ReturnToLive,
            &mut terminal,
            Some(&pty),
            &mut size,
            &writes,
            &mut presentation,
            &mut selection,
        )?);
        assert_eq!(terminal.scrollbar()?.offset, before_pressure);
        assert_eq!(presentation.revision, 3);
        assert!(writes.borrow().is_empty());
        Ok(())
    }

    #[test]
    fn outside_mouse_reporting_uses_held_button_semantics() -> Result {
        let mut terminal = terminal()?;
        terminal.vt_write(b"\x1b[?1003h\x1b[?1016h");
        let wheel = ClientMessage::Mouse(session::MouseEvent {
            action: MouseAction::Press,
            button: Some(MouseButton::Four),
            modifiers: Modifiers::empty(),
            x: 641.0,
            y: 1.0,
        });
        assert!(encode_input(&terminal, INITIAL_SIZE, wheel)?.is_empty());

        let motion = ClientMessage::Mouse(session::MouseEvent {
            action: MouseAction::Motion,
            button: Some(MouseButton::Left),
            modifiers: Modifiers::empty(),
            x: 641.0,
            y: 1.0,
        });

        assert_eq!(
            encode_input(&terminal, INITIAL_SIZE, motion)?,
            b"\x1b[<32;641;1M"
        );
        Ok(())
    }

    #[test]
    fn kitty_release_never_falls_back_to_text() -> Result {
        let mut terminal = terminal()?;
        terminal.vt_write(b"\x1b[>3u");
        let release = |key, text: &str| {
            ClientMessage::Key(KeyEvent {
                action: KeyAction::Release,
                key,
                modifiers: Modifiers::empty(),
                consumed_modifiers: Modifiers::empty(),
                composing: false,
                text: Some(text.into()),
                unshifted_codepoint: None,
            })
        };

        assert_eq!(
            encode_input(&terminal, INITIAL_SIZE, release(PhysicalKey::SPACE, " "))?,
            b"\x1b[32;1:3u"
        );
        assert!(
            encode_input(
                &terminal,
                INITIAL_SIZE,
                release(PhysicalKey::UNIDENTIFIED, "!")
            )?
            .is_empty()
        );
        Ok(())
    }

    #[test]
    fn every_protocol_physical_key_reaches_the_terminal_mapper() {
        for raw in 0..=PhysicalKey::MAX_RAW {
            assert!(
                Key::try_from(u32::from(raw)).is_ok(),
                "protocol physical key {raw} has no terminal mapping"
            );
        }
    }
}
