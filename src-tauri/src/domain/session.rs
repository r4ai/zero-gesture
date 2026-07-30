use std::collections::HashMap;

use crate::config::{Action, GestureStep};

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
#[derive(Debug, Clone)]
pub(crate) struct ReleaseBinding {
    pub(crate) trigger: TriggerButton,
    pub(crate) sequence: Vec<GestureStep>,
    pub(crate) action: Action,
    pub(crate) label: String,
}

/// One hold-mode binding used by the gesture machine.
#[derive(Debug, Clone)]
pub(crate) struct HoldBinding {
    pub(crate) trigger: TriggerButton,
    pub(crate) sequence: Vec<GestureStep>,
    pub(crate) step: GestureStep,
    pub(crate) action: Action,
    pub(crate) label: String,
}

/// Precompiled bindings for one application ID or the default set.
#[derive(Debug, Clone)]
pub(crate) struct AppBindingSet {
    pub(crate) release_bindings: Vec<ReleaseBinding>,
    pub(crate) hold_bindings: Vec<HoldBinding>,
}

/// Immutable recognition and binding configuration owned by a gesture machine.
#[derive(Debug, Clone)]
pub(crate) struct GestureConfig {
    pub(crate) safety_timeout_ms: u32,
    pub(crate) min_segment_px: i32,
    pub(crate) direction_switch_confirm_px: i32,
    pub(crate) axis_ambiguity_deadzone_px: i32,
    pub(crate) replay_distance_threshold_px: i32,
    pub(crate) max_gesture_steps: usize,
    pub(crate) binding_sets: HashMap<String, AppBindingSet>,
}

impl GestureConfig {
    fn has_any_binding_for_trigger(&self, trigger: TriggerButton) -> bool {
        self.binding_sets.values().any(|set| {
            set.release_bindings
                .iter()
                .any(|binding| binding.trigger == trigger)
                || set
                    .hold_bindings
                    .iter()
                    .any(|binding| binding.trigger == trigger)
        })
    }

