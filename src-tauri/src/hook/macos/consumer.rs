//! Run-loop-side consumer for normalized macOS input.

use std::time::Instant;

use super::super::owner::{ActionWork, ContextRoute, ContextView, NativeInputOwner};
#[cfg(target_os = "macos")]
use super::context::ContextWorker;
use super::{NormalizedInput, TapState};
use crate::config::ConfigSnapshotReader;
use crate::domain::input::SessionId;
use crate::domain::{MouseEvent, Point};
use crate::executor::macos::{ExecutionOutcome, MacosActionExecutor};

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
    owner: NativeInputOwner,
    executor: Option<MacosActionExecutor>,
    current_context: Option<ContextView>,
    pending_action: Option<SessionId>,
    executor_alive: bool,
}

impl MacosInputConsumer {
    pub(super) fn new(reader: ConfigSnapshotReader, executor: MacosActionExecutor) -> Self {
        Self {
            owner: NativeInputOwner::new(reader),
            executor: Some(executor),
            current_context: None,
            pending_action: None,
            executor_alive: true,
        }
    }

    pub(super) fn consume(
        &mut self,
        input: NormalizedInput,
        context: Option<ContextView>,
        tick: u32,
    ) {
        self.current_context = context;
        self.owner.set_context(context);
        let _ = self.owner.callback(input.event, input.point, tick);
        self.drain_owner_work(tick);
    }

    #[cfg(target_os = "macos")]
    fn context_route(&mut self, event: MouseEvent) -> ContextRoute {
        self.owner.context_route(event)
    }

    pub(super) fn safety_timer(&mut self, tick: u32) {
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
                                <= super::super::owner::CONTEXT_MAX_AGE_MS
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

    pub(super) fn shutdown(mut self) {
        self.owner.shutdown();
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
        process_input_step(input, clock, context, consumer, functions);
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
    pub(super) route: fn(&mut C, MouseEvent) -> ContextRoute,
    pub(super) set_needed: fn(&mut W, bool),
    pub(super) observe: fn(&mut W, MouseEvent, Point, u64),
    pub(super) latest: fn(&mut W, Point, u32) -> Option<ContextView>,
    pub(super) consume: fn(&mut C, NormalizedInput, Option<ContextView>, u32),
}

fn process_input_step<C, W>(
    input: NormalizedInput,
    clock: &mut OwnerClock,
    context: &mut W,
    consumer: &mut C,
    functions: &InputStepFunctions<C, W>,
) {
    let tick = clock.observe(input.timestamp_ns);
    let route = (functions.route)(consumer, input.event);
    let needed = route != ContextRoute::Inactive;
    (functions.set_needed)(context, needed);
    if route == ContextRoute::Observe {
        (functions.observe)(context, input.event, input.point, input.timestamp_ns);
    }
    let current = needed
        .then(|| (functions.latest)(context, input.point, tick))
        .flatten();
    (functions.consume)(consumer, input, current, tick);
}
