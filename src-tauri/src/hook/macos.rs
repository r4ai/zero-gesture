//! macOS listen-only event-tap owner.
//!
//! The Core Graphics callback compares one process marker, normalizes fields
//! already present in foreign events, appends to one fixed SPSC queue, and
//! updates atomics. The run-loop side drains that queue into the existing
//! input owner, bounded context worker, and bounded action executor.

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
#[cfg(target_os = "macos")]
use std::time::Instant;

use crossbeam_channel::Sender;
#[cfg(target_os = "macos")]
use log::warn;

#[cfg(any(target_os = "macos", test))]
use super::macos_context::observes_event;
#[cfg(target_os = "macos")]
use super::macos_context::ContextWorker;
use super::owner::{ActionWork, ContextView, NativeInputOwner};
use super::{HookEvent, HookFailure};
use crate::config::ConfigSnapshotReader;
use crate::domain::input::SessionId;
use crate::domain::{MouseEvent, Point, TriggerButton};
use crate::executor::macos::{ExecutionOutcome, MacosActionExecutor};

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
#[cfg(target_os = "macos")]
const EVENT_FIELD_SOURCE_USER_DATA: u32 = 55;
const MIDDLE_MOUSE_BUTTON: i64 = 2;

#[cfg(target_os = "macos")]
const SESSION_EVENT_TAP: u32 = 1;
#[cfg(target_os = "macos")]
const HEAD_INSERT_EVENT_TAP: u32 = 0;
const LISTEN_ONLY_EVENT_TAP: u32 = 1;

