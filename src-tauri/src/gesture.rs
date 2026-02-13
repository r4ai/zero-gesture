//! Gesture recognition from mouse movement sequences.
//!
//! Converts mouse movements into directions (L/R/U/D) and recognizes patterns
//! of up to 2 consecutive movement segments to identify one of 16 gesture types.

use std::cmp::Ordering;

use log::debug;
use serde::{Deserialize, Serialize};

use crate::config::AppConfig;

/// A single direction of mouse movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Recognized gesture types: 16 distinct gesture patterns.
///
/// The mapping between gesture names and direction sequences:
/// - Single direction: [L], [R], [U], [D]
/// - Two-segment gestures: all combinations of direction pairs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GestureKind {
    // Single direction (4 types)
    Left,
    Right,
    Up,
    Down,
    // Two-segment gestures (12 types)
    DownRight, // [D, R]
    LeftUp,    // [L, U]
    RightUp,   // [R, U]
    RightDown, // [R, D]
    UpLeft,    // [U, L]
    UpRight,   // [U, R]
    DownLeft,  // [D, L]
    LeftDown,  // [L, D]
    DownUp,    // [D, U]
    UpDown,    // [U, D]
    LeftRight, // [L, R]
    RightLeft, // [R, L]
}

/// Recognizes gesture patterns from accumulated mouse movement segments.
///
/// Tracks mouse movement in real time, accumulating distance within each
/// direction and "completing" a segment when movement changes direction
/// significantly. Stores up to 2 confirmed segments for pattern matching.
///
/// If mouse movement produces more than 2 distinct segments, additional
/// segments are not stored — but recognition can still succeed if the
/// current (unconfirmed) direction matches the last confirmed segment,
/// effectively collapsing back to 2 segments.
#[derive(Debug)]
pub struct GestureRecognizer {
    /// Confirmed segments (capped at 2).
    segments: Vec<Direction>,
    /// Last recorded point (x, y).
    last_point: Option<(i32, i32)>,
    /// Current direction being accumulated.
    current_dir: Option<Direction>,
    /// Candidate direction waiting for hysteresis confirmation.
    pending_dir: Option<Direction>,
    /// Distance accumulated in the pending direction (pixels).
    pending_accum: i32,
    /// Distance accumulated in the current segment (pixels).
    segment_accum: i32,
    /// Minimum distance (in pixels) before a segment is confirmed.
    min_segment_px: i32,
    /// Minimum distance (in pixels) required to accept a direction change.
    direction_switch_confirm_px: i32,
    /// Deadzone used to ignore tiny diagonal moves with no clear dominant axis.
    axis_ambiguity_deadzone_px: i32,
}

impl GestureRecognizer {
    /// Default hysteresis threshold for direction changes.
    const DEFAULT_DIRECTION_SWITCH_CONFIRM_PX: i32 = 8;
    /// Default deadzone for ambiguous tiny diagonal movement.
    const DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX: i32 = 2;

    /// Creates a new gesture recognizer with the given minimum segment distance.
    pub fn new(min_segment_px: i32) -> Self {
        Self {
            segments: Vec::new(),
            last_point: None,
            current_dir: None,
            pending_dir: None,
            pending_accum: 0,
            segment_accum: 0,
            min_segment_px,
            direction_switch_confirm_px: Self::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX,
            axis_ambiguity_deadzone_px: Self::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX,
        }
    }

