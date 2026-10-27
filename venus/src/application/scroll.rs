use orbit_protocol::session::{ClientMessage, MAX_SCROLL_ROWS, VerticalDirection};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use winit::event::TouchPhase;
use yazelix_venus::ScenePreview;

const SELECTION_SCROLL_INTERVAL: Duration = Duration::from_millis(40);
const SCROLL_SAMPLE_WINDOW: Duration = Duration::from_millis(150);
const MAX_SCROLL_SAMPLES: usize = 256;
const PRECISION_SCROLL_GAIN: f64 = 2.0;
const SCROLL_DECAY: f64 = 4.0;
const MIN_FLING_VELOCITY: f64 = 40.0;
const MAX_FLING_VELOCITY: f64 = 8_000.0;
// Orbit's output queue lets a newer frame replace a preview answer it has not started
// writing, so a frame that follows a preview request starts this wait for the answer.
const PREVIEW_ANSWER_GRACE: Duration = Duration::from_millis(50);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PendingViewportRequest {
    Scroll { rows: i16, cancelled: bool },
    SelectionTick,
}

#[derive(Debug, Default)]
pub(super) struct TerminalScroll {
    pixels: f64,
    line_fraction: f64,
    terminal_lines: f64,
    samples: VecDeque<(Instant, f64)>,
    pub(super) phase_active: bool,
    pub(super) velocity: f64,
    last_advance: Option<Instant>,
    // The outstanding PreviewVertical's direction, and when a frame first followed it.
    preview_pending: Option<(VerticalDirection, Option<Instant>)>,
}

impl TerminalScroll {
    pub(super) fn push_lines(&mut self, lines: f64, cell_height: f64) {
        if !lines.is_finite() || !cell_height.is_finite() || cell_height <= 0.0 {
            return;
        }
        self.stop_gesture();
        self.terminal_lines += lines;
        self.line_fraction += lines;
        let whole = self.line_fraction.trunc();
        self.line_fraction -= whole;
        self.pixels += whole * 3.0 * cell_height;
    }

    pub(super) fn push_pixels(
        &mut self,
        pixels: f64,
        phase: TouchPhase,
        now: Instant,
        cell_height: f64,
    ) {
        if !pixels.is_finite() || !cell_height.is_finite() || cell_height <= 0.0 {
            return;
        }
        let native_pixels = pixels;
        let pixels = native_pixels * PRECISION_SCROLL_GAIN;
        match phase {
            TouchPhase::Started => {
                self.stop_gesture();
                self.phase_active = true;
                self.record_sample(now, pixels);
            }
            TouchPhase::Moved => {
                if !self.phase_active {
                    // A discrete Wayland axis may leave winit's shared phase at Moved.
                    self.stop_gesture();
                    self.phase_active = true;
                }
                self.record_sample(now, pixels);
            }
            TouchPhase::Ended if self.phase_active => {
                self.record_sample(now, pixels);
                self.phase_active = false;
                self.velocity = self.release_velocity();
                self.last_advance = (self.velocity != 0.0).then_some(now);
            }
            TouchPhase::Ended | TouchPhase::Cancelled => self.stop_gesture(),
        }
        if phase != TouchPhase::Cancelled {
            self.terminal_lines += native_pixels / cell_height;
            self.pixels += pixels;
        }
    }

    fn record_sample(&mut self, now: Instant, pixels: f64) {
        while self.samples.len() >= MAX_SCROLL_SAMPLES
            || self.samples.front().is_some_and(|(time, _)| {
                now.saturating_duration_since(*time) > SCROLL_SAMPLE_WINDOW
            })
        {
            self.samples.pop_front();
        }
        self.samples.push_back((now, pixels));
    }

