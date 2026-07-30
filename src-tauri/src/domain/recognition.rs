//! Portable gesture recognition from pointer movement and input steps.
//!
//! The recognizer accumulates:
//! - directional movement segments (`up`, `down`, `left`, `right`)
//! - explicit mouse-input steps (e.g. `wheel_up`)
//!
//! and produces a sequence that can be matched against user-defined bindings.

use std::cmp::Ordering;

use crate::config::{
    GestureStep, DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX, DEFAULT_DIRECTION_SWITCH_CONFIRM_PX,
    DEFAULT_MIN_SEGMENT_PX, MAX_GESTURE_STEPS,
};

/// Internal movement direction used while sampling mouse points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Recognizes gesture sequences from mouse movement and explicit input steps.
#[derive(Debug)]
pub(super) struct GestureRecognizer {
    /// Confirmed sequence steps (movement and explicit input events).
    steps: [GestureStep; MAX_GESTURE_STEPS],
    /// Number of initialized entries in `steps`.
    step_count: u8,
    /// Last recorded cursor position.
    last_point: Option<(i32, i32)>,
    /// Direction currently being accumulated.
    current_dir: Option<Direction>,
    /// Candidate direction waiting for hysteresis confirmation.
    pending_dir: Option<Direction>,
    /// Accumulated distance in the pending direction.
    pending_accum: i32,
    /// Accumulated distance for the current direction segment.
    segment_accum: i32,
    /// Minimum distance required to confirm a movement segment.
    min_segment_px: i32,
    /// Minimum distance required to switch direction.
    direction_switch_confirm_px: i32,
    /// Deadzone used to ignore tiny ambiguous diagonal motion.
    axis_ambiguity_deadzone_px: i32,
    /// Hard limit for sequence length.
    max_steps: u8,
    /// Set once sequence length exceeds `max_steps`.
    overflowed: bool,
}

impl GestureRecognizer {
    /// Creates a new gesture recognizer with explicit thresholds and limit.
    pub(super) fn new(
        min_segment_px: i32,
        direction_switch_confirm_px: i32,
        axis_ambiguity_deadzone_px: i32,
        max_steps: usize,
    ) -> Self {
        Self {
            steps: [GestureStep::Up; MAX_GESTURE_STEPS],
            step_count: 0,
            last_point: None,
            current_dir: None,
            pending_dir: None,
            pending_accum: 0,
            segment_accum: 0,
            min_segment_px,
            direction_switch_confirm_px,
            axis_ambiguity_deadzone_px,
            max_steps: u8::try_from(max_steps)
                .expect("validated maximum gesture steps must fit in u8"),
            overflowed: false,
        }
    }

    /// Adds a mouse point and updates movement segments.
    ///
    /// Direction changes are accepted only after hysteresis
    /// (`direction_switch_confirm_px`) to reduce jitter.
    pub(super) fn add_point(&mut self, x: i32, y: i32) {
        if self.last_point.is_none() {
            self.last_point = Some((x, y));
            return;
        }

        let (lx, ly) = self.last_point.expect("checked is_some");
        let dx = x - lx;
        let dy = y - ly;

        if dx == 0 && dy == 0 {
            return;
        }

        let new_dir = match Self::classify_direction(dx, dy, self.axis_ambiguity_deadzone_px) {
            Some(dir) => dir,
            None => {
                self.last_point = Some((x, y));
                return;
            }
        };

        let distance = Self::distance_in_primary_axis(new_dir, dx, dy);

        match self.current_dir {
            None => {
                self.accumulate_pending(new_dir, distance);
                if self.pending_accum >= self.direction_switch_confirm_px {
                    self.current_dir = self.pending_dir.take();
                    self.segment_accum = self.pending_accum;
                    self.pending_accum = 0;
                }
            }
            Some(current) if current == new_dir => {
                self.pending_dir = None;
                self.pending_accum = 0;
                self.segment_accum += distance;
            }
            Some(current) => {
                self.accumulate_pending(new_dir, distance);
                if self.pending_accum >= self.direction_switch_confirm_px {
                    self.confirm_segment(current);
                    self.current_dir = self.pending_dir.take();
                    self.segment_accum = self.pending_accum;
                    self.pending_accum = 0;
                }
            }
        }

        self.last_point = Some((x, y));
    }

