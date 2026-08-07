//! macOS active event-tap owner.
//!
//! The Core Graphics callback compares one process marker, normalizes fields
//! already present in foreign events, evaluates the existing input owner, and
//! appends context observations to one fixed SPSC queue. The run-loop side
//! drains context observations and already-decided effects.

use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

mod callback;
mod consumer;
mod context;
mod run_loop;

use super::owner::NativeInputOwner;
use crate::domain::{Disposition, MouseEvent, Point, TriggerButton};
#[cfg(target_os = "macos")]
pub(super) use run_loop::run_loop_macos;

const EVENT_QUEUE_CAPACITY: usize = 64;

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
const MIDDLE_MOUSE_BUTTON: i64 = 2;

const ACTIVE_EVENT_TAP: u32 = 0;

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
    // The Event Tap callback and run-loop drain execute on the one native
    // owner thread. The option is installed before active input is enabled,
    // gated off before teardown, and taken only after tap invalidation.
    owner: UnsafeCell<Option<NativeInputOwner>>,
    active_input: AtomicBool,
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
        Self::with_owner(marker, None)
    }

    fn with_owner(marker: i64, owner: Option<NativeInputOwner>) -> Self {
        Self {
            marker,
            owner: UnsafeCell::new(owner),
            active_input: AtomicBool::new(false),
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

    fn capture_raw(&self, raw: RawInput) -> Option<(NormalizedInput, bool)> {
        let event = normalize_event(raw.event_type, raw.button, raw.scroll)?;
        self.received.fetch_add(1, Ordering::Relaxed);
        let input = NormalizedInput {
            event,
            point: Point::new(raw.x.round() as i32, raw.y.round() as i32),
            timestamp_ns: raw.timestamp_ns,
        };
        let enqueued = self.queue.push(input);
        if enqueued {
            self.enqueued.fetch_add(1, Ordering::Relaxed);
        } else {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
        Some((input, enqueued))
    }

    fn decide(&self, input: NormalizedInput, observation_reserved: bool) -> Disposition {
        if !self.active_input.load(Ordering::Acquire) {
            return Disposition::Pass;
        }
        // SAFETY: Core Graphics invokes this callback on the run-loop owner
        // thread. The run-loop never accesses the owner until callback return.
        let owner = unsafe { &mut *self.owner.get() };
        owner.as_mut().map_or(Disposition::Pass, |owner| {
            owner.callback_with_start_reservation(
                input.event,
                input.point,
                (input.timestamp_ns / 1_000_000) as u32,
                observation_reserved,
            )
        })
    }

    fn enable_active_input(&self) {
        // SAFETY: setup and callback use the same owner thread, and callback
        // entry is gated until after this check.
        debug_assert!(unsafe { (&*self.owner.get()).is_some() });
        self.active_input.store(true, Ordering::Release);
    }

    fn disable_active_input(&self) {
        self.active_input.store(false, Ordering::Release);
    }

    fn with_owner_mut<T>(&self, use_owner: impl FnOnce(&mut NativeInputOwner) -> T) -> Option<T> {
        // SAFETY: called only from the Event Tap owner thread outside callback.
        unsafe { (&mut *self.owner.get()).as_mut().map(use_owner) }
    }

    fn detach_owner(&self) -> Option<NativeInputOwner> {
        debug_assert!(!self.active_input.load(Ordering::Acquire));
        // SAFETY: teardown has disabled and invalidated the tap, so no callback
        // can race the owner-thread take.
        unsafe { (&mut *self.owner.get()).take() }
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
        options: ACTIVE_EVENT_TAP,
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
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::super::owner::{ContextRoute, ContextView};
    use super::super::HookEvent;
    use super::callback::capture_callback_event;
    #[cfg(target_os = "macos")]
    use super::callback::event_tap_callback;
    use super::consumer::{drain_input, InputStepFunctions, MacosInputConsumer, OwnerClock};
    use super::context::ContextWorker;
    #[cfg(target_os = "macos")]
    use super::run_loop::post_access_ready;
    use super::run_loop::{
        classify_run_loop_result, dispatch_startup_failure, executor_marker, prepare_marked_state,
        publish_ready, run_non_timeout_owner_step, RunLoopDisposition, StartupFailure,
    };
    use super::*;
    use crate::config::{
        ActiveConfig, BindingRecord, ConfigDocument, ConfigOwner, ConfigSnapshotReader,
        DocumentAction, GestureBinding, GestureMode, GesturePattern, GestureStep, Key,
    };
    use crate::domain::input::tests::count_allocations;
    use crate::domain::{BindingSetId, TriggerButton as DomainTriggerButton};
    use crate::executor::macos::{ExecutionOutcome, ExecutorWork, MacosActionExecutor};
    use crossbeam_channel::Sender;
    #[cfg(target_os = "macos")]
    use objc2_core_foundation::CGPoint;
    #[cfg(target_os = "macos")]
    use objc2_core_graphics::{CGEvent, CGEventField, CGEventType, CGMouseButton};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum InputStepCall {
        Route,
        Needed(bool),
        Observe,
        Latest,
        Consume(bool),
    }

    struct FakeInputConsumer {
        route: ContextRoute,
        calls: Rc<RefCell<Vec<InputStepCall>>>,
    }

    struct FakeContextWorker {
        calls: Rc<RefCell<Vec<InputStepCall>>>,
    }

    fn fake_context_route(
        consumer: &mut FakeInputConsumer,
        _: &TapState,
        _: MouseEvent,
    ) -> ContextRoute {
        consumer.calls.borrow_mut().push(InputStepCall::Route);
        consumer.route
    }

    fn fake_set_needed(context: &mut FakeContextWorker, needed: bool) {
        context
            .calls
            .borrow_mut()
            .push(InputStepCall::Needed(needed));
    }

    fn fake_observe(context: &mut FakeContextWorker, _: MouseEvent, _: Point, _: u64) {
        context.calls.borrow_mut().push(InputStepCall::Observe);
    }

    fn fake_latest(
        context: &mut FakeContextWorker,
        point: Point,
        tick: u32,
    ) -> Option<ContextView> {
        context.calls.borrow_mut().push(InputStepCall::Latest);
        Some(ContextView {
            generation: 1,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: crate::domain::input::TargetToken(1),
            point,
            updated_tick: tick,
        })
    }

    fn fake_consume(
        consumer: &mut FakeInputConsumer,
        _: &TapState,
        _: &mut FakeContextWorker,
        _: NormalizedInput,
        context: Option<ContextView>,
        _: u32,
    ) {
        consumer
            .calls
            .borrow_mut()
            .push(InputStepCall::Consume(context.is_some()));
    }

    fn fake_input_step_functions() -> InputStepFunctions<FakeInputConsumer, FakeContextWorker> {
        InputStepFunctions {
            route: fake_context_route,
            set_needed: fake_set_needed,
            observe: fake_observe,
            latest: fake_latest,
            consume: fake_consume,
        }
    }

    fn callback_raw(event_type: u32) -> RawInput {
        RawInput {
            event_type,
            button: 0,
            scroll: 0,
            x: 1.0,
            y: 2.0,
            timestamp_ns: 3,
        }
    }

    struct DropRecorder(Sender<()>);

    impl Drop for DropRecorder {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
        }
    }

    #[cfg(target_os = "macos")]
    struct OwnedTestEvent(objc2_core_foundation::CFRetained<CGEvent>);

    #[cfg(target_os = "macos")]
    impl OwnedTestEvent {
        fn mouse_move(marker: i64, x: f64, y: f64) -> Self {
            Self::mouse(CGEventType::MouseMoved, CGMouseButton(0), marker, x, y)
        }

        fn mouse(
            event_type: CGEventType,
            button: CGMouseButton,
            marker: i64,
            x: f64,
            y: f64,
        ) -> Self {
            let event =
                CGEvent::new_mouse_event(None, event_type, CGPoint { x, y }, button).unwrap();
            CGEvent::set_integer_value_field(
                Some(&event),
                CGEventField::EventSourceUserData,
                marker,
            );
            Self(event)
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

    fn callback_and_drain(
        state: &TapState,
        context: &mut ContextWorker,
        consumer: &mut MacosInputConsumer,
        clock: &mut OwnerClock,
        input: NormalizedInput,
    ) -> crate::domain::Disposition {
        let disposition = state.decide(input, state.queue.push(input));
        drain_input(
            state,
            context,
            consumer,
            clock,
            &InputStepFunctions {
                route: MacosInputConsumer::context_route,
                set_needed: ContextWorker::set_needed,
                observe: ContextWorker::observe,
                latest: ContextWorker::latest,
                consume: MacosInputConsumer::consume,
            },
        );
        disposition
    }

    fn drive_right_release(
        state: &TapState,
        context: &mut ContextWorker,
        consumer: &mut MacosInputConsumer,
        clock: &mut OwnerClock,
    ) {
        let start = Point::new(0, 0);
        let end = Point::new(100, 0);
        assert_eq!(
            callback_and_drain(
                state,
                context,
                consumer,
                clock,
                input(MouseEvent::ButtonDown(DomainTriggerButton::Right), start, 1),
            ),
            crate::domain::Disposition::Suppress
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        while consumer.has_pending_activation() && Instant::now() < deadline {
            consumer.safety_timer(state, context, 1);
            thread::yield_now();
        }
        assert!(!consumer.has_pending_activation());
        assert_eq!(
            callback_and_drain(
                state,
                context,
                consumer,
                clock,
                input(MouseEvent::MouseMove, end, 2),
            ),
            crate::domain::Disposition::Pass
        );
        assert_eq!(
            callback_and_drain(
                state,
                context,
                consumer,
                clock,
                input(MouseEvent::ButtonUp(DomainTriggerButton::Right), end, 3),
            ),
            crate::domain::Disposition::Suppress
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
        let raw_read = Cell::new(false);

        let (_, allocations) = count_allocations(|| {
            capture_callback_event(&state, 41, || {
                raw_read.set(true);
                callback_raw(EVENT_MOUSE_MOVED)
            });
        });

        assert_eq!(allocations, 0);
        assert!(!raw_read.get());
        assert!(state.queue.pop().is_none());
        assert_eq!(state.snapshot().received, 0);
    }

    #[test]
    fn foreign_tagged_callback_event_reaches_input_queue() {
        let state = TapState::with_marker(41);
        let raw_read = Cell::new(false);

        capture_callback_event(&state, 42, || {
            raw_read.set(true);
            callback_raw(EVENT_MOUSE_MOVED)
        });

        assert!(raw_read.get());
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
    fn active_callback_decides_suppression_without_allocating() {
        let (_directory, _config_owner, reader, generation) = consumer_reader();
        let point = Point::new(1, 2);
        let mut owner = super::super::owner::NativeInputOwner::new(reader);
        owner.set_context(Some(ContextView {
            generation,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: crate::domain::input::TargetToken(9),
            point,
            updated_tick: 3,
        }));
        let state = TapState::with_owner(41, Some(owner));
        state.enable_active_input();

        let (disposition, allocations) = count_allocations(|| {
            capture_callback_event(&state, 42, || RawInput {
                event_type: EVENT_RIGHT_MOUSE_DOWN,
                button: 0,
                scroll: 0,
                x: 1.0,
                y: 2.0,
                timestamp_ns: 3_000_000,
            })
        });

        assert_eq!(disposition, crate::domain::Disposition::Suppress);
        assert_eq!(allocations, 0);
        assert!(matches!(
            state.with_owner_mut(|owner| owner.pop_action()).flatten(),
            Some(super::super::owner::ActionWork::Activate { .. })
        ));
    }

    #[test]
    fn callback_queue_overload_cannot_start_new_suppression() {
        let (_directory, _config_owner, reader, generation) = consumer_reader();
        let point = Point::new(1, 2);
        let mut owner = super::super::owner::NativeInputOwner::new(reader);
        owner.set_context(Some(ContextView {
            generation,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: crate::domain::input::TargetToken(9),
            point,
            updated_tick: 3,
        }));
        let state = TapState::with_owner(41, Some(owner));
        state.enable_active_input();
        for timestamp_ns in 0..EVENT_QUEUE_CAPACITY as u64 {
            state.capture_raw(RawInput {
                event_type: EVENT_MOUSE_MOVED,
                button: 0,
                scroll: 0,
                x: 0.0,
                y: 0.0,
                timestamp_ns,
            });
        }

        let disposition = capture_callback_event(&state, 42, || RawInput {
            event_type: EVENT_RIGHT_MOUSE_DOWN,
            button: 0,
            scroll: 0,
            x: 1.0,
            y: 2.0,
            timestamp_ns: 3_000_000,
        });

        assert_eq!(disposition, crate::domain::Disposition::Pass);
        assert!(state
            .with_owner_mut(|owner| owner.pop_action())
            .flatten()
            .is_none());
    }

    #[test]
    fn production_input_step_orders_and_gates_context_before_consumer() {
        let cases = [
            (
                ContextRoute::Observe,
                vec![
                    InputStepCall::Route,
                    InputStepCall::Needed(true),
                    InputStepCall::Observe,
                    InputStepCall::Latest,
                    InputStepCall::Consume(true),
                ],
            ),
            (
                ContextRoute::Cached,
                vec![
                    InputStepCall::Route,
                    InputStepCall::Needed(true),
                    InputStepCall::Latest,
                    InputStepCall::Consume(true),
                ],
            ),
            (
                ContextRoute::Inactive,
                vec![
                    InputStepCall::Route,
                    InputStepCall::Needed(false),
                    InputStepCall::Consume(false),
                ],
            ),
        ];

        for (route, expected) in cases {
            let calls = Rc::new(RefCell::new(Vec::new()));
            let mut context = FakeContextWorker {
                calls: Rc::clone(&calls),
            };
            let mut consumer = FakeInputConsumer {
                route,
                calls: Rc::clone(&calls),
            };
            let mut clock = OwnerClock::new();
            let state = TapState::new();
            let queued_input = input(MouseEvent::MouseMove, Point::new(4, 5), 6);
            assert!(state.queue.push(queued_input));

            drain_input(
                &state,
                &mut context,
                &mut consumer,
                &mut clock,
                &fake_input_step_functions(),
            );

            assert_eq!(*calls.borrow(), expected);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unavailable_post_access_publishes_ready_and_keeps_original_event_pass_through() {
        let (_directory, _config_owner, reader, generation) = consumer_reader();
        let physical =
            OwnedTestEvent::mouse(CGEventType::RightMouseDown, CGMouseButton(1), 42, 1.0, 2.0);
        let pointer = std::ptr::NonNull::from(&*physical.0);
        let tick = (CGEvent::timestamp(Some(&physical.0)) / 1_000_000) as u32;
        let mut owner = super::super::owner::NativeInputOwner::new(reader);
        owner.set_context(Some(ContextView {
            generation,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: crate::domain::input::TargetToken(9),
            point: Point::new(1, 2),
            updated_tick: tick,
        }));
        let state = TapState::with_owner(41, Some(owner));
        let (events, receiver) = crossbeam_channel::bounded(1);

        assert!(!post_access_ready(&state, &events, || false).unwrap());
        assert!(matches!(receiver.recv().unwrap(), HookEvent::Ready(1)));
        let returned = unsafe {
            event_tap_callback(
                std::ptr::null_mut(),
                CGEventType::RightMouseDown,
                pointer,
                std::ptr::from_ref(&state).cast_mut().cast(),
            )
        };

        assert_eq!(returned, pointer.as_ptr());
        assert!(state
            .with_owner_mut(|owner| owner.pop_action())
            .flatten()
            .is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn p04b2_mouse_mask_and_non_suppressing_pointer_contract_are_preserved() {
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
        assert_eq!(event_tap_spec().mask, expected_mask);

        let state = TapState::with_marker(41);
        let event = OwnedTestEvent::mouse_move(41, 1.0, 2.0);
        let pointer = std::ptr::NonNull::from(&*event.0);
        let returned = unsafe {
            event_tap_callback(
                std::ptr::null_mut(),
                CGEventType::MouseMoved,
                pointer,
                std::ptr::from_ref(&state).cast_mut().cast(),
            )
        };

        assert_eq!(returned, pointer.as_ptr());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn actual_callback_filters_self_and_enqueues_a_real_foreign_event() {
        let state = TapState::with_marker(41);
        let self_event = OwnedTestEvent::mouse_move(41, 1.0, 2.0);
        let foreign_event = OwnedTestEvent::mouse_move(42, 3.0, 4.0);
        let self_pointer = std::ptr::NonNull::from(&*self_event.0);
        let foreign_pointer = std::ptr::NonNull::from(&*foreign_event.0);

        let returned = unsafe {
            event_tap_callback(
                std::ptr::null_mut(),
                CGEventType::MouseMoved,
                self_pointer,
                std::ptr::from_ref(&state).cast_mut().cast(),
            )
        };
        assert_eq!(returned, self_pointer.as_ptr());
        assert!(state.queue.pop().is_none());
        assert_eq!(state.snapshot().received, 0);

        let returned = unsafe {
            event_tap_callback(
                std::ptr::null_mut(),
                CGEventType::MouseMoved,
                foreign_pointer,
                std::ptr::from_ref(&state).cast_mut().cast(),
            )
        };
        assert_eq!(returned, foreign_pointer.as_ptr());
        let input = state.queue.pop().unwrap();
        assert_eq!(input.event, MouseEvent::MouseMove);
        assert_eq!(input.point, Point::new(3, 4));
        assert_eq!(state.snapshot().received, 1);

        for event_type in [
            CGEventType::TapDisabledByTimeout,
            CGEventType::TapDisabledByUserInput,
        ] {
            let returned = unsafe {
                event_tap_callback(
                    std::ptr::null_mut(),
                    event_type,
                    foreign_pointer,
                    std::ptr::from_ref(&state).cast_mut().cast(),
                )
            };
            assert_eq!(returned, foreign_pointer.as_ptr());
        }
        assert_eq!(state.snapshot().disabled, 2);
        assert!(state.take_reenable_request());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn actual_callback_only_enqueues_and_actual_drain_later_invokes_consumer() {
        let state = TapState::with_marker(41);
        let event = OwnedTestEvent::mouse_move(42, 1.0, 2.0);
        let worker_calls = Rc::new(RefCell::new(Vec::new()));
        let mut context = FakeContextWorker {
            calls: Rc::clone(&worker_calls),
        };
        let mut consumer = FakeInputConsumer {
            route: ContextRoute::Observe,
            calls: Rc::clone(&worker_calls),
        };
        let mut clock = OwnerClock::new();
        let event_pointer = std::ptr::NonNull::from(&*event.0);

        let returned = unsafe {
            event_tap_callback(
                std::ptr::null_mut(),
                CGEventType::MouseMoved,
                event_pointer,
                std::ptr::from_ref(&state).cast_mut().cast(),
            )
        };
        assert_eq!(returned, event_pointer.as_ptr());
        assert!(worker_calls.borrow().is_empty());

        drain_input(
            &state,
            &mut context,
            &mut consumer,
            &mut clock,
            &fake_input_step_functions(),
        );
        assert_eq!(
            *worker_calls.borrow(),
            [
                InputStepCall::Route,
                InputStepCall::Needed(true),
                InputStepCall::Observe,
                InputStepCall::Latest,
                InputStepCall::Consume(true),
            ]
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn actual_callback_returns_null_only_for_explicit_suppression() {
        let (_directory, _config_owner, reader, generation) = consumer_reader();
        let physical =
            OwnedTestEvent::mouse(CGEventType::RightMouseDown, CGMouseButton(1), 42, 1.0, 2.0);
        let physical_pointer = std::ptr::NonNull::from(&*physical.0);
        let tick = (CGEvent::timestamp(Some(&physical.0)) / 1_000_000) as u32;
        let mut owner = super::super::owner::NativeInputOwner::new(reader);
        owner.set_context(Some(ContextView {
            generation,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: crate::domain::input::TargetToken(9),
            point: Point::new(1, 2),
            updated_tick: tick,
        }));
        let state = TapState::with_owner(41, Some(owner));
        state.enable_active_input();

        let returned = unsafe {
            event_tap_callback(
                std::ptr::null_mut(),
                CGEventType::RightMouseDown,
                physical_pointer,
                std::ptr::from_ref(&state).cast_mut().cast(),
            )
        };
        assert!(returned.is_null());

        let self_event =
            OwnedTestEvent::mouse(CGEventType::RightMouseDown, CGMouseButton(1), 41, 1.0, 2.0);
        let self_pointer = std::ptr::NonNull::from(&*self_event.0);
        let returned = unsafe {
            event_tap_callback(
                std::ptr::null_mut(),
                CGEventType::RightMouseDown,
                self_pointer,
                std::ptr::from_ref(&state).cast_mut().cast(),
            )
        };
        assert_eq!(returned, self_pointer.as_ptr());

        let unknown = CGEvent::new_keyboard_event(None, 0, true).unwrap();
        let unknown_pointer = std::ptr::NonNull::from(&*unknown);
        let returned = unsafe {
            event_tap_callback(
                std::ptr::null_mut(),
                CGEventType::KeyDown,
                unknown_pointer,
                std::ptr::from_ref(&state).cast_mut().cast(),
            )
        };
        assert_eq!(returned, unknown_pointer.as_ptr());

        let returned = unsafe {
            event_tap_callback(
                std::ptr::null_mut(),
                CGEventType::TapDisabledByTimeout,
                physical_pointer,
                std::ptr::from_ref(&state).cast_mut().cast(),
            )
        };
        assert_eq!(returned, physical_pointer.as_ptr());
    }

    #[test]
    fn fresh_context_selects_and_dispatches_the_macos_action() {
        let (_directory, _owner, reader, _generation) = consumer_reader();
        let dispatched = Arc::new(AtomicUsize::new(0));
        let worker_dispatched = Arc::clone(&dispatched);
        let executor = MacosActionExecutor::spawn_with(41, move |work, marker| {
            let ExecutorWork::Keyboard { action, repeat } = work else {
                panic!("expected keyboard action");
            };
            assert_eq!(
                action,
                &crate::config::Action::Keyboard {
                    keys: vec!["a".to_string()]
                }
            );
            assert_eq!(marker, 41);
            assert_eq!(*repeat, 1);
            worker_dispatched.fetch_add(1, Ordering::Relaxed);
            ExecutionOutcome::Posted
        })
        .unwrap();
        let target = crate::domain::input::TargetToken(9);
        let mut context_worker = ContextWorker::spawn_test(reader.clone(), Some(target));
        let state =
            TapState::with_owner(41, Some(super::super::owner::NativeInputOwner::new(reader)));
        state.enable_active_input();
        let mut consumer = MacosInputConsumer::new(executor);
        let mut clock = OwnerClock::new();
        let point = Point::new(0, 0);
        context_worker.observe(MouseEvent::MouseMove, point, 1_000_000);
        let deadline = Instant::now() + Duration::from_secs(1);
        while context_worker.latest(point, 1).is_none() && Instant::now() < deadline {
            thread::yield_now();
        }
        consumer.refresh_context(&state, &mut context_worker, 1);
        drive_right_release(&state, &mut context_worker, &mut consumer, &mut clock);

        let deadline = Instant::now() + Duration::from_secs(1);
        while dispatched.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            consumer.safety_timer(&state, &mut context_worker, 3);
            thread::yield_now();
        }
        assert_eq!(dispatched.load(Ordering::Relaxed), 1);
        consumer.prepare_shutdown(&state, &mut context_worker, 3);
        state.detach_owner();
        consumer.finish_shutdown();
        context_worker.shutdown();
    }

    #[test]
    fn changed_focus_fails_activation_and_replays_the_captured_trigger() {
        let (_directory, _owner, reader, generation) = consumer_reader();
        let replays = Arc::new(AtomicUsize::new(0));
        let worker_replays = Arc::clone(&replays);
        let executor = MacosActionExecutor::spawn_with(41, move |work, marker| {
            let ExecutorWork::Replay {
                trigger,
                down_at,
                up_at,
            } = work
            else {
                panic!("activation failure must not dispatch the action");
            };
            assert_eq!(
                (*trigger, *down_at, *up_at, marker),
                (
                    DomainTriggerButton::Right,
                    Point::new(0, 0),
                    Point::new(10, 20),
                    41,
                )
            );
            worker_replays.fetch_add(1, Ordering::Relaxed);
            ExecutionOutcome::Posted
        })
        .unwrap();
        let mut context_worker =
            ContextWorker::spawn_test(reader.clone(), Some(crate::domain::input::TargetToken(10)));
        let mut owner = super::super::owner::NativeInputOwner::new(reader);
        owner.set_context(Some(ContextView {
            generation,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: crate::domain::input::TargetToken(9),
            point: Point::new(0, 0),
            updated_tick: 1,
        }));
        let state = TapState::with_owner(41, Some(owner));
        state.enable_active_input();
        let mut consumer = MacosInputConsumer::new(executor);
        let mut clock = OwnerClock::new();

        assert_eq!(
            callback_and_drain(
                &state,
                &mut context_worker,
                &mut consumer,
                &mut clock,
                input(
                    MouseEvent::ButtonDown(DomainTriggerButton::Right),
                    Point::new(0, 0),
                    1,
                ),
            ),
            crate::domain::Disposition::Suppress
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        while consumer.has_pending_activation() && Instant::now() < deadline {
            consumer.safety_timer(&state, &mut context_worker, 1);
            thread::yield_now();
        }
        assert!(!consumer.has_pending_activation());
        assert_eq!(
            callback_and_drain(
                &state,
                &mut context_worker,
                &mut consumer,
                &mut clock,
                input(
                    MouseEvent::ButtonUp(DomainTriggerButton::Right),
                    Point::new(10, 20),
                    2,
                ),
            ),
            crate::domain::Disposition::Suppress
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        while replays.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
            consumer.safety_timer(&state, &mut context_worker, 2);
            thread::yield_now();
        }
        assert_eq!(replays.load(Ordering::Relaxed), 1);

        consumer.prepare_shutdown(&state, &mut context_worker, 2);
        state.detach_owner();
        consumer.finish_shutdown();
        context_worker.shutdown();
    }

    #[test]
    fn shutdown_does_not_replay_after_an_accepted_action_completed_unpolled() {
        let (_directory, _owner, reader, _generation) = consumer_reader();
        let actions = Arc::new(AtomicUsize::new(0));
        let replays = Arc::new(AtomicUsize::new(0));
        let worker_actions = Arc::clone(&actions);
        let worker_replays = Arc::clone(&replays);
        let executor = MacosActionExecutor::spawn_with(41, move |work, _| {
            match work {
                ExecutorWork::Keyboard { .. } => {
                    worker_actions.fetch_add(1, Ordering::Relaxed);
                }
                ExecutorWork::Replay { .. } => {
                    worker_replays.fetch_add(1, Ordering::Relaxed);
                }
            }
            ExecutionOutcome::Posted
        })
        .unwrap();
        let target = crate::domain::input::TargetToken(9);
        let mut context_worker = ContextWorker::spawn_test(reader.clone(), Some(target));
        let state =
            TapState::with_owner(41, Some(super::super::owner::NativeInputOwner::new(reader)));
        state.enable_active_input();
        let mut consumer = MacosInputConsumer::new(executor);
        let mut clock = OwnerClock::new();
        let point = Point::new(0, 0);
        context_worker.observe(MouseEvent::MouseMove, point, 1_000_000);
        let deadline = Instant::now() + Duration::from_secs(1);
        while context_worker.latest(point, 1).is_none() && Instant::now() < deadline {
            thread::yield_now();
        }
        consumer.refresh_context(&state, &mut context_worker, 1);
        drive_right_release(&state, &mut context_worker, &mut consumer, &mut clock);

        let deadline = Instant::now() + Duration::from_secs(1);
        while !consumer.has_unpolled_executor_result() && Instant::now() < deadline {
            thread::yield_now();
        }
        assert!(consumer.has_unpolled_executor_result());
        consumer.prepare_shutdown(&state, &mut context_worker, 3);
        state.detach_owner();
        consumer.finish_shutdown();

        assert_eq!(actions.load(Ordering::Relaxed), 1);
        assert_eq!(replays.load(Ordering::Relaxed), 0);
        context_worker.shutdown();
    }

    #[test]
    fn shutdown_disables_callback_and_drains_one_reserved_replay() {
        let (_directory, _owner, reader, generation) = consumer_reader();
        let replayed = Arc::new(AtomicUsize::new(0));
        let worker_replayed = Arc::clone(&replayed);
        let executor = MacosActionExecutor::spawn_with(41, move |work, _| {
            assert!(matches!(
                work,
                ExecutorWork::Replay {
                    trigger: DomainTriggerButton::Right,
                    down_at,
                    up_at,
                } if *down_at == Point::new(4, 5) && *up_at == Point::new(4, 5)
            ));
            worker_replayed.fetch_add(1, Ordering::Relaxed);
            ExecutionOutcome::Posted
        })
        .unwrap();
        let mut context_worker =
            ContextWorker::spawn_test(reader.clone(), Some(crate::domain::input::TargetToken(9)));
        let mut owner = super::super::owner::NativeInputOwner::new(reader);
        owner.set_context(Some(ContextView {
            generation,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: crate::domain::input::TargetToken(9),
            point: Point::new(4, 5),
            updated_tick: 1,
        }));
        let state = TapState::with_owner(41, Some(owner));
        state.enable_active_input();
        assert_eq!(
            state.decide(
                input(
                    MouseEvent::ButtonDown(DomainTriggerButton::Right),
                    Point::new(4, 5),
                    1,
                ),
                true,
            ),
            crate::domain::Disposition::Suppress
        );
        let mut consumer = MacosInputConsumer::new(executor);

        consumer.prepare_shutdown(&state, &mut context_worker, 1);
        assert_eq!(
            state.decide(input(MouseEvent::MouseMove, Point::new(9, 9), 2), true,),
            crate::domain::Disposition::Pass
        );
        state.detach_owner();
        consumer.finish_shutdown();
        assert_eq!(replayed.load(Ordering::Relaxed), 1);
        context_worker.shutdown();
    }

    #[test]
    fn unknown_or_stale_context_never_dispatches_a_macos_action() {
        for first_context in [None, Some(0_u32)] {
            let (_directory, _owner, reader, generation) = consumer_reader();
            let dispatched = Arc::new(AtomicUsize::new(0));
            let worker_dispatched = Arc::clone(&dispatched);
            let executor = MacosActionExecutor::spawn_with(41, move |_, _| {
                worker_dispatched.fetch_add(1, Ordering::Relaxed);
                ExecutionOutcome::Posted
            })
            .unwrap();
            let mut context_worker = ContextWorker::spawn_test(reader.clone(), None);
            let state =
                TapState::with_owner(41, Some(super::super::owner::NativeInputOwner::new(reader)));
            state.enable_active_input();
            let mut consumer = MacosInputConsumer::new(executor);
            let mut clock = OwnerClock::new();
            let context = first_context.map(|updated_tick| ContextView {
                generation,
                binding_set: BindingSetId::from_index(0).unwrap(),
                target: crate::domain::input::TargetToken(9),
                point: Point::new(0, 0),
                updated_tick,
            });
            state.with_owner_mut(|owner| owner.set_context(context));
            callback_and_drain(
                &state,
                &mut context_worker,
                &mut consumer,
                &mut clock,
                input(
                    MouseEvent::ButtonDown(DomainTriggerButton::Right),
                    Point::new(0, 0),
                    if first_context.is_some() { 101 } else { 1 },
                ),
            );
            callback_and_drain(
                &state,
                &mut context_worker,
                &mut consumer,
                &mut clock,
                input(MouseEvent::MouseMove, Point::new(100, 0), 102),
            );
            callback_and_drain(
                &state,
                &mut context_worker,
                &mut consumer,
                &mut clock,
                input(
                    MouseEvent::ButtonUp(DomainTriggerButton::Right),
                    Point::new(100, 0),
                    103,
                ),
            );
            thread::sleep(Duration::from_millis(20));
            consumer.safety_timer(&state, &mut context_worker, 103);
            assert_eq!(dispatched.load(Ordering::Relaxed), 0);
            consumer.prepare_shutdown(&state, &mut context_worker, 103);
            state.detach_owner();
            consumer.finish_shutdown();
            context_worker.shutdown();
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
    fn process_marker_is_copied_before_tap_install_and_executor_spawn() {
        let calls = Rc::new(RefCell::new(Vec::new()));
        let tap_calls = Rc::clone(&calls);
        let (state, ()) = prepare_marked_state(41, None, move |state| {
            tap_calls.borrow_mut().push(("tap", state.marker));
        });
        calls
            .borrow_mut()
            .push(("executor", executor_marker(&state)));

        assert_eq!(*calls.borrow(), [("tap", 41), ("executor", 41)]);
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
    fn event_tap_spec_is_suppress_capable_with_exact_mouse_mask() {
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
                options: 0,
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
        assert_eq!(classify_run_loop_result(3), RunLoopDisposition::Continue);
        for result in [1, 2, 4, -1] {
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