    fn release_velocity(&mut self) -> f64 {
        let velocity = self
            .samples
            .front()
            .zip(self.samples.back())
            .and_then(|((first, _), (last, _))| {
                let seconds = last.saturating_duration_since(*first).as_secs_f64();
                (seconds > 0.0)
                    .then(|| self.samples.iter().map(|(_, pixels)| pixels).sum::<f64>() / seconds)
            })
            .unwrap_or(0.0)
            .clamp(-MAX_FLING_VELOCITY, MAX_FLING_VELOCITY);
        self.samples.clear();
        if velocity.abs() < MIN_FLING_VELOCITY {
            0.0
        } else {
            velocity
        }
    }

    pub(super) fn advance(&mut self, now: Instant) {
        let Some(last) = self.last_advance else {
            return;
        };
        self.last_advance = Some(now);
        let elapsed = now.saturating_duration_since(last).as_secs_f64();
        let next_velocity = self.velocity * (-SCROLL_DECAY * elapsed).exp();
        self.pixels += (self.velocity - next_velocity) / SCROLL_DECAY;
        self.velocity = next_velocity;
        if self.velocity.abs() / SCROLL_DECAY < 1.0 {
            self.stop_fling();
        }
    }

    fn stop_fling(&mut self) {
        self.velocity = 0.0;
        self.last_advance = None;
    }

    fn stop_gesture(&mut self) {
        self.samples.clear();
        self.phase_active = false;
        self.stop_fling();
    }

