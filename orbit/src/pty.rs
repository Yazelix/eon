//! The PTY child and the terminal that reads its output.

use crate::{
    Timer,
    boundary::{Effect, TickState},
    errno,
    frame::{FrameExtractor, SCROLLBACK_BUDGET_BYTES, viewport_preview},
    input::encode_input,
    platform,
    pointer::{Pointer, phase_position},
};
use libghostty_vt::{
    Terminal,
    screen::Screen as GhosttyScreen,
    style::RgbColor,
    terminal::{
        ClipboardLocation as GhosttyClipboardLocation, ClipboardWrite, ClipboardWriteError, Mode,
        ScrollViewport,
    },
};
use orbit_protocol::{
    Frame,
    session::{
        self, ClientMessage, ClipboardLocation, Metadata, ScrollOutcome, ServerMessage,
        VerticalDirection, WheelOutcome,
    },
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    ffi::OsString,
    fs::File,
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, RawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, ExitStatus, Stdio},
    rc::Rc,
    thread,
    time::Duration,
};

/// Client input and terminal replies wait here for the PTY, bounded at
/// one maximum paste plus a bracketed-paste frame.
const MAX_PTY_WRITE_BYTES: usize = session::MAX_PASTE_BYTES + 16;

/// The pause before reading a hung-up PTY again.
const PTY_EIO_RETRY_DELAY: Duration = Duration::from_millis(10);

/// Bytes waiting for the PTY: client input and libghostty's replies to the
/// child's queries, in one order.
#[derive(Clone, Default)]
pub(crate) struct PtyWrites(Rc<RefCell<VecDeque<u8>>>);

impl PtyWrites {
    /// Whether `len` more bytes fit the queue.
    pub(crate) fn fits(&self, len: usize) -> bool {
        self.0.borrow().len().saturating_add(len) <= MAX_PTY_WRITE_BYTES
    }

    /// Queues bytes whole, or reports that they do not fit.
    pub(crate) fn push(&self, bytes: &[u8]) -> bool {
        if !self.fits(bytes.len()) {
            return false;
        }
        self.0.borrow_mut().extend(bytes);
        true
    }

    /// Drops every queued byte and its storage, once the PTY has closed.
    pub(crate) fn discard(&self) {
        drop(self.0.take());
    }
}

/// The bound on clipboard writes captured in one PTY turn.
const MAX_CLIPBOARD_WRITES: usize = 64;

/// The terminal's clipboard writes from one PTY turn, captured only while
/// Bend's last `Capture` turned capture on (ORB-C11). Each is
/// one nonempty UTF-8 `text/plain` text without NUL; together they stay under
/// 64 writes and one maximum copy, and a turn that passes either bound keeps
/// nothing more and marks the turn overflowed.
#[derive(Default)]
pub(crate) struct ClipboardWrites {
    enabled: bool,
    pub(crate) writes: VecDeque<(ClipboardLocation, String)>,
    pub(crate) overflowed: bool,
}

impl ClipboardWrites {
    fn capture(&mut self, write: &ClipboardWrite<'_>) -> Result<(), ClipboardWriteError> {
        if !self.enabled {
            return Err(ClipboardWriteError::Denied);
        }
        if self.overflowed {
            return Err(ClipboardWriteError::Busy);
        }
        // libghostty normalizes every destination to these three; the safe
        // API reports an unknown one as the standard clipboard.
        let location = match write.location() {
            GhosttyClipboardLocation::Standard => ClipboardLocation::Standard,
            GhosttyClipboardLocation::Selection => ClipboardLocation::Selection,
            GhosttyClipboardLocation::Primary => ClipboardLocation::Primary,
        };
        // A clear has no representation, and a write may carry several.
        let mut contents = write.contents();
        let (Some(content), None) = (contents.next(), contents.next()) else {
            return Err(ClipboardWriteError::Unsupported);
        };
        if content.mime != "text/plain" {
            return Err(ClipboardWriteError::Unsupported);
        }
        let text =
            std::str::from_utf8(content.data).map_err(|_| ClipboardWriteError::InvalidData)?;
        if text.is_empty() || text.len() > session::MAX_COPY_BYTES || text.contains('\0') {
            return Err(ClipboardWriteError::InvalidData);
        }
        let bytes: usize = self.writes.iter().map(|(_, text)| text.len()).sum();
        if self.writes.len() >= MAX_CLIPBOARD_WRITES || bytes + text.len() > session::MAX_COPY_BYTES
        {
            self.overflowed = true;
            return Err(ClipboardWriteError::Busy);
        }
        self.writes.push_back((location, text.to_owned()));
        Ok(())
    }
}

