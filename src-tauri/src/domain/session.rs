use std::sync::Arc;

use crate::config::GestureStep;

use super::recognition::GestureRecognizer;

/// A normalized two-axis point whose coordinate conversion remains platform-owned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Point {
    pub(crate) x: i32,
    pub(crate) y: i32,
}

impl Point {
    pub(crate) const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }
}

/// Mouse buttons that can start or participate in a gesture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TriggerButton {
    Left,
    Right,
    Middle,
}

impl TriggerButton {
    fn to_step(self) -> GestureStep {
        match self {
            Self::Left => GestureStep::LeftClick,
            Self::Right => GestureStep::RightClick,
            Self::Middle => GestureStep::MiddleClick,
        }
    }
}

/// Numeric identity of one compiled binding set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct BindingSetId(u32);

impl BindingSetId {
    pub(crate) fn from_index(index: usize) -> Option<Self> {
        u32::try_from(index).ok().map(Self)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Numeric identity of one compiled action and display label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ActionId(u32);

impl ActionId {
    pub(crate) fn from_index(index: usize) -> Option<Self> {
        u32::try_from(index).ok().map(Self)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Platform-normalized mouse input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseEvent {
    ButtonDown(TriggerButton),
    ButtonUp(TriggerButton),
    MouseMove,
    WheelUp(u16),
    WheelDown(u16),
    Other,
}

/// One release-mode binding used by the gesture machine.
#[derive(Debug)]
pub(crate) struct ReleaseBinding {
    pub(crate) trigger: TriggerButton,
    pub(crate) sequence: Vec<GestureStep>,
    pub(crate) action: ActionId,
}

/// One hold-mode binding used by the gesture machine.
#[derive(Debug)]
pub(crate) struct HoldBinding {
    pub(crate) trigger: TriggerButton,
    pub(crate) sequence: Vec<GestureStep>,
    pub(crate) step: GestureStep,
    pub(crate) action: ActionId,
}

/// Precompiled bindings for one application ID or the default set.
#[derive(Debug)]
pub(crate) struct AppBindingSet {
    pub(crate) release_bindings: Vec<ReleaseBinding>,
    pub(crate) hold_bindings: Vec<HoldBinding>,
}

/// Immutable recognition and binding configuration owned by a gesture machine.
#[derive(Debug)]
pub(crate) struct GestureConfig {
    pub(crate) safety_timeout_ms: u32,
    pub(crate) min_segment_px: i32,
    pub(crate) direction_switch_confirm_px: i32,
    pub(crate) axis_ambiguity_deadzone_px: i32,
    pub(crate) replay_distance_threshold_px: i32,
    pub(crate) max_gesture_steps: usize,
    pub(crate) default_binding_set: BindingSetId,
    pub(crate) binding_sets: Vec<AppBindingSet>,
}

impl GestureConfig {
    #[cfg(test)]
    fn has_any_binding_for_trigger(&self, trigger: TriggerButton) -> bool {
        self.binding_sets.iter().any(|set| {
            set.release_bindings
                .iter()
                .any(|binding| binding.trigger == trigger)
                || set
                    .hold_bindings
                    .iter()
                    .any(|binding| binding.trigger == trigger)
        })
    }

    fn has_binding_for_trigger(
        &self,
        trigger: TriggerButton,
        binding_set: Option<BindingSetId>,
    ) -> bool {
        self.matched_binding_set(binding_set).is_some_and(|set| {
            set.release_bindings
                .iter()
                .any(|binding| binding.trigger == trigger)
                || set
                    .hold_bindings
                    .iter()
                    .any(|binding| binding.trigger == trigger)
        }) || self.default_binding_set().is_some_and(|set| {
            set.release_bindings
                .iter()
                .any(|binding| binding.trigger == trigger)
                || set
                    .hold_bindings
                    .iter()
                    .any(|binding| binding.trigger == trigger)
        })
    }

    fn resolve_release(
        &self,
        trigger: TriggerButton,
        sequence: &[GestureStep],
        binding_set: Option<BindingSetId>,
    ) -> Option<&ReleaseBinding> {
        self.matched_binding_set(binding_set)
            .and_then(|set| {
                set.release_bindings
                    .iter()
                    .find(|binding| binding.trigger == trigger && binding.sequence == sequence)
            })
            .or_else(|| {
                self.default_binding_set().and_then(|set| {
                    set.release_bindings
                        .iter()
                        .find(|binding| binding.trigger == trigger && binding.sequence == sequence)
                })
            })
    }

    fn resolve_hold(
        &self,
        trigger: TriggerButton,
        sequence: &[GestureStep],
        step: GestureStep,
        binding_set: Option<BindingSetId>,
    ) -> Option<&HoldBinding> {
        resolve_hold_from_set(
            self.matched_binding_set(binding_set),
            trigger,
            sequence,
            step,
        )
        .or_else(|| resolve_hold_from_set(self.default_binding_set(), trigger, sequence, step))
    }

    fn matched_binding_set(&self, binding_set: Option<BindingSetId>) -> Option<&AppBindingSet> {
        binding_set
            .filter(|set| *set != self.default_binding_set)
            .and_then(|set| self.binding_sets.get(set.index()))
    }

    fn default_binding_set(&self) -> Option<&AppBindingSet> {
        self.binding_sets.get(self.default_binding_set.index())
    }
}

/// Typed input accepted by [`GestureMachine`].
pub(crate) enum GestureInput {
    Pointer {
        event: MouseEvent,
        point: Point,
        tick: u32,
        binding_set: Option<BindingSetId>,
    },
    SafetyTimer {
        tick: u32,
    },
}

/// Whether the platform must pass or suppress the physical input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Disposition {
    Pass,
    Suppress,
}

/// A terminal trigger replay request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplayRequest {
    pub(crate) trigger: TriggerButton,
    pub(crate) down_at: Point,
    pub(crate) up_at: Point,
}

/// The one session transition selected for an input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GestureTransition {
    Continue,
    ContinueWithAction { action: ActionId, repeat: u16 },
    Complete,
    FinishWithAction { action: ActionId },
    Replay(ReplayRequest),
    Cancel,
}

/// A platform-neutral rendering effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderEffect {
    StartGesture,
    TrackPoint(Point),
    UpdateLabel(Option<ActionId>),
    EndGesture,
}

