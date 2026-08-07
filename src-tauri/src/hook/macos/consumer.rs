//! Run-loop-side consumer for normalized macOS input.

use std::time::Instant;

use super::super::owner::{ActionWork, ContextRoute, ContextView};
use super::context::ContextWorker;
use super::{NormalizedInput, TapState};
use crate::domain::input::{SessionId, TargetToken};
use crate::domain::{MouseEvent, Point};
use crate::executor::macos::{ExecutionOutcome, ExecutorWorkKind, MacosActionExecutor};

pub(super) struct OwnerClock {
    tick: u32,
    observed_at: Instant,
}

impl OwnerClock {
    pub(super) fn new() -> Self {
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

    #[cfg(target_os = "macos")]
    pub(super) fn current(&self) -> u32 {
        self.tick
            .wrapping_add(self.observed_at.elapsed().as_millis() as u32)
    }
}

pub(super) struct MacosInputConsumer {
    executor: Option<MacosActionExecutor>,
    pending_activation: Option<PendingActivation>,
    pending_executor: Option<(SessionId, ExecutorWorkKind)>,
    executor_alive: bool,
}

#[derive(Clone, Copy)]
struct PendingActivation {
    session: SessionId,
    target: TargetToken,
    request_id: u64,
    point: Point,
}

impl MacosInputConsumer {
    pub(super) fn new(executor: MacosActionExecutor) -> Self {
        Self {
            executor: Some(executor),
            pending_activation: None,
            pending_executor: None,
            executor_alive: true,
        }
    }

    pub(super) fn consume(
        &mut self,
        state: &TapState,
        context_worker: &mut ContextWorker,
        input: NormalizedInput,
        context: Option<ContextView>,
        tick: u32,
    ) {
        state.with_owner_mut(|owner| owner.set_context(context));
        self.poll_activation(state, context_worker, tick);
        self.drain_owner_work(state, context_worker, tick, Some(input.point));
    }

    #[cfg(any(target_os = "macos", test))]
    pub(super) fn context_route(&mut self, state: &TapState, event: MouseEvent) -> ContextRoute {
        state
            .with_owner_mut(|owner| owner.context_route(event))
            .unwrap_or(ContextRoute::Inactive)
    }

    #[cfg(test)]
    pub(super) fn has_pending_activation(&self) -> bool {
        self.pending_activation.is_some()
    }

    #[cfg(test)]
    pub(super) fn has_unpolled_executor_result(&self) -> bool {
        self.executor
            .as_ref()
            .is_some_and(MacosActionExecutor::has_result)
    }

    #[cfg(any(target_os = "macos", test))]
    pub(super) fn refresh_context(
        &mut self,
        state: &TapState,
        context: &mut ContextWorker,
        tick: u32,
    ) {
        let latest = context.latest_observed(tick);
        state.with_owner_mut(|owner| owner.set_context(latest));
        self.poll_activation(state, context, tick);
    }

    #[cfg(any(target_os = "macos", test))]
    pub(super) fn safety_timer(
        &mut self,
        state: &TapState,
        context: &mut ContextWorker,
        tick: u32,
    ) {
        self.refresh_context(state, context, tick);
        self.poll_executor(state);
        state.with_owner_mut(|owner| owner.safety_timer(tick));
        self.drain_owner_work(state, context, tick, None);
    }

    fn drain_owner_work(
        &mut self,
        state: &TapState,
        context: &mut ContextWorker,
        tick: u32,
        input_point: Option<Point>,
    ) {
        self.poll_executor(state);
        while let Some(work) = state.with_owner_mut(|owner| owner.pop_action()).flatten() {
            match work {
                ActionWork::Activate { session, target } => {
                    let request_id = context.last_request_id();
                    match (request_id, input_point, self.pending_activation) {
                        (0, _, _) | (_, None, _) | (_, _, Some(_)) => {
                            state.with_owner_mut(|owner| owner.activation_result(session, false));
                        }
                        (_, Some(point), None) => {
                            self.pending_activation = Some(PendingActivation {
                                session,
                                target,
                                request_id,
                                point,
                            });
                        }
                    }
                }
                ActionWork::Dispatch {
                    session,
                    generation,
                    action,
                    repeat,
                } => self.dispatch(state, session, generation, action, repeat),
                ActionWork::Replay {
                    session,
                    trigger,
                    down_at,
                    up_at,
                } => self.replay(state, session, trigger, down_at, up_at),
            }
        }
        while state
            .with_owner_mut(|owner| owner.pop_render())
            .flatten()
            .is_some()
        {}
        self.poll_activation(state, context, tick);
    }

    fn dispatch(
        &mut self,
        state: &TapState,
        session: SessionId,
        generation: crate::domain::input::ConfigGeneration,
        action: crate::domain::ActionId,
        repeat: u16,
    ) {
        let Some(runtime) = state
            .with_owner_mut(|owner| owner.runtime(generation))
            .flatten()
        else {
            self.degrade(state);
            return;
        };
        let delivered = self.executor_alive
            && self.pending_executor.is_none()
            && self.executor.as_ref().is_some_and(|executor| {
                executor.try_dispatch(session, runtime.action(action).clone(), repeat)
            });
        if delivered {
            self.pending_executor = Some((session, ExecutorWorkKind::Action));
        } else {
            state.with_owner_mut(|owner| owner.action_failed_before_injection(session));
        }
    }