#[cfg(target_os = "macos")]
extern "C" {
    fn arc4random_buf(buffer: *mut c_void, length: usize);
}

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
    marker: i64,
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
        Self::with_marker(0)
    }

    fn with_marker(marker: i64) -> Self {
        Self {
            marker,
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

    fn capture_tagged_raw(&self, marker: i64, raw: RawInput) {
        if marker != self.marker {
            self.capture_raw(raw);
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

#[cfg(target_os = "macos")]
struct OwnerClock {
    tick: u32,
    observed_at: Instant,
}

#[cfg(target_os = "macos")]
impl OwnerClock {
    fn new() -> Self {
        Self {
            tick: 0,
            observed_at: Instant::now(),
        }
    }

    fn observe(&mut self, timestamp_ns: u64) -> u32 {
        self.tick = (timestamp_ns / 1_000_000) as u32;
        self.observed_at = Instant::now();
        self.tick
    }

    fn current(&self) -> u32 {
        self.tick
            .wrapping_add(self.observed_at.elapsed().as_millis() as u32)
    }
}

struct MacosInputConsumer {
    owner: NativeInputOwner,
    executor: Option<MacosActionExecutor>,
    current_context: Option<ContextView>,
    pending_action: Option<SessionId>,
    executor_alive: bool,
}

impl MacosInputConsumer {
    fn new(reader: ConfigSnapshotReader, executor: MacosActionExecutor) -> Self {
        Self {
            owner: NativeInputOwner::new(reader),
            executor: Some(executor),
            current_context: None,
            pending_action: None,
            executor_alive: true,
        }
    }

    fn consume(&mut self, input: NormalizedInput, context: Option<ContextView>, tick: u32) {
        self.current_context = context;
        self.owner.set_context(context);
        let _ = self.owner.callback(input.event, input.point, tick);
        self.drain_owner_work(tick);
    }

    fn safety_timer(&mut self, tick: u32) {
        self.poll_executor();
        self.owner.safety_timer(tick);
        self.drain_owner_work(tick);
    }

    fn drain_owner_work(&mut self, tick: u32) {
        self.poll_executor();
        while let Some(work) = self.owner.pop_action() {
            match work {
                ActionWork::Activate { session, target } => {
                    let ready = self.current_context.is_some_and(|context| {
                        context.target == target
                            && tick.wrapping_sub(context.updated_tick)
                                <= super::owner::CONTEXT_MAX_AGE_MS
                    });
                    self.owner.activation_result(session, ready);
                }
                ActionWork::Dispatch {
                    session,
                    generation,
                    action,
                    repeat,
                } => self.dispatch(session, generation, action, repeat),
                ActionWork::Replay { .. } => {}
            }
        }
        while self.owner.pop_render().is_some() {}
    }

    fn dispatch(
        &mut self,
        session: SessionId,
        generation: crate::domain::input::ConfigGeneration,
        action: crate::domain::ActionId,
        repeat: u16,
    ) {
        let Some(runtime) = self.owner.runtime(generation) else {
            self.owner.executor_fault();
            return;
        };
        let delivered = self.executor_alive
            && self.pending_action.is_none()
            && self.executor.as_ref().is_some_and(|executor| {
                executor.try_dispatch(session, runtime.action(action).clone(), repeat)
            });
        if delivered {
            self.pending_action = Some(session);
        } else {
            self.owner.action_failed_before_injection(session);
        }
    }

    fn poll_executor(&mut self) {
        loop {
            let result = self.executor.as_ref().map(MacosActionExecutor::poll);
            match result {
                Some(Ok(Some(result))) if self.pending_action == Some(result.session) => {
                    self.pending_action = None;
                    match result.outcome {
                        ExecutionOutcome::Posted => {
                            self.owner.injection_started(result.session);
                            self.owner.action_completed(result.session);
                        }
                        ExecutionOutcome::FailedBeforeInjection => {
                            self.owner.action_failed_before_injection(result.session);
                        }
                        ExecutionOutcome::FailedAfterInjection => {
                            self.owner.injection_started(result.session);
                            self.owner.action_failed_after_injection(result.session);
                        }
                    }
                }
                Some(Ok(Some(_))) => self.owner.executor_fault(),
                Some(Ok(None)) | None => break,
                Some(Err(())) => {
                    self.executor_alive = false;
                    if self.pending_action.take().is_some() {
                        self.owner.executor_fault();
                    }
                    break;
                }
            }
        }
    }

    fn shutdown(mut self) {
        self.owner.shutdown();
        if let Some(executor) = self.executor.take() {
            executor.shutdown();
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
    reader: ConfigSnapshotReader,
    stop: Arc<AtomicBool>,
    events: Sender<HookEvent>,
) -> Result<(), HookFailure> {
    let marker = unsafe { process_instance_marker() };
    let state = Box::new(TapState::with_marker(marker));
    let mode = unsafe { start_event_tap(state.as_ref()) };
    match mode {
        StartupMode::Active(resources) => {
            run_active(reader, stop, events, marker, state, resources)
        }
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
    reader: ConfigSnapshotReader,
    stop: Arc<AtomicBool>,
    events: Sender<HookEvent>,
    marker: i64,
    state: Box<TapState>,
    resources: TapResources,
) -> Result<(), HookFailure> {
    let mut context = ContextWorker::spawn(reader.clone())?;
    let executor = MacosActionExecutor::spawn(marker)
        .map_err(|_| HookFailure::new("macOS action", "failed to start"))?;
    let mut consumer = MacosInputConsumer::new(reader, executor);
    let mut clock = OwnerClock::new();
    publish_ready(&events)?;
    while !stop.load(Ordering::Acquire) {
        let result =
            unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, RUN_LOOP_SLICE_SECONDS, false) };
        if classify_run_loop_result(result) == RunLoopDisposition::Degrade {
            drain_input(&state, &mut context, &mut consumer, &mut clock);
            run_non_timeout_owner_step(&stop, resources);
            consumer.shutdown();
            context.shutdown();
            return Ok(());
        }
        drain_input(&state, &mut context, &mut consumer, &mut clock);
        consumer.safety_timer(clock.current());
        state.reenable_if_requested(resources.tap);
    }
    drain_input(&state, &mut context, &mut consumer, &mut clock);
    drop(resources);
    consumer.shutdown();
    context.shutdown();
    Ok(())
}

#[cfg(target_os = "macos")]
fn drain_input(
    state: &TapState,
    context: &mut ContextWorker,
    consumer: &mut MacosInputConsumer,
    clock: &mut OwnerClock,
) {
    state.drain(|input| {
        let tick = clock.observe(input.timestamp_ns);
        if observes_event(input.event) {
            context.observe(input.event, input.point, input.timestamp_ns);
        }
        let current = context.latest(input.point, tick);
        consumer.consume(input, current, tick);
    });
}

#[cfg(target_os = "macos")]
unsafe fn process_instance_marker() -> i64 {
    let mut marker = 0_u64;
    arc4random_buf(
        std::ptr::from_mut(&mut marker).cast::<c_void>(),
        std::mem::size_of::<u64>(),
    );
    if marker == 0 {
        1
    } else {
        marker as i64
    }
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

    let marker = CGEventGetIntegerValueField(event, EVENT_FIELD_SOURCE_USER_DATA);
    if marker == state.marker {
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
    use std::sync::atomic::AtomicUsize;
    use std::time::Instant;

    use super::*;
    use crate::config::{
        ActiveConfig, BindingRecord, ConfigDocument, ConfigOwner, DocumentAction, GestureBinding,
        GestureMode, GesturePattern, GestureStep, Key,
    };
    use crate::domain::input::tests::count_allocations;
    use crate::domain::{BindingSetId, TriggerButton as DomainTriggerButton};

    struct DropRecorder(Sender<()>);

    impl Drop for DropRecorder {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
        }
    }

    fn consumer_reader() -> (tempfile::TempDir, ConfigOwner, ConfigSnapshotReader, u64) {
        let mut document = ConfigDocument::default();
        document.applications.clear();
        document.bindings = vec![BindingRecord::Shared(GestureBinding {
            id: "right-a".to_string(),
            label: None,
            application_id: None,
            gesture: GesturePattern {
                trigger: crate::config::TriggerButton::RightClick,
                mode: GestureMode::Release,
                sequence: vec![GestureStep::Right],
                step: None,
            },
            action: DocumentAction::Keyboard { keys: vec![Key::A] },
        })];
        let active = ActiveConfig::from_document(document).unwrap();
        let directory = tempfile::tempdir().unwrap();
        crate::config::save_atomic(&active, directory.path()).unwrap();
        let (owner, _) = ConfigOwner::startup(directory.path());
        let reader = owner.reader();
        let generation = reader.read().unwrap().generation();
        (directory, owner, reader, generation)
    }

    fn input(event: MouseEvent, point: Point, tick: u32) -> NormalizedInput {
        NormalizedInput {
            event,
            point,
            timestamp_ns: u64::from(tick) * 1_000_000,
        }
    }

    fn drive_right_release(consumer: &mut MacosInputConsumer, first_context: Option<ContextView>) {
        let start = Point::new(0, 0);
        let end = Point::new(100, 0);
        consumer.consume(
            input(MouseEvent::ButtonDown(DomainTriggerButton::Right), start, 1),
            first_context,
            1,
        );
        consumer.consume(input(MouseEvent::MouseMove, end, 2), None, 2);
        consumer.consume(
            input(MouseEvent::ButtonUp(DomainTriggerButton::Right), end, 3),
            None,
            3,
        );
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
    fn self_tagged_callback_event_is_filtered_before_input_queue() {
        let state = TapState::with_marker(41);

        let (_, allocations) = count_allocations(|| {
            state.capture_tagged_raw(
                41,
                RawInput {
                    event_type: EVENT_MOUSE_MOVED,
                    button: 0,
                    scroll: 0,
                    x: 1.0,
                    y: 2.0,
                    timestamp_ns: 3,
                },
            );
        });

        assert_eq!(allocations, 0);
        assert!(state.queue.pop().is_none());
        assert_eq!(state.snapshot().received, 0);
    }

    #[test]
    fn foreign_tagged_callback_event_reaches_input_queue() {
        let state = TapState::with_marker(41);

        state.capture_tagged_raw(
            42,
            RawInput {
                event_type: EVENT_MOUSE_MOVED,
                button: 0,
                scroll: 0,
                x: 1.0,
                y: 2.0,
                timestamp_ns: 3,
            },
        );

        assert_eq!(
            state.queue.pop(),
            Some(NormalizedInput {
                event: MouseEvent::MouseMove,
                point: Point::new(1, 2),
                timestamp_ns: 3,
            })
        );
    }

    #[test]
    fn fresh_context_selects_and_dispatches_the_macos_action() {
        let (_directory, _owner, reader, generation) = consumer_reader();
        let dispatched = Arc::new(AtomicUsize::new(0));
        let worker_dispatched = Arc::clone(&dispatched);
        let executor = MacosActionExecutor::spawn_with(41, move |action, marker, repeat| {
            assert_eq!(
                action,
                &crate::config::Action::Keyboard {
                    keys: vec!["a".to_string()]
                }
            );
            assert_eq!(marker, 41);
            assert_eq!(repeat, 1);
            worker_dispatched.fetch_add(1, Ordering::Relaxed);
            ExecutionOutcome::Posted
        })
        .unwrap();
        let mut consumer = MacosInputConsumer::new(reader, executor);
        let point = Point::new(0, 0);

        drive_right_release(
            &mut consumer,
            Some(ContextView {
                generation,
                binding_set: BindingSetId::from_index(0).unwrap(),
                target: crate::domain::input::TargetToken(9),
                point,
                updated_tick: 1,
            }),
        );

        let deadline = Instant::now() + Duration::from_secs(1);
        while dispatched.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            consumer.safety_timer(3);
            thread::yield_now();
        }
        assert_eq!(dispatched.load(Ordering::Relaxed), 1);
        consumer.shutdown();
    }

    #[test]
    fn unknown_or_stale_context_never_dispatches_a_macos_action() {
        for first_context in [None, Some(0_u32)] {
            let (_directory, _owner, reader, generation) = consumer_reader();
            let dispatched = Arc::new(AtomicUsize::new(0));
            let worker_dispatched = Arc::clone(&dispatched);
            let executor = MacosActionExecutor::spawn_with(41, move |_, _, _| {
                worker_dispatched.fetch_add(1, Ordering::Relaxed);
                ExecutionOutcome::Posted
            })
            .unwrap();
            let mut consumer = MacosInputConsumer::new(reader, executor);
            let context = first_context.map(|updated_tick| ContextView {
                generation,
                binding_set: BindingSetId::from_index(0).unwrap(),
                target: crate::domain::input::TargetToken(9),
                point: Point::new(0, 0),
                updated_tick,
            });

            if first_context.is_some() {
                consumer.consume(
                    input(
                        MouseEvent::ButtonDown(DomainTriggerButton::Right),
                        Point::new(0, 0),
                        101,
                    ),
                    context,
                    101,
                );
                consumer.consume(
                    input(MouseEvent::MouseMove, Point::new(100, 0), 102),
                    None,
                    102,
                );
                consumer.consume(
                    input(
                        MouseEvent::ButtonUp(DomainTriggerButton::Right),
                        Point::new(100, 0),
                        103,
                    ),
                    None,
                    103,
                );
            } else {
                drive_right_release(&mut consumer, context);
            }
            thread::sleep(Duration::from_millis(20));
            consumer.safety_timer(103);
            assert_eq!(dispatched.load(Ordering::Relaxed), 0);
            consumer.shutdown();
        }
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
    fn resident_owner_connects_context_only_after_callback_queue() {
        let macos_source = include_str!("macos.rs")
            .split("mod tests {")
            .next()
            .unwrap();
        let bootstrap_source = include_str!("mod.rs");

        assert!(macos_source.contains("ContextWorker::spawn(reader.clone())"));
        assert!(macos_source.contains("MacosActionExecutor::spawn(marker)"));
        assert!(macos_source.contains("context.observe(input.event"));
        assert!(macos_source.contains("context.latest(input.point, tick)"));
        assert!(bootstrap_source.contains("run_loop_macos(reader"));
    }

    #[test]
    fn callback_remains_isolated_from_context_and_action_workers() {
        let source = include_str!("macos.rs");
        let callback_source = source
            .split("unsafe extern \"C\" fn event_tap_callback")
            .nth(1)
            .unwrap()
            .split("fn normalize_event")
            .next()
            .unwrap();

        assert!(!callback_source.contains("ContextWorker"));
        assert!(!callback_source.contains("context.observe"));
        assert!(!callback_source.contains("MacosActionExecutor"));
    }

    #[test]
    fn production_context_observation_is_limited_to_action_routing_events() {
        assert!(observes_event(MouseEvent::MouseMove));
        assert!(observes_event(MouseEvent::ButtonDown(
            DomainTriggerButton::Right
        )));
        assert!(!observes_event(MouseEvent::ButtonUp(
            DomainTriggerButton::Right
        )));
        assert!(!observes_event(MouseEvent::WheelUp(1)));
        assert!(!observes_event(MouseEvent::WheelDown(1)));
        assert!(!observes_event(MouseEvent::Other));

        let source = include_str!("macos.rs");
        let drain = source
            .split("fn drain_input(")
            .nth(1)
            .unwrap()
            .split("#[cfg(target_os = \"macos\")]")
            .next()
            .unwrap();
        assert!(drain.contains("if observes_event(input.event)"));
        assert!(drain.contains("context.observe(input.event"));
    }

    #[test]
    fn process_marker_is_copied_before_tap_install_and_executor_spawn() {
        let source = include_str!("macos.rs")
            .split("mod tests {")
            .next()
            .unwrap();
        let marker = source.find("process_instance_marker()").unwrap();
        let callback_state = source.find("TapState::with_marker(marker)").unwrap();
        let tap_install = source.find("start_event_tap(state.as_ref())").unwrap();
        let executor = source.find("MacosActionExecutor::spawn(marker)").unwrap();

        assert!(marker < callback_state);
        assert!(callback_state < tap_install);
        assert!(tap_install < executor);
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