/// Up to two rendering effects emitted by one transition.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RenderEffects([Option<RenderEffect>; 2]);

impl RenderEffects {
    pub(super) fn push(&mut self, effect: RenderEffect) {
        let slot = self
            .0
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("gesture transition emitted more than two render effects");
        *slot = Some(effect);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.0.iter().flatten().count()
    }

    #[cfg(test)]
    fn last(&self) -> Option<&RenderEffect> {
        self.0.iter().flatten().last()
    }
}

impl IntoIterator for RenderEffects {
    type Item = RenderEffect;
    type IntoIter = std::iter::Flatten<std::array::IntoIter<Option<RenderEffect>, 2>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter().flatten()
    }
}

/// Closed decision returned for one input.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Decision {
    pub(crate) disposition: Disposition,
    pub(crate) transition: GestureTransition,
    pub(crate) render: RenderEffects,
}

impl Decision {
    fn pass(transition: GestureTransition) -> Self {
        Self {
            disposition: Disposition::Pass,
            transition,
            render: RenderEffects::default(),
        }
    }
}

enum SessionState {
    Idle,
    Gesturing {
        entered_tick: u32,
        config: Arc<GestureConfig>,
        trigger: TriggerButton,
        recognizer: GestureRecognizer,
        origin: Point,
        last_point: Point,
        travel_distance_px: f64,
        last_label: Option<ActionId>,
        binding_set: Option<BindingSetId>,
        used_hold_action: bool,
    },
}

/// Owned portable gesture recognition and session decision module.
pub(crate) struct GestureMachine {
    config: Arc<GestureConfig>,
    state: SessionState,
}

impl GestureMachine {
    pub(crate) fn new(config: Arc<GestureConfig>) -> Self {
        Self {
            config,
            state: SessionState::Idle,
        }
    }