    /// Adds an explicit non-movement step (e.g. wheel up/down).
    pub(super) fn add_input_step(&mut self, step: GestureStep) {
        self.flush_current_segment();
        self.push_step(step, false);
    }

    /// Returns the current effective gesture sequence (without finalizing).
    ///
    /// Includes an in-progress movement segment when it is already over
    /// `min_segment_px`.
    pub(super) fn current_sequence(&self) -> Option<GestureSequence> {
        if self.overflowed {
            return None;
        }

        let mut seq = GestureSequence {
            steps: self.steps,
            len: self.step_count,
        };
        if let Some(dir) = self.current_dir {
            if self.segment_accum >= self.min_segment_px {
                let step = Self::direction_to_step(dir);
                if seq.as_slice().last() != Some(&step) {
                    if seq.len >= self.max_steps {
                        return None;
                    }
                    seq.steps[usize::from(seq.len)] = step;
                    seq.len += 1;
                }
            }
        }

        if seq.len == 0 {
            None
        } else {
            Some(seq)
        }
    }

    /// Finalizes ongoing movement and returns the final sequence.
    ///
    /// Returns `None` when no valid steps were captured, or when the sequence
    /// overflowed the configured step limit.
    pub(super) fn finalize_sequence(&mut self) -> Option<GestureSequence> {
        self.flush_current_segment();
        if self.overflowed || self.step_count == 0 {
            None
        } else {
            Some(GestureSequence {
                steps: self.steps,
                len: self.step_count,
            })
        }
    }

    /// Resets the recognized sequence while keeping cursor tracking active.
    ///
    /// This clears all confirmed and in-progress steps so subsequent input is
    /// interpreted as a fresh sequence within the same gesture session.
    pub(super) fn reset_sequence(&mut self) {
        self.step_count = 0;
        self.current_dir = None;
        self.pending_dir = None;
        self.pending_accum = 0;
        self.segment_accum = 0;
        self.overflowed = false;
    }

    fn classify_direction(dx: i32, dy: i32, ambiguity_deadzone_px: i32) -> Option<Direction> {
        let abs_dx = dx.abs();
        let abs_dy = dy.abs();

        if abs_dx > 0
            && abs_dy > 0
            && abs_dx.min(abs_dy) <= ambiguity_deadzone_px
            && (abs_dx - abs_dy).abs() <= ambiguity_deadzone_px
        {
            return None;
        }

        Some(match abs_dx.cmp(&abs_dy) {
            Ordering::Greater => {
                if dx > 0 {
                    Direction::Right
                } else {
                    Direction::Left
                }
            }
            Ordering::Less => {
                if dy > 0 {
                    Direction::Down
                } else {
                    Direction::Up
                }
            }
            Ordering::Equal => {
                if dx > 0 {
                    Direction::Right
                } else if dx < 0 {
                    Direction::Left
                } else if dy > 0 {
                    Direction::Down
                } else {
                    Direction::Up
                }
            }
        })
    }

    fn distance_in_primary_axis(dir: Direction, dx: i32, dy: i32) -> i32 {
        match dir {
            Direction::Left | Direction::Right => dx.abs(),
            Direction::Up | Direction::Down => dy.abs(),
        }
    }

    fn accumulate_pending(&mut self, dir: Direction, distance: i32) {
        if self.pending_dir == Some(dir) {
            self.pending_accum += distance;
        } else {
            self.pending_dir = Some(dir);
            self.pending_accum = distance;
        }
    }