/// One read of the PTY: the bytes fed to the terminal, its close, or nothing
/// to read yet.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PtyRead {
    Bytes(usize),
    Closed,
    Blocked(RawFd),
}

pub(crate) struct Orbit {
    pub(crate) master: File,
    pub(crate) pid: u32,
    /// The direct child until it is reaped.
    pub(crate) child: Option<Child>,
    pub(crate) terminal: Terminal<'static, 'static>,
    pub(crate) extractor: FrameExtractor,
    pub(crate) hold: Timer,
    hold_armed: bool,
    surface: session::SurfaceSize,
    /// The child's exit status once it is reaped.
    reaped: Option<ExitStatus>,
    /// Bytes for the PTY, written as it drains so the host never waits on it.
    pub(crate) writes: PtyWrites,
    /// A terminal reply did not fit the queue; the session fails
    /// rather than drop part of an answer.
    reply_overflow: Rc<Cell<bool>>,
    /// The clipboard writes of the current PTY turn.
    pub(crate) clipboard: Rc<RefCell<ClipboardWrites>>,
    /// libghostty's gesture and the frozen copy.
    pub(crate) pointer: Pointer,
}

impl Orbit {
    /// Spawns the PTY child as the host's direct child. A palette replaces
    /// the terminal's default ANSI colors, which a reset restores.
    pub(crate) fn spawn(
        command: &[OsString],
        palette: Option<[RgbColor; 16]>,
        cols: u16,
        rows: u16,
    ) -> io::Result<Self> {
        let mut terminal = Terminal::new(cols, rows).map_err(|_| errno(libc::EIO))?;
        terminal
            .set_scrollback_max_bytes(Some(SCROLLBACK_BUDGET_BYTES))
            .map_err(|_| errno(libc::EIO))?;
        if let Some(colors) = palette {
            let mut defaults = terminal
                .default_color_palette()
                .map_err(|_| errno(libc::EIO))?;
            defaults.0[..16].copy_from_slice(&colors);
            terminal
                .set_default_color_palette(Some(defaults))
                .map_err(|_| errno(libc::EIO))?;
        }
        // Orbit alone answers the child's terminal queries (ORB-C2).
        let writes = PtyWrites::default();
        let reply_overflow = Rc::new(Cell::new(false));
        let (replies, overflow) = (writes.clone(), Rc::clone(&reply_overflow));
        terminal
            .on_pty_write(move |_, bytes| {
                if !replies.push(bytes) {
                    overflow.set(true);
                }
            })
            .map_err(|_| errno(libc::EIO))?;
        let clipboard = Rc::new(RefCell::new(ClipboardWrites::default()));
        let captured = Rc::clone(&clipboard);
        terminal
            .on_clipboard_write(move |_, write| {
                let result = captured.borrow_mut().capture(&write);
                write.reply(result, false);
            })
            .map_err(|_| errno(libc::EIO))?;
        let extractor = FrameExtractor::new()?;
        let hold = Timer::default();
        let (master, child) = spawn_pty_child(command, cols, rows)?;
        let pid = child.id();
        Ok(Self {
            master,
            pid,
            child: Some(child),
            terminal,
            extractor,
            hold,
            hold_armed: false,
            surface: session::SurfaceSize {
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
            },
            reaped: None,
            writes,
            reply_overflow,
            clipboard,
            pointer: Pointer::default(),
        })
    }

    /// Starts a PTY turn: no clipboard write from an earlier turn is retained
    /// or replayed.
    pub(crate) fn start_turn(&self) {
        let mut clipboard = self.clipboard.borrow_mut();
        clipboard.writes.clear();
        clipboard.overflowed = false;
    }