    /// Adds a new point to the recognizer, potentially recognizing direction changes.
    ///
    /// Calculates the direction of movement from the last recorded point to the
    /// new point. If the direction differs from the currently accumulated segment,
    /// the old segment is confirmed (if it exceeds the minimum distance) and a
    /// new segment begins.
    pub fn add_point(&mut self, x: i32, y: i32) {
        // Initialize the first point without calculating direction.
        if self.last_point.is_none() {
            self.last_point = Some((x, y));
            return;
        }

        let (lx, ly) = self.last_point.unwrap();
        let dx = x - lx;
        let dy = y - ly;

        // Skip zero-movement points.
        if dx == 0 && dy == 0 {
            return;
        }

        // Determine the primary direction based on which delta is larger in
        // magnitude. Tiny near-diagonal movement is treated as ambiguous jitter.
        let new_dir = match Self::classify_direction(dx, dy, self.axis_ambiguity_deadzone_px) {
            Some(dir) => dir,
            None => return,
        };

        let distance = Self::distance_in_primary_axis(new_dir, dx, dy);

        match self.current_dir {
            // No active direction yet: lock the first direction only after it
            // crosses a hysteresis threshold, so tiny low-speed wobble does not
            // pick a wrong axis immediately.
            None => {
                self.accumulate_pending(new_dir, distance);
                if self.pending_accum >= self.direction_switch_confirm_px {
                    self.current_dir = self.pending_dir.take();
                    self.segment_accum = self.pending_accum;
                    self.pending_accum = 0;
                }
            }
            Some(current) if new_dir == current => {
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
                // If both are equal, prefer horizontal over vertical.
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

    fn confirm_segment(&mut self, current: Direction) {
        if self.segment_accum < self.min_segment_px {
            return;
        }

        // Only push if it differs from the last confirmed segment (no duplicates)
        // and we haven't reached the 2-segment cap yet.
        if self.segments.last() == Some(&current) {
            return;
        }

        if self.segments.len() < 2 {
            self.segments.push(current);
            debug!(
                "Segment confirmed: {:?} (segments: {:?})",
                current, self.segments
            );
        } else {
            debug!(
                "Segment {:?} dropped (cap reached, segments: {:?})",
                current, self.segments
            );
        }
    }

    /// Attempts to recognize a gesture from the current segments and ongoing movement.
    ///
    /// Returns the matched [`GestureKind`] if the segments match one of the
    /// 16 defined patterns, or `None` if no match is found.
    ///
    /// Also considers the current accumulated direction if it hasn't been
    /// confirmed yet (i.e., no direction change has occurred).
    ///
    /// Only uses the last 2 segments (confirmed + current) for matching.
    pub fn recognize(&self) -> Option<GestureKind> {
        // Build the effective sequence: confirmed segments + current direction (if significant).
        // Skip the current direction if it duplicates the last confirmed segment.
        let mut effective_segments = self.segments.clone();
        if let Some(dir) = self.current_dir {
            if self.segment_accum >= self.min_segment_px && effective_segments.last() != Some(&dir)
            {
                effective_segments.push(dir);
            }
        }

        // 3+ effective segments means the gesture is too complex to match.
        if effective_segments.len() > 2 {
            return None;
        }

        match effective_segments.len() {
            1 => match effective_segments[0] {
                Direction::Left => Some(GestureKind::Left),
                Direction::Right => Some(GestureKind::Right),
                Direction::Up => Some(GestureKind::Up),
                Direction::Down => Some(GestureKind::Down),
            },
            2 => match (effective_segments[0], effective_segments[1]) {
                (Direction::Down, Direction::Right) => Some(GestureKind::DownRight),
                (Direction::Left, Direction::Up) => Some(GestureKind::LeftUp),
                (Direction::Right, Direction::Up) => Some(GestureKind::RightUp),
                (Direction::Right, Direction::Down) => Some(GestureKind::RightDown),
                (Direction::Up, Direction::Left) => Some(GestureKind::UpLeft),
                (Direction::Up, Direction::Right) => Some(GestureKind::UpRight),
                (Direction::Down, Direction::Left) => Some(GestureKind::DownLeft),
                (Direction::Left, Direction::Down) => Some(GestureKind::LeftDown),
                (Direction::Down, Direction::Up) => Some(GestureKind::DownUp),
                (Direction::Up, Direction::Down) => Some(GestureKind::UpDown),
                (Direction::Left, Direction::Right) => Some(GestureKind::LeftRight),
                (Direction::Right, Direction::Left) => Some(GestureKind::RightLeft),
                _ => None,
            },
            _ => None,
        }
    }
}

impl Default for GestureRecognizer {
    fn default() -> Self {
        Self::new(AppConfig::DEFAULT_MIN_SEGMENT_PX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_direction_left() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
        rec.add_point(70, 100); // Move left
        rec.add_point(50, 100); // Continue left

        assert_eq!(rec.recognize(), Some(GestureKind::Left));
    }

    #[test]
    fn test_single_direction_right() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        rec.add_point(130, 100);
        rec.add_point(150, 100);

        assert_eq!(rec.recognize(), Some(GestureKind::Right));
    }

    #[test]
    fn test_single_direction_up() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        rec.add_point(100, 70);
        rec.add_point(100, 50);

        assert_eq!(rec.recognize(), Some(GestureKind::Up));
    }

    #[test]
    fn test_single_direction_down() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        rec.add_point(100, 130);
        rec.add_point(100, 150);

        assert_eq!(rec.recognize(), Some(GestureKind::Down));
    }

    #[test]
    fn test_two_segment_down_right() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move down
        rec.add_point(100, 130);
        rec.add_point(100, 150);
        rec.add_point(100, 170);
        // Move right
        rec.add_point(130, 170);
        rec.add_point(150, 170);
        rec.add_point(170, 170);

        assert_eq!(rec.recognize(), Some(GestureKind::DownRight));
    }