    fn has_binding_for_trigger(&self, trigger: TriggerButton, matched_app: Option<&str>) -> bool {
        self.matched_binding_set(matched_app).is_some_and(|set| {
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
        matched_app: Option<&str>,
    ) -> Option<&ReleaseBinding> {
        self.matched_binding_set(matched_app)
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
        matched_app: Option<&str>,
    ) -> Option<&HoldBinding> {
        resolve_hold_from_set(
            self.matched_binding_set(matched_app),
            trigger,
            sequence,
            step,
        )
        .or_else(|| resolve_hold_from_set(self.default_binding_set(), trigger, sequence, step))
    }

    fn matched_binding_set(&self, matched_app: Option<&str>) -> Option<&AppBindingSet> {
        matched_app
            .filter(|app_id| *app_id != crate::config::DEFAULT_APP_ID)
            .and_then(|app_id| self.binding_sets.get(app_id))
    }

    fn default_binding_set(&self) -> Option<&AppBindingSet> {
        self.binding_sets.get(crate::config::DEFAULT_APP_ID)
    }
}

/// Typed input accepted by [`GestureMachine`].
pub(crate) enum GestureInput {
    Pointer {
        event: MouseEvent,
        point: Point,
        tick: u32,
        matched_app: Option<String>,
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GestureTransition {
    Continue,
    ContinueWithAction { action: Action, repeat: u16 },
    Complete,
    FinishWithAction { action: Action },
    Replay(ReplayRequest),
    Cancel,
}

/// A platform-neutral rendering effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RenderEffect {
    StartGesture,
    TrackPoint(Point),
    UpdateLabel(Option<String>),
    EndGesture,
}

/// Up to two rendering effects emitted by one transition.
#[derive(Debug, Default)]
pub(crate) struct RenderEffects([Option<RenderEffect>; 2]);

impl RenderEffects {
    fn push(&mut self, effect: RenderEffect) {
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
        trigger: TriggerButton,
        recognizer: GestureRecognizer,
        origin: Point,
        last_point: Point,
        travel_distance_px: f64,
        last_label: Option<String>,
        matched_app: Option<String>,
        used_hold_action: bool,
    },
}

/// Owned portable gesture recognition and session decision module.
pub(crate) struct GestureMachine {
    config: GestureConfig,
    state: SessionState,
}

impl GestureMachine {
    pub(crate) fn new(config: GestureConfig) -> Self {
        Self {
            config,
            state: SessionState::Idle,
        }
    }

    /// Returns whether this idle machine can start with `trigger`.
    ///
    /// Windows uses this before performing its existing target activation and
    /// application matching. Active sessions never request fresh context.
    pub(crate) fn can_start(&self, trigger: TriggerButton) -> bool {
        matches!(self.state, SessionState::Idle) && self.config.has_any_binding_for_trigger(trigger)
    }

    /// Evaluates one normalized input and returns the effects for the caller.
    pub(crate) fn handle(&mut self, input: GestureInput) -> Decision {
        match input {
            GestureInput::Pointer {
                event,
                point,
                tick,
                matched_app,
            } => self.handle_pointer(event, point, tick, matched_app),
            GestureInput::SafetyTimer { tick } => self.handle_safety_timer(tick),
        }
    }

    fn handle_pointer(
        &mut self,
        event: MouseEvent,
        point: Point,
        tick: u32,
        matched_app: Option<String>,
    ) -> Decision {
        match &mut self.state {
            SessionState::Idle => {
                let MouseEvent::ButtonDown(trigger) = event else {
                    return Decision::pass(GestureTransition::Continue);
                };
                if !self
                    .config
                    .has_binding_for_trigger(trigger, matched_app.as_deref())
                {
                    return Decision::pass(GestureTransition::Continue);
                }

                let mut recognizer = GestureRecognizer::new(
                    self.config.min_segment_px,
                    self.config.direction_switch_confirm_px,
                    self.config.axis_ambiguity_deadzone_px,
                    self.config.max_gesture_steps,
                );
                recognizer.add_point(point.x, point.y);

                self.state = SessionState::Gesturing {
                    entered_tick: tick,
                    trigger,
                    recognizer,
                    origin: point,
                    last_point: point,
                    travel_distance_px: 0.0,
                    last_label: None,
                    matched_app,
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
                trigger,
                recognizer,
                origin,
                last_point,
                travel_distance_px,
                last_label,
                matched_app,
                used_hold_action,
                ..
            } => match event {
                MouseEvent::MouseMove => {
                    *travel_distance_px += segment_distance(*last_point, point);
                    *last_point = point;
                    recognizer.add_point(point.x, point.y);

                    let next_label =
                        resolve_label(&self.config, *trigger, recognizer, matched_app.as_deref());
                    let mut decision = Decision::pass(GestureTransition::Continue);
                    decision.render.push(RenderEffect::TrackPoint(point));
                    update_label(&mut decision.render, last_label, next_label);
                    decision
                }
                MouseEvent::WheelUp(steps) => process_wheel_input(
                    WheelContext {
                        config: &self.config,
                        trigger: *trigger,
                        recognizer,
                        matched_app: matched_app.as_deref(),
                        last_label,
                        used_hold_action,
                    },
                    GestureStep::WheelUp,
                    steps,
                ),
                MouseEvent::WheelDown(steps) => process_wheel_input(
                    WheelContext {
                        config: &self.config,
                        trigger: *trigger,
                        recognizer,
                        matched_app: matched_app.as_deref(),
                        last_label,
                        used_hold_action,
                    },
                    GestureStep::WheelDown,
                    steps,
                ),
                MouseEvent::ButtonDown(button) => {
                    recognizer.add_input_step(button.to_step());
                    let next_label =
                        resolve_label(&self.config, *trigger, recognizer, matched_app.as_deref());
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
                    let matched_action = sequence.as_deref().and_then(|sequence| {
                        self.config
                            .resolve_release(*trigger, sequence, matched_app.as_deref())
                            .map(|binding| binding.action.clone())
                    });
                    let transition = if let Some(action) = matched_action {
                        GestureTransition::FinishWithAction { action }
                    } else if !*used_hold_action
                        && should_replay_unmatched(
                            *travel_distance_px,
                            self.config.replay_distance_threshold_px,
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
            SessionState::Gesturing { entered_tick, .. } => {
                tick.wrapping_sub(*entered_tick) > self.config.safety_timeout_ms
            }
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
    matched_app: Option<&'a str>,
    last_label: &'a mut Option<String>,
    used_hold_action: &'a mut bool,
}

fn process_wheel_input(ctx: WheelContext<'_>, step: GestureStep, steps: u16) -> Decision {
    if steps == 0 {
        return Decision::pass(GestureTransition::Continue);
    }

    let current_sequence = ctx.recognizer.current_sequence().unwrap_or_default();
    if let Some(binding) =
        ctx.config
            .resolve_hold(ctx.trigger, &current_sequence, step, ctx.matched_app)
    {
        *ctx.used_hold_action = true;
        let mut decision = Decision {
            disposition: Disposition::Suppress,
            transition: GestureTransition::ContinueWithAction {
                action: binding.action.clone(),
                repeat: steps,
            },
            render: RenderEffects::default(),
        };
        update_label(
            &mut decision.render,
            ctx.last_label,
            Some(binding.label.clone()),
        );
        ctx.recognizer.reset_sequence();
        return decision;
    }

    for _ in 0..usize::from(steps) {
        ctx.recognizer.add_input_step(step);
    }
    let next_label = resolve_label(ctx.config, ctx.trigger, ctx.recognizer, ctx.matched_app);
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
    matched_app: Option<&str>,
) -> Option<String> {
    recognizer.current_sequence().and_then(|sequence| {
        config
            .resolve_release(trigger, &sequence, matched_app)
            .map(|binding| binding.label.clone())
    })
}

fn update_label(
    render: &mut RenderEffects,
    last_label: &mut Option<String>,
    next_label: Option<String>,
) {
    if *last_label != next_label {
        render.push(RenderEffect::UpdateLabel(next_label.clone()));
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

    fn key_action(key: &str) -> Action {
        Action::Keyboard {
            keys: vec![key.to_string()],
        }
    }

    fn release_binding(
        trigger: TriggerButton,
        sequence: Vec<GestureStep>,
        action: Action,
        label: &str,
    ) -> ReleaseBinding {
        ReleaseBinding {
            trigger,
            sequence,
            action,
            label: label.to_string(),
        }
    }

    fn hold_binding(
        trigger: TriggerButton,
        sequence: Vec<GestureStep>,
        step: GestureStep,
        action: Action,
        label: &str,
    ) -> HoldBinding {
        HoldBinding {
            trigger,
            sequence,
            step,
            action,
            label: label.to_string(),
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
            binding_sets: HashMap::from([(
                crate::config::DEFAULT_APP_ID.to_string(),
                AppBindingSet {
                    release_bindings,
                    hold_bindings,
                },
            )]),
        }
    }

    fn pointer(event: MouseEvent, x: i32, y: i32, tick: u32) -> GestureInput {
        GestureInput::Pointer {
            event,
            point: Point::new(x, y),
            tick,
            matched_app: None,
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

    fn assert_action(transition: GestureTransition, expected: &Action, repeat: u16) {
        match transition {
            GestureTransition::ContinueWithAction {
                action,
                repeat: actual,
            } => {
                assert_eq!(&action, expected);
                assert_eq!(actual, repeat);
            }
            GestureTransition::FinishWithAction { action } => {
                assert_eq!(&action, expected);
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
        let mut machine = GestureMachine::new(config);

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
        let mut machine = GestureMachine::new(config);

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
        let mut machine = GestureMachine::new(config);
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
            action.clone(),
            "Reload",
        )]);
        let mut machine = GestureMachine::new(config);
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
        assert_action(decision.transition, &action, 1);
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
        let mut machine = GestureMachine::new(config);
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
        let mut machine = GestureMachine::new(config);
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
        let mut machine = GestureMachine::new(config);
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
        let mut machine = GestureMachine::new(config);
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
        let mut machine = GestureMachine::new(config);
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
        let mut machine = GestureMachine::new(config);
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
            action.clone(),
            "PageUp",
        )]);
        let mut machine = GestureMachine::new(config);
        start(&mut machine, TriggerButton::Middle, Point::new(0, 0), 100);
        machine.handle(pointer(MouseEvent::WheelUp(1), 0, 0, 110));

        let decision = machine.handle(pointer(
            MouseEvent::ButtonUp(TriggerButton::Middle),
            0,
            0,
            120,
        ));

        assert_action(decision.transition, &action, 1);
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
                action.clone(),
                "PageUp",
            )],
        );
        let mut machine = GestureMachine::new(config);
        start(&mut machine, TriggerButton::Right, Point::new(0, 0), 100);

        let decision = machine.handle(pointer(MouseEvent::WheelUp(2), 0, 0, 110));

        assert_eq!(decision.disposition, Disposition::Suppress);
        assert_action(decision.transition, &action, 2);
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
        let mut machine = GestureMachine::new(config);
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
                action.clone(),
                "Right + WheelDown",
            )],
        );
        let mut machine = GestureMachine::new(config);
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 150, 100, 1010));

        let decision = machine.handle(pointer(MouseEvent::WheelDown(1), 150, 100, 1020));

        assert_action(decision.transition, &action, 1);
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
                    specific_action.clone(),
                    "Right WheelDown",
                ),
            ],
        );
        let mut machine = GestureMachine::new(config);
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 150, 100, 1010));

        let decision = machine.handle(pointer(MouseEvent::WheelDown(1), 150, 100, 1020));

        assert_action(decision.transition, &specific_action, 1);
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
                    wildcard_action.clone(),
                    "Any WheelUp",
                ),
                hold_binding(
                    TriggerButton::Right,
                    vec![GestureStep::Right],
                    GestureStep::WheelUp,
                    specific_action.clone(),
                    "Right WheelUp",
                ),
            ],
        );
        let mut machine = GestureMachine::new(config);
        start(
            &mut machine,
            TriggerButton::Right,
            Point::new(100, 100),
            1000,
        );
        machine.handle(pointer(MouseEvent::MouseMove, 150, 100, 1010));

        let first = machine.handle(pointer(MouseEvent::WheelUp(1), 150, 100, 1020));
        let second = machine.handle(pointer(MouseEvent::WheelUp(1), 150, 100, 1030));

        assert_action(first.transition, &specific_action, 1);
        assert_action(second.transition, &wildcard_action, 1);
    }

    #[test]
    fn non_trigger_button_up_does_not_end_gesture() {
        let config = test_config(vec![release_binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("x"),
            "X",
        )]);
        let mut machine = GestureMachine::new(config);
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
            default_action.clone(),
            "Default",
        )]);
        config.binding_sets.insert(
            "explorer".to_string(),
            AppBindingSet {
                release_bindings: vec![release_binding(
                    TriggerButton::Right,
                    vec![GestureStep::Right],
                    app_action.clone(),
                    "App",
                )],
                hold_bindings: Vec::new(),
            },
        );

        let app = config
            .resolve_release(
                TriggerButton::Right,
                &[GestureStep::Right],
                Some("explorer"),
            )
            .expect("app binding");
        assert_eq!(app.action, app_action);

        let fallback = config
            .resolve_release(TriggerButton::Right, &[GestureStep::Right], Some("unknown"))
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
                default_action.clone(),
                "Default",
            )],
        );
        config.binding_sets.insert(
            "explorer".to_string(),
            AppBindingSet {
                release_bindings: Vec::new(),
                hold_bindings: vec![hold_binding(
                    TriggerButton::Right,
                    Vec::new(),
                    GestureStep::WheelUp,
                    app_action.clone(),
                    "App",
                )],
            },
        );

        let app = config
            .resolve_hold(
                TriggerButton::Right,
                &[],
                GestureStep::WheelUp,
                Some("explorer"),
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
        let mut machine = GestureMachine::new(config);
        start(&mut machine, TriggerButton::Right, Point::new(0, 0), 100);

        let pending = machine.handle(pointer(MouseEvent::MouseMove, 7, 0, 110));
        let confirmed = machine.handle(pointer(MouseEvent::MouseMove, 8, 0, 120));

        assert_eq!(pending.render.len(), 1);
        assert!(matches!(
            confirmed.render.last(),
            Some(RenderEffect::UpdateLabel(Some(label))) if label == "Right"
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
        let mut machine = GestureMachine::new(config);
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
            Some(RenderEffect::UpdateLabel(Some(label))) if label == "Right Down"
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
        let mut machine = GestureMachine::new(config);
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