    pub(super) fn cancel(&mut self, pending: &mut Option<PendingViewportRequest>) {
        // A written batch or preview still needs its ordered response before another request.
        *self = Self {
            preview_pending: self.preview_pending,
            ..Self::default()
        };
        if let Some(PendingViewportRequest::Scroll { cancelled, .. }) = pending {
            *cancelled = true;
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    fn direction(&self) -> Option<VerticalDirection> {
        if self.pixels > 0.0 {
            Some(VerticalDirection::Up)
        } else if self.pixels < 0.0 {
            Some(VerticalDirection::Down)
        } else {
            None
        }
    }

    /// Releases the hold for an answer in the requested direction; an earlier
    /// attachment's late answer may name the other one.
    pub(super) fn preview_arrived(&mut self, direction: VerticalDirection) {
        if self
            .preview_pending
            .is_some_and(|(requested, _)| requested == direction)
        {
            self.preview_pending = None;
        }
    }

    pub(super) fn frame_arrived(&mut self, now: Instant) {
        if let Some((_, followed @ None)) = &mut self.preview_pending {
            *followed = Some(now);
        }
    }

    pub(super) fn preview_retry_at(&self) -> Option<Instant> {
        self.preview_pending
            .and_then(|(_, followed)| followed)
            .map(|followed| followed + PREVIEW_ANSWER_GRACE)
    }

    /// Gives up on an answer a frame may have replaced, once its grace has passed.
    pub(super) fn retry_preview(&mut self, now: Instant) -> bool {
        let due = self.preview_retry_at().is_some_and(|at| now >= at);
        if due {
            self.preview_pending = None;
        }
        due
    }

    pub(super) fn resolve_preview(&mut self, preview: Option<&ScenePreview>) -> Option<f64> {
        match preview {
            Some(ScenePreview::TerminalOwned { direction, .. })
                if Some(*direction) == self.direction() =>
            {
                self.pixels = 0.0;
                self.line_fraction = 0.0;
                self.stop_gesture();
                Some(std::mem::take(&mut self.terminal_lines))
            }
            Some(ScenePreview::Viewport {
                direction,
                edge_reached,
                rows,
                ..
            }) if Some(*direction) == self.direction() => {
                self.terminal_lines = 0.0;
                if *edge_reached && rows.is_empty() {
                    self.pixels = 0.0;
                    self.stop_gesture();
                }
                None
            }
            _ => None,
        }
    }

    pub(super) fn next_request(
        &mut self,
        pending: Option<PendingViewportRequest>,
        frame_revision: u64,
        preview: Option<&ScenePreview>,
        cell_height: f64,
    ) -> Option<ClientMessage> {
        if pending.is_some()
            || self.preview_pending.is_some()
            || !cell_height.is_finite()
            || cell_height <= 0.0
        {
            return None;
        }
        let direction = self.direction()?;
        let (preview_revision, rows) = match preview {
            Some(ScenePreview::Viewport {
                frame_revision: revision,
                direction: candidate,
                rows,
                ..
            }) if *candidate == direction => (*revision, rows),
            _ => {
                self.preview_pending = Some((direction, None));
                return Some(ClientMessage::PreviewVertical {
                    frame_revision,
                    direction,
                });
            }
        };
        let available = i64::try_from(rows.len())
            .unwrap_or(i64::MAX)
            .min(i64::from(MAX_SCROLL_ROWS));
        if available == 0 {
            return None;
        }
        let crossed = (self.pixels / cell_height).trunc();
        if crossed == 0.0 {
            return None;
        }
        let requested = (-(crossed as i64)).clamp(-available, available) as i16;
        Some(ClientMessage::ScrollVertical {
            frame_revision: preview_revision,
            rows: requested,
        })
    }

    pub(super) fn accept_batch(
        &mut self,
        pending: &mut Option<PendingViewportRequest>,
        requested_rows: i16,
        applied_rows: i16,
        cell_height: f64,
    ) -> bool {
        let Some(PendingViewportRequest::Scroll { rows, cancelled }) = *pending else {
            return false;
        };
        if rows != requested_rows {
            return false;
        }
        *pending = None;
        if !cancelled {
            self.pixels += f64::from(applied_rows) * cell_height;
        }
        true
    }

    pub(super) fn offset(&self, preview: Option<&ScenePreview>, cell_height: f64) -> f32 {
        let Some(direction) = self.direction() else {
            return 0.0;
        };
        let Some(ScenePreview::Viewport {
            direction: candidate,
            rows,
            ..
        }) = preview
        else {
            return 0.0;
        };
        if *candidate != direction || rows.is_empty() {
            return 0.0;
        }
        let limit = rows.len() as f64 * cell_height;
        self.pixels.clamp(-limit, limit) as f32
    }

    pub(super) fn active(&self, pending: Option<PendingViewportRequest>) -> bool {
        self.velocity != 0.0
            || self.pixels != 0.0
            || matches!(
                pending,
                Some(PendingViewportRequest::Scroll {
                    cancelled: false,
                    ..
                })
            )
    }
}

#[derive(Debug, Default)]
pub(super) struct SelectionAutoscroll {
    edge: bool,
    pub(super) next: Option<Instant>,
    awaiting_frame: Option<u64>,
    stopped: bool,
}

impl SelectionAutoscroll {
    pub(super) fn edge(
        &mut self,
        at_top: bool,
        now: Instant,
        pending: Option<PendingViewportRequest>,
    ) {
        if !at_top {
            self.cancel();
            return;
        }
        if !self.edge {
            self.edge = true;
            self.stopped = false;
        }
        self.schedule(now, pending);
    }

    fn schedule(&mut self, now: Instant, pending: Option<PendingViewportRequest>) {
        if self.edge
            && !self.stopped
            && pending != Some(PendingViewportRequest::SelectionTick)
            && self.awaiting_frame.is_none()
            && self.next.is_none()
        {
            self.next = Some(now + SELECTION_SCROLL_INTERVAL);
        }
    }

    pub(super) fn take_due(
        &mut self,
        now: Instant,
        pending: Option<PendingViewportRequest>,
    ) -> bool {
        if self.next.is_none_or(|next| now < next) || pending.is_some() {
            return false;
        }
        self.next = None;
        true
    }

    pub(super) fn accept(
        &mut self,
        pending: &mut Option<PendingViewportRequest>,
        applied_rows: i16,
        frame_revision: u64,
    ) -> bool {
        if *pending != Some(PendingViewportRequest::SelectionTick)
            || !matches!(applied_rows, -1..=0)
        {
            return false;
        }
        *pending = None;
        if applied_rows == 0 {
            self.stopped = true;
        } else {
            self.awaiting_frame = Some(frame_revision);
        }
        true
    }

    pub(super) fn presented(
        &mut self,
        revision: u64,
        now: Instant,
        pending: Option<PendingViewportRequest>,
    ) {
        if self
            .awaiting_frame
            .is_some_and(|required| revision >= required)
        {
            self.awaiting_frame = None;
        }
        self.schedule(now, pending);
    }

    pub(super) fn fail(&mut self) {
        self.next = None;
        self.awaiting_frame = None;
        self.stopped = true;
    }

    pub(super) fn cancel(&mut self) {
        self.edge = false;
        self.next = None;
        self.awaiting_frame = None;
        self.stopped = false;
        // A sent tick still needs its ordered response after release.
    }

    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }
}

pub(super) fn finish_scroll_failure(
    pending: &mut Option<PendingViewportRequest>,
    terminal_scroll: &mut TerminalScroll,
    selection_scroll: &mut SelectionAutoscroll,
    expected_selection_rejection: bool,
) {
    if !expected_selection_rejection {
        terminal_scroll.reset();
    }
    if pending.take() == Some(PendingViewportRequest::SelectionTick) {
        selection_scroll.fail();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_scroll_keeps_fractional_input_and_elapsed_time_physics() {
        let start = Instant::now();
        let mut scroll = TerminalScroll::default();

        scroll.push_lines(0.4, 20.0);
        scroll.push_lines(0.6, 20.0);
        assert_eq!(scroll.pixels, 60.0);

        scroll.cancel(&mut None);
        scroll.push_pixels(0.0, TouchPhase::Started, start, 20.0);
        scroll.push_pixels(
            60.0,
            TouchPhase::Moved,
            start + Duration::from_millis(100),
            20.0,
        );
        scroll.push_pixels(
            0.0,
            TouchPhase::Ended,
            start + Duration::from_millis(100),
            20.0,
        );
        assert_eq!(scroll.pixels, 120.0);
        assert_eq!(scroll.velocity, 1_200.0);

        let before = scroll.pixels;
        scroll.advance(start + Duration::from_millis(200));
        let one_tick = scroll.pixels - before;

        let mut split = TerminalScroll::default();
        split.push_pixels(0.0, TouchPhase::Started, start, 20.0);
        split.push_pixels(
            60.0,
            TouchPhase::Moved,
            start + Duration::from_millis(100),
            20.0,
        );
        split.push_pixels(
            0.0,
            TouchPhase::Ended,
            start + Duration::from_millis(100),
            20.0,
        );
        let before = split.pixels;
        split.advance(start + Duration::from_millis(150));
        split.advance(start + Duration::from_millis(200));
        assert!(((split.pixels - before) - one_tick).abs() < 1e-9);

        let mut incomplete = TerminalScroll::default();
        incomplete.push_pixels(100.0, TouchPhase::Moved, start, 20.0);
        assert_eq!(incomplete.velocity, 0.0);

        let mut after_wheel = TerminalScroll::default();
        after_wheel.push_lines(1.0, 20.0);
        after_wheel.push_pixels(0.0, TouchPhase::Moved, start, 20.0);
        after_wheel.push_pixels(
            60.0,
            TouchPhase::Moved,
            start + Duration::from_millis(50),
            20.0,
        );
        after_wheel.push_pixels(
            0.0,
            TouchPhase::Ended,
            start + Duration::from_millis(50),
            20.0,
        );
        assert_eq!(after_wheel.velocity, 2_400.0);

        let mut capped = TerminalScroll::default();
        capped.push_pixels(0.0, TouchPhase::Started, start, 20.0);
        capped.push_pixels(
            1_000.0,
            TouchPhase::Moved,
            start + Duration::from_millis(1),
            20.0,
        );
        capped.push_pixels(
            0.0,
            TouchPhase::Ended,
            start + Duration::from_millis(1),
            20.0,
        );
        assert_eq!(capped.velocity, MAX_FLING_VELOCITY);

        let mut bounded = TerminalScroll::default();
        for _ in 0..=MAX_SCROLL_SAMPLES {
            bounded.push_pixels(1.0, TouchPhase::Moved, start, 20.0);
        }
        assert_eq!(bounded.samples.len(), MAX_SCROLL_SAMPLES);

        let mut expired = TerminalScroll::default();
        expired.push_pixels(10.0, TouchPhase::Started, start, 20.0);
        expired.push_pixels(
            10.0,
            TouchPhase::Moved,
            start + SCROLL_SAMPLE_WINDOW + Duration::from_millis(1),
            20.0,
        );
        expired.push_pixels(
            0.0,
            TouchPhase::Ended,
            start + SCROLL_SAMPLE_WINDOW + Duration::from_millis(1),
            20.0,
        );
        assert_eq!(expired.velocity, 0.0);

        let mut paused = TerminalScroll::default();
        paused.push_pixels(0.0, TouchPhase::Started, start, 20.0);
        paused.push_pixels(
            60.0,
            TouchPhase::Moved,
            start + Duration::from_millis(50),
            20.0,
        );
        paused.push_pixels(
            0.0,
            TouchPhase::Ended,
            start + Duration::from_millis(250),
            20.0,
        );
        assert_eq!(paused.velocity, 0.0);

        let mut interrupted = TerminalScroll::default();
        interrupted.push_pixels(0.0, TouchPhase::Started, start, 20.0);
        interrupted.push_pixels(
            60.0,
            TouchPhase::Moved,
            start + Duration::from_millis(50),
            20.0,
        );
        interrupted.push_lines(1.0, 20.0);
        interrupted.push_pixels(
            0.0,
            TouchPhase::Ended,
            start + Duration::from_millis(50),
            20.0,
        );
        assert_eq!(interrupted.velocity, 0.0);

        let mut cancelled = TerminalScroll::default();
        cancelled.push_pixels(20.0, TouchPhase::Cancelled, start, 20.0);
        assert_eq!(cancelled.pixels, 0.0);
        assert_eq!(cancelled.terminal_lines, 0.0);
    }

    #[test]
    fn terminal_scroll_coalesces_one_signed_batch_without_losing_distance() {
        let start = Instant::now();
        let mut pending = None;
        assert!(!TerminalScroll::default().accept_batch(&mut pending, -1, -1, 20.0));
        let row = yazelix_venus::DrawRow {
            wrapped: false,
            wrap_continuation: false,
            kitty_virtual_placeholder: false,
            cells: Vec::new(),
        };
        let preview = ScenePreview::Viewport {
            frame_revision: 7,
            direction: VerticalDirection::Up,
            edge_reached: false,
            rows: vec![row.clone(); 5],
        };
        let mut continuous = TerminalScroll::default();
        continuous.push_pixels(20.0, TouchPhase::Moved, start, 20.0);
        assert_eq!(
            continuous.next_request(None, 8, Some(&preview), 20.0),
            Some(ClientMessage::ScrollVertical {
                frame_revision: 7,
                rows: -2,
            })
        );

        let mut scroll = TerminalScroll::default();
        scroll.push_pixels(50.0, TouchPhase::Moved, start, 20.0);

        assert_eq!(
            scroll.next_request(pending, 7, None, 20.0),
            Some(ClientMessage::PreviewVertical {
                frame_revision: 7,
                direction: VerticalDirection::Up,
            })
        );
        assert_eq!(scroll.next_request(pending, 7, None, 20.0), None);
        scroll.preview_arrived(VerticalDirection::Up);
        scroll.resolve_preview(Some(&preview));
        assert_eq!(scroll.offset(Some(&preview), 20.0), 100.0);
        assert_eq!(
            scroll.next_request(pending, 7, Some(&preview), 20.0),
            Some(ClientMessage::ScrollVertical {
                frame_revision: 7,
                rows: -5,
            })
        );

        pending = Some(PendingViewportRequest::Scroll {
            rows: -5,
            cancelled: false,
        });
        scroll.push_pixels(40.0, TouchPhase::Moved, start, 20.0);
        assert_eq!(scroll.next_request(pending, 8, None, 20.0), None);
        assert!(!scroll.accept_batch(&mut pending, -4, -4, 20.0));
        assert!(scroll.accept_batch(&mut pending, -5, -5, 20.0));
        assert_eq!(scroll.pixels, 80.0);

        let mut cancelled = TerminalScroll::default();
        let mut cancelled_pending = None;
        cancelled.push_pixels(20.0, TouchPhase::Moved, start, 20.0);
        cancelled.next_request(cancelled_pending, 7, None, 20.0);
        cancelled.preview_arrived(VerticalDirection::Up);
        cancelled.resolve_preview(Some(&preview));
        cancelled.next_request(cancelled_pending, 7, Some(&preview), 20.0);
        cancelled_pending = Some(PendingViewportRequest::Scroll {
            rows: -2,
            cancelled: false,
        });
        cancelled.cancel(&mut cancelled_pending);
        assert!(!cancelled.active(cancelled_pending));
        cancelled.push_pixels(20.0, TouchPhase::Moved, start, 20.0);
        assert_eq!(
            cancelled.next_request(cancelled_pending, 7, Some(&preview), 20.0),
            None
        );
        assert!(cancelled.accept_batch(&mut cancelled_pending, -2, -2, 20.0));
        assert_eq!(cancelled.pixels, 40.0);

        let down_preview = ScenePreview::Viewport {
            frame_revision: 7,
            direction: VerticalDirection::Down,
            edge_reached: true,
            rows: vec![row; 3],
        };
        let mut down = TerminalScroll::default();
        down.push_pixels(-50.0, TouchPhase::Moved, start, 20.0);
        down.preview_arrived(VerticalDirection::Down);
        down.resolve_preview(Some(&down_preview));
        assert_eq!(down.offset(Some(&down_preview), 20.0), -60.0);
        assert_eq!(
            down.next_request(None, 7, Some(&down_preview), 20.0),
            Some(ClientMessage::ScrollVertical {
                frame_revision: 7,
                rows: 3,
            })
        );

        assert_eq!(
            scroll.next_request(pending, 9, None, 20.0),
            Some(ClientMessage::PreviewVertical {
                frame_revision: 9,
                direction: VerticalDirection::Up,
            })
        );

        let edge = ScenePreview::Viewport {
            frame_revision: 10,
            direction: VerticalDirection::Up,
            edge_reached: true,
            rows: Vec::new(),
        };
        scroll.preview_arrived(VerticalDirection::Up);
        scroll.resolve_preview(Some(&edge));
        assert_eq!(scroll.pixels, 0.0);
        assert_eq!(scroll.velocity, 0.0);

        let mut terminal = TerminalScroll::default();
        terminal.push_lines(1.0, 20.0);
        let routed = ScenePreview::TerminalOwned {
            frame_revision: 7,
            direction: VerticalDirection::Up,
        };
        assert_eq!(terminal.resolve_preview(Some(&routed)), Some(1.0));
        assert_eq!(terminal.pixels, 0.0);

        terminal.push_pixels(20.0, TouchPhase::Moved, start, 20.0);
        assert_eq!(terminal.resolve_preview(Some(&routed)), Some(1.0));
        assert_eq!(terminal.pixels, 0.0);

        terminal.push_pixels(0.0, TouchPhase::Started, start, 20.0);
        terminal.push_pixels(
            20.0,
            TouchPhase::Moved,
            start + Duration::from_millis(50),
            20.0,
        );
        assert_eq!(terminal.resolve_preview(Some(&routed)), Some(1.0));
        terminal.push_pixels(
            0.0,
            TouchPhase::Ended,
            start + Duration::from_millis(50),
            20.0,
        );
        assert_eq!(terminal.velocity, 0.0);
    }

    #[test]
    fn terminal_scroll_keeps_one_preview_outstanding_across_frames() {
        let start = Instant::now();
        let mut scroll = TerminalScroll::default();
        scroll.push_pixels(0.0, TouchPhase::Started, start, 20.0);
        scroll.push_pixels(50.0, TouchPhase::Moved, start, 20.0);
        let mut previews = 0;
        for revision in 7..17 {
            let now = start + Duration::from_millis(revision - 7);
            assert!(!scroll.retry_preview(now));
            if let Some(message) = scroll.next_request(None, revision, None, 20.0) {
                assert_eq!(
                    message,
                    ClientMessage::PreviewVertical {
                        frame_revision: 7,
                        direction: VerticalDirection::Up,
                    }
                );
                previews += 1;
            }
            scroll.frame_arrived(now);
            scroll.push_pixels(10.0, TouchPhase::Moved, now, 20.0);
        }
        assert_eq!(previews, 1);
        assert!(scroll.active(None));

        // A preview answered at an older revision leaves one fresh request.
        scroll.preview_arrived(VerticalDirection::Up);
        assert_eq!(scroll.preview_retry_at(), None);
        assert_eq!(
            scroll.next_request(None, 17, None, 20.0),
            Some(ClientMessage::PreviewVertical {
                frame_revision: 17,
                direction: VerticalDirection::Up,
            })
        );

        // A frame may have replaced the answer in Orbit's queue: after the grace
        // that follows the first such frame, exactly one request replaces it.
        assert_eq!(scroll.preview_retry_at(), None);
        scroll.frame_arrived(start + Duration::from_millis(20));
        scroll.frame_arrived(start + Duration::from_millis(40));
        let retry = start + Duration::from_millis(20) + PREVIEW_ANSWER_GRACE;
        assert_eq!(scroll.preview_retry_at(), Some(retry));
        assert!(!scroll.retry_preview(retry - Duration::from_millis(1)));
        assert_eq!(scroll.next_request(None, 18, None, 20.0), None);
        assert!(scroll.retry_preview(retry));
        assert_eq!(
            scroll.next_request(None, 18, None, 20.0),
            Some(ClientMessage::PreviewVertical {
                frame_revision: 18,
                direction: VerticalDirection::Up,
            })
        );
        assert_eq!(scroll.next_request(None, 19, None, 20.0), None);

        // A cancelled gesture's preview still holds the next gesture's request, and
        // only an answer in its direction releases it.
        scroll.cancel(&mut None);
        assert!(!scroll.active(None));
        scroll.push_pixels(-20.0, TouchPhase::Moved, start, 20.0);
        assert_eq!(scroll.next_request(None, 18, None, 20.0), None);
        scroll.preview_arrived(VerticalDirection::Down);
        assert_eq!(scroll.next_request(None, 18, None, 20.0), None);
        scroll.preview_arrived(VerticalDirection::Up);
        assert_eq!(
            scroll.next_request(None, 18, None, 20.0),
            Some(ClientMessage::PreviewVertical {
                frame_revision: 18,
                direction: VerticalDirection::Down,
            })
        );

        // A failed request, lost or swapped attachment, or reconnect expects no answer.
        assert_eq!(scroll.next_request(None, 19, None, 20.0), None);
        finish_scroll_failure(
            &mut None,
            &mut scroll,
            &mut SelectionAutoscroll::default(),
            false,
        );
        scroll.push_pixels(20.0, TouchPhase::Moved, start, 20.0);
        assert!(scroll.next_request(None, 19, None, 20.0).is_some());
        scroll.reset();
        scroll.push_pixels(20.0, TouchPhase::Moved, start, 20.0);
        assert!(scroll.next_request(None, 20, None, 20.0).is_some());
    }

    #[test]
    fn held_top_edge_waits_for_each_authoritative_frame_and_stops() {
        let start = Instant::now();
        let mut pending = None;
        let mut scroll = SelectionAutoscroll::default();
        scroll.edge(true, start, pending);
        pending = Some(PendingViewportRequest::Scroll {
            rows: -1,
            cancelled: false,
        });
        assert!(!scroll.take_due(start + SELECTION_SCROLL_INTERVAL, pending));
        assert!(!scroll.accept(&mut pending, -1, 6));
        assert!(TerminalScroll::default().accept_batch(&mut pending, -1, -1, 20.0));
        assert!(!scroll.take_due(start, pending));
        assert!(scroll.take_due(start + SELECTION_SCROLL_INTERVAL, pending));
        pending = Some(PendingViewportRequest::SelectionTick);
        assert!(!scroll.take_due(start + SELECTION_SCROLL_INTERVAL * 2, pending));
        assert!(scroll.accept(&mut pending, -1, 7));
        assert!(!scroll.take_due(start + SELECTION_SCROLL_INTERVAL * 3, pending));
        scroll.presented(6, start + SELECTION_SCROLL_INTERVAL * 3, pending);
        assert!(scroll.next.is_none());
        scroll.presented(7, start + SELECTION_SCROLL_INTERVAL * 3, pending);
        assert!(scroll.take_due(start + SELECTION_SCROLL_INTERVAL * 4, pending));
        pending = Some(PendingViewportRequest::SelectionTick);
        assert!(scroll.accept(&mut pending, 0, 8));
        scroll.presented(8, start + SELECTION_SCROLL_INTERVAL * 4, pending);
        assert!(!scroll.take_due(start + SELECTION_SCROLL_INTERVAL * 5, pending));
        scroll.edge(false, start + SELECTION_SCROLL_INTERVAL * 5, pending);
        scroll.edge(true, start + SELECTION_SCROLL_INTERVAL * 5, pending);
        assert!(scroll.take_due(start + SELECTION_SCROLL_INTERVAL * 6, pending));
        pending = Some(PendingViewportRequest::SelectionTick);
        scroll.cancel();
        assert!(scroll.accept(&mut pending, -1, 9));
        scroll.presented(9, start + SELECTION_SCROLL_INTERVAL * 6, pending);
        assert!(!scroll.take_due(start + SELECTION_SCROLL_INTERVAL * 7, pending));
        scroll.edge(true, start + SELECTION_SCROLL_INTERVAL * 7, pending);
        assert!(scroll.take_due(start + SELECTION_SCROLL_INTERVAL * 8, pending));
        pending = Some(PendingViewportRequest::SelectionTick);
        finish_scroll_failure(
            &mut pending,
            &mut TerminalScroll::default(),
            &mut scroll,
            true,
        );
        scroll.presented(10, start + SELECTION_SCROLL_INTERVAL * 8, pending);
        assert!(!scroll.take_due(start + SELECTION_SCROLL_INTERVAL * 9, pending));
    }

    #[test]
    fn rejected_selection_tick_preserves_wheel_input_after_release() {
        let start = Instant::now();
        let mut request = None;
        let mut wheel = TerminalScroll::default();
        let mut selection = SelectionAutoscroll::default();
        selection.edge(true, start, request);
        assert!(selection.take_due(start + SELECTION_SCROLL_INTERVAL, request));
        request = Some(PendingViewportRequest::SelectionTick);
        selection.cancel();
        wheel.push_lines(1.0, 20.0);
        let pending = wheel.pixels;

        finish_scroll_failure(&mut request, &mut wheel, &mut selection, true);
        assert_eq!(wheel.pixels, pending);
        assert_eq!(request, None);

        finish_scroll_failure(&mut request, &mut wheel, &mut selection, false);
        assert_eq!(wheel.pixels, 0.0);
    }
}
