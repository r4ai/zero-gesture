//! Gesture recognition from mouse movement sequences.
//!
//! Converts mouse movements into directions (L/R/U/D) and recognizes patterns
//! of up to 2 consecutive movement segments to identify one of 16 gesture types.

use std::cmp::Ordering;

use log::debug;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GestureKind {
    // Single direction (4 types)
    Left,
    Right,
    Up,
    Down,
    // Two-segment gestures (12 types)
    DownRight,   // [D, R]
    LeftUp,      // [L, U]
    RightUp,     // [R, U]
    RightDown,   // [R, D]
    UpLeft,      // [U, L]
    UpRight,     // [U, R]
    DownLeft,    // [D, L]
    LeftDown,    // [L, D]
    DownUp,      // [D, U]
    UpDown,      // [U, D]
    LeftRight,   // [L, R]
    RightLeft,   // [R, L]
}

/// Recognizes gesture patterns from accumulated mouse movement segments.
///
/// Tracks mouse movement in real time, accumulating distance within each
/// direction and "completing" a segment when movement changes direction
/// significantly. Maintains up to 2 confirmed segments for pattern matching.
#[derive(Debug)]
pub struct GestureRecognizer {
    /// Confirmed segments (max 2), stored as they are completed.
    segments: Vec<Direction>,
    /// Last recorded point (x, y).
    last_point: Option<(i32, i32)>,
    /// Current direction being accumulated.
    current_dir: Option<Direction>,
    /// Distance accumulated in the current segment (pixels).
    segment_accum: i32,
}

impl GestureRecognizer {
    /// Minimum distance (in pixels) before a segment is confirmed.
    const MIN_SEGMENT_PX: i32 = 30;

    /// Creates a new gesture recognizer.
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
            last_point: None,
            current_dir: None,
            segment_accum: 0,
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

        // Determine the primary direction based on which delta is larger in magnitude.
        let new_dir = match dx.abs().cmp(&dy.abs()) {
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
        };

        // If direction changed, confirm the previous segment and start a new one.
        if let Some(current) = self.current_dir {
            if new_dir != current && self.segment_accum >= Self::MIN_SEGMENT_PX {
                // Only push if it differs from the last confirmed segment (no duplicates).
                if self.segments.last() != Some(&current) {
                    self.segments.push(current);
                    if self.segments.len() > 2 {
                        self.segments.remove(0);
                    }
                    debug!("Segment confirmed: {:?} (segments: {:?})", current, self.segments);
                }
                self.segment_accum = 0;
            }
        }

        // Accumulate distance and update state.
        self.current_dir = Some(new_dir);
        // Accumulate the movement distance in the primary direction.
        let distance = match new_dir {
            Direction::Left | Direction::Right => dx.abs(),
            Direction::Up | Direction::Down => dy.abs(),
        };
        self.segment_accum += distance;
        self.last_point = Some((x, y));
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
            if self.segment_accum >= Self::MIN_SEGMENT_PX
                && effective_segments.last() != Some(&dir)
            {
                effective_segments.push(dir);
            }
        }

        // Keep only the last 2 segments.
        if effective_segments.len() > 2 {
            effective_segments = effective_segments[effective_segments.len() - 2..].to_vec();
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
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_direction_left() {
        let mut rec = GestureRecognizer::new();
        rec.add_point(100, 100); // Start
        rec.add_point(70, 100);  // Move left
        rec.add_point(50, 100);  // Continue left

        assert_eq!(rec.recognize(), Some(GestureKind::Left));
    }

    #[test]
    fn test_single_direction_right() {
        let mut rec = GestureRecognizer::new();
        rec.add_point(100, 100);
        rec.add_point(130, 100);
        rec.add_point(150, 100);

        assert_eq!(rec.recognize(), Some(GestureKind::Right));
    }

    #[test]
    fn test_single_direction_up() {
        let mut rec = GestureRecognizer::new();
        rec.add_point(100, 100);
        rec.add_point(100, 70);
        rec.add_point(100, 50);

        assert_eq!(rec.recognize(), Some(GestureKind::Up));
    }

    #[test]
    fn test_single_direction_down() {
        let mut rec = GestureRecognizer::new();
        rec.add_point(100, 100);
        rec.add_point(100, 130);
        rec.add_point(100, 150);

        assert_eq!(rec.recognize(), Some(GestureKind::Down));
    }

    #[test]
    fn test_two_segment_down_right() {
        let mut rec = GestureRecognizer::new();
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
        let mut rec = GestureRecognizer::new();
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
        let mut rec = GestureRecognizer::new();
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
        let mut rec = GestureRecognizer::new();
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
        let mut rec = GestureRecognizer::new();
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
        let mut rec = GestureRecognizer::new();
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
        let mut rec = GestureRecognizer::new();
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
        let mut rec = GestureRecognizer::new();
        rec.add_point(100, 100);
        // Move with equal horizontal and vertical component
        rec.add_point(120, 120);
        rec.add_point(140, 140);

        // Should prefer horizontal (Right)
        assert_eq!(rec.recognize(), Some(GestureKind::Right));
    }

    #[test]
    fn test_diagonal_prefers_vertical() {
        let mut rec = GestureRecognizer::new();
        rec.add_point(100, 100);
        // Move with larger vertical component
        rec.add_point(110, 140);
        rec.add_point(115, 170);

        // Should recognize as Down (vertical dominates)
        assert_eq!(rec.recognize(), Some(GestureKind::Down));
    }

    #[test]
    fn test_more_than_two_segments_keeps_last_two() {
        let mut rec = GestureRecognizer::new();
        rec.add_point(100, 100); // Start
        // Move right
        for i in 1..=5 {
            rec.add_point(100 + i * 10, 100);
        }
        // Move down
        for i in 1..=5 {
            rec.add_point(150, 100 + i * 10);
        }
        // Move left
        for i in 1..=5 {
            rec.add_point(150 - i * 10, 150);
        }

        // Only last two segments (Down, Left) should be kept
        assert_eq!(rec.recognize(), Some(GestureKind::DownLeft));
    }

    #[test]
    fn test_same_direction_not_duplicated() {
        let mut rec = GestureRecognizer::new();
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
}
