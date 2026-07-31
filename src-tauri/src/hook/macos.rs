//! macOS listen-only event-tap owner.
//!
//! The Core Graphics callback only normalizes fields already present in the
//! event, appends to one fixed SPSC queue, and updates atomics. The run-loop
//! side drains the queue and performs conservative tap re-enablement.

use std::cell::UnsafeCell;
#[cfg(target_os = "macos")]
use std::ffi::c_void;
use std::mem::MaybeUninit;
#[cfg(target_os = "macos")]
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam_channel::Sender;
#[cfg(target_os = "macos")]
use log::warn;

use super::{HookEvent, HookFailure};
use crate::domain::{MouseEvent, Point, TriggerButton};

const EVENT_QUEUE_CAPACITY: usize = 64;
#[cfg(target_os = "macos")]
const RUN_LOOP_SLICE_SECONDS: f64 = 0.01;
const DEGRADED_STOP_POLL: Duration = Duration::from_millis(10);

const EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const EVENT_LEFT_MOUSE_UP: u32 = 2;
const EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const EVENT_RIGHT_MOUSE_UP: u32 = 4;
const EVENT_MOUSE_MOVED: u32 = 5;
const EVENT_LEFT_MOUSE_DRAGGED: u32 = 6;
const EVENT_RIGHT_MOUSE_DRAGGED: u32 = 7;
const EVENT_SCROLL_WHEEL: u32 = 22;
const EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const EVENT_OTHER_MOUSE_UP: u32 = 26;
const EVENT_OTHER_MOUSE_DRAGGED: u32 = 27;
#[cfg(target_os = "macos")]
const EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = u32::MAX - 1;
#[cfg(target_os = "macos")]
const EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = u32::MAX;

#[cfg(target_os = "macos")]
const EVENT_FIELD_MOUSE_BUTTON_NUMBER: u32 = 3;
#[cfg(target_os = "macos")]
const EVENT_FIELD_SCROLL_DELTA_AXIS_1: u32 = 11;
const MIDDLE_MOUSE_BUTTON: i64 = 2;

#[cfg(target_os = "macos")]
const SESSION_EVENT_TAP: u32 = 1;
#[cfg(target_os = "macos")]
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const LISTEN_ONLY_EVENT_TAP: u32 = 1;

#[cfg(target_os = "macos")]
type CGEventRef = *mut c_void;
#[cfg(target_os = "macos")]
type CFMachPortRef = *mut c_void;
#[cfg(target_os = "macos")]
type CFRunLoopRef = *mut c_void;
#[cfg(target_os = "macos")]
type CFRunLoopSourceRef = *mut c_void;
#[cfg(target_os = "macos")]
type CFStringRef = *const c_void;