    fn replay(
        &mut self,
        state: &TapState,
        session: SessionId,
        trigger: crate::domain::TriggerButton,
        down_at: Point,
        up_at: Point,
    ) {
        let delivered = self.executor_alive
            && self.pending_executor.is_none()
            && self
                .executor
                .as_ref()
                .is_some_and(|executor| executor.try_replay(session, trigger, down_at, up_at));
        if delivered {
            self.pending_executor = Some((session, ExecutorWorkKind::Replay));
        } else {
            self.degrade(state);
        }
    }

    fn poll_activation(&mut self, state: &TapState, context: &ContextWorker, tick: u32) {
        let Some(pending) = self.pending_activation else {
            return;
        };
        let Some(ready) =
            context.activation_result(pending.request_id, pending.target, pending.point, tick)
        else {
            return;
        };
        self.pending_activation = None;
        state.with_owner_mut(|owner| owner.activation_result(pending.session, ready));
    }

    fn poll_executor(&mut self, state: &TapState) {
        loop {
            let result = self.executor.as_ref().map(MacosActionExecutor::poll);
            match result {
                Some(Ok(Some(result)))
                    if self.pending_executor == Some((result.session, result.kind)) =>
                {
                    self.pending_executor = None;
                    match (result.kind, result.outcome) {
                        (ExecutorWorkKind::Action, ExecutionOutcome::Posted) => {
                            state.with_owner_mut(|owner| {
                                owner.injection_started(result.session);
                                owner.action_completed(result.session);
                            });
                        }
                        (ExecutorWorkKind::Action, ExecutionOutcome::FailedBeforeInjection) => {
                            state.with_owner_mut(|owner| {
                                owner.action_failed_before_injection(result.session);
                            });
                        }
                        (ExecutorWorkKind::Action, ExecutionOutcome::FailedAfterInjection) => {
                            state.with_owner_mut(|owner| {
                                owner.injection_started(result.session);
                                owner.action_failed_after_injection(result.session);
                            });
                        }
                        (ExecutorWorkKind::Replay, ExecutionOutcome::Posted) => {}
                        (ExecutorWorkKind::Replay, _) => self.degrade(state),
                    }
                }
                Some(Ok(Some(_))) => self.degrade(state),
                Some(Ok(None)) | None => break,
                Some(Err(())) => {
                    self.degrade(state);
                    break;
                }
            }
        }
    }

    fn degrade(&mut self, state: &TapState) {
        self.executor_alive = false;
        self.pending_executor = None;
        state.disable_active_input();
        state.with_owner_mut(|owner| owner.executor_fault());
    }

    #[cfg(any(target_os = "macos", test))]
    pub(super) fn prepare_shutdown(
        &mut self,
        state: &TapState,
        context: &mut ContextWorker,
        tick: u32,
    ) {
        state.disable_active_input();
        if self.pending_executor.is_some() {
            state.with_owner_mut(|owner| owner.shutdown());
            return;
        }
        state.with_owner_mut(|owner| owner.shutdown_with_replay());
        self.drain_owner_work(state, context, tick, None);
    }

    pub(super) fn finish_shutdown(mut self) {
        if let Some(executor) = self.executor.take() {
            executor.shutdown();
        }
    }
}

pub(super) fn drain_input<C, W>(
    state: &TapState,
    context: &mut W,
    consumer: &mut C,
    clock: &mut OwnerClock,
    functions: &InputStepFunctions<C, W>,
) {
    state.drain(|input| {
        process_input_step(state, input, clock, context, consumer, functions);
    });
}

#[cfg(target_os = "macos")]
pub(super) const MACOS_INPUT_STEP_FUNCTIONS: InputStepFunctions<MacosInputConsumer, ContextWorker> =
    InputStepFunctions {
        route: MacosInputConsumer::context_route,
        set_needed: ContextWorker::set_needed,
        observe: ContextWorker::observe,
        latest: ContextWorker::latest,
        consume: MacosInputConsumer::consume,
    };

pub(super) struct InputStepFunctions<C, W> {
    pub(super) route: fn(&mut C, &TapState, MouseEvent) -> ContextRoute,
    pub(super) set_needed: fn(&mut W, bool),
    pub(super) observe: fn(&mut W, MouseEvent, Point, u64),
    pub(super) latest: fn(&mut W, Point, u32) -> Option<ContextView>,
    pub(super) consume: fn(&mut C, &TapState, &mut W, NormalizedInput, Option<ContextView>, u32),
}

fn process_input_step<C, W>(
    state: &TapState,
    input: NormalizedInput,
    clock: &mut OwnerClock,
    context: &mut W,
    consumer: &mut C,
    functions: &InputStepFunctions<C, W>,
) {
    let tick = clock.observe(input.timestamp_ns);
    let route = (functions.route)(consumer, state, input.event);
    let needed = route != ContextRoute::Inactive;
    (functions.set_needed)(context, needed);
    if route == ContextRoute::Observe {
        (functions.observe)(context, input.event, input.point, input.timestamp_ns);
    }
    let current = needed
        .then(|| (functions.latest)(context, input.point, tick))
        .flatten();
    (functions.consume)(consumer, state, context, input, current, tick);
}