    /// Captures the terminal's clipboard writes from the next PTY turn on, or
    /// refuses them, as Bend's `Capture` decides.
    pub(crate) fn capture_clipboard(&self, on: bool) {
        self.clipboard.borrow_mut().enabled = on;
    }

    /// Reaps the child if it has exited and reports its exit code.
    pub(crate) fn try_reap(&mut self) -> io::Result<Option<u32>> {
        if self.reaped.is_none() {
            self.reaped = self
                .child
                .as_mut()
                .ok_or_else(|| errno(libc::ECHILD))?
                .try_wait()?;
            if self.reaped.is_some() {
                self.child = None;
            }
        }
        Ok(self.reaped.map(child_code))
    }

    /// The reaped child's exact exit status.
    pub(crate) fn exit_status(&self) -> Option<ExitStatus> {
        self.reaped
    }

    /// Feeds one PTY read to the terminal and reports its length, or that the
    /// PTY has closed or has nothing to read.
    /// Linux reports EIO while no process holds the terminal open, which a
    /// live child may do briefly before reopening it, so EIO closes the PTY
    /// only after the direct child has exited (ORB-C1); until then it waits
    /// briefly, so a hung-up PTY cannot spin.
    pub(crate) fn read_into(&mut self, data: &mut [u8]) -> io::Result<PtyRead> {
        loop {
            match self.master.read(data) {
                Ok(0) => return Ok(PtyRead::Closed),
                Ok(len) => {
                    let watched = self.pointer.watch(&self.terminal)?;
                    self.terminal.vt_write(&data[..len]);
                    if let Some(tracked_before) = watched {
                        self.pointer.after_output(&self.terminal, tracked_before)?;
                    }
                    if self.reply_overflow.get() {
                        return Err(errno(libc::ENOBUFS));
                    }
                    return Ok(PtyRead::Bytes(len));
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    return Ok(PtyRead::Blocked(self.master.as_raw_fd()));
                }
                Err(error) if error.raw_os_error() == Some(libc::EIO) => {
                    if self.try_reap()?.is_some() {
                        return Ok(PtyRead::Closed);
                    }
                    thread::sleep(PTY_EIO_RETRY_DELAY);
                    return Ok(PtyRead::Blocked(self.master.as_raw_fd()));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Whether queued bytes wait for the PTY to drain.
    pub(crate) fn writes_pending(&self) -> bool {
        !self.writes.0.borrow().is_empty()
    }

    /// Writes queued bytes until the PTY would block. A closed PTY discards
    /// them; the read side reports the close.
    pub(crate) fn flush_writes(&mut self) -> io::Result<()> {
        let mut writes = self.writes.0.borrow_mut();
        while !writes.is_empty() {
            let (front, back) = writes.as_slices();
            let bytes = if front.is_empty() { back } else { front };
            match self.master.write(bytes) {
                Ok(0) => writes.clear(),
                Ok(written) => drop(writes.drain(..written)),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.raw_os_error() == Some(libc::EIO) => writes.clear(),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    fn resize(&mut self, size: session::SurfaceSize) -> io::Result<()> {
        platform::set_window_size(
            self.master.as_raw_fd(),
            size.cols,
            size.rows,
            u16::try_from(size.screen_width).unwrap_or(u16::MAX),
            u16::try_from(size.screen_height).unwrap_or(u16::MAX),
        )?;
        self.terminal
            .resize(size.cols, size.rows, size.cell_width, size.cell_height)
            .map_err(|_| errno(libc::EIO))?;
        self.surface = size;
        Ok(())
    }

    /// Reports libghostty's synchronized-output mode. The bound is armed once
    /// while the mode is set and cleared when it is not.
    pub(crate) fn synchronized(&mut self, bound_ms: u32) -> io::Result<bool> {
        if bound_ms == 0 {
            return Err(errno(libc::EINVAL));
        }
        let held = self
            .terminal
            .mode(Mode::SYNC_OUTPUT)
            .map_err(|_| errno(libc::EIO))?;
        if held != self.hold_armed {
            self.hold.set(held.then_some(bound_ms));
            self.hold_armed = held;
        }
        Ok(held)
    }

    /// Ends a synchronized update and clears its bound; reports whether one was held.
    pub(crate) fn release(&mut self) -> io::Result<bool> {
        let held = self
            .terminal
            .mode(Mode::SYNC_OUTPUT)
            .map_err(|_| errno(libc::EIO))?;
        if held {
            self.terminal
                .set_mode(Mode::SYNC_OUTPUT, false)
                .map_err(|_| errno(libc::EIO))?;
        }
        if self.hold_armed {
            self.hold.set(None);
            self.hold_armed = false;
        }
        Ok(held)
    }

    /// Whether the terminal tracks the mouse, so a pointer sequence without
    /// Shift goes to it.
    pub(crate) fn tracking(&self) -> io::Result<bool> {
        self.terminal
            .is_mouse_tracking()
            .map_err(|_| errno(libc::EIO))
    }

    /// The mouse input a pointer phase sends the terminal on its route: a left
    /// press, motion or release at its position.
    pub(crate) fn pointer_input(&self, action: session::SelectionAction) -> io::Result<Vec<u8>> {
        let position = phase_position(action).ok_or_else(|| errno(libc::EINVAL))?;
        let (mouse_action, modifiers) = match action {
            session::SelectionAction::Begin { modifiers, .. } => {
                (session::MouseAction::Press, modifiers)
            }
            session::SelectionAction::Update { modifiers, .. } => {
                (session::MouseAction::Motion, modifiers)
            }
            session::SelectionAction::Finish { modifiers, .. } => {
                (session::MouseAction::Release, modifiers)
            }
            _ => return Err(errno(libc::EINVAL)),
        };
        let input = ClientMessage::Mouse(session::MouseEvent {
            action: mouse_action,
            button: Some(session::MouseButton::Left),
            modifiers,
            x: position.x,
            y: position.y,
        });
        encode_input(&self.terminal, self.surface, input)
    }

    /// Carries out a host phase; reports whether a selection is presented
    /// after it, or None if it failed.
    pub(crate) fn select_host(&mut self, action: session::SelectionAction) -> Option<bool> {
        self.pointer
            .host_phase(action, &self.terminal, self.surface)
            .ok()
    }

    /// Whether an upward selection tick from this position may run on the
    /// held gesture: only from the top row, while its anchor is valid (fact
    /// Tick).
    pub(crate) fn tick_state(&self, position: session::SelectionPosition) -> io::Result<TickState> {
        Ok(
            if crate::pointer::viewport_point(position, self.surface)
                .is_none_or(|point| point.y != 0)
            {
                TickState::NotTopRow
            } else if !self.pointer.anchored(&self.terminal)? {
                TickState::AnchorLost
            } else {
                TickState::Fits
            },
        )
    }

    /// Carries out one upward selection tick: the viewport and the tracked
    /// selection move at most one row, and one scroll outcome carries the
    /// frame at `revision` and the next window up, with whether a selection is
    /// presented after it. None if the gesture failed, which clears it.
    pub(crate) fn tick(
        &mut self,
        position: session::SelectionPosition,
        revision: u64,
    ) -> io::Result<Option<(ServerMessage, bool)>> {
        let offset = |terminal: &Terminal<'static, 'static>| {
            terminal
                .scrollbar()
                .map(|scrollbar| scrollbar.offset)
                .map_err(|_| errno(libc::EIO))
        };
        let before = offset(&self.terminal)?;
        let Ok(presented) = self.pointer.tick(&self.terminal, position, self.surface) else {
            return Ok(None);
        };
        let applied_rows = i16::try_from(i128::from(offset(&self.terminal)?) - i128::from(before))
            .ok()
            .filter(|rows| matches!(rows, -1..=0))
            .ok_or_else(|| errno(libc::EIO))?;
        let frame = self.frame(revision)?;
        let next = viewport_preview(&self.terminal, &self.extractor, VerticalDirection::Up)?;
        Ok(Some((
            ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
                requested_rows: -1,
                applied_rows,
                frame: Box::new(frame),
                next,
            }),
            presented,
        )))
    }

    /// Whether a host phase's position lies outside the viewport.
    pub(crate) fn outside(&self, action: session::SelectionAction) -> bool {
        phase_position(action)
            .and_then(|position| crate::pointer::viewport_point(position, self.surface))
            .is_none()
    }

    /// Whether the alternate screen is active.
    pub(crate) fn alternate(&self) -> io::Result<bool> {
        Ok(matches!(
            self.terminal
                .active_screen()
                .map_err(|_| errno(libc::EIO))?,
            GhosttyScreen::Alternate
        ))
    }

    /// Whether mouse tracking or alternate-screen mode 1007 routes the
    /// vertical wheel to the terminal.
    pub(crate) fn routed(&self) -> io::Result<bool> {
        let terminal = &self.terminal;
        Ok(terminal.is_mouse_tracking().map_err(|_| errno(libc::EIO))?
            || self.alternate()?
                && terminal
                    .mode(Mode::ALT_SCROLL)
                    .map_err(|_| errno(libc::EIO))?)
    }

    /// The observer's view at `revision`: the title and working directory.
    pub(crate) fn metadata(&self, revision: u64) -> io::Result<Metadata> {
        let (title, working_directory) = self.title_and_directory()?;
        Ok(Metadata {
            revision,
            title: title.to_owned(),
            working_directory: working_directory.to_owned(),
        })
    }

    /// The terminal's title and working directory.
    pub(crate) fn title_and_directory(&self) -> io::Result<(&str, &str)> {
        Ok((
            self.terminal.title().map_err(|_| errno(libc::EIO))?,
            self.terminal.pwd().map_err(|_| errno(libc::EIO))?,
        ))
    }

    /// The current terminal at `revision`, which Bend names (ORB-C4).
    pub(crate) fn frame(&mut self, revision: u64) -> io::Result<Frame> {
        self.extractor.frame(revision, &self.terminal)
    }

    /// Commits one signed viewport move: libghostty clamps it at the history
    /// edges, and the outcome carries the frame at `revision`. No PTY input is
    /// sent.
    pub(crate) fn scroll(&mut self, rows: i16, revision: u64) -> io::Result<ServerMessage> {
        let applied_rows =
            i16::try_from(self.move_viewport(isize::from(rows))?).map_err(|_| errno(libc::EIO))?;
        if applied_rows != 0
            && (applied_rows.is_negative() != rows.is_negative()
                || applied_rows.unsigned_abs() > rows.unsigned_abs())
        {
            return Err(errno(libc::EIO));
        }
        let frame = self.frame(revision)?;
        let direction = if rows.is_negative() {
            VerticalDirection::Up
        } else {
            VerticalDirection::Down
        };
        let next = viewport_preview(&self.terminal, &self.extractor, direction)?;
        Ok(ServerMessage::ScrollOutcome(ScrollOutcome::Viewport {
            requested_rows: rows,
            applied_rows,
            frame: Box::new(frame),
            next,
        }))
    }

    /// Moves the viewport by `rows`, clamped at the history edges, and returns
    /// the rows it moved.
    fn move_viewport(&mut self, rows: isize) -> io::Result<i128> {
        let offset = |terminal: &Terminal<'static, 'static>| {
            terminal
                .scrollbar()
                .map(|scrollbar| scrollbar.offset)
                .map_err(|_| errno(libc::EIO))
        };
        let before = offset(&self.terminal)?;
        self.terminal.scroll_viewport(ScrollViewport::Delta(rows));
        Ok(i128::from(offset(&self.terminal)?) - i128::from(before))
    }

    /// Moves the viewport one row for a physical wheel that the terminal does
    /// not own; the outcome carries the frame at `revision`.
    /// No PTY input is sent.
    pub(crate) fn wheel(
        &mut self,
        direction: VerticalDirection,
        revision: u64,
    ) -> io::Result<ServerMessage> {
        let applied_rows = self.move_viewport(match direction {
            VerticalDirection::Up => -1,
            VerticalDirection::Down => 1,
        })?;
        let applied_rows = i8::try_from(applied_rows)
            .ok()
            .filter(|rows| matches!(rows, -1..=1))
            .ok_or_else(|| errno(libc::EIO))?;
        Ok(ServerMessage::WheelOutcome(WheelOutcome::Viewport {
            applied_rows,
            frame: Box::new(self.frame(revision)?),
        }))
    }

    /// The input a terminal-routed vertical wheel sends: the mouse event under
    /// mouse tracking, otherwise an arrow-key press, as alternate-screen mode
    /// 1007 asks.
    pub(crate) fn routed_wheel(
        &self,
        input: session::MouseEvent,
        direction: VerticalDirection,
    ) -> io::Result<Vec<u8>> {
        let tracking = self
            .terminal
            .is_mouse_tracking()
            .map_err(|_| errno(libc::EIO))?;
        let routed = if tracking {
            ClientMessage::Mouse(input)
        } else {
            ClientMessage::Key(session::KeyEvent {
                action: session::KeyAction::Press,
                key: match direction {
                    VerticalDirection::Up => session::PhysicalKey::ARROW_UP,
                    VerticalDirection::Down => session::PhysicalKey::ARROW_DOWN,
                },
                modifiers: session::Modifiers::empty(),
                consumed_modifiers: session::Modifiers::empty(),
                composing: false,
                text: None,
                unshifted_codepoint: None,
            })
        };
        encode_input(&self.terminal, self.surface, routed)
    }

    /// Moves the viewport to live output once and publishes one frame at
    /// `revision`. No PTY input is sent.
    pub(crate) fn return_to_live(&mut self, revision: u64) -> io::Result<ServerMessage> {
        self.bottom();
        Ok(ServerMessage::Frame(Box::new(self.frame(revision)?)))
    }

    /// Applies one input request and reports its effect: a resize, nothing
    /// to write, bytes queued for the PTY (from a key or other input), or a
    /// full queue that took nothing.
    pub(crate) fn apply(&mut self, request: ClientMessage) -> io::Result<Effect> {
        if let ClientMessage::Resize(size) = request {
            self.resize(size)?;
            return Ok(Effect::Resize);
        }
        let key = matches!(request, ClientMessage::Key(_));
        let bytes = encode_input(&self.terminal, self.surface, request)?;
        if bytes.is_empty() {
            return Ok(Effect::Quiet);
        }
        if !self.writes.push(&bytes) {
            return Ok(Effect::Full);
        }
        self.flush_writes()?;
        Ok(if key { Effect::Key } else { Effect::Write })
    }

    /// The terminal's foreground process group, if it has one.
    pub(crate) fn foreground_group(&self) -> Option<platform::pid_t> {
        platform::foreground_group(self.master.as_raw_fd())
    }

    /// Whether the primary screen's viewport is above live output.
    pub(crate) fn above_live(&self) -> io::Result<bool> {
        if self.alternate()? {
            return Ok(false);
        }
        let scrollbar = self.terminal.scrollbar().map_err(|_| errno(libc::EIO))?;
        Ok(scrollbar.offset.saturating_add(scrollbar.len) < scrollbar.total)
    }

    /// Moves the viewport to live output; the next frame shows it.
    pub(crate) fn bottom(&mut self) {
        self.terminal.scroll_viewport(ScrollViewport::Bottom);
    }
}

fn spawn_pty_child(command: &[OsString], cols: u16, rows: u16) -> io::Result<(File, Child)> {
    let (program, arguments) = command.split_first().ok_or_else(|| errno(libc::EINVAL))?;
    let (master, slave) = platform::open_pty(cols, rows)?;
    let stdin = slave.try_clone()?;
    let stdout = slave.try_clone()?;
    let parent = std::process::id() as platform::pid_t;
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(slave))
        .env("TERM", "eon")
        .env("COLORTERM", "truecolor");
    // SAFETY: Only async-signal-safe syscalls execute between fork and exec.
    unsafe {
        command.pre_exec(move || platform::configure_child(parent));
    }
    let child = command.spawn()?;
    Ok((master, child))
}

/// The child's exit code as Orbit reports it, in Exited and as
/// the server's own status: a child killed by a signal has code 1.
fn child_code(status: ExitStatus) -> u32 {
    status.code().unwrap_or(1) as u32
}

pub(crate) fn status_code(status: ExitStatus) -> u32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| status.signal().map_or(1, |signal| 128 + signal)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;

    #[test]
    fn clipboard_capture_is_attached_typed_and_bounded() {
        let clipboard = Rc::new(RefCell::new(ClipboardWrites::default()));
        let last_error = Rc::new(Cell::new(None));
        let (sink, error_sink) = (Rc::clone(&clipboard), Rc::clone(&last_error));
        let mut terminal = Terminal::new(80, 24).unwrap();
        terminal
            .on_clipboard_write(move |_, write| {
                let result = sink.borrow_mut().capture(&write);
                error_sink.set(result.err());
                write.reply(result, false);
            })
            .unwrap();
        let writes = || clipboard.borrow().writes.clone();

        // Nothing is captured while capture is off.
        terminal.vt_write(b"\x1b]52;c;ZGV0YWNoZWQ=\x1b\\");
        assert_eq!(last_error.get(), Some(ClipboardWriteError::Denied));
        assert!(writes().is_empty());

        // A clear, text with NUL and text that is not UTF-8 are refused.
        clipboard.borrow_mut().enabled = true;
        terminal.vt_write(
            b"\x1b]52;c;\x1b\\\x1b]52;c;YQBi\x1b\\\x1b]52;c;/w==\x1b\\\x1b]52;c;b25l\x1b\\\x1b]52;s;dHdv\x1b\\\x1b]52;p;dGhyZWU=\x1b\\",
        );
        assert_eq!(
            writes(),
            [
                (ClipboardLocation::Standard, "one".into()),
                (ClipboardLocation::Selection, "two".into()),
                (ClipboardLocation::Primary, "three".into()),
            ]
        );

        // One write above the maximum copy is refused alone.
        let mut oversized = b"\x1b]52;c;".to_vec();
        oversized.extend_from_slice(&b"eHh4".repeat(session::MAX_COPY_BYTES / 3));
        oversized.extend_from_slice(b"eHg=\x1b\\");
        terminal.vt_write(&oversized);
        assert_eq!(last_error.get(), Some(ClipboardWriteError::InvalidData));
        assert_eq!(writes().len(), 3);
        assert!(!clipboard.borrow().overflowed);

        // Passing the turn's count bound keeps nothing more.
        for _ in 3..=MAX_CLIPBOARD_WRITES {
            terminal.vt_write(b"\x1b]52;c;eA==\x1b\\");
        }
        assert!(clipboard.borrow().overflowed);
        assert_eq!(writes().len(), MAX_CLIPBOARD_WRITES);
        terminal.vt_write(b"\x1b]52;c;eA==\x1b\\");
        assert_eq!(last_error.get(), Some(ClipboardWriteError::Busy));

        // So does passing the turn's byte bound.
        *clipboard.borrow_mut() = ClipboardWrites {
            enabled: true,
            writes: VecDeque::from([(
                ClipboardLocation::Standard,
                "x".repeat(session::MAX_COPY_BYTES - 1),
            )]),
            overflowed: false,
        };
        terminal.vt_write(b"\x1b]52;c;eHg=\x1b\\\x1b]52;c;eA==\x1b\\");
        assert_eq!(last_error.get(), Some(ClipboardWriteError::Busy));
        assert_eq!(writes().len(), 1);
        assert!(clipboard.borrow().overflowed);
    }

    #[test]
    fn spawn_failure_releases_partial_resources() {
        let _guard = FORK_AND_SOCKET_TEST.lock().unwrap();
        assert_eq!(
            code(spawn_pty_child(
                &[OsString::from("/does/not/exist")],
                80,
                24
            )),
            Some(libc::ENOENT)
        );

        let mut command = Command::new("/bin/true");
        // SAFETY: The hook exercises only the child-side setup path.
        unsafe { command.pre_exec(|| platform::configure_child(platform::pid_t::MAX)) };
        assert_eq!(
            command.spawn().unwrap_err().raw_os_error(),
            Some(libc::ESRCH)
        );
    }
}
