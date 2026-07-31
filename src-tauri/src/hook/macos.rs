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
#[cfg(target_os = "macos")]
use crate::config::ConfigSnapshotReader;
use crate::domain::{MouseEvent, Point, TriggerButton};

const EVENT_QUEUE_CAPACITY: usize = 64;
#[cfg(target_os = "macos")]
const RUN_LOOP_SLICE_SECONDS: f64 = 0.01;
const DEGRADED_STOP_POLL: Duration = Duration::from_millis(10);
const RUN_LOOP_FINISHED: i32 = 1;
const RUN_LOOP_STOPPED: i32 = 2;
const RUN_LOOP_TIMED_OUT: i32 = 3;
const RUN_LOOP_HANDLED_SOURCE: i32 = 4;

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
    event: MouseEvent,
    point: Point,
    timestamp_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EventTapSpec {
    options: u32,
    mask: u64,
}

#[derive(Clone, Copy)]
struct RawInput {
    event_type: u32,
    button: i64,
    scroll: i64,
    x: f64,
    y: f64,
    timestamp_ns: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunLoopDisposition {
    Continue,
    Degrade,
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

    fn capture_raw(&self, raw: RawInput) {
        let Some(event) = normalize_event(raw.event_type, raw.button, raw.scroll) else {
            return;
        };
        self.received.fetch_add(1, Ordering::Relaxed);
        let input = NormalizedInput {
            event,
            point: Point::new(raw.x.round() as i32, raw.y.round() as i32),
            timestamp_ns: raw.timestamp_ns,
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

    fn drain(&self, mut observe: impl FnMut(NormalizedInput)) {
        while let Some(input) = self.queue.pop() {
            observe(input);
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

#[derive(Clone, Copy)]
enum StartupFailure {
    PermissionDenied,
    CreationFailed,
}

#[cfg(target_os = "macos")]
pub(super) fn run_loop_macos(
    _reader: ConfigSnapshotReader,
    stop: Arc<AtomicBool>,
    events: Sender<HookEvent>,
) -> Result<(), HookFailure> {
    let state = Box::new(TapState::new());
    let mode = unsafe { start_event_tap(state.as_ref()) };
    match mode {
        StartupMode::Active(resources) => run_active(stop, events, state, resources),
        StartupMode::PermissionDenied => {
            dispatch_startup_failure(StartupFailure::PermissionDenied, stop, events)
        }
        StartupMode::CreationFailed => {
            dispatch_startup_failure(StartupFailure::CreationFailed, stop, events)
        }
    }
}

fn dispatch_startup_failure(
    failure: StartupFailure,
    stop: Arc<AtomicBool>,
    events: Sender<HookEvent>,
) -> Result<(), HookFailure> {
    #[cfg(target_os = "macos")]
    match failure {
        StartupFailure::PermissionDenied => {
            warn!("macOS input owner is pass-through: Listen Event access is unavailable");
        }
        StartupFailure::CreationFailed => {
            warn!("macOS input owner is pass-through: event tap creation failed");
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = failure;
    run_degraded(stop, events)
}

fn run_degraded(stop: Arc<AtomicBool>, events: Sender<HookEvent>) -> Result<(), HookFailure> {
    publish_ready(&events)?;
    wait_for_stop(&stop);
    Ok(())
}

fn wait_for_stop(stop: &AtomicBool) {
    while !stop.load(Ordering::Acquire) {
        thread::sleep(DEGRADED_STOP_POLL);
    }
}

fn run_non_timeout_owner_step<R>(stop: &AtomicBool, resources: R) {
    drop(resources);
    wait_for_stop(stop);
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
        let result =
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, RUN_LOOP_SLICE_SECONDS, false) };
        if classify_run_loop_result(result) == RunLoopDisposition::Degrade {
            state.drain(drop);
            run_non_timeout_owner_step(&stop, (resources, state));
            return Ok(());
        }
        state.drain(drop);
        state.reenable_if_requested(resources.tap);
    }
    state.drain(drop);
    drop(resources);
    Ok(())
}

fn classify_run_loop_result(result: i32) -> RunLoopDisposition {
    if result == RUN_LOOP_TIMED_OUT {
        RunLoopDisposition::Continue
    } else {
        RunLoopDisposition::Degrade
    }
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
    let spec = event_tap_spec();
    let tap = CGEventTapCreate(
        SESSION_EVENT_TAP,
        HEAD_INSERT_EVENT_TAP,
        spec.options,
        spec.mask,
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
    state.capture_raw(RawInput {
        event_type,
        button,
        scroll,
        x: location.x,
        y: location.y,
        timestamp_ns: CGEventGetTimestamp(event),
    });
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

const fn event_tap_spec() -> EventTapSpec {
    EventTapSpec {
        options: LISTEN_ONLY_EVENT_TAP,
        mask: event_mask(EVENT_LEFT_MOUSE_DOWN)
            | event_mask(EVENT_LEFT_MOUSE_UP)
            | event_mask(EVENT_RIGHT_MOUSE_DOWN)
            | event_mask(EVENT_RIGHT_MOUSE_UP)
            | event_mask(EVENT_MOUSE_MOVED)
            | event_mask(EVENT_LEFT_MOUSE_DRAGGED)
            | event_mask(EVENT_RIGHT_MOUSE_DRAGGED)
            | event_mask(EVENT_SCROLL_WHEEL)
            | event_mask(EVENT_OTHER_MOUSE_DOWN)
            | event_mask(EVENT_OTHER_MOUSE_UP)
            | event_mask(EVENT_OTHER_MOUSE_DRAGGED),
    }
}

const fn event_mask(event_type: u32) -> u64 {
    1_u64 << event_type
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::input::tests::count_allocations;

    struct DropRecorder(Sender<()>);

    impl Drop for DropRecorder {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
        }
    }

    #[test]
    fn callback_core_normalizes_mouse_input_without_allocating() {
        let state = TapState::new();
        let (_, allocations) = count_allocations(|| {
            state.capture_raw(RawInput {
                event_type: EVENT_RIGHT_MOUSE_DOWN,
                button: 0,
                scroll: 0,
                x: 17.4,
                y: -8.6,
                timestamp_ns: 41,
            });
            state.capture_raw(RawInput {
                event_type: EVENT_MOUSE_MOVED,
                button: 0,
                scroll: 0,
                x: 5.6,
                y: 9.4,
                timestamp_ns: 42,
            });
            state.capture_raw(RawInput {
                event_type: EVENT_SCROLL_WHEEL,
                button: 0,
                scroll: -3,
                x: -2.6,
                y: 11.7,
                timestamp_ns: 43,
            });
        });

        assert_eq!(allocations, 0);
        assert_eq!(
            state.queue.pop(),
            Some(NormalizedInput {
                event: MouseEvent::ButtonDown(TriggerButton::Right),
                point: Point::new(17, -9),
                timestamp_ns: 41,
            })
        );
        assert_eq!(
            state.queue.pop(),
            Some(NormalizedInput {
                event: MouseEvent::MouseMove,
                point: Point::new(6, 9),
                timestamp_ns: 42,
            })
        );
        assert_eq!(
            state.queue.pop(),
            Some(NormalizedInput {
                event: MouseEvent::WheelDown(3),
                point: Point::new(-3, 12),
                timestamp_ns: 43,
            })
        );
    }

    #[test]
    fn callback_queue_overload_drops_new_input_and_preserves_fifo_order() {
        let state = TapState::new();
        for timestamp_ns in 0..EVENT_QUEUE_CAPACITY as u64 {
            state.capture_raw(RawInput {
                event_type: EVENT_MOUSE_MOVED,
                button: 0,
                scroll: 0,
                x: timestamp_ns as f64,
                y: -(timestamp_ns as f64),
                timestamp_ns,
            });
        }
        state.capture_raw(RawInput {
            event_type: EVENT_MOUSE_MOVED,
            button: 0,
            scroll: 0,
            x: 99.0,
            y: 99.0,
            timestamp_ns: 99,
        });

        for expected in 0..EVENT_QUEUE_CAPACITY as u64 {
            let input = state.queue.pop().unwrap();
            assert_eq!(input.point, Point::new(expected as i32, -(expected as i32)));
            assert_eq!(input.timestamp_ns, expected);
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
    fn callback_capture_stops_at_the_queue_before_context_resolution() {
        let state = TapState::new();
        state.capture_raw(RawInput {
            event_type: EVENT_RIGHT_MOUSE_DOWN,
            button: 0,
            scroll: 0,
            x: 10.0,
            y: 20.0,
            timestamp_ns: 30_000_000,
        });

        assert_eq!(state.snapshot().processed, 0);
        let mut observed = None;
        state.drain(|input| observed = Some(input));
        assert_eq!(
            observed,
            Some(NormalizedInput {
                event: MouseEvent::ButtonDown(TriggerButton::Right),
                point: Point::new(10, 20),
                timestamp_ns: 30_000_000,
            })
        );
        assert_eq!(state.snapshot().processed, 1);
    }

    #[test]
    fn resident_owner_keeps_context_queries_disconnected_until_consumer_phase() {
        let source = include_str!("macos.rs")
            .split("mod tests {")
            .next()
            .unwrap();

        assert!(!source.contains(concat!("Context", "Worker")));
        assert!(!source.contains(concat!("macos_", "context::")));
        assert!(!source.contains(concat!("context.", "observe(")));
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
    fn spsc_consumer_receives_distinct_inputs_in_callback_order() {
        let state = TapState::new();
        let raw_inputs = [
            RawInput {
                event_type: EVENT_LEFT_MOUSE_DOWN,
                button: 0,
                scroll: 0,
                x: 1.0,
                y: 2.0,
                timestamp_ns: 10,
            },
            RawInput {
                event_type: EVENT_MOUSE_MOVED,
                button: 0,
                scroll: 0,
                x: 3.0,
                y: 4.0,
                timestamp_ns: 20,
            },
            RawInput {
                event_type: EVENT_SCROLL_WHEEL,
                button: 0,
                scroll: 2,
                x: 5.0,
                y: 6.0,
                timestamp_ns: 30,
            },
        ];
        for raw in raw_inputs {
            state.capture_raw(raw);
        }

        assert_eq!(
            state.queue.pop(),
            Some(NormalizedInput {
                event: MouseEvent::ButtonDown(TriggerButton::Left),
                point: Point::new(1, 2),
                timestamp_ns: 10,
            })
        );
        assert_eq!(
            state.queue.pop(),
            Some(NormalizedInput {
                event: MouseEvent::MouseMove,
                point: Point::new(3, 4),
                timestamp_ns: 20,
            })
        );
        assert_eq!(
            state.queue.pop(),
            Some(NormalizedInput {
                event: MouseEvent::WheelUp(2),
                point: Point::new(5, 6),
                timestamp_ns: 30,
            })
        );
        assert!(state.queue.pop().is_none());
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
    fn event_tap_spec_is_exactly_listen_only_mouse_observation() {
        let expected_mask = event_mask(EVENT_LEFT_MOUSE_DOWN)
            | event_mask(EVENT_LEFT_MOUSE_UP)
            | event_mask(EVENT_RIGHT_MOUSE_DOWN)
            | event_mask(EVENT_RIGHT_MOUSE_UP)
            | event_mask(EVENT_MOUSE_MOVED)
            | event_mask(EVENT_LEFT_MOUSE_DRAGGED)
            | event_mask(EVENT_RIGHT_MOUSE_DRAGGED)
            | event_mask(EVENT_SCROLL_WHEEL)
            | event_mask(EVENT_OTHER_MOUSE_DOWN)
            | event_mask(EVENT_OTHER_MOUSE_UP)
            | event_mask(EVENT_OTHER_MOUSE_DRAGGED);

        assert_eq!(
            event_tap_spec(),
            EventTapSpec {
                options: 1,
                mask: expected_mask,
            }
        );
    }

    #[test]
    fn degraded_owner_publishes_ready_and_stops_deterministically() {
        for failure in [
            StartupFailure::PermissionDenied,
            StartupFailure::CreationFailed,
        ] {
            let stop = Arc::new(AtomicBool::new(false));
            let (events, receiver) = crossbeam_channel::bounded(1);
            let (completed, completion) = crossbeam_channel::bounded(1);
            let thread_stop = Arc::clone(&stop);
            let handle = thread::spawn(move || {
                dispatch_startup_failure(failure, thread_stop, events).unwrap();
                completed.send(()).unwrap();
            });

            assert!(matches!(
                receiver.recv_timeout(Duration::from_millis(100)),
                Ok(HookEvent::Ready(1))
            ));
            assert!(!handle.is_finished());
            stop.store(true, Ordering::Release);
            completion.recv_timeout(Duration::from_millis(100)).unwrap();
            handle.join().unwrap();
        }
    }

    #[test]
    fn non_timeout_owner_step_drops_resources_waits_and_does_not_republish_ready() {
        assert_eq!(
            classify_run_loop_result(RUN_LOOP_TIMED_OUT),
            RunLoopDisposition::Continue
        );
        for result in [
            RUN_LOOP_FINISHED,
            RUN_LOOP_STOPPED,
            RUN_LOOP_HANDLED_SOURCE,
            -1,
        ] {
            assert_eq!(
                classify_run_loop_result(result),
                RunLoopDisposition::Degrade
            );
        }

        let stop = Arc::new(AtomicBool::new(false));
        let (events, receiver) = crossbeam_channel::bounded(2);
        let (dropped, drop_observed) = crossbeam_channel::bounded(1);
        let (completed, completion) = crossbeam_channel::bounded(1);
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            publish_ready(&events).unwrap();
            run_non_timeout_owner_step(&thread_stop, DropRecorder(dropped));
            completed.send(()).unwrap();
        });

        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(100)),
            Ok(HookEvent::Ready(1))
        ));
        drop_observed
            .recv_timeout(Duration::from_millis(100))
            .unwrap();
        assert!(receiver.try_recv().is_err());
        assert!(!handle.is_finished());
        stop.store(true, Ordering::Release);
        completion.recv_timeout(Duration::from_millis(100)).unwrap();
        handle.join().unwrap();
        assert!(receiver.try_recv().is_err());
    }
}