#[cfg(target_os = "macos")]
#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[cfg(target_os = "macos")]
type EventTapCallback = unsafe extern "C" fn(
    proxy: *mut c_void,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: Option<EventTapCallback>,
        user_info: *mut c_void,
    ) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventTapIsEnabled(tap: CFMachPortRef) -> bool;
    fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    fn CGEventGetTimestamp(event: CGEventRef) -> u64;
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopDefaultMode: CFStringRef;

    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFMachPortInvalidate(port: CFMachPortRef);
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRemoveSource(run_loop: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    fn CFRunLoopRunInMode(mode: CFStringRef, seconds: f64, return_after_source: bool) -> i32;
    fn CFRelease(value: *const c_void);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NormalizedInput {
    sequence: u64,
    event: MouseEvent,
    point: Point,
    timestamp_ns: u64,
}

struct SpscQueue<T: Copy, const N: usize> {
    slots: [UnsafeCell<MaybeUninit<T>>; N],
    head: AtomicUsize,
    tail: AtomicUsize,
}

// The queue contract has exactly one callback producer and one run-loop
// consumer. Release/acquire publication prevents a consumer from reading a
// slot before the producer finishes writing it.
unsafe impl<T: Copy + Send, const N: usize> Sync for SpscQueue<T, N> {}

impl<T: Copy, const N: usize> SpscQueue<T, N> {
    fn new() -> Self {
        assert!(N > 0);
        Self {
            slots: [const { UnsafeCell::new(MaybeUninit::uninit()) }; N],
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    fn push(&self, value: T) -> bool {
        let tail = self.tail.load(Ordering::Relaxed);
        let head = self.head.load(Ordering::Acquire);
        if tail.wrapping_sub(head) == N {
            return false;
        }
        unsafe {
            (*self.slots[tail % N].get()).write(value);
        }
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        true
    }

    fn pop(&self) -> Option<T> {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return None;
        }
        let value = unsafe { (*self.slots[head % N].get()).assume_init_read() };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(value)
    }
}

struct TapState {
    queue: SpscQueue<NormalizedInput, EVENT_QUEUE_CAPACITY>,
    next_sequence: AtomicU64,
    received: AtomicU64,
    enqueued: AtomicU64,
    dropped: AtomicU64,
    processed: AtomicU64,
    disabled: AtomicU64,
    reenable_requested: AtomicBool,
    #[cfg(target_os = "macos")]
    reenable_attempts: AtomicU64,
    #[cfg(target_os = "macos")]
    reenable_failures: AtomicU64,
}

impl TapState {
    fn new() -> Self {
        Self {
            queue: SpscQueue::new(),
            next_sequence: AtomicU64::new(0),
            received: AtomicU64::new(0),
            enqueued: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            processed: AtomicU64::new(0),
            disabled: AtomicU64::new(0),
            reenable_requested: AtomicBool::new(false),
            #[cfg(target_os = "macos")]
            reenable_attempts: AtomicU64::new(0),
            #[cfg(target_os = "macos")]
            reenable_failures: AtomicU64::new(0),
        }
    }

    fn capture(&self, event: MouseEvent, point: Point, timestamp_ns: u64) {
        self.received.fetch_add(1, Ordering::Relaxed);
        let input = NormalizedInput {
            sequence: self.next_sequence.fetch_add(1, Ordering::Relaxed),
            event,
            point,
            timestamp_ns,
        };
        if self.queue.push(input) {
            self.enqueued.fetch_add(1, Ordering::Relaxed);
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn note_disabled(&self) {
        self.disabled.fetch_add(1, Ordering::Relaxed);
        self.reenable_requested.store(true, Ordering::Release);
    }

    fn take_reenable_request(&self) -> bool {
        self.reenable_requested.swap(false, Ordering::AcqRel)
    }

    fn drain(&self) {
        while self.queue.pop().is_some() {
            // P04b2 deliberately stops at the normalized Engine boundary.
            // Context, InputKernel, suppression, actions, and rendering are
            // connected only in later phases.
            self.processed.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[cfg(target_os = "macos")]
    fn reenable_if_requested(&self, tap: CFMachPortRef) {
        if !self.take_reenable_request() {
            return;
        }
        self.reenable_attempts.fetch_add(1, Ordering::Relaxed);
        unsafe {
            CGEventTapEnable(tap, true);
            if !CGEventTapIsEnabled(tap) {
                self.reenable_failures.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    #[cfg(test)]
    fn snapshot(&self) -> KpiSnapshot {
        KpiSnapshot {
            received: self.received.load(Ordering::Relaxed),
            enqueued: self.enqueued.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            processed: self.processed.load(Ordering::Relaxed),
            disabled: self.disabled.load(Ordering::Relaxed),
        }
    }
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
struct KpiSnapshot {
    received: u64,
    enqueued: u64,
    dropped: u64,
    processed: u64,
    disabled: u64,
}

#[cfg(target_os = "macos")]
struct TapResources {
    run_loop: CFRunLoopRef,
    source: CFRunLoopSourceRef,
    tap: CFMachPortRef,
}

#[cfg(target_os = "macos")]
impl Drop for TapResources {
    fn drop(&mut self) {
        unsafe {
            CGEventTapEnable(self.tap, false);
            CFRunLoopRemoveSource(self.run_loop, self.source, kCFRunLoopDefaultMode);
            CFMachPortInvalidate(self.tap);
            CFRelease(self.source.cast_const());
            CFRelease(self.tap.cast_const());
        }
    }
}

#[cfg(target_os = "macos")]
enum StartupMode {
    Active(TapResources),
    PermissionDenied,
    CreationFailed,
}

#[cfg(target_os = "macos")]
pub(super) fn run_loop_macos(
    stop: Arc<AtomicBool>,
    events: Sender<HookEvent>,
) -> Result<(), HookFailure> {
    let state = Box::new(TapState::new());
    let mode = unsafe { start_event_tap(state.as_ref()) };
    match mode {
        StartupMode::Active(resources) => run_active(stop, events, state, resources),
        StartupMode::PermissionDenied => {
            warn!("macOS input owner is pass-through: Listen Event access is unavailable");
            run_degraded(stop, events)
        }
        StartupMode::CreationFailed => {
            warn!("macOS input owner is pass-through: event tap creation failed");
            run_degraded(stop, events)
        }
    }
}

fn run_degraded(stop: Arc<AtomicBool>, events: Sender<HookEvent>) -> Result<(), HookFailure> {
    publish_ready(&events)?;
    while !stop.load(Ordering::Acquire) {
        thread::sleep(DEGRADED_STOP_POLL);
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn run_active(
    stop: Arc<AtomicBool>,
    events: Sender<HookEvent>,
    state: Box<TapState>,
    resources: TapResources,
) -> Result<(), HookFailure> {
    publish_ready(&events)?;
    while !stop.load(Ordering::Acquire) {
        unsafe {
            CFRunLoopRunInMode(kCFRunLoopDefaultMode, RUN_LOOP_SLICE_SECONDS, false);
        }
        state.drain();
        state.reenable_if_requested(resources.tap);
    }
    state.drain();
    drop(resources);
    Ok(())
}

fn publish_ready(events: &Sender<HookEvent>) -> Result<(), HookFailure> {
    events
        .send(HookEvent::Ready(1))
        .map_err(|_| HookFailure::new("event tap", "readiness receiver disappeared"))
}

#[cfg(target_os = "macos")]
unsafe fn start_event_tap(state: &TapState) -> StartupMode {
    if !CGPreflightListenEventAccess() {
        return StartupMode::PermissionDenied;
    }
    let tap = CGEventTapCreate(
        SESSION_EVENT_TAP,
        HEAD_INSERT_EVENT_TAP,
        LISTEN_ONLY_EVENT_TAP,
        mouse_event_mask(),
        Some(event_tap_callback),
        ptr::from_ref(state).cast_mut().cast(),
    );
    if tap.is_null() {
        return StartupMode::CreationFailed;
    }
    let source = CFMachPortCreateRunLoopSource(ptr::null(), tap, 0);
    if source.is_null() {
        CFMachPortInvalidate(tap);
        CFRelease(tap.cast_const());
        return StartupMode::CreationFailed;
    }
    let run_loop = CFRunLoopGetCurrent();
    CFRunLoopAddSource(run_loop, source, kCFRunLoopDefaultMode);
    CGEventTapEnable(tap, true);
    if !CGEventTapIsEnabled(tap) {
        CFRunLoopRemoveSource(run_loop, source, kCFRunLoopDefaultMode);
        CFMachPortInvalidate(tap);
        CFRelease(source.cast_const());
        CFRelease(tap.cast_const());
        return StartupMode::CreationFailed;
    }
    StartupMode::Active(TapResources {
        run_loop,
        source,
        tap,
    })
}

#[cfg(target_os = "macos")]
unsafe extern "C" fn event_tap_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    let state = &*user_info.cast::<TapState>();
    if matches!(
        event_type,
        EVENT_TAP_DISABLED_BY_TIMEOUT | EVENT_TAP_DISABLED_BY_USER_INPUT
    ) {
        state.note_disabled();
        return event;
    }

    let location = CGEventGetLocation(event);
    let button = if matches!(event_type, EVENT_OTHER_MOUSE_DOWN | EVENT_OTHER_MOUSE_UP) {
        CGEventGetIntegerValueField(event, EVENT_FIELD_MOUSE_BUTTON_NUMBER)
    } else {
        0
    };
    let scroll = if event_type == EVENT_SCROLL_WHEEL {
        CGEventGetIntegerValueField(event, EVENT_FIELD_SCROLL_DELTA_AXIS_1)
    } else {
        0
    };
    if let Some(mouse_event) = normalize_event(event_type, button, scroll) {
        state.capture(
            mouse_event,
            Point::new(location.x.round() as i32, location.y.round() as i32),
            CGEventGetTimestamp(event),
        );
    }
    event
}

fn normalize_event(event_type: u32, button: i64, scroll: i64) -> Option<MouseEvent> {
    match event_type {
        EVENT_LEFT_MOUSE_DOWN => Some(MouseEvent::ButtonDown(TriggerButton::Left)),
        EVENT_LEFT_MOUSE_UP => Some(MouseEvent::ButtonUp(TriggerButton::Left)),
        EVENT_RIGHT_MOUSE_DOWN => Some(MouseEvent::ButtonDown(TriggerButton::Right)),
        EVENT_RIGHT_MOUSE_UP => Some(MouseEvent::ButtonUp(TriggerButton::Right)),
        EVENT_OTHER_MOUSE_DOWN if button == MIDDLE_MOUSE_BUTTON => {
            Some(MouseEvent::ButtonDown(TriggerButton::Middle))
        }
        EVENT_OTHER_MOUSE_UP if button == MIDDLE_MOUSE_BUTTON => {
            Some(MouseEvent::ButtonUp(TriggerButton::Middle))
        }
        EVENT_MOUSE_MOVED
        | EVENT_LEFT_MOUSE_DRAGGED
        | EVENT_RIGHT_MOUSE_DRAGGED
        | EVENT_OTHER_MOUSE_DRAGGED => Some(MouseEvent::MouseMove),
        EVENT_SCROLL_WHEEL if scroll > 0 => Some(MouseEvent::WheelUp(scroll_steps(scroll))),
        EVENT_SCROLL_WHEEL if scroll < 0 => Some(MouseEvent::WheelDown(scroll_steps(scroll))),
        EVENT_SCROLL_WHEEL => Some(MouseEvent::Other),
        EVENT_OTHER_MOUSE_DOWN | EVENT_OTHER_MOUSE_UP => Some(MouseEvent::Other),
        _ => None,
    }
}

fn scroll_steps(delta: i64) -> u16 {
    delta.unsigned_abs().min(u64::from(u16::MAX)) as u16
}

const fn mouse_event_mask() -> u64 {
    event_mask(EVENT_LEFT_MOUSE_DOWN)
        | event_mask(EVENT_LEFT_MOUSE_UP)
        | event_mask(EVENT_RIGHT_MOUSE_DOWN)
        | event_mask(EVENT_RIGHT_MOUSE_UP)
        | event_mask(EVENT_MOUSE_MOVED)
        | event_mask(EVENT_LEFT_MOUSE_DRAGGED)
        | event_mask(EVENT_RIGHT_MOUSE_DRAGGED)
        | event_mask(EVENT_SCROLL_WHEEL)
        | event_mask(EVENT_OTHER_MOUSE_DOWN)
        | event_mask(EVENT_OTHER_MOUSE_UP)
        | event_mask(EVENT_OTHER_MOUSE_DRAGGED)
}

const fn event_mask(event_type: u32) -> u64 {
    1_u64 << event_type
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::input::tests::count_allocations;

    #[test]
    fn callback_core_normalizes_mouse_input_without_allocating() {
        let state = TapState::new();
        let point = Point::new(17, -9);
        let (_, allocations) = count_allocations(|| {
            state.capture(MouseEvent::ButtonDown(TriggerButton::Right), point, 41);
            state.capture(MouseEvent::MouseMove, point, 42);
            state.capture(MouseEvent::WheelDown(3), point, 43);
        });

        assert_eq!(allocations, 0);
        assert_eq!(
            state.queue.pop(),
            Some(NormalizedInput {
                sequence: 0,
                event: MouseEvent::ButtonDown(TriggerButton::Right),
                point,
                timestamp_ns: 41,
            })
        );
        assert_eq!(state.queue.pop().unwrap().sequence, 1);
        assert_eq!(state.queue.pop().unwrap().sequence, 2);
    }

    #[test]
    fn callback_queue_overload_drops_new_input_and_preserves_fifo_order() {
        let state = TapState::new();
        for timestamp_ns in 0..EVENT_QUEUE_CAPACITY as u64 {
            state.capture(MouseEvent::MouseMove, Point::new(0, 0), timestamp_ns);
        }
        state.capture(MouseEvent::MouseMove, Point::new(1, 1), 99);

        for expected in 0..EVENT_QUEUE_CAPACITY as u64 {
            assert_eq!(state.queue.pop().unwrap().sequence, expected);
        }
        assert!(state.queue.pop().is_none());
        assert_eq!(
            state.snapshot(),
            KpiSnapshot {
                received: 65,
                enqueued: 64,
                dropped: 1,
                processed: 0,
                disabled: 0,
            }
        );
    }

    #[test]
    fn tap_disable_notifications_coalesce_for_worker_side_reenable() {
        let state = TapState::new();
        state.note_disabled();
        state.note_disabled();

        assert!(state.take_reenable_request());
        assert!(!state.take_reenable_request());
        assert_eq!(state.snapshot().disabled, 2);
    }

    #[test]
    fn worker_drain_consumes_every_accepted_input_in_callback_order() {
        let state = TapState::new();
        state.capture(MouseEvent::MouseMove, Point::new(1, 2), 1);
        state.capture(MouseEvent::MouseMove, Point::new(3, 4), 2);

        state.drain();

        assert!(state.queue.pop().is_none());
        assert_eq!(state.snapshot().processed, 2);
    }

    #[test]
    fn macos_event_mapping_matches_the_portable_mouse_boundary() {
        assert_eq!(
            normalize_event(EVENT_LEFT_MOUSE_DOWN, 0, 0),
            Some(MouseEvent::ButtonDown(TriggerButton::Left))
        );
        assert_eq!(
            normalize_event(EVENT_OTHER_MOUSE_UP, MIDDLE_MOUSE_BUTTON, 0),
            Some(MouseEvent::ButtonUp(TriggerButton::Middle))
        );
        assert_eq!(
            normalize_event(EVENT_SCROLL_WHEEL, 0, -(u16::MAX as i64) - 1),
            Some(MouseEvent::WheelDown(u16::MAX))
        );
        assert_eq!(
            normalize_event(EVENT_OTHER_MOUSE_DOWN, 4, 0),
            Some(MouseEvent::Other)
        );
    }

    #[test]
    fn listen_only_mask_contains_every_supported_mouse_event() {
        assert_eq!(LISTEN_ONLY_EVENT_TAP, 1);
        for event_type in [
            EVENT_LEFT_MOUSE_DOWN,
            EVENT_LEFT_MOUSE_UP,
            EVENT_RIGHT_MOUSE_DOWN,
            EVENT_RIGHT_MOUSE_UP,
            EVENT_MOUSE_MOVED,
            EVENT_LEFT_MOUSE_DRAGGED,
            EVENT_RIGHT_MOUSE_DRAGGED,
            EVENT_SCROLL_WHEEL,
            EVENT_OTHER_MOUSE_DOWN,
            EVENT_OTHER_MOUSE_UP,
            EVENT_OTHER_MOUSE_DRAGGED,
        ] {
            assert_ne!(mouse_event_mask() & event_mask(event_type), 0);
        }
    }

    #[test]
    fn degraded_owner_publishes_ready_and_stops_deterministically() {
        let stop = Arc::new(AtomicBool::new(true));
        let (events, receiver) = crossbeam_channel::bounded(1);

        run_degraded(stop, events).unwrap();

        assert!(matches!(receiver.try_recv(), Ok(HookEvent::Ready(1))));
    }
}
