use std::sync::Arc;

use crate::config::{ConfigSnapshotReader, RuntimeConfig};
use crate::domain::input::{
    CallbackFacts, ConfigGeneration, InputEffect, InputEvent, InputKernel, InputMode, Reservation,
    SessionId, TargetToken,
};
use crate::domain::{ActionId, BindingSetId, Disposition, MouseEvent, Point, TriggerButton};

const ACTION_CAPACITY: usize = 16;
const RENDER_CAPACITY: usize = 64;
const CONTEXT_MAX_AGE_MS: u32 = 100;

#[derive(Clone, Copy)]
pub(super) struct ContextView {
    pub(super) generation: u64,
    pub(super) binding_set: BindingSetId,
    pub(super) target: TargetToken,
    pub(super) point: Point,
    pub(super) updated_tick: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ActionWork {
    Activate {
        session: SessionId,
        target: TargetToken,
    },
    Dispatch {
        session: SessionId,
        generation: ConfigGeneration,
        action: ActionId,
        repeat: u16,
    },
    Replay {
        session: SessionId,
        trigger: TriggerButton,
        down_at: Point,
        up_at: Point,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RenderWork {
    Start {
        session: SessionId,
        generation: ConfigGeneration,
    },
    Point {
        session: SessionId,
        point: Point,
    },
    Label {
        session: SessionId,
        action: Option<ActionId>,
    },
    End {
        session: SessionId,
    },
}

struct FixedLane<T: Copy, const N: usize> {
    items: [Option<T>; N],
    head: usize,
    len: usize,
}

impl<T: Copy, const N: usize> FixedLane<T, N> {
    fn new() -> Self {
        Self {
            items: [None; N],
            head: 0,
            len: 0,
        }
    }

    fn free(&self) -> usize {
        N - self.len
    }

    fn push(&mut self, item: T) -> bool {
        if self.len == N {
            return false;
        }
        let tail = (self.head + self.len) % N;
        self.items[tail] = Some(item);
        self.len += 1;
        true
    }

    fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let item = self.items[self.head].take();
        self.head = (self.head + 1) % N;
        self.len -= 1;
        item
    }
}

struct RenderLane {
    queue: FixedLane<RenderWork, RENDER_CAPACITY>,
    terminal_reserved: bool,
}

impl RenderLane {
    fn new() -> Self {
        Self {
            queue: FixedLane::new(),
            terminal_reserved: false,
        }
    }

    fn can_start(&self) -> bool {
        !self.terminal_reserved && self.queue.free() >= 2
    }

    fn start(&mut self, work: RenderWork) {
        debug_assert!(self.can_start());
        self.terminal_reserved = true;
        let inserted = self.queue.push(work);
        debug_assert!(inserted);
    }

    fn lossy(&mut self, work: RenderWork) {
        let reserved = usize::from(self.terminal_reserved);
        if self.queue.free() > reserved {
            self.queue.push(work);
        }
    }

    fn end(&mut self, work: RenderWork) {
        debug_assert!(self.terminal_reserved);
        let inserted = self.queue.push(work);
        debug_assert!(inserted);
        self.terminal_reserved = false;
    }
}

/// Actual native input owner used by the Windows low-level hook callback.
///
/// The callback-facing method performs only fixed atomic snapshot reads,
/// fixed-capacity transitions, and bounded lane insertion.
pub(super) struct NativeInputOwner {
    reader: ConfigSnapshotReader,
    kernel: Option<InputKernel>,
    current_generation: Option<ConfigGeneration>,
    current_runtime: Option<Arc<RuntimeConfig>>,
    session_runtime: Option<Arc<RuntimeConfig>>,
    context: Option<ContextView>,
    actions: FixedLane<ActionWork, ACTION_CAPACITY>,
    renderer: RenderLane,
    replay_reserved: bool,
    action_wakeup: bool,
    render_wakeup: bool,
}

impl NativeInputOwner {
    pub(super) fn new(reader: ConfigSnapshotReader) -> Self {
        Self {
            reader,
            kernel: None,
            current_generation: None,
            current_runtime: None,
            session_runtime: None,
            context: None,
            actions: FixedLane::new(),
            renderer: RenderLane::new(),
            replay_reserved: false,
            action_wakeup: false,
            render_wakeup: false,
        }
    }

    pub(super) fn set_context(&mut self, context: Option<ContextView>) {
        self.context = context;
    }

    pub(super) fn callback(&mut self, event: MouseEvent, point: Point, tick: u32) -> Disposition {
        let active_generation = self
            .kernel
            .as_ref()
            .and_then(InputKernel::pinned_generation);
        if active_generation.is_none() {
            self.refresh_config();
        }
        let facts = if active_generation.is_some() {
            CallbackFacts::Continue {
                action_delivery: reservation(
                    self.actions.free() > usize::from(self.replay_reserved),
                ),
                completion_replay: reservation(self.replay_reserved),
            }
        } else {
            let Some(runtime) = self.current_runtime.as_ref() else {
                return Disposition::Pass;
            };
            if !runtime.enabled {
                return Disposition::Pass;
            }
            self.start_facts(point, tick)
        };
        let Some(kernel) = self.kernel.as_mut() else {
            return Disposition::Pass;
        };
        let decision = kernel.handle(InputEvent::Pointer {
            event,
            point,
            tick,
            facts,
        });
        let disposition = decision.disposition;
        let mode = decision.mode;
        self.apply_effects(decision.effects());
        self.sync_session_runtime();
        if mode == InputMode::Bypass {
            self.replay_reserved = false;
        }
        disposition
    }

    pub(super) fn safety_timer(&mut self, tick: u32) {
        self.handle_owner_event(InputEvent::SafetyTimer { tick });
    }

    pub(super) fn activation_result(&mut self, session: SessionId, ready: bool) {
        self.handle_owner_event(if ready {
            InputEvent::ActivationReady(session)
        } else {
            InputEvent::ActivationFailed(session)
        });
    }

    pub(super) fn injection_started(&mut self, session: SessionId) {
        self.handle_owner_event(InputEvent::InjectionStarted(session));
    }

    pub(super) fn action_completed(&mut self, session: SessionId) {
        self.handle_owner_event(InputEvent::ActionCompleted(session));
    }

    pub(super) fn action_failed_before_injection(&mut self, session: SessionId) {
        self.handle_owner_event(InputEvent::ActionFailedBeforeInjection(session));
    }

    pub(super) fn action_failed_after_injection(&mut self, session: SessionId) {
        self.handle_owner_event(InputEvent::ActionFailedAfterInjection(session));
    }

    pub(super) fn executor_fault(&mut self) {
        self.handle_owner_event(InputEvent::ExecutorFault);
    }

    pub(super) fn renderer_fault(&mut self) {
        self.handle_owner_event(InputEvent::RendererFault);
    }

    pub(super) fn shutdown(&mut self) {
        self.handle_owner_event(InputEvent::Shutdown);
        self.actions = FixedLane::new();
        self.renderer = RenderLane::new();
        self.replay_reserved = false;
        self.session_runtime = None;
        self.action_wakeup = false;
        self.render_wakeup = false;
    }

    pub(super) fn pop_action(&mut self) -> Option<ActionWork> {
        self.actions.pop()
    }

    pub(super) fn pop_render(&mut self) -> Option<RenderWork> {
        self.renderer.queue.pop()
    }

    pub(super) fn take_wakeups(&mut self) -> (bool, bool) {
        let wakeups = (self.action_wakeup, self.render_wakeup);
        self.action_wakeup = false;
        self.render_wakeup = false;
        wakeups
    }

    pub(super) fn runtime(&self, generation: ConfigGeneration) -> Option<Arc<RuntimeConfig>> {
        if self.current_generation == Some(generation) {
            return self.current_runtime.as_ref().map(Arc::clone);
        }
        self.session_runtime.as_ref().and_then(|runtime| {
            self.kernel
                .as_ref()
                .and_then(InputKernel::pinned_generation)
                .filter(|pinned| *pinned == generation)
                .map(|_| Arc::clone(runtime))
        })
    }

    fn refresh_config(&mut self) {
        let Some(snapshot) = self.reader.read() else {
            self.current_generation = None;
            self.current_runtime = None;
            return;
        };
        let generation = ConfigGeneration(snapshot.generation());
        if self.current_generation == Some(generation) {
            return;
        }
        let runtime = snapshot.runtime();
        match self.kernel.as_mut() {
            Some(kernel) => kernel.publish_config(generation, Arc::clone(&runtime.gesture)),
            None => {
                self.kernel = Some(InputKernel::new(generation, Arc::clone(&runtime.gesture)));
            }
        }
        self.current_generation = Some(generation);
        self.current_runtime = Some(runtime);
    }

    fn start_facts(&self, point: Point, tick: u32) -> CallbackFacts {
        let Some(context) = self.context else {
            return CallbackFacts::MissingContext;
        };
        if context.point != point || tick.wrapping_sub(context.updated_tick) > CONTEXT_MAX_AGE_MS {
            return CallbackFacts::StaleContext;
        }
        let Some(generation) = self.current_generation else {
            return CallbackFacts::MissingContext;
        };
        if context.generation != generation.0 {
            return CallbackFacts::WrongGeneration;
        }
        let capacity = !self.replay_reserved && self.actions.free() >= 2;
        CallbackFacts::Start {
            generation,
            binding_set: context.binding_set,
            target: context.target,
            activation: reservation(capacity),
            render_lifecycle: reservation(self.renderer.can_start()),
            replay: reservation(capacity),
        }
    }

    fn handle_owner_event(&mut self, event: InputEvent) {
        let Some(kernel) = self.kernel.as_mut() else {
            return;
        };
        let decision = kernel.handle(event);
        self.apply_effects(decision.effects());
        self.sync_session_runtime();
    }

    fn apply_effects(&mut self, effects: impl Iterator<Item = InputEffect>) {
        for effect in effects {
            match effect {
                InputEffect::ActivateTarget { session, target } => {
                    self.replay_reserved = true;
                    let inserted = self.actions.push(ActionWork::Activate { session, target });
                    debug_assert!(inserted);
                    self.action_wakeup = true;
                }
                InputEffect::DispatchAction {
                    session,
                    generation,
                    action,
                    repeat,
                } => {
                    let inserted = self.actions.push(ActionWork::Dispatch {
                        session,
                        generation,
                        action,
                        repeat,
                    });
                    debug_assert!(inserted);
                    self.action_wakeup = true;
                }
                InputEffect::ReplayTrigger {
                    session,
                    trigger,
                    down_at,
                    up_at,
                } => {
                    let inserted = self.actions.push(ActionWork::Replay {
                        session,
                        trigger,
                        down_at,
                        up_at,
                    });
                    debug_assert!(inserted);
                    self.replay_reserved = false;
                    self.action_wakeup = true;
                }
                InputEffect::RenderStart {
                    session,
                    generation,
                } => self.renderer.start(RenderWork::Start {
                    session,
                    generation,
                }),
                InputEffect::RenderPoint { session, point } => {
                    self.renderer.lossy(RenderWork::Point { session, point });
                }
                InputEffect::RenderLabel { session, action } => {
                    self.renderer.lossy(RenderWork::Label { session, action });
                }
                InputEffect::RenderEnd { session } => {
                    self.renderer.end(RenderWork::End { session });
                }
            }
            if matches!(
                effect,
                InputEffect::RenderStart { .. }
                    | InputEffect::RenderPoint { .. }
                    | InputEffect::RenderLabel { .. }
                    | InputEffect::RenderEnd { .. }
            ) {
                self.render_wakeup = true;
            }
        }
    }

    fn sync_session_runtime(&mut self) {
        let pinned = self
            .kernel
            .as_ref()
            .and_then(InputKernel::pinned_generation);
        match pinned {
            Some(generation) if self.session_runtime.is_none() => {
                debug_assert_eq!(self.current_generation, Some(generation));
                self.session_runtime = self.current_runtime.as_ref().map(Arc::clone);
            }
            None => {
                self.session_runtime = None;
                self.replay_reserved = false;
            }
            _ => {}
        }
    }
}

fn reservation(available: bool) -> Reservation {
    if available {
        Reservation::Reserved
    } else {
        Reservation::CapacityExhausted
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use tempfile::TempDir;

    use super::*;
    use crate::config::{self, ConfigDocument, ConfigOwner};
    use crate::domain::input::tests::count_allocations;

    fn setup() -> (TempDir, ConfigOwner, NativeInputOwner) {
        let directory = tempfile::tempdir().unwrap();
        config::save_atomic(
            &config::ActiveConfig::from_document(ConfigDocument::default()).unwrap(),
            directory.path(),
        )
        .unwrap();
        let (writer, _) = ConfigOwner::startup(directory.path());
        let owner = NativeInputOwner::new(writer.reader());
        (directory, writer, owner)
    }

    fn context(owner: &mut NativeInputOwner, generation: u64, point: Point, tick: u32) {
        owner.set_context(Some(ContextView {
            generation,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: TargetToken(17),
            point,
            updated_tick: tick,
        }));
    }

    fn start(owner: &mut NativeInputOwner, tick: u32) -> SessionId {
        let point = Point::new(0, 0);
        context(
            owner,
            owner.current_generation.map_or(1, |value| value.0),
            point,
            tick,
        );
        assert_eq!(
            owner.callback(MouseEvent::ButtonDown(TriggerButton::Right), point, tick),
            Disposition::Suppress
        );
        let ActionWork::Activate { session, .. } = owner.pop_action().unwrap() else {
            panic!("start must queue activation first");
        };
        owner.activation_result(session, true);
        session
    }

    fn move_up_and_release(owner: &mut NativeInputOwner, tick: u32) -> SessionId {
        assert_eq!(
            owner.callback(MouseEvent::MouseMove, Point::new(0, -24), tick),
            Disposition::Pass
        );
        assert_eq!(
            owner.callback(
                MouseEvent::ButtonUp(TriggerButton::Right),
                Point::new(0, -24),
                tick + 1
            ),
            Disposition::Suppress
        );
        let ActionWork::Dispatch { session, .. } = owner.pop_action().unwrap() else {
            panic!("release must queue an action");
        };
        session
    }

    fn complete(owner: &mut NativeInputOwner, session: SessionId) {
        owner.injection_started(session);
        owner.action_completed(session);
    }

    fn publish(writer: &mut ConfigOwner, revision: u64, document: &ConfigDocument) -> u64 {
        let bytes = serde_json::to_vec(document).unwrap();
        let prepared = writer.prepare(7, revision, &bytes, Instant::now()).unwrap();
        writer
            .commit(
                7,
                prepared.token,
                prepared.base_revision,
                prepared.base_generation,
                Instant::now(),
            )
            .unwrap()
            .generation
    }

    #[test]
    fn repeated_input_accepts_a_second_gesture_after_completion() {
        let (_directory, _writer, mut owner) = setup();
        let first = start(&mut owner, 1);
        assert_eq!(move_up_and_release(&mut owner, 2), first);
        complete(&mut owner, first);
        let second = start(&mut owner, 10);
        assert_eq!(move_up_and_release(&mut owner, 11), second);
        assert_ne!(first, second);
    }

    #[test]
    fn activation_and_action_lane_preserve_fifo() {
        let (_directory, _writer, mut owner) = setup();
        let point = Point::new(0, 0);
        context(&mut owner, 1, point, 1);
        owner.callback(MouseEvent::ButtonDown(TriggerButton::Right), point, 1);
        let ActionWork::Activate { session, .. } = owner.pop_action().unwrap() else {
            panic!("activation must be the first action-lane item");
        };
        owner.activation_result(session, true);
        owner.callback(MouseEvent::MouseMove, Point::new(0, -24), 2);
        owner.callback(
            MouseEvent::ButtonUp(TriggerButton::Right),
            Point::new(0, -24),
            3,
        );
        assert!(matches!(
            owner.pop_action(),
            Some(ActionWork::Dispatch {
                session: queued,
                ..
            }) if queued == session
        ));
    }

    #[test]
    fn native_owner_pins_generation_until_action_completion_then_replaces_it() {
        let (_directory, mut writer, mut owner) = setup();
        let first = start(&mut owner, 1);
        let mut changed = ConfigDocument::default();
        changed.shared.appearance.trail_thickness = 9.0;
        assert_eq!(publish(&mut writer, 1, &changed), 2);
        owner.callback(MouseEvent::MouseMove, Point::new(0, -24), 2);
        owner.callback(
            MouseEvent::ButtonUp(TriggerButton::Right),
            Point::new(0, -24),
            3,
        );
        let ActionWork::Dispatch { generation, .. } = owner.pop_action().unwrap() else {
            panic!("expected dispatch");
        };
        assert_eq!(generation, ConfigGeneration(1));
        complete(&mut owner, first);

        context(&mut owner, 2, Point::new(0, 0), 10);
        assert_eq!(
            owner.callback(
                MouseEvent::ButtonDown(TriggerButton::Right),
                Point::new(0, 0),
                10
            ),
            Disposition::Suppress
        );
        assert_eq!(owner.current_generation, Some(ConfigGeneration(2)));
    }

    #[test]
    fn native_owner_overload_fails_open_before_suppressing_a_trigger() {
        let (_directory, _writer, mut owner) = setup();
        owner.refresh_config();
        while owner.actions.push(ActionWork::Replay {
            session: SessionId(99),
            trigger: TriggerButton::Right,
            down_at: Point::new(0, 0),
            up_at: Point::new(0, 0),
        }) {}
        context(&mut owner, 1, Point::new(0, 0), 1);
        assert_eq!(
            owner.callback(
                MouseEvent::ButtonDown(TriggerButton::Right),
                Point::new(0, 0),
                1
            ),
            Disposition::Pass
        );
        assert!(owner.kernel.as_ref().unwrap().pinned_generation().is_none());
    }

    #[test]
    fn native_owner_replays_the_exact_suppressed_physical_up_point() {
        let (_directory, _writer, mut owner) = setup();
        let point = Point::new(10, 20);
        context(&mut owner, 1, point, 1);
        assert_eq!(
            owner.callback(MouseEvent::ButtonDown(TriggerButton::Right), point, 1),
            Disposition::Suppress
        );
        let ActionWork::Activate { session, .. } = owner.pop_action().unwrap() else {
            panic!("expected activation");
        };
        owner.activation_result(session, false);
        let up = Point::new(30, 40);
        assert_eq!(
            owner.callback(MouseEvent::ButtonUp(TriggerButton::Right), up, 2),
            Disposition::Suppress
        );
        let ActionWork::Replay { down_at, up_at, .. } = owner.pop_action().unwrap() else {
            panic!("expected replay");
        };
        assert_eq!((down_at, up_at), (point, up));
        assert!(owner.pop_action().is_none());
    }

    #[test]
    fn delayed_activation_failure_after_injection_never_replays() {
        let (_directory, _writer, mut owner) = setup();
        let session = start(&mut owner, 1);
        assert_eq!(move_up_and_release(&mut owner, 2), session);
        owner.injection_started(session);
        owner.activation_result(session, false);
        assert!(!matches!(
            owner.pop_action(),
            Some(ActionWork::Replay { .. })
        ));
        assert_eq!(
            owner.callback(MouseEvent::MouseMove, Point::new(1, 1), 9),
            Disposition::Pass
        );
    }

    #[test]
    fn renderer_point_overload_drops_without_delaying_action_delivery() {
        let (_directory, _writer, mut owner) = setup();
        let session = start(&mut owner, 1);
        for tick in 2..=200 {
            owner.callback(MouseEvent::MouseMove, Point::new(0, -(tick as i32)), tick);
        }
        owner.callback(
            MouseEvent::ButtonUp(TriggerButton::Right),
            Point::new(0, -200),
            201,
        );
        assert!(matches!(
            owner.pop_action(),
            Some(ActionWork::Dispatch {
                session: queued,
                ..
            }) if queued == session
        ));
        assert!(owner.renderer.queue.len <= RENDER_CAPACITY);
    }

    #[test]
    fn renderer_generation_changes_only_after_the_pinned_gesture_finishes() {
        let (_directory, mut writer, mut owner) = setup();
        let first = start(&mut owner, 1);
        let first_generation = loop {
            let Some(work) = owner.pop_render() else {
                panic!("first gesture must queue renderer start");
            };
            if let RenderWork::Start { generation, .. } = work {
                break generation;
            }
        };
        assert_eq!(first_generation, ConfigGeneration(1));

        let mut changed = ConfigDocument::default();
        changed.shared.appearance.trail_thickness = 5.0;
        publish(&mut writer, 1, &changed);
        move_up_and_release(&mut owner, 2);
        complete(&mut owner, first);
        while owner.pop_render().is_some() {}

        context(&mut owner, 2, Point::new(0, 0), 10);
        owner.callback(
            MouseEvent::ButtonDown(TriggerButton::Right),
            Point::new(0, 0),
            10,
        );
        let next_generation = loop {
            let Some(work) = owner.pop_render() else {
                panic!("next gesture must queue renderer start");
            };
            if let RenderWork::Start { generation, .. } = work {
                break generation;
            }
        };
        assert_eq!(next_generation, ConfigGeneration(2));
    }

    #[test]
    fn actual_callback_owner_path_performs_no_heap_allocation() {
        let (_directory, _writer, mut owner) = setup();
        owner.refresh_config();
        context(&mut owner, 1, Point::new(0, 0), 1);
        let (disposition, allocations) = count_allocations(|| {
            owner.callback(
                MouseEvent::ButtonDown(TriggerButton::Right),
                Point::new(0, 0),
                1,
            )
        });
        assert_eq!(disposition, Disposition::Suppress);
        assert_eq!(allocations, 0);
    }

    #[test]
    fn shutdown_clears_all_suppression_and_is_idempotent() {
        let (_directory, _writer, mut owner) = setup();
        start(&mut owner, 1);
        owner.shutdown();
        owner.shutdown();
        assert_eq!(
            owner.callback(
                MouseEvent::ButtonUp(TriggerButton::Right),
                Point::new(0, 0),
                2
            ),
            Disposition::Pass
        );
        assert!(owner.session_runtime.is_none());
        assert!(!owner.replay_reserved);
        assert_eq!(owner.actions.len, 0);
    }
}
