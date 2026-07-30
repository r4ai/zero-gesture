use std::sync::Arc;

use super::{
    ActionId, BindingSetId, Decision, Disposition, GestureConfig, GestureInput, GestureMachine,
    GestureTransition, MouseEvent, Point, RenderEffect, TriggerButton,
};

/// Identity of one immutable compiled configuration view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConfigGeneration(pub(crate) u64);

/// Identity of one input session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SessionId(pub(crate) u64);

/// Platform-neutral token resolved by the future native context adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetToken(pub(crate) u64);

/// Result of one nonblocking reservation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reservation {
    Reserved,
    CapacityExhausted,
    OwnerUnavailable,
}

/// Closed callback facts supplied by the future native adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallbackFacts {
    Start {
        generation: ConfigGeneration,
        binding_set: BindingSetId,
        target: TargetToken,
        activation: Reservation,
        render_lifecycle: Reservation,
        replay: Reservation,
    },
    MissingContext,
    StaleContext,
    WrongGeneration,
    Continue {
        action_delivery: Reservation,
        completion_replay: Reservation,
    },
}

impl CallbackFacts {
    fn action_reserved(self) -> bool {
        matches!(
            self,
            Self::Continue {
                action_delivery: Reservation::Reserved,
                completion_replay: Reservation::Reserved,
            }
        )
    }
}

/// Closed input accepted by [`InputKernel`].
pub(crate) enum InputEvent {
    Pointer {
        event: MouseEvent,
        point: Point,
        tick: u32,
        facts: CallbackFacts,
    },
    SafetyTimer {
        tick: u32,
    },
    ActivationReady(SessionId),
    ActivationFailed(SessionId),
    InjectionStarted(SessionId),
    ActionCompleted(SessionId),
    ActionFailedBeforeInjection(SessionId),
    ActionFailedAfterInjection(SessionId),
    ContextFault,
    ExecutorFault,
    RendererFault,
    Shutdown,
}

/// One fixed numeric effect emitted by the kernel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputEffect {
    ActivateTarget {
        session: SessionId,
        target: TargetToken,
    },
    DispatchAction {
        session: SessionId,
        generation: ConfigGeneration,
        action: ActionId,
        repeat: u16,
    },
    ReplayTrigger {
        session: SessionId,
        trigger: TriggerButton,
        down_at: Point,
        up_at: Point,
    },
    RenderStart {
        session: SessionId,
        generation: ConfigGeneration,
    },
    RenderPoint {
        session: SessionId,
        point: Point,
    },
    RenderLabel {
        session: SessionId,
        action: Option<ActionId>,
    },
    RenderEnd {
        session: SessionId,
    },
}

/// Current fail-open mode after one input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputMode {
    Operational,
    ReplayPending,
    Bypass,
}

/// Fixed-capacity effect sequence for one input.
#[derive(Debug, Clone, Copy, Default)]
struct InputEffects {
    items: [Option<InputEffect>; 3],
}

impl InputEffects {
    fn push(&mut self, effect: InputEffect) {
        let slot = self
            .items
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("one input emitted more than three effects");
        *slot = Some(effect);
    }
}

/// Closed, fixed-capacity decision returned for one input.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InputDecision {
    pub(crate) disposition: Disposition,
    effects: InputEffects,
    pub(crate) mode: InputMode,
}

