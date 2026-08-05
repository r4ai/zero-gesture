//! Engine-owned one-shot window capture state.
//!
//! The native callback only performs fixed atomic operations and stores the
//! click point. Window lookup and metadata collection happen later on the
//! Engine IPC thread.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::domain::{MouseEvent, Point, TriggerButton};

const PHASE_MASK: u64 = 0b11;
const PHASE_CANCELLED: u64 = 0;
const PHASE_ACTIVE: u64 = 1;
const PHASE_WRITING: u64 = 2;
const PHASE_CAPTURED: u64 = 3;
const MAX_EPOCH: u64 = u64::MAX >> 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CaptureError {
    Stale,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CapturePoll {
    Pending,
    Captured(Point),
}

pub(crate) struct WindowCapture {
    next_epoch: AtomicU64,
    state: AtomicU64,
    owner_session: AtomicU64,
    capture_id: AtomicU64,
    point: AtomicU64,
    shut_down: AtomicBool,
}

impl WindowCapture {
    pub(crate) const fn new() -> Self {
        Self {
            next_epoch: AtomicU64::new(0),
            state: AtomicU64::new(0),
            owner_session: AtomicU64::new(0),
            capture_id: AtomicU64::new(0),
            point: AtomicU64::new(0),
            shut_down: AtomicBool::new(false),
        }
    }

    pub(crate) fn begin(&self, owner_session: u64, capture_id: u64) -> Result<u64, CaptureError> {
        if self.shut_down.load(Ordering::Acquire) {
            return Err(CaptureError::Unavailable);
        }
        let epoch = self.next_epoch()?;
        self.state
            .store(pack_state(epoch, PHASE_CANCELLED), Ordering::Release);
        self.owner_session.store(owner_session, Ordering::Relaxed);
        self.capture_id.store(capture_id, Ordering::Relaxed);
        self.state
            .store(pack_state(epoch, PHASE_ACTIVE), Ordering::Release);
        Ok(epoch)
    }

    pub(crate) fn poll(
        &self,
        owner_session: u64,
        capture_id: u64,
        epoch: u64,
    ) -> Result<CapturePoll, CaptureError> {
        let state = self.matching_state(owner_session, capture_id, epoch)?;
        match phase(state) {
            PHASE_ACTIVE | PHASE_WRITING => Ok(CapturePoll::Pending),
            PHASE_CAPTURED => Ok(CapturePoll::Captured(unpack_point(
                self.point.load(Ordering::Acquire),
            ))),
            _ => Err(CaptureError::Stale),
        }
    }

    pub(crate) fn cancel(
        &self,
        owner_session: u64,
        capture_id: u64,
        epoch: u64,
    ) -> Result<(), CaptureError> {
        loop {
            let state = self.matching_state(owner_session, capture_id, epoch)?;
            if phase(state) == PHASE_CANCELLED {
                return Err(CaptureError::Stale);
            }
            if self
                .state
                .compare_exchange(
                    state,
                    pack_state(epoch, PHASE_CANCELLED),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    pub(crate) fn disconnect(&self, owner_session: u64) {
        loop {
            let state = self.state.load(Ordering::Acquire);
            if phase(state) == PHASE_CANCELLED
                || self.owner_session.load(Ordering::Relaxed) != owner_session
            {
                return;
            }
            if self
                .state
                .compare_exchange(
                    state,
                    pack_state(epoch(state), PHASE_CANCELLED),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return;
            }
        }
    }

    pub(crate) fn shutdown(&self) {
        self.shut_down.store(true, Ordering::Release);
        self.state.store(0, Ordering::Release);
    }

    /// Callback-facing capture operation: one state load/CAS and one point
    /// store for the matching real left-button down. A stale/replaced race
    /// returns `false`, so normal input remains fail-open.
    pub(crate) fn try_record(&self, event: MouseEvent, point: Point) -> bool {
        if event != MouseEvent::ButtonDown(TriggerButton::Left) {
            return false;
        }
        let active = self.state.load(Ordering::Acquire);
        if phase(active) != PHASE_ACTIVE
            || self
                .state
                .compare_exchange(
                    active,
                    pack_state(epoch(active), PHASE_WRITING),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
        {
            return false;
        }
        self.point.store(pack_point(point), Ordering::Relaxed);
        self.state
            .compare_exchange(
                pack_state(epoch(active), PHASE_WRITING),
                pack_state(epoch(active), PHASE_CAPTURED),
                Ordering::Release,
                Ordering::Relaxed,
            )
            .is_ok()
    }

    fn matching_state(
        &self,
        owner_session: u64,
        capture_id: u64,
        requested_epoch: u64,
    ) -> Result<u64, CaptureError> {
        let state = self.state.load(Ordering::Acquire);
        let matches = epoch(state) == requested_epoch
            && self.owner_session.load(Ordering::Relaxed) == owner_session
            && self.capture_id.load(Ordering::Relaxed) == capture_id
            && self.state.load(Ordering::Acquire) == state;
        matches.then_some(state).ok_or(CaptureError::Stale)
    }

    fn next_epoch(&self) -> Result<u64, CaptureError> {
        let mut current = self.next_epoch.load(Ordering::Relaxed);
        loop {
            if current == MAX_EPOCH {
                return Err(CaptureError::Unavailable);
            }
            match self.next_epoch.compare_exchange_weak(
                current,
                current + 1,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(current + 1),
                Err(actual) => current = actual,
            }
        }
    }
}

fn pack_state(epoch: u64, phase: u64) -> u64 {
    epoch << 2 | phase
}

fn epoch(state: u64) -> u64 {
    state >> 2
}

fn phase(state: u64) -> u64 {
    state & PHASE_MASK
}

fn pack_point(point: Point) -> u64 {
    u64::from(point.x as u32) << 32 | u64::from(point.y as u32)
}

fn unpack_point(point: u64) -> Point {
    Point::new((point >> 32) as u32 as i32, point as u32 as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn begin_replaces_the_active_epoch_and_stale_results_never_apply() {
        let capture = WindowCapture::new();
        let first = capture.begin(41, 7).unwrap();
        let second = capture.begin(41, 8).unwrap();

        assert!(capture.try_record(
            MouseEvent::ButtonDown(TriggerButton::Left),
            Point::new(20, 30)
        ));
        assert_eq!(capture.poll(41, 7, first), Err(CaptureError::Stale));
        assert_eq!(
            capture.poll(41, 8, second),
            Ok(CapturePoll::Captured(Point::new(20, 30)))
        );
    }

    #[test]
    fn cancel_and_disconnect_invalidate_only_the_matching_owner() {
        let capture = WindowCapture::new();
        let epoch = capture.begin(11, 100).unwrap();
        capture.cancel(11, 100, epoch).unwrap();
        assert!(!capture.try_record(
            MouseEvent::ButtonDown(TriggerButton::Left),
            Point::new(1, 2)
        ));

        let replacement = capture.begin(12, 101).unwrap();
        capture.disconnect(11);
        assert_eq!(capture.poll(12, 101, replacement), Ok(CapturePoll::Pending));
        capture.disconnect(12);
        assert_eq!(capture.poll(12, 101, replacement), Err(CaptureError::Stale));
    }

    #[test]
    fn overload_or_non_capture_input_remains_fail_open() {
        let capture = WindowCapture::new();
        capture.begin(1, 1).unwrap();
        assert!(!capture.try_record(MouseEvent::MouseMove, Point::new(5, 6)));
        assert!(capture.try_record(
            MouseEvent::ButtonDown(TriggerButton::Left),
            Point::new(7, 8)
        ));
        assert!(!capture.try_record(
            MouseEvent::ButtonDown(TriggerButton::Left),
            Point::new(9, 10)
        ));
    }

    #[test]
    fn engine_shutdown_invalidates_capture_and_rejects_new_begin() {
        let capture = WindowCapture::new();
        let epoch = capture.begin(1, 1).unwrap();
        capture.shutdown();

        assert_eq!(capture.poll(1, 1, epoch), Err(CaptureError::Stale));
        assert_eq!(capture.begin(1, 2), Err(CaptureError::Unavailable));
        assert!(!capture.try_record(
            MouseEvent::ButtonDown(TriggerButton::Left),
            Point::new(1, 2)
        ));
    }
}
