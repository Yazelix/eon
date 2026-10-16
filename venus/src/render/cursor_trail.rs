use super::{
    CellMetrics, CursorPoint, RectangleBatch, SceneColor, cursor_bounds, push_cursor, rect_corners,
    valid_rect,
};
use crate::{DrawCursor, Scene, SceneRect};

const SHORT_CURSOR_ANIMATION: f32 = 0.04;
const LONG_CURSOR_ANIMATION: f32 = 0.15;
const MAX_CURSOR_DELTA: f32 = 0.1;
const CURSOR_SETTLED: f32 = 0.01;

#[derive(Clone, Copy, Debug, Default)]
struct CursorSpring {
    displacement: f32,
    velocity: f32,
}

impl CursorSpring {
    fn advance(&mut self, delta: f32, duration: f32) -> bool {
        let delta = delta.clamp(0.0, MAX_CURSOR_DELTA);
        if !self.displacement.is_finite()
            || !self.velocity.is_finite()
            || !duration.is_finite()
            || duration <= delta
        {
            *self = Self::default();
            return false;
        }
        if self.displacement.abs() < CURSOR_SETTLED {
            *self = Self::default();
            return false;
        }

        let omega = 4.0 / duration;
        let a = self.displacement;
        let b = a * omega + self.velocity;
        let decay = (-omega * delta).exp();
        self.displacement = (a + b * delta) * decay;
        self.velocity = decay * (-a * omega - b * delta * omega + b);
        if !self.displacement.is_finite() || self.displacement.abs() < CURSOR_SETTLED {
            *self = Self::default();
            false
        } else {
            true
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct AnimatedCorner {
    x: CursorSpring,
    y: CursorSpring,
    duration: f32,
}

#[derive(Clone, Debug, Default)]
pub(super) struct CursorAnimation {
    corners: [AnimatedCorner; 4],
    pub(super) target: Option<SceneRect>,
}

impl CursorAnimation {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    fn update(
        &mut self,
        target: SceneRect,
        viewport: SceneRect,
        cell_width: f32,
        delta: f32,
        duration_scale: f32,
    ) -> bool {
        if !valid_rect(target)
            || !valid_rect(viewport)
            || !duration_scale.is_finite()
            || !(0.25..=4.0).contains(&duration_scale)
        {
            self.reset();
            return false;
        }
        if self.target.is_none() {
            self.snap(target);
            return false;
        }
        if self
            .target
            .is_some_and(|previous| rect_changed(previous, target))
        {
            self.start_jump(target, cell_width, duration_scale);
            return self.advance(0.0);
        }
        self.advance(delta)
    }

    fn snap(&mut self, target: SceneRect) {
        self.target = Some(target);
        self.corners = [AnimatedCorner::default(); 4];
    }

    fn start_jump(&mut self, target: SceneRect, cell_width: f32, duration_scale: f32) {
        let previous = self.target.expect("an initialized cursor has a target");
        let movement = CursorPoint {
            x: target.left + target.width * 0.5 - previous.left - previous.width * 0.5,
            y: target.top + target.height * 0.5 - previous.top - previous.height * 0.5,
        };
        let short = movement.x.abs() <= cell_width * 2.001 && movement.y.abs() < CURSOR_SETTLED;
        let durations = if short {
            [SHORT_CURSOR_ANIMATION * duration_scale; 4]
        } else {
            ranked_cursor_durations(movement, duration_scale)
        };
        let previous = rect_corners(previous);
        let destinations = rect_corners(target);
        for (index, duration) in durations.into_iter().enumerate() {
            let corner = &mut self.corners[index];
            corner.x.displacement =
                destinations[index].x - (previous[index].x - corner.x.displacement);
            corner.y.displacement =
                destinations[index].y - (previous[index].y - corner.y.displacement);
            corner.duration = duration;
        }
        self.target = Some(target);
    }

    fn advance(&mut self, delta: f32) -> bool {
        let mut active = false;
        for corner in &mut self.corners {
            active |= corner.x.advance(delta, corner.duration);
            active |= corner.y.advance(delta, corner.duration);
        }
        active
    }

    fn corners(&self) -> [CursorPoint; 4] {
        let target = rect_corners(self.target.unwrap_or_default());
        [0, 1, 2, 3].map(|index| CursorPoint {
            x: target[index].x - self.corners[index].x.displacement,
            y: target[index].y - self.corners[index].y.displacement,
        })
    }

    pub(super) fn is_active(&self) -> bool {
        self.corners
            .iter()
            .any(|corner| corner.x.displacement != 0.0 || corner.y.displacement != 0.0)
    }
}

fn ranked_cursor_durations(movement: CursorPoint, scale: f32) -> [f32; 4] {
    let length = (movement.x * movement.x + movement.y * movement.y)
        .sqrt()
        .max(f32::EPSILON);
    let direction = CursorPoint {
        x: movement.x / length,
        y: movement.y / length,
    };
    let relatives = [(-0.5_f32, -0.5_f32), (0.5, -0.5), (0.5, 0.5), (-0.5, 0.5)];
    let ranked = relatives.map(|(x, y)| x * direction.x + y * direction.y);
    let mut order = [0_usize, 1, 2, 3];
    order.sort_by(|left, right| {
        ranked[*left]
            .total_cmp(&ranked[*right])
            .then(left.cmp(right))
    });
    let mut durations = [0.0; 4];
    for (rank, index) in order.into_iter().enumerate() {
        durations[index] = match rank {
            0 => LONG_CURSOR_ANIMATION * scale,
            1 => LONG_CURSOR_ANIMATION * scale * 0.5,
            _ => 0.0,
        };
    }
    durations
}

fn rect_changed(left: SceneRect, right: SceneRect) -> bool {
    (left.left - right.left).abs() >= CURSOR_SETTLED
        || (left.top - right.top).abs() >= CURSOR_SETTLED
        || (left.width - right.width).abs() >= CURSOR_SETTLED
        || (left.height - right.height).abs() >= CURSOR_SETTLED
}

pub(super) fn inset_cursor_quad(mut points: [CursorPoint; 4], amount: f32) -> [CursorPoint; 4] {
    let center = CursorPoint {
        x: points.iter().map(|point| point.x).sum::<f32>() / 4.0,
        y: points.iter().map(|point| point.y).sum::<f32>() / 4.0,
    };
    for point in &mut points {
        point.x += (center.x - point.x).clamp(-amount, amount);
        point.y += (center.y - point.y).clamp(-amount, amount);
    }
    points
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_tail_cursor(
    rectangles: &mut RectangleBatch,
    animation: &mut CursorAnimation,
    scene: &Scene,
    blink_visible: bool,
    metrics: CellMetrics,
    viewport: SceneRect,
    trail_color: SceneColor,
    outline_color: SceneColor,
    duration_scale: f32,
    delta: f32,
) -> bool {
    let Some(cursor) = scene
        .cursor
        .filter(|cursor| cursor.visible && (blink_visible || !cursor.blinking))
    else {
        animation.reset();
        return false;
    };
    let Some(bounds) = cursor_bounds(scene, cursor, metrics, viewport) else {
        animation.reset();
        return false;
    };
    let active = animation.update(bounds, viewport, metrics.width, delta, duration_scale);
    if active {
        let corners = animation.corners();
        rectangles.push_quad(corners, outline_color, 1.0);
        rectangles.push_quad(inset_cursor_quad(corners, metrics.scale), trail_color, 1.0);
    }
    push_cursor(
        rectangles,
        DrawCursor {
            color: if cursor.explicit_color {
                cursor.color
            } else {
                trail_color
            },
            ..cursor
        },
        bounds,
        metrics,
        (!cursor.explicit_color).then_some(outline_color),
    );
    active
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_spring_snaps_then_distinguishes_short_long_and_scaled_motion() {
        let origin = SceneRect {
            left: 10.0,
            top: 20.0,
            width: 10.0,
            height: 18.0,
        };
        let short = SceneRect {
            left: 20.0,
            ..origin
        };
        let long = SceneRect {
            left: 60.0,
            top: 74.0,
            ..origin
        };
        let viewport = SceneRect {
            left: 0.0,
            top: 0.0,
            width: 200.0,
            height: 200.0,
        };
        let cell_width = origin.width;

        let mut animation = CursorAnimation::default();
        assert!(!animation.update(origin, viewport, cell_width, 0.0, 1.0));
        assert_eq!(animation.corners(), rect_corners(origin));
        assert!(animation.update(short, viewport, cell_width, 0.0, 1.0));
        assert!(!animation.update(short, viewport, cell_width, 0.05, 1.0));
        assert_eq!(animation.corners(), rect_corners(short));

        animation.reset();
        assert!(!animation.update(origin, viewport, cell_width, 0.0, 1.0));
        assert!(animation.update(long, viewport, cell_width, 0.0, 1.0));
        assert!(animation.update(long, viewport, cell_width, 0.05, 1.0));

        let wide_origin = SceneRect {
            width: 20.0,
            ..origin
        };
        let mut wide_three_cells = SceneRect {
            left: 40.0,
            ..wide_origin
        };
        let mut wide_animation = CursorAnimation::default();
        assert!(!wide_animation.update(wide_origin, viewport, cell_width, 0.0, 1.0));
        assert!(wide_animation.update(wide_three_cells, viewport, cell_width, 0.0, 1.0));
        assert!(wide_animation.update(wide_three_cells, viewport, cell_width, 0.05, 1.0));
        let before_retarget = wide_animation.corners()[0].x;
        wide_three_cells.left = 80.0;
        assert!(wide_animation.update(wide_three_cells, viewport, cell_width, 0.0, 1.0));
        assert!((wide_animation.corners()[0].x - before_retarget).abs() < CURSOR_SETTLED);

        let started = |scale| {
            let mut animation = CursorAnimation::default();
            assert!(!animation.update(origin, viewport, cell_width, 0.0, scale));
            assert!(animation.update(long, viewport, cell_width, 0.0, scale));
            animation
        };
        let mut fast = started(0.25);
        let mut slow = started(4.0);
        assert!(!fast.update(long, viewport, cell_width, 0.05, 0.25));
        assert!(slow.update(long, viewport, cell_width, 0.05, 4.0));

        let mut bounded = started(4.0);
        let mut huge_delta = bounded.clone();
        assert_eq!(
            bounded.update(long, viewport, cell_width, 0.1, 4.0),
            huge_delta.update(long, viewport, cell_width, 10.0, 4.0)
        );
        assert_eq!(bounded.corners(), huge_delta.corners());

        for _ in 0..120 {
            if !slow.update(long, viewport, cell_width, 1.0 / 60.0, 4.0) {
                break;
            }
        }
        assert!(!slow.is_active());
        assert_eq!(slow.corners(), rect_corners(long));

        let moved = SceneRect { left: 80.0, ..long };
        assert!(!slow.update(long, viewport, cell_width, 0.0, 1.0));
        assert!(slow.update(moved, viewport, cell_width, 0.0, 1.0));
        assert!(slow.update(moved, viewport, cell_width, 0.01, 1.0));
        let before_switch = slow.corners();
        assert!(slow.update(long, viewport, cell_width, 0.0, 1.0));
        assert_eq!(slow.corners(), before_switch);
        assert!(slow.update(
            origin,
            SceneRect {
                top: 10.0,
                ..viewport
            },
            cell_width,
            0.0,
            1.0,
        ));
        assert!(
            slow.corners()
                .iter()
                .zip(before_switch)
                .any(|(corner, previous)| *corner == previous)
        );
    }
}