impl InputDecision {
    pub(crate) fn effects(&self) -> impl Iterator<Item = InputEffect> + '_ {
        self.effects.items.iter().flatten().copied()
    }

    fn pass(mode: InputMode) -> Self {
        Self {
            disposition: Disposition::Pass,
            effects: InputEffects::default(),
            mode,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionPhase {
    InjectionStarted,
    Completed,
    FailedBeforeInjection,
    FailedAfterInjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EssentialOwner {
    Context,
    Executor,
    Renderer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationState {
    Pending,
    Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionPhase {
    PendingBeforeInjection,
    InjectionStarted,
}

#[derive(Debug, Clone, Copy)]
enum PhysicalUp {
    Pending,
    ObservedAndSuppressed(Point),
}

#[derive(Debug, Clone, Copy)]
struct CompletionRecord {
    phase: CompletionPhase,
    physical_up: PhysicalUp,
}

struct ActiveSession {
    id: SessionId,
    generation: ConfigGeneration,
    trigger: TriggerButton,
    down_at: Point,
    last_point: Point,
    activation: ActivationState,
    completion: Option<CompletionRecord>,
    recognition_active: bool,
}

struct ReplayPending {
    id: SessionId,
    trigger: TriggerButton,
    down_at: Point,
    last_point: Point,
}

enum KernelState {
    Idle,
    Active(ActiveSession),
    ReplayPending(ReplayPending),
    Bypass,
}

/// Pure owner kernel for input/session safety policy.
///
/// `handle` performs no I/O, waiting, logging, or heap allocation. The future
/// native adapter supplies pre-resolved context and reservation outcomes.
pub(crate) struct InputKernel {
    active_generation: ConfigGeneration,
    machine: GestureMachine,
    next_session: u64,
    state: KernelState,
}

impl InputKernel {
    pub(crate) fn new(generation: ConfigGeneration, config: Arc<GestureConfig>) -> InputKernel {
        Self {
            active_generation: generation,
            machine: GestureMachine::new(config),
            next_session: 1,
            state: KernelState::Idle,
        }
    }

    /// Publishes the view used only by gestures that start after this call.
    pub(crate) fn publish_config(
        &mut self,
        generation: ConfigGeneration,
        config: Arc<GestureConfig>,
    ) {
        self.active_generation = generation;
        self.machine.publish_config(config);
    }

    pub(crate) fn handle(&mut self, input: InputEvent) -> InputDecision {
        if matches!(self.state, KernelState::Bypass) {
            return InputDecision::pass(InputMode::Bypass);
        }

        match input {
            InputEvent::Shutdown => {
                self.machine.cancel();
                self.state = KernelState::Bypass;
                InputDecision::pass(InputMode::Bypass)
            }
            event @ (InputEvent::ContextFault
            | InputEvent::ExecutorFault
            | InputEvent::RendererFault) => self.handle_owner_fault(owner_fault(event)),
            event @ (InputEvent::ActivationReady(session)
            | InputEvent::ActivationFailed(session)) => {
                self.handle_activation(session, matches!(event, InputEvent::ActivationReady(_)))
            }
            event @ (InputEvent::InjectionStarted(session)
            | InputEvent::ActionCompleted(session)
            | InputEvent::ActionFailedBeforeInjection(session)
            | InputEvent::ActionFailedAfterInjection(session)) => {
                self.handle_action_progress(session, action_phase(event))
            }
            InputEvent::SafetyTimer { tick } => self.handle_safety_timer(tick),
            InputEvent::Pointer {
                event,
                point,
                tick,
                facts,
            } => self.handle_pointer(event, point, tick, facts),
        }
    }

    fn handle_pointer(
        &mut self,
        event: MouseEvent,
        point: Point,
        tick: u32,
        facts: CallbackFacts,
    ) -> InputDecision {
        let state = std::mem::replace(&mut self.state, KernelState::Bypass);
        match state {
            KernelState::Idle => self.start_session(event, point, tick, facts),
            KernelState::Active(session) => {
                self.continue_session(session, event, point, tick, facts)
            }
            KernelState::ReplayPending(mut pending) => {
                pending.last_point = point;
                if event == MouseEvent::ButtonUp(pending.trigger) {
                    let mut effects = InputEffects::default();
                    effects.push(InputEffect::ReplayTrigger {
                        session: pending.id,
                        trigger: pending.trigger,
                        down_at: pending.down_at,
                        up_at: point,
                    });
                    self.state = KernelState::Bypass;
                    InputDecision {
                        disposition: Disposition::Suppress,
                        effects,
                        mode: InputMode::Bypass,
                    }
                } else {
                    self.state = KernelState::ReplayPending(pending);
                    InputDecision::pass(InputMode::ReplayPending)
                }
            }
            KernelState::Bypass => {
                self.state = KernelState::Bypass;
                InputDecision::pass(InputMode::Bypass)
            }
        }
    }

    fn start_session(
        &mut self,
        event: MouseEvent,
        point: Point,
        tick: u32,
        facts: CallbackFacts,
    ) -> InputDecision {
        let MouseEvent::ButtonDown(trigger) = event else {
            self.state = KernelState::Idle;
            return InputDecision::pass(InputMode::Operational);
        };
        let CallbackFacts::Start {
            generation,
            binding_set,
            target,
            activation: Reservation::Reserved,
            render_lifecycle: Reservation::Reserved,
            replay: Reservation::Reserved,
        } = facts
        else {
            self.state = KernelState::Idle;
            return InputDecision::pass(InputMode::Operational);
        };
        if generation != self.active_generation {
            self.state = KernelState::Idle;
            return InputDecision::pass(InputMode::Operational);
        }
        let Some(session) = self.take_session_id() else {
            self.state = KernelState::Bypass;
            return InputDecision::pass(InputMode::Bypass);
        };

        let gesture = self.machine.handle(GestureInput::Pointer {
            event,
            point,
            tick,
            binding_set: Some(binding_set),
        });
        if gesture.disposition == Disposition::Pass {
            self.state = KernelState::Idle;
            return InputDecision::pass(InputMode::Operational);
        }

        let mut effects = InputEffects::default();
        effects.push(InputEffect::ActivateTarget { session, target });
        append_render(&mut effects, session, generation, gesture);
        self.state = KernelState::Active(ActiveSession {
            id: session,
            generation,
            trigger,
            down_at: point,
            last_point: point,
            activation: ActivationState::Pending,
            completion: None,
            recognition_active: true,
        });
        InputDecision {
            disposition: Disposition::Suppress,
            effects,
            mode: InputMode::Operational,
        }
    }

    fn continue_session(
        &mut self,
        mut session: ActiveSession,
        event: MouseEvent,
        point: Point,
        tick: u32,
        facts: CallbackFacts,
    ) -> InputDecision {
        if session.completion.is_some()
            && matches!(event, MouseEvent::WheelUp(_) | MouseEvent::WheelDown(_))
        {
            self.state = KernelState::Active(session);
            return InputDecision::pass(InputMode::Operational);
        }
        session.last_point = point;
        observe_physical_up(session.completion.as_mut(), event, session.trigger, point);
        if !session.recognition_active {
            self.state = KernelState::Active(session);
            return InputDecision::pass(InputMode::Operational);
        }

        let gesture = self.machine.handle(GestureInput::Pointer {
            event,
            point,
            tick,
            binding_set: None,
        });
        let mut effects = InputEffects::default();
        append_render(&mut effects, session.id, session.generation, gesture);

        match gesture.transition {
            GestureTransition::Continue => {
                self.state = KernelState::Active(session);
            }
            GestureTransition::Complete | GestureTransition::Cancel => {
                session.recognition_active = false;
                self.state = if session.completion.is_some() {
                    KernelState::Active(session)
                } else {
                    KernelState::Idle
                };
            }
            GestureTransition::Replay(replay) => {
                effects.push(InputEffect::ReplayTrigger {
                    session: session.id,
                    trigger: replay.trigger,
                    down_at: replay.down_at,
                    up_at: replay.up_at,
                });
                self.state = KernelState::Idle;
            }
            GestureTransition::ContinueWithAction { action, repeat } => {
                return self
                    .accept_action(session, event, point, action, repeat, true, facts, effects);
            }
            GestureTransition::FinishWithAction { action } => {
                return self.accept_action(session, event, point, action, 1, false, facts, effects);
            }
        }

        InputDecision {
            disposition: gesture.disposition,
            effects,
            mode: InputMode::Operational,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn accept_action(
        &mut self,
        mut session: ActiveSession,
        event: MouseEvent,
        point: Point,
        action: ActionId,
        repeat: u16,
        recognition_continues: bool,
        facts: CallbackFacts,
        mut effects: InputEffects,
    ) -> InputDecision {
        session.recognition_active = recognition_continues;
        let accepted = session.activation == ActivationState::Ready
            && session.completion.is_none()
            && facts.action_reserved();
        if !accepted {
            return self.fail_before_injection(session, event, point, effects);
        }

        let physical_up = if event == MouseEvent::ButtonUp(session.trigger) {
            PhysicalUp::ObservedAndSuppressed(point)
        } else {
            PhysicalUp::Pending
        };
        session.completion = Some(CompletionRecord {
            phase: CompletionPhase::PendingBeforeInjection,
            physical_up,
        });
        effects.push(InputEffect::DispatchAction {
            session: session.id,
            generation: session.generation,
            action,
            repeat,
        });
        self.state = KernelState::Active(session);
        InputDecision {
            disposition: Disposition::Suppress,
            effects,
            mode: InputMode::Operational,
        }
    }

    fn fail_before_injection(
        &mut self,
        session: ActiveSession,
        event: MouseEvent,
        point: Point,
        mut effects: InputEffects,
    ) -> InputDecision {
        if session.recognition_active {
            self.machine.cancel();
            effects.push(InputEffect::RenderEnd {
                session: session.id,
            });
        }
        let physical_up = session
            .completion
            .and_then(|completion| match completion.physical_up {
                PhysicalUp::Pending => None,
                PhysicalUp::ObservedAndSuppressed(point) => Some(point),
            })
            .or_else(|| (event == MouseEvent::ButtonUp(session.trigger)).then_some(point));
        if let Some(up_at) = physical_up {
            effects.push(InputEffect::ReplayTrigger {
                session: session.id,
                trigger: session.trigger,
                down_at: session.down_at,
                up_at,
            });
            self.state = KernelState::Bypass;
            InputDecision {
                disposition: Disposition::Suppress,
                effects,
                mode: InputMode::Bypass,
            }
        } else {
            self.state = KernelState::ReplayPending(ReplayPending {
                id: session.id,
                trigger: session.trigger,
                down_at: session.down_at,
                last_point: point,
            });
            InputDecision {
                disposition: Disposition::Pass,
                effects,
                mode: InputMode::ReplayPending,
            }
        }
    }

    fn handle_activation(&mut self, id: SessionId, ready: bool) -> InputDecision {
        let state = std::mem::replace(&mut self.state, KernelState::Bypass);
        let KernelState::Active(mut session) = state else {
            self.state = state;
            return InputDecision::pass(mode_of(&self.state));
        };
        if session.id != id {
            self.state = KernelState::Active(session);
            return InputDecision::pass(InputMode::Operational);
        }

        if ready {
            if session.activation == ActivationState::Pending {
                session.activation = ActivationState::Ready;
            }
            self.state = KernelState::Active(session);
            InputDecision::pass(InputMode::Operational)
        } else if session.activation == ActivationState::Pending {
            let point = session.last_point;
            self.fail_before_injection(session, MouseEvent::Other, point, InputEffects::default())
        } else if session
            .completion
            .is_some_and(|completion| completion.phase == CompletionPhase::InjectionStarted)
        {
            self.fail_after_injection(session)
        } else {
            self.state = KernelState::Active(session);
            InputDecision::pass(InputMode::Operational)
        }
    }

    fn handle_action_progress(&mut self, id: SessionId, progress: ActionPhase) -> InputDecision {
        let state = std::mem::replace(&mut self.state, KernelState::Bypass);
        let KernelState::Active(mut session) = state else {
            self.state = state;
            return InputDecision::pass(mode_of(&self.state));
        };
        if session.id != id {
            self.state = KernelState::Active(session);
            return InputDecision::pass(InputMode::Operational);
        }
        let Some(mut completion) = session.completion else {
            self.state = KernelState::Active(session);
            return InputDecision::pass(InputMode::Operational);
        };

        match (completion.phase, progress) {
            (CompletionPhase::PendingBeforeInjection, ActionPhase::InjectionStarted) => {
                completion.phase = CompletionPhase::InjectionStarted;
                session.completion = Some(completion);
                self.state = KernelState::Active(session);
                InputDecision::pass(InputMode::Operational)
            }
            (CompletionPhase::PendingBeforeInjection, ActionPhase::FailedBeforeInjection) => {
                let point = session.last_point;
                self.fail_before_injection(
                    session,
                    MouseEvent::Other,
                    point,
                    InputEffects::default(),
                )
            }
            (CompletionPhase::InjectionStarted, ActionPhase::Completed) => {
                session.completion = None;
                self.state = if session.recognition_active {
                    KernelState::Active(session)
                } else {
                    KernelState::Idle
                };
                InputDecision::pass(InputMode::Operational)
            }
            (CompletionPhase::InjectionStarted, ActionPhase::FailedAfterInjection) => {
                self.fail_after_injection(session)
            }
            _ => self.fail_after_injection(session),
        }
    }

    fn fail_after_injection(&mut self, session: ActiveSession) -> InputDecision {
        let mut effects = InputEffects::default();
        if session.recognition_active {
            self.machine.cancel();
            effects.push(InputEffect::RenderEnd {
                session: session.id,
            });
        }
        self.state = KernelState::Bypass;
        InputDecision {
            disposition: Disposition::Pass,
            effects,
            mode: InputMode::Bypass,
        }
    }

    fn handle_owner_fault(&mut self, owner: EssentialOwner) -> InputDecision {
        let state = std::mem::replace(&mut self.state, KernelState::Bypass);
        match state {
            KernelState::Idle => {
                self.state = KernelState::Bypass;
                InputDecision::pass(InputMode::Bypass)
            }
            KernelState::Active(session)
                if session.completion.is_some_and(|completion| {
                    completion.phase == CompletionPhase::InjectionStarted
                }) =>
            {
                self.fail_after_injection(session)
            }
            KernelState::Active(mut session) => {
                let point = session.last_point;
                if owner == EssentialOwner::Renderer {
                    self.machine.cancel();
                    session.recognition_active = false;
                }
                self.fail_before_injection(
                    session,
                    MouseEvent::Other,
                    point,
                    InputEffects::default(),
                )
            }
            KernelState::ReplayPending(pending) => {
                self.state = KernelState::ReplayPending(pending);
                InputDecision::pass(InputMode::ReplayPending)
            }
            KernelState::Bypass => {
                self.state = KernelState::Bypass;
                InputDecision::pass(InputMode::Bypass)
            }
        }
    }

    fn handle_safety_timer(&mut self, tick: u32) -> InputDecision {
        let state = std::mem::replace(&mut self.state, KernelState::Bypass);
        let KernelState::Active(session) = state else {
            self.state = state;
            return InputDecision::pass(mode_of(&self.state));
        };
        if !session.recognition_active {
            self.state = KernelState::Active(session);
            return InputDecision::pass(InputMode::Operational);
        }
        let gesture = self.machine.handle(GestureInput::SafetyTimer { tick });
        if gesture.transition != GestureTransition::Cancel {
            self.state = KernelState::Active(session);
            return InputDecision::pass(InputMode::Operational);
        }
        let mut effects = InputEffects::default();
        append_render(&mut effects, session.id, session.generation, gesture);
        let point = session.last_point;
        self.fail_before_injection(session, MouseEvent::Other, point, effects)
    }

    fn take_session_id(&mut self) -> Option<SessionId> {
        let id = SessionId(self.next_session);
        self.next_session = self.next_session.checked_add(1)?;
        Some(id)
    }
}

fn append_render(
    effects: &mut InputEffects,
    session: SessionId,
    generation: ConfigGeneration,
    decision: Decision,
) {
    for render in decision.render {
        effects.push(match render {
            RenderEffect::StartGesture => InputEffect::RenderStart {
                session,
                generation,
            },
            RenderEffect::TrackPoint(point) => InputEffect::RenderPoint { session, point },
            RenderEffect::UpdateLabel(action) => InputEffect::RenderLabel { session, action },
            RenderEffect::EndGesture => InputEffect::RenderEnd { session },
        });
    }
}

fn observe_physical_up(
    completion: Option<&mut CompletionRecord>,
    event: MouseEvent,
    trigger: TriggerButton,
    point: Point,
) {
    if let Some(completion) = completion {
        if event == MouseEvent::ButtonUp(trigger) {
            completion.physical_up = PhysicalUp::ObservedAndSuppressed(point);
        }
    }
}

fn mode_of(state: &KernelState) -> InputMode {
    match state {
        KernelState::Idle | KernelState::Active(_) => InputMode::Operational,
        KernelState::ReplayPending(_) => InputMode::ReplayPending,
        KernelState::Bypass => InputMode::Bypass,
    }
}

fn owner_fault(event: InputEvent) -> EssentialOwner {
    match event {
        InputEvent::ContextFault => EssentialOwner::Context,
        InputEvent::ExecutorFault => EssentialOwner::Executor,
        InputEvent::RendererFault => EssentialOwner::Renderer,
        _ => unreachable!("owner_fault is called only for owner fault inputs"),
    }
}

fn action_phase(event: InputEvent) -> ActionPhase {
    match event {
        InputEvent::InjectionStarted(_) => ActionPhase::InjectionStarted,
        InputEvent::ActionCompleted(_) => ActionPhase::Completed,
        InputEvent::ActionFailedBeforeInjection(_) => ActionPhase::FailedBeforeInjection,
        InputEvent::ActionFailedAfterInjection(_) => ActionPhase::FailedAfterInjection,
        _ => unreachable!("action_phase is called only for action progress inputs"),
    }
}

#[cfg(test)]
mod tests {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    use crate::config::GestureStep;

    use super::*;
    use crate::domain::{AppBindingSet, HoldBinding, ReleaseBinding};

    struct ThreadCountingAllocator;

    thread_local! {
        static COUNTING: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    unsafe impl GlobalAlloc for ThreadCountingAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            COUNTING.with(|counting| {
                if counting.get() {
                    ALLOCATIONS.with(|count| count.set(count.get() + 1));
                }
            });
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { System.dealloc(ptr, layout) };
        }

        unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            COUNTING.with(|counting| {
                if counting.get() {
                    ALLOCATIONS.with(|count| count.set(count.get() + 1));
                }
            });
            unsafe { System.realloc(ptr, layout, size) }
        }
    }

    #[global_allocator]
    static TEST_ALLOCATOR: ThreadCountingAllocator = ThreadCountingAllocator;

    fn count_allocations<T>(run: impl FnOnce() -> T) -> (T, usize) {
        ALLOCATIONS.with(|count| count.set(0));
        COUNTING.with(|counting| counting.set(true));
        let result = run();
        COUNTING.with(|counting| counting.set(false));
        let allocations = ALLOCATIONS.with(Cell::get);
        (result, allocations)
    }

    fn action(index: usize) -> ActionId {
        ActionId::from_index(index).unwrap()
    }

    fn config(action: ActionId) -> Arc<GestureConfig> {
        Arc::new(GestureConfig {
            safety_timeout_ms: 2_000,
            min_segment_px: 1,
            direction_switch_confirm_px: 1,
            axis_ambiguity_deadzone_px: 0,
            replay_distance_threshold_px: 8,
            max_gesture_steps: 8,
            default_binding_set: BindingSetId::from_index(0).unwrap(),
            binding_sets: vec![AppBindingSet {
                release_bindings: vec![ReleaseBinding {
                    trigger: TriggerButton::Right,
                    sequence: vec![GestureStep::Right],
                    action,
                }],
                hold_bindings: Vec::new(),
            }],
        })
    }

    fn hold_config(action: ActionId) -> Arc<GestureConfig> {
        let mut config = config(action);
        let binding_set = &mut Arc::get_mut(&mut config).unwrap().binding_sets[0];
        binding_set.release_bindings.clear();
        binding_set.hold_bindings.push(HoldBinding {
            trigger: TriggerButton::Right,
            sequence: Vec::new(),
            step: GestureStep::WheelUp,
            action,
        });
        config
    }

    fn reserved_start(generation: ConfigGeneration) -> CallbackFacts {
        CallbackFacts::Start {
            generation,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: TargetToken(17),
            activation: Reservation::Reserved,
            render_lifecycle: Reservation::Reserved,
            replay: Reservation::Reserved,
        }
    }

    fn reserved_continue() -> CallbackFacts {
        CallbackFacts::Continue {
            action_delivery: Reservation::Reserved,
            completion_replay: Reservation::Reserved,
        }
    }

    fn pointer(event: MouseEvent, point: Point, tick: u32, facts: CallbackFacts) -> InputEvent {
        InputEvent::Pointer {
            event,
            point,
            tick,
            facts,
        }
    }

    fn start(kernel: &mut InputKernel, facts: CallbackFacts) -> (InputDecision, SessionId) {
        let decision = kernel.handle(pointer(
            MouseEvent::ButtonDown(TriggerButton::Right),
            Point::new(0, 0),
            1,
            facts,
        ));
        let session = decision
            .effects()
            .find_map(|effect| match effect {
                InputEffect::ActivateTarget { session, .. } => Some(session),
                _ => None,
            })
            .unwrap();
        (decision, session)
    }

    fn ready(kernel: &mut InputKernel, session: SessionId) {
        kernel.handle(InputEvent::ActivationReady(session));
    }

    fn move_right(kernel: &mut InputKernel) {
        kernel.handle(pointer(
            MouseEvent::MouseMove,
            Point::new(20, 0),
            2,
            reserved_continue(),
        ));
    }

    fn release(kernel: &mut InputKernel, facts: CallbackFacts) -> InputDecision {
        kernel.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            Point::new(20, 0),
            3,
            facts,
        ))
    }

    fn effects(decision: InputDecision) -> Vec<InputEffect> {
        decision.effects().collect()
    }

    #[test]
    fn fresh_context_starts_with_activation_before_render() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));

        let (decision, session) = start(&mut kernel, reserved_start(generation));

        assert_eq!(decision.disposition, Disposition::Suppress);
        assert_eq!(
            effects(decision),
            vec![
                InputEffect::ActivateTarget {
                    session,
                    target: TargetToken(17),
                },
                InputEffect::RenderStart {
                    session,
                    generation,
                },
                InputEffect::RenderPoint {
                    session,
                    point: Point::new(0, 0),
                },
            ]
        );
    }

    #[test]
    fn missing_context_passes_without_starting() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));

        let decision = kernel.handle(pointer(
            MouseEvent::ButtonDown(TriggerButton::Right),
            Point::new(0, 0),
            1,
            CallbackFacts::MissingContext,
        ));

        assert_eq!(decision.disposition, Disposition::Pass);
        assert!(effects(decision).is_empty());
    }

    #[test]
    fn stale_context_passes_without_starting() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));

        let decision = kernel.handle(pointer(
            MouseEvent::ButtonDown(TriggerButton::Right),
            Point::new(0, 0),
            1,
            CallbackFacts::StaleContext,
        ));

        assert_eq!(decision.disposition, Disposition::Pass);
        assert!(effects(decision).is_empty());
    }

    #[test]
    fn wrong_generation_context_passes_without_starting() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));

        let decision = kernel.handle(pointer(
            MouseEvent::ButtonDown(TriggerButton::Right),
            Point::new(0, 0),
            1,
            CallbackFacts::WrongGeneration,
        ));

        assert_eq!(decision.disposition, Disposition::Pass);
        assert!(effects(decision).is_empty());
    }

    #[test]
    fn activation_capacity_exhaustion_passes_without_starting() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));
        let facts = CallbackFacts::Start {
            generation,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: TargetToken(17),
            activation: Reservation::CapacityExhausted,
            render_lifecycle: Reservation::Reserved,
            replay: Reservation::Reserved,
        };

        let decision = kernel.handle(pointer(
            MouseEvent::ButtonDown(TriggerButton::Right),
            Point::new(0, 0),
            1,
            facts,
        ));

        assert_eq!(decision.disposition, Disposition::Pass);
        assert!(effects(decision).is_empty());
    }

    #[test]
    fn stale_ready_is_ignored() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));
        let (_, session) = start(&mut kernel, reserved_start(generation));

        let stale = kernel.handle(InputEvent::ActivationReady(SessionId(session.0 + 1)));
        move_right(&mut kernel);
        let release = release(&mut kernel, reserved_continue());

        assert!(effects(stale).is_empty());
        assert!(!effects(release)
            .iter()
            .any(|effect| matches!(effect, InputEffect::DispatchAction { .. })));
    }

    #[test]
    fn ready_session_emits_action_after_activation() {
        let generation = ConfigGeneration(1);
        let expected_action = action(1);
        let mut kernel = InputKernel::new(generation, config(expected_action));
        let (start, session) = start(&mut kernel, reserved_start(generation));
        ready(&mut kernel, session);
        move_right(&mut kernel);

        let release = release(&mut kernel, reserved_continue());

        assert!(matches!(
            effects(start)[0],
            InputEffect::ActivateTarget {
                session: emitted,
                ..
            } if emitted == session
        ));
        assert!(effects(release).contains(&InputEffect::DispatchAction {
            session,
            generation,
            action: expected_action,
            repeat: 1,
        }));
    }

    #[test]
    fn pending_hold_action_passes_fast_wheel_then_reuses_completed_slot() {
        let generation = ConfigGeneration(1);
        let expected_action = action(1);
        let mut kernel = InputKernel::new(generation, hold_config(expected_action));
        let (_, session) = start(&mut kernel, reserved_start(generation));
        ready(&mut kernel, session);

        let first = kernel.handle(pointer(
            MouseEvent::WheelUp(1),
            Point::new(0, 0),
            2,
            reserved_continue(),
        ));
        let pending = kernel.handle(pointer(
            MouseEvent::WheelUp(1),
            Point::new(0, 0),
            3,
            reserved_continue(),
        ));
        kernel.handle(InputEvent::InjectionStarted(session));
        kernel.handle(InputEvent::ActionCompleted(session));
        let reused = kernel.handle(pointer(
            MouseEvent::WheelUp(1),
            Point::new(0, 0),
            4,
            reserved_continue(),
        ));

        let dispatched = InputEffect::DispatchAction {
            session,
            generation,
            action: expected_action,
            repeat: 1,
        };
        assert!(effects(first).contains(&dispatched));
        assert_eq!(pending.disposition, Disposition::Pass);
        assert!(effects(pending).is_empty());
        assert!(effects(reused).contains(&dispatched));
    }

    #[test]
    fn action_capacity_exhaustion_replays_captured_trigger() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));
        let (_, session) = start(&mut kernel, reserved_start(generation));
        ready(&mut kernel, session);
        move_right(&mut kernel);
        let facts = CallbackFacts::Continue {
            action_delivery: Reservation::CapacityExhausted,
            completion_replay: Reservation::Reserved,
        };

        let decision = release(&mut kernel, facts);

        assert_eq!(decision.disposition, Disposition::Suppress);
        assert_eq!(decision.mode, InputMode::Bypass);
        assert!(effects(decision).contains(&InputEffect::ReplayTrigger {
            session,
            trigger: TriggerButton::Right,
            down_at: Point::new(0, 0),
            up_at: Point::new(20, 0),
        }));
    }

    #[test]
    fn before_injection_failure_replays_once_with_balanced_buttons() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));
        let (_, session) = start(&mut kernel, reserved_start(generation));
        ready(&mut kernel, session);
        move_right(&mut kernel);
        release(&mut kernel, reserved_continue());

        let failed = kernel.handle(InputEvent::ActionFailedBeforeInjection(session));
        let repeated = kernel.handle(InputEvent::ActionFailedBeforeInjection(session));

        assert_eq!(
            effects(failed),
            vec![InputEffect::ReplayTrigger {
                session,
                trigger: TriggerButton::Right,
                down_at: Point::new(0, 0),
                up_at: Point::new(20, 0),
            }]
        );
        assert!(effects(repeated).is_empty());
    }

    #[test]
    fn before_injection_replay_keeps_suppressed_physical_up_point() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, hold_config(action(1)));
        let (_, session) = start(&mut kernel, reserved_start(generation));
        ready(&mut kernel, session);
        kernel.handle(pointer(
            MouseEvent::WheelUp(1),
            Point::new(0, 0),
            2,
            reserved_continue(),
        ));
        kernel.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            Point::new(10, 11),
            3,
            reserved_continue(),
        ));
        kernel.handle(pointer(
            MouseEvent::MouseMove,
            Point::new(90, 91),
            4,
            reserved_continue(),
        ));

        let failed = kernel.handle(InputEvent::ActionFailedBeforeInjection(session));

        assert!(effects(failed).contains(&InputEffect::ReplayTrigger {
            session,
            trigger: TriggerButton::Right,
            down_at: Point::new(0, 0),
            up_at: Point::new(10, 11),
        }));
    }

    #[test]
    fn after_injection_failure_bypasses_without_replay() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));
        let (_, session) = start(&mut kernel, reserved_start(generation));
        ready(&mut kernel, session);
        move_right(&mut kernel);
        release(&mut kernel, reserved_continue());
        kernel.handle(InputEvent::InjectionStarted(session));

        let failed = kernel.handle(InputEvent::ActionFailedAfterInjection(session));

        assert_eq!(failed.mode, InputMode::Bypass);
        assert!(!effects(failed)
            .iter()
            .any(|effect| matches!(effect, InputEffect::ReplayTrigger { .. })));
    }

    #[test]
    fn late_activation_failure_after_injection_bypasses_without_replay() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));
        let (_, session) = start(&mut kernel, reserved_start(generation));
        ready(&mut kernel, session);
        move_right(&mut kernel);
        release(&mut kernel, reserved_continue());
        kernel.handle(InputEvent::InjectionStarted(session));

        let failed = kernel.handle(InputEvent::ActivationFailed(session));

        assert_eq!(failed.mode, InputMode::Bypass);
        assert!(!effects(failed)
            .iter()
            .any(|effect| matches!(effect, InputEffect::ReplayTrigger { .. })));
    }

    #[test]
    fn renderer_lifecycle_failure_stops_capture_and_enters_bypass() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));
        let (_, session) = start(&mut kernel, reserved_start(generation));

        let failed = kernel.handle(InputEvent::RendererFault);
        let replay = kernel.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            Point::new(0, 0),
            2,
            reserved_continue(),
        ));

        assert_eq!(failed.mode, InputMode::ReplayPending);
        assert_eq!(replay.mode, InputMode::Bypass);
        assert!(effects(replay).contains(&InputEffect::ReplayTrigger {
            session,
            trigger: TriggerButton::Right,
            down_at: Point::new(0, 0),
            up_at: Point::new(0, 0),
        }));
    }

    #[test]
    fn active_session_pins_config_generation() {
        let first_generation = ConfigGeneration(1);
        let second_generation = ConfigGeneration(2);
        let first_action = action(1);
        let second_action = action(2);
        let mut kernel = InputKernel::new(first_generation, config(first_action));
        let (_, first_session) = start(&mut kernel, reserved_start(first_generation));
        ready(&mut kernel, first_session);
        kernel.publish_config(second_generation, config(second_action));
        move_right(&mut kernel);

        let first_release = release(&mut kernel, reserved_continue());
        kernel.handle(InputEvent::InjectionStarted(first_session));
        kernel.handle(InputEvent::ActionCompleted(first_session));
        let (_, second_session) = start(&mut kernel, reserved_start(second_generation));
        ready(&mut kernel, second_session);
        move_right(&mut kernel);
        let second_release = release(&mut kernel, reserved_continue());

        assert!(
            effects(first_release).contains(&InputEffect::DispatchAction {
                session: first_session,
                generation: first_generation,
                action: first_action,
                repeat: 1,
            })
        );
        assert!(
            effects(second_release).contains(&InputEffect::DispatchAction {
                session: second_session,
                generation: second_generation,
                action: second_action,
                repeat: 1,
            })
        );
    }

    #[test]
    fn active_session_pins_safety_timeout() {
        let first_generation = ConfigGeneration(1);
        let second_generation = ConfigGeneration(2);
        let mut first_config = config(action(1));
        Arc::get_mut(&mut first_config).unwrap().safety_timeout_ms = 2_000;
        let mut second_config = config(action(2));
        Arc::get_mut(&mut second_config).unwrap().safety_timeout_ms = 1;
        let mut kernel = InputKernel::new(first_generation, first_config);
        start(&mut kernel, reserved_start(first_generation));
        kernel.publish_config(second_generation, second_config);

        let decision = kernel.handle(InputEvent::SafetyTimer { tick: 3 });

        assert_eq!(decision.mode, InputMode::Operational);
        assert!(!effects(decision)
            .iter()
            .any(|effect| matches!(effect, InputEffect::RenderEnd { .. })));
    }

    #[test]
    fn shutdown_is_idempotent_and_stays_bypass() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));

        let first = kernel.handle(InputEvent::Shutdown);
        let second = kernel.handle(InputEvent::Shutdown);
        let trigger = kernel.handle(pointer(
            MouseEvent::ButtonDown(TriggerButton::Right),
            Point::new(0, 0),
            1,
            reserved_start(generation),
        ));

        assert_eq!(first.mode, InputMode::Bypass);
        assert_eq!(second.mode, InputMode::Bypass);
        assert_eq!(trigger.disposition, Disposition::Pass);
    }

    #[test]
    fn representative_handle_path_allocates_zero_times() {
        let generation = ConfigGeneration(1);
        let mut kernel = InputKernel::new(generation, config(action(1)));

        let (_, allocations) = count_allocations(|| {
            let started = kernel.handle(pointer(
                MouseEvent::ButtonDown(TriggerButton::Right),
                Point::new(0, 0),
                1,
                reserved_start(generation),
            ));
            let session = started
                .effects()
                .find_map(|effect| match effect {
                    InputEffect::ActivateTarget { session, .. } => Some(session),
                    _ => None,
                })
                .unwrap();
            kernel.handle(InputEvent::ActivationReady(session));
            move_right(&mut kernel);
            release(&mut kernel, reserved_continue());
            kernel.handle(InputEvent::InjectionStarted(session));
            kernel.handle(InputEvent::ActionCompleted(session));
        });

        assert_eq!(allocations, 0);
    }
}