    #[test]
    fn test_two_segment_left_up() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move left
        rec.add_point(70, 100);
        rec.add_point(50, 100);
        rec.add_point(30, 100);
        // Move up
        rec.add_point(30, 70);
        rec.add_point(30, 50);
        rec.add_point(30, 30);

        assert_eq!(rec.recognize(), Some(GestureKind::LeftUp));
    }

    #[test]
    fn test_two_segment_right_up() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move right
        rec.add_point(130, 100);
        rec.add_point(150, 100);
        rec.add_point(170, 100);
        // Move up
        rec.add_point(170, 70);
        rec.add_point(170, 50);
        rec.add_point(170, 30);

        assert_eq!(rec.recognize(), Some(GestureKind::RightUp));
    }

    #[test]
    fn test_two_segment_right_down() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move right
        rec.add_point(130, 100);
        rec.add_point(150, 100);
        rec.add_point(170, 100);
        // Move down
        rec.add_point(170, 130);
        rec.add_point(170, 150);
        rec.add_point(170, 170);

        assert_eq!(rec.recognize(), Some(GestureKind::RightDown));
    }

    #[test]
    fn test_two_segment_up_left() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move up
        rec.add_point(100, 70);
        rec.add_point(100, 50);
        rec.add_point(100, 30);
        // Move left
        rec.add_point(70, 30);
        rec.add_point(50, 30);
        rec.add_point(30, 30);

        assert_eq!(rec.recognize(), Some(GestureKind::UpLeft));
    }

    #[test]
    fn test_two_segment_up_right() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move up
        rec.add_point(100, 70);
        rec.add_point(100, 50);
        rec.add_point(100, 30);
        // Move right
        rec.add_point(130, 30);
        rec.add_point(150, 30);
        rec.add_point(170, 30);

        assert_eq!(rec.recognize(), Some(GestureKind::UpRight));
    }

    #[test]
    fn test_short_segment_not_confirmed() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move left only a small amount (< MIN_SEGMENT_PX)
        rec.add_point(95, 100);
        // Then move down significantly
        rec.add_point(95, 150);
        rec.add_point(95, 170);

        // Only the down movement should be recognized as a single segment.
        assert_eq!(rec.recognize(), Some(GestureKind::Down));
    }

    #[test]
    fn test_diagonal_prefers_horizontal() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        // Move with equal horizontal and vertical component
        rec.add_point(120, 120);
        rec.add_point(140, 140);

        // Should prefer horizontal (Right)
        assert_eq!(rec.recognize(), Some(GestureKind::Right));
    }

    #[test]
    fn test_diagonal_prefers_vertical() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        // Move with larger vertical component
        rec.add_point(110, 140);
        rec.add_point(115, 170);

        // Should recognize as Down (vertical dominates)
        assert_eq!(rec.recognize(), Some(GestureKind::Down));
    }

    #[test]
    fn test_three_distinct_segments_returns_none() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move right
        for i in 1..=5 {
            rec.add_point(100 + i * 10, 100);
        }
        // Move down
        for i in 1..=5 {
            rec.add_point(150, 100 + i * 10);
        }
        // Move left — current direction is a 3rd distinct direction
        for i in 1..=5 {
            rec.add_point(150 - i * 10, 150);
        }

        // 3 distinct effective segments [Right, Down, Left] → None
        assert_eq!(rec.recognize(), None);
    }

    #[test]
    fn test_three_segments_recovers_if_current_matches_last() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move right
        for i in 1..=5 {
            rec.add_point(100 + i * 10, 100);
        }
        // Move down
        for i in 1..=5 {
            rec.add_point(150, 100 + i * 10);
        }
        // Move left (3rd distinct direction, but only intermediate)
        for i in 1..=5 {
            rec.add_point(150 - i * 10, 150);
        }
        // Move back down — current direction matches 2nd segment (Down)
        for i in 1..=5 {
            rec.add_point(100, 150 + i * 10);
        }

        // effective_segments = [Right, Down] (current Down == last segment, skipped)
        assert_eq!(rec.recognize(), Some(GestureKind::RightDown));
    }

    #[test]
    fn test_same_direction_not_duplicated() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100); // Start
                                 // Move right significantly
        rec.add_point(140, 100);
        // Briefly move left (< MIN_SEGMENT_PX, so Right is confirmed but Left is not)
        rec.add_point(130, 100);
        // Move right again significantly
        rec.add_point(170, 100);
        rec.add_point(200, 100);

        // Should still be just Right, not Right+Right
        assert_eq!(rec.recognize(), Some(GestureKind::Right));
    }

    #[test]
    fn test_initial_direction_hysteresis_handles_slow_down_with_wobble() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);

        // Slow downward movement with repeated tiny horizontal wobble.
        rec.add_point(100, 109);
        rec.add_point(101, 110);
        rec.add_point(101, 119);
        rec.add_point(102, 120);
        rec.add_point(102, 129);
        rec.add_point(103, 130);
        rec.add_point(103, 139);

        assert_eq!(rec.recognize(), Some(GestureKind::Down));
    }

    #[test]
    fn test_two_segment_down_left() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        for i in 1..=5 {
            rec.add_point(100, 100 + i * 10);
        }
        for i in 1..=5 {
            rec.add_point(100 - i * 10, 150);
        }
        assert_eq!(rec.recognize(), Some(GestureKind::DownLeft));
    }

    #[test]
    fn test_two_segment_left_down() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        for i in 1..=5 {
            rec.add_point(100 - i * 10, 100);
        }
        for i in 1..=5 {
            rec.add_point(50, 100 + i * 10);
        }
        assert_eq!(rec.recognize(), Some(GestureKind::LeftDown));
    }

    #[test]
    fn test_two_segment_down_up() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        for i in 1..=5 {
            rec.add_point(100, 100 + i * 10);
        }
        for i in 1..=5 {
            rec.add_point(100, 150 - i * 10);
        }
        assert_eq!(rec.recognize(), Some(GestureKind::DownUp));
    }

    #[test]
    fn test_two_segment_up_down() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        for i in 1..=5 {
            rec.add_point(100, 100 - i * 10);
        }
        for i in 1..=5 {
            rec.add_point(100, 50 + i * 10);
        }
        assert_eq!(rec.recognize(), Some(GestureKind::UpDown));
    }

    #[test]
    fn test_two_segment_left_right() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        for i in 1..=5 {
            rec.add_point(100 - i * 10, 100);
        }
        for i in 1..=5 {
            rec.add_point(50 + i * 10, 100);
        }
        assert_eq!(rec.recognize(), Some(GestureKind::LeftRight));
    }

    #[test]
    fn test_two_segment_right_left() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        for i in 1..=5 {
            rec.add_point(100 + i * 10, 100);
        }
        for i in 1..=5 {
            rec.add_point(150 - i * 10, 100);
        }
        assert_eq!(rec.recognize(), Some(GestureKind::RightLeft));
    }

    #[test]
    fn test_micro_diagonal_move_is_treated_as_ambiguous() {
        let mut rec = GestureRecognizer::default();
        rec.add_point(100, 100);
        rec.add_point(101, 101);

        assert_eq!(rec.current_dir, None);
        assert_eq!(rec.pending_dir, None);
        assert_eq!(rec.pending_accum, 0);
    }
}