    /// Returns whether this idle machine can start with `trigger`.
    ///
    /// Windows uses this before performing its existing target activation and
    /// application matching. Active sessions never request fresh context.
    #[cfg(test)]
    pub(crate) fn can_start(&self, trigger: TriggerButton) -> bool {
        matches!(self.state, SessionState::Idle) && self.config.has_any_binding_for_trigger(trigger)
    }

    pub(crate) fn publish_config(&mut self, config: Arc<GestureConfig>) {
        self.config = config;
    }

    pub(super) fn cancel(&mut self) {
        self.state = SessionState::Idle;
    }

    /// Evaluates one normalized input and returns the effects for the caller.
    pub(crate) fn handle(&mut self, input: GestureInput) -> Decision {
        match input {
            GestureInput::Pointer {
                event,
                point,
                tick,
                binding_set,
            } => self.handle_pointer(event, point, tick, binding_set),
            GestureInput::SafetyTimer { tick } => self.handle_safety_timer(tick),
        }
    }

    fn handle_pointer(
        &mut self,
        event: MouseEvent,
        point: Point,
        tick: u32,
        binding_set: Option<BindingSetId>,
    ) -> Decision {
        match &mut self.state {
            SessionState::Idle => {
                let MouseEvent::ButtonDown(trigger) = event else {
                    return Decision::pass(GestureTransition::Continue);
                };
                if !self.config.has_binding_for_trigger(trigger, binding_set) {
                    return Decision::pass(GestureTransition::Continue);
                }

                let config = Arc::clone(&self.config);
                let mut recognizer = GestureRecognizer::new(
                    config.min_segment_px,
                    config.direction_switch_confirm_px,
                    config.axis_ambiguity_deadzone_px,
                    config.max_gesture_steps,
                );
                recognizer.add_point(point.x, point.y);

                self.state = SessionState::Gesturing {
                    entered_tick: tick,
                    config,
                    trigger,
                    recognizer,
                    origin: point,
                    last_point: point,
                    travel_distance_px: 0.0,
                    last_label: None,
                    binding_set,
                    used_hold_action: false,
                };

                let mut decision = Decision {
                    disposition: Disposition::Suppress,
                    transition: GestureTransition::Continue,
                    render: RenderEffects::default(),
                };
                decision.render.push(RenderEffect::StartGesture);
                decision.render.push(RenderEffect::TrackPoint(point));
                decision
            }
            SessionState::Gesturing {
                config,
                trigger,
                recognizer,
                origin,
                last_point,
                travel_distance_px,
                last_label,
                binding_set,
                used_hold_action,
                ..
            } => match event {
                MouseEvent::MouseMove => {
                    *travel_distance_px += segment_distance(*last_point, point);
                    *last_point = point;
                    recognizer.add_point(point.x, point.y);

                    let next_label = resolve_label(config, *trigger, recognizer, *binding_set);
                    let mut decision = Decision::pass(GestureTransition::Continue);
                    decision.render.push(RenderEffect::TrackPoint(point));
                    update_label(&mut decision.render, last_label, next_label);
                    decision
                }
                MouseEvent::WheelUp(steps) => process_wheel_input(
                    WheelContext {
                        config,
                        trigger: *trigger,
                        recognizer,
                        binding_set: *binding_set,
                        last_label,
                        used_hold_action,
                    },
                    GestureStep::WheelUp,
                    steps,
                ),
                MouseEvent::WheelDown(steps) => process_wheel_input(
                    WheelContext {
                        config,
                        trigger: *trigger,
                        recognizer,
                        binding_set: *binding_set,
                        last_label,
                        used_hold_action,
                    },
                    GestureStep::WheelDown,
                    steps,
                ),
                MouseEvent::ButtonDown(button) => {
                    recognizer.add_input_step(button.to_step());
                    let next_label = resolve_label(config, *trigger, recognizer, *binding_set);
                    let mut decision = Decision {
                        disposition: Disposition::Suppress,
                        transition: GestureTransition::Continue,
                        render: RenderEffects::default(),
                    };
                    update_label(&mut decision.render, last_label, next_label);
                    decision
                }
                MouseEvent::ButtonUp(button) => {
                    if button != *trigger {
                        return Decision {
                            disposition: Disposition::Suppress,
                            transition: GestureTransition::Continue,
                            render: RenderEffects::default(),
                        };
                    }

                    let sequence = recognizer.finalize_sequence();
                    let matched_action = sequence.as_ref().and_then(|sequence| {
                        config
                            .resolve_release(*trigger, sequence.as_slice(), *binding_set)
                            .map(|binding| binding.action)
                    });
                    let transition = if let Some(action) = matched_action {
                        GestureTransition::FinishWithAction { action }
                    } else if !*used_hold_action
                        && should_replay_unmatched(
                            *travel_distance_px,
                            config.replay_distance_threshold_px,
                        )
                    {
                        GestureTransition::Replay(ReplayRequest {
                            trigger: *trigger,
                            down_at: *origin,
                            up_at: point,
                        })
                    } else {
                        GestureTransition::Complete
                    };

                    self.state = SessionState::Idle;
                    let mut decision = Decision {
                        disposition: Disposition::Suppress,
                        transition,
                        render: RenderEffects::default(),
                    };
                    decision.render.push(RenderEffect::EndGesture);
                    decision
                }
                MouseEvent::Other => Decision::pass(GestureTransition::Continue),
            },
        }
    }