    fn flush_current_segment(&mut self) {
        if self.current_dir.is_none() && self.pending_accum >= self.direction_switch_confirm_px {
            self.current_dir = self.pending_dir.take();
            self.segment_accum = self.pending_accum;
            self.pending_accum = 0;
        }

        if let Some(current) = self.current_dir {
            self.confirm_segment(current);
        }

        self.current_dir = None;
        self.pending_dir = None;
        self.pending_accum = 0;
        self.segment_accum = 0;
    }

    fn confirm_segment(&mut self, current: Direction) {
        if self.segment_accum < self.min_segment_px {
            return;
        }
        let step = Self::direction_to_step(current);
        self.push_step(step, true);
    }

    fn push_step(&mut self, step: GestureStep, dedupe_consecutive: bool) {
        let steps = &self.steps[..usize::from(self.step_count)];
        if dedupe_consecutive && steps.last() == Some(&step) {
            return;
        }
        if self.step_count >= self.max_steps {
            self.overflowed = true;
            return;
        }
        self.steps[usize::from(self.step_count)] = step;
        self.step_count += 1;
    }

    fn direction_to_step(dir: Direction) -> GestureStep {
        match dir {
            Direction::Left => GestureStep::Left,
            Direction::Right => GestureStep::Right,
            Direction::Up => GestureStep::Up,
            Direction::Down => GestureStep::Down,
        }
    }
}

/// One fixed-capacity recognized sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GestureSequence {
    steps: [GestureStep; MAX_GESTURE_STEPS],
    len: u8,
}

impl GestureSequence {
    pub(super) fn as_slice(&self) -> &[GestureStep] {
        &self.steps[..usize::from(self.len)]
    }
}

impl Default for GestureRecognizer {
    fn default() -> Self {
        Self::new(
            DEFAULT_MIN_SEGMENT_PX,
            DEFAULT_DIRECTION_SWITCH_CONFIRM_PX,
            DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX,
            MAX_GESTURE_STEPS,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_mixed_movement_and_input_steps() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        rec.add_point(160, 100);
        rec.add_input_step(GestureStep::WheelUp);
        rec.add_point(160, 160);

        assert_eq!(
            rec.finalize_sequence()
                .map(|sequence| sequence.as_slice().to_vec()),
            Some(vec![
                GestureStep::Right,
                GestureStep::WheelUp,
                GestureStep::Down
            ])
        );
    }

    #[test]
    fn over_max_steps_invalidates_sequence() {
        let mut rec = GestureRecognizer::new(1, 1, 0, 2);
        rec.add_input_step(GestureStep::WheelUp);
        rec.add_input_step(GestureStep::WheelDown);
        rec.add_input_step(GestureStep::WheelUp);

        assert!(rec.current_sequence().is_none());
        assert!(rec.finalize_sequence().is_none());
    }

    #[test]
    fn finalize_flushes_current_direction() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        rec.add_point(100, 160);

        assert_eq!(
            rec.current_sequence()
                .map(|sequence| sequence.as_slice().to_vec()),
            Some(vec![GestureStep::Down])
        );
        assert_eq!(
            rec.finalize_sequence()
                .map(|sequence| sequence.as_slice().to_vec()),
            Some(vec![GestureStep::Down])
        );
    }

    #[test]
    fn reset_sequence_clears_steps_and_accepts_new_input() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        rec.add_point(160, 100);
        rec.add_input_step(GestureStep::WheelUp);

        assert_eq!(
            rec.current_sequence()
                .map(|sequence| sequence.as_slice().to_vec()),
            Some(vec![GestureStep::Right, GestureStep::WheelUp])
        );

        rec.reset_sequence();
        assert!(rec.current_sequence().is_none());

        rec.add_input_step(GestureStep::WheelDown);
        assert_eq!(
            rec.finalize_sequence()
                .map(|sequence| sequence.as_slice().to_vec()),
            Some(vec![GestureStep::WheelDown])
        );
    }
}