    fn handle_safety_timer(&mut self, tick: u32) -> Decision {
        let expired = match &self.state {
            SessionState::Idle => false,
            SessionState::Gesturing {
                entered_tick,
                config,
                ..
            } => tick.wrapping_sub(*entered_tick) > config.safety_timeout_ms,
        };
        if !expired {
            return Decision::pass(GestureTransition::Continue);
        }

        self.state = SessionState::Idle;
        let mut decision = Decision::pass(GestureTransition::Cancel);
        decision.render.push(RenderEffect::EndGesture);
        decision
    }
}

struct WheelContext<'a> {
    config: &'a GestureConfig,
    trigger: TriggerButton,
    recognizer: &'a mut GestureRecognizer,
    binding_set: Option<BindingSetId>,
    last_label: &'a mut Option<ActionId>,
    used_hold_action: &'a mut bool,
}

fn process_wheel_input(ctx: WheelContext<'_>, step: GestureStep, steps: u16) -> Decision {
    if steps == 0 {
        return Decision::pass(GestureTransition::Continue);
    }

    let current_sequence = ctx.recognizer.current_sequence();
    let sequence = current_sequence
        .as_ref()
        .map_or(&[][..], |sequence| sequence.as_slice());
    if let Some(binding) = ctx
        .config
        .resolve_hold(ctx.trigger, sequence, step, ctx.binding_set)
    {
        *ctx.used_hold_action = true;
        let mut decision = Decision {
            disposition: Disposition::Suppress,
            transition: GestureTransition::ContinueWithAction {
                action: binding.action,
                repeat: steps,
            },
            render: RenderEffects::default(),
        };
        update_label(&mut decision.render, ctx.last_label, Some(binding.action));
        ctx.recognizer.reset_sequence();
        return decision;
    }

    for _ in 0..usize::from(steps) {
        ctx.recognizer.add_input_step(step);
        if ctx.recognizer.current_sequence().is_none() {
            break;
        }
    }
    let next_label = resolve_label(ctx.config, ctx.trigger, ctx.recognizer, ctx.binding_set);
    let mut decision = Decision {
        disposition: Disposition::Suppress,
        transition: GestureTransition::Continue,
        render: RenderEffects::default(),
    };
    update_label(&mut decision.render, ctx.last_label, next_label);
    decision
}

fn resolve_hold_from_set<'a>(
    set: Option<&'a AppBindingSet>,
    trigger: TriggerButton,
    sequence: &[GestureStep],
    step: GestureStep,
) -> Option<&'a HoldBinding> {
    let set = set?;
    set.hold_bindings
        .iter()
        .find(|binding| {
            binding.trigger == trigger && binding.step == step && binding.sequence == sequence
        })
        .or_else(|| {
            set.hold_bindings.iter().find(|binding| {
                binding.trigger == trigger && binding.step == step && binding.sequence.is_empty()
            })
        })
}

fn resolve_label(
    config: &GestureConfig,
    trigger: TriggerButton,
    recognizer: &GestureRecognizer,
    binding_set: Option<BindingSetId>,
) -> Option<ActionId> {
    recognizer.current_sequence().and_then(|sequence| {
        config
            .resolve_release(trigger, sequence.as_slice(), binding_set)
            .map(|binding| binding.action)
    })
}

fn update_label(
    render: &mut RenderEffects,
    last_label: &mut Option<ActionId>,
    next_label: Option<ActionId>,
) {
    if *last_label != next_label {
        render.push(RenderEffect::UpdateLabel(next_label));
        *last_label = next_label;
    }
}

fn should_replay_unmatched(travel_distance_px: f64, threshold: i32) -> bool {
    travel_distance_px <= f64::from(threshold)
}

fn segment_distance(a: Point, b: Point) -> f64 {
    let dx = f64::from(b.x - a.x);
    let dy = f64::from(b.y - a.y);
    dx.hypot(dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_action(key: &str) -> ActionId {
        ActionId::from_index(usize::from(key.as_bytes()[0])).unwrap()
    }

    fn release_binding(
        trigger: TriggerButton,
        sequence: Vec<GestureStep>,
        action: ActionId,
        _label: &str,
    ) -> ReleaseBinding {
        ReleaseBinding {
            trigger,
            sequence,
            action,
        }
    }

    fn hold_binding(
        trigger: TriggerButton,
        sequence: Vec<GestureStep>,
        step: GestureStep,
        action: ActionId,
        _label: &str,
    ) -> HoldBinding {
        HoldBinding {
            trigger,
            sequence,
            step,
            action,
        }
    }

    fn test_config(release_bindings: Vec<ReleaseBinding>) -> GestureConfig {
        test_config_with_hold(release_bindings, Vec::new())
    }

    fn test_config_with_hold(
        release_bindings: Vec<ReleaseBinding>,
        hold_bindings: Vec<HoldBinding>,
    ) -> GestureConfig {
        GestureConfig {
            safety_timeout_ms: 2000,
            min_segment_px: 1,
            direction_switch_confirm_px: 1,
            axis_ambiguity_deadzone_px: 0,
            replay_distance_threshold_px: 8,
            max_gesture_steps: 8,
            default_binding_set: BindingSetId::from_index(0).unwrap(),
            binding_sets: vec![AppBindingSet {
                release_bindings,
                hold_bindings,
            }],
        }
    }

    fn pointer(event: MouseEvent, x: i32, y: i32, tick: u32) -> GestureInput {
        GestureInput::Pointer {
            event,
            point: Point::new(x, y),
            tick,
            binding_set: None,
        }
    }

    fn start(machine: &mut GestureMachine, trigger: TriggerButton, point: Point, tick: u32) {
        machine.handle(pointer(
            MouseEvent::ButtonDown(trigger),
            point.x,
            point.y,
            tick,
        ));
    }

    fn assert_action(transition: GestureTransition, expected: ActionId, repeat: u16) {
        match transition {
            GestureTransition::ContinueWithAction {
                action,
                repeat: actual,
            } => {
                assert_eq!(action, expected);
                assert_eq!(actual, repeat);
            }
            GestureTransition::FinishWithAction { action } => {
                assert_eq!(action, expected);
                assert_eq!(repeat, 1);
            }
            other => panic!("expected action transition, got {other:?}"),
        }
    }

    #[test]
    fn idle_starts_gesture_on_configured_trigger() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "A",
        )]);
        let mut machine = GestureMachine::new(config.into());

        let decision = machine.handle(pointer(
            MouseEvent::ButtonDown(TriggerButton::Right),
            100,
            200,
            1000,
        ));

        assert_eq!(decision.disposition, Disposition::Suppress);
        assert_eq!(decision.transition, GestureTransition::Continue);
        assert_eq!(decision.render.len(), 2);
        assert!(matches!(
            decision.render.0[0],
            Some(RenderEffect::StartGesture)
        ));
        assert!(!machine.can_start(TriggerButton::Right));
    }

    #[test]
    fn idle_ignores_unconfigured_trigger() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "A",
        )]);
        let mut machine = GestureMachine::new(config.into());

        let decision = machine.handle(pointer(
            MouseEvent::ButtonDown(TriggerButton::Left),
            100,
            200,
            1000,
        ));

        assert_eq!(decision.disposition, Disposition::Pass);
        assert_eq!(decision.render.len(), 0);
        assert!(!machine.can_start(TriggerButton::Left));
        assert!(machine.can_start(TriggerButton::Right));
    }

    #[test]
    fn mouse_move_is_passed_while_gesture_is_active() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "A",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );

        let decision = machine.handle(pointer(MouseEvent::MouseMove, 120, 100, 1010));

        assert_eq!(decision.disposition, Disposition::Pass);
        assert!(matches!(
            decision.render.0[0],
            Some(RenderEffect::TrackPoint(Point { x: 120, y: 100 }))
        ));
        assert!(!machine.can_start(TriggerButton::Right));
    }

    #[test]
    fn executes_action_on_trigger_up_when_sequence_matches() {
        let action = key_action("r");
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right, GestureStep::Down],
            action,
            "Reload",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 150, 100, 1010));
        machine.handle(pointer(MouseEvent::MouseMove, 150, 150, 1020));

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            150,
            150,
            1030,
        ));

        assert_eq!(decision.disposition, Disposition::Suppress);
        assert_action(decision.transition, action, 1);
        assert!(matches!(
            decision.render.last(),
            Some(RenderEffect::EndGesture)
        ));
        assert!(machine.can_start(TriggerButton::Right));
    }

    #[test]
    fn trigger_click_without_matching_sequence_requests_replay() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "A",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            100,
            100,
            1010,
        ));

        assert_eq!(
            decision.transition,
            GestureTransition::Replay(ReplayRequest {
                trigger: TriggerButton::Right,
                down_at: Point::new(100, 100),
                up_at: Point::new(100, 100),
            })
        );
    }

    #[test]
    fn unmatched_sequence_with_short_move_requests_replay() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 105, 103, 1010));

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            106,
            104,
            1020,
        ));

        assert!(matches!(decision.transition, GestureTransition::Replay(_)));
    }

    #[test]
    fn unmatched_sequence_with_long_move_does_not_request_replay() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 150, 100, 1010));

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            160,
            120,
            1020,
        ));

        assert_eq!(decision.transition, GestureTransition::Complete);
    }

    #[test]
    fn unmatched_sequence_with_small_displacement_but_large_travel_does_not_request_replay() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 160, 100, 1010));
        machine.handle(pointer(MouseEvent::MouseMove, 100, 100, 1020));

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            102,
            100,
            1030,
        ));

        assert_eq!(decision.transition, GestureTransition::Complete);
    }

    #[test]
    fn unmatched_sequence_replay_threshold_is_configurable() {
        let mut config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        config.replay_distance_threshold_px = 32;
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 119, 100, 1010));

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            120,
            100,
            1020,
        ));

        assert!(matches!(decision.transition, GestureTransition::Replay(_)));
    }

    #[test]
    fn replay_threshold_is_not_coupled_to_recognition_thresholds() {
        let mut config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        config.min_segment_px = 30;
        config.direction_switch_confirm_px = 24;
        config.replay_distance_threshold_px = 6;
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 107, 100, 1005));

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            107,
            100,
            1010,
        ));

        assert_eq!(decision.transition, GestureTransition::Complete);
    }

    #[test]
    fn supports_wheel_input_in_sequence() {
        let action = key_action("pageup");
        let config = test_config(vec![release_binding(
            TriggerButton::Middle,
            vec![GestureStep::WheelUp],
            action,
            "PageUp",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(&mut machine, TriggerButton::Middle, Point::new(0, 0), 100);
        machine.handle(pointer(MouseEvent::WheelUp(1), 0, 0, 110));

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Middle),
            0,
            0,
            120,
        ));

        assert_action(decision.transition, action, 1);
    }

    #[test]
    fn hold_wheel_executes_immediately_with_repeat_count() {
        let action = key_action("pageup");
        let config = test_config_with_hold(
            Vec::new(),
            vec![hold_binding(
                TriggerButton::Right,
                Vec::new(),
                GestureStep::WheelUp,
                action,
                "PageUp",
            )],
        );
        let mut machine = GestureMachine::new(config.into());
        start(&mut machine, TriggerButton::Right, Point::new(0, 0), 100);

        let decision = machine.handle(pointer(MouseEvent::WheelUp(2), 0, 0, 110));

        assert_eq!(decision.disposition, Disposition::Suppress);
        assert_action(decision.transition, action, 2);
    }

    #[test]
    fn hold_wheel_usage_disables_unmatched_trigger_replay() {
        let action = key_action("pagedown");
        let config = test_config_with_hold(
            Vec::new(),
            vec![hold_binding(
                TriggerButton::Right,
                Vec::new(),
                GestureStep::WheelDown,
                action,
                "PageDown",
            )],
        );
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::WheelDown(1), 100, 100, 1010));

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Right),
            100,
            100,
            1020,
        ));

        assert_eq!(decision.transition, GestureTransition::Complete);
    }

    #[test]
    fn hold_wheel_can_require_specific_sequence_state() {
        let action = key_action("pagedown");
        let config = test_config_with_hold(
            Vec::new(),
            vec![hold_binding(
                TriggerButton::Right,
                vec![GestureStep::Right],
                GestureStep::WheelDown,
                action,
                "Right + WheelDown",
            )],
        );
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 150, 100, 1010));

        let decision = machine.handle(pointer(MouseEvent::WheelDown(1), 150, 100, 1020));

        assert_action(decision.transition, action, 1);
    }

    #[test]
    fn hold_wheel_specific_sequence_overrides_wildcard_binding() {
        let specific_action = key_action("b");
        let config = test_config_with_hold(
            Vec::new(),
            vec![
                hold_binding(
                    TriggerButton::Right,
                    Vec::new(),
                    GestureStep::WheelDown,
                    key_action("a"),
                    "Any WheelDown",
                ),
                hold_binding(
                    TriggerButton::Right,
                    vec![GestureStep::Right],
                    GestureStep::WheelDown,
                    specific_action,
                    "Right WheelDown",
                ),
            ],
        );
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 150, 100, 1010));

        let decision = machine.handle(pointer(MouseEvent::WheelDown(1), 150, 100, 1020));

        assert_action(decision.transition, specific_action, 1);
    }

    #[test]
    fn hold_wheel_match_resets_recognized_sequence() {
        let wildcard_action = key_action("w");
        let specific_action = key_action("s");
        let config = test_config_with_hold(
            Vec::new(),
            vec![
                hold_binding(
                    TriggerButton::Right,
                    Vec::new(),
                    GestureStep::WheelUp,
                    wildcard_action,
                    "Any WheelUp",
                ),
                hold_binding(
                    TriggerButton::Right,
                    vec![GestureStep::Right],
                    GestureStep::WheelUp,
                    specific_action,
                    "Right WheelUp",
                ),
            ],
        );
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 150, 100, 1010));

        let first = machine.handle(pointer(MouseEvent::WheelUp(1), 150, 100, 1020));
        let second = machine.handle(pointer(MouseEvent::WheelUp(1), 150, 100, 1030));

        assert_action(first.transition, specific_action, 1);
        assert_action(second.transition, wildcard_action, 1);
    }

    #[test]
    fn non_trigger_button_up_does_not_end_gesture() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("x"),
            "X",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(&mut machine, TriggerButton::Right, Point::new(0, 0), 100);

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Left),
            0,
            0,
            110,
        ));

        assert_eq!(decision.disposition, Disposition::Suppress);
        assert_eq!(decision.transition, GestureTransition::Continue);
        assert!(!machine.can_start(TriggerButton::Right));
    }

    #[test]
    fn resolve_binding_prefers_app_specific_then_fallback() {
        let default_action = key_action("a");
        let app_action = key_action("b");
        let mut config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            default_action,
            "Default",
        )]);
        let app_set = BindingSetId::from_index(config.binding_sets.len()).unwrap();
        config.binding_sets.push(AppBindingSet {
            release_bindings: vec![release_binding(
                TriggerButton::Right,
                vec![GestureStep::Right],
                app_action,
                "App",
            )],
            hold_bindings: Vec::new(),
        });

        let app = config
            .resolve_release(TriggerButton::Right, &[GestureStep::Right], Some(app_set))
            .expect("app binding");
        assert_eq!(app.action, app_action);

        let fallback = config
            .resolve_release(
                TriggerButton::Right,
                &[GestureStep::Right],
                Some(BindingSetId::from_index(99).unwrap()),
            )
            .expect("default fallback");
        assert_eq!(fallback.action, default_action);

        let no_app = config
            .resolve_release(TriggerButton::Right, &[GestureStep::Right], None)
            .expect("default binding without matched app");
        assert_eq!(no_app.action, default_action);
    }

    #[test]
    fn resolve_hold_binding_prefers_app_specific_then_fallback() {
        let default_action = key_action("a");
        let app_action = key_action("b");
        let mut config = test_config_with_hold(
            Vec::new(),
            vec![hold_binding(
                TriggerButton::Right,
                Vec::new(),
                GestureStep::WheelUp,
                default_action,
                "Default",
            )],
        );
        let app_set = BindingSetId::from_index(config.binding_sets.len()).unwrap();
        config.binding_sets.push(AppBindingSet {
            release_bindings: Vec::new(),
            hold_bindings: vec![hold_binding(
                TriggerButton::Right,
                Vec::new(),
                GestureStep::WheelUp,
                app_action,
                "App",
            )],
        });

        let app = config
            .resolve_hold(
                TriggerButton::Right,
                &[],
                GestureStep::WheelUp,
                Some(app_set),
            )
            .expect("app hold binding");
        assert_eq!(app.action, app_action);

        let no_app = config
            .resolve_hold(TriggerButton::Right, &[], GestureStep::WheelUp, None)
            .expect("default hold binding without matched app");
        assert_eq!(no_app.action, default_action);
    }

    #[test]
    fn direction_remains_pending_until_switch_threshold_is_reached() {
        let mut config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "Right",
        )]);
        config.direction_switch_confirm_px = 8;
        let mut machine = GestureMachine::new(config.into());
        start(&mut machine, TriggerButton::Right, Point::new(0, 0), 100);

        let pending = machine.handle(pointer(MouseEvent::MouseMove, 7, 0, 110));
        let confirmed = machine.handle(pointer(MouseEvent::MouseMove, 8, 0, 120));

        assert_eq!(pending.render.len(), 1);
        assert!(matches!(
            confirmed.render.last(),
            Some(RenderEffect::UpdateLabel(Some(label))) if *label == key_action("a")
        ));
    }

    #[test]
    fn recognizes_multi_segment_direction_sequence() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right, GestureStep::Down],
            key_action("a"),
            "Right Down",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            100,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 150, 100, 110));

        let decision = machine.handle(pointer(MouseEvent::MouseMove, 150, 150, 120));

        assert!(matches!(
            decision.render.last(),
            Some(RenderEffect::UpdateLabel(Some(label))) if *label == key_action("a")
        ));
    }

    #[test]
    fn safety_timeout_works_with_wrapping_ticks() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "A",
        )]);
        let mut machine = GestureMachine::new(config.into());
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(0, 0),
            u32::MAX - 500,
        );

        let decision = machine.handle(GestureInput::SafetyTimer { tick: 2500 });

        assert_eq!(decision.transition, GestureTransition::Cancel);
        assert!(matches!(
            decision.render.last(),
            Some(RenderEffect::EndGesture)
        ));
        assert!(machine.can_start(TriggerButton::Right));
    }
}
