use std::collections::HashMap;

use log::{debug, info, trace};

use crate::config::GestureStep;
use crate::executor::Action;
use crate::gesture::GestureRecognizer;
use crate::overlay::OverlayCommand;

use super::app_match::{AppBindingSet, CompiledGestureBinding, CompiledHoldBinding};
use super::trigger::TriggerButton;

/// Snapshot of configuration relevant to the hook, taken once at startup.
///
/// By copying the needed values out of [`SharedConfig`](crate::SharedConfig)
/// before entering the hook thread, we avoid taking locks in the hot path.
/// Updating the live config requires restarting the hook thread.
#[derive(Debug, Clone)]
pub(super) struct HookConfig {
    pub(super) safety_timeout_ms: u32,
    pub(super) min_segment_px: i32,
    pub(super) direction_switch_confirm_px: i32,
    pub(super) axis_ambiguity_deadzone_px: i32,
    pub(super) replay_distance_threshold_px: i32,
    pub(super) gesture_activation_mode: crate::config::GestureActivationMode,
    pub(super) max_gesture_steps: usize,
    /// Compiled app definitions for per-app matching.
    pub(super) apps: Vec<super::app_match::CompiledApp>,
    /// Per-app bindings, keyed by app ID. Includes `"default"`.
    pub(super) binding_sets: HashMap<String, AppBindingSet>,
}

impl HookConfig {
    /// Returns `true` when any binding (default or app-specific) can start with `trigger`.
    pub(super) fn has_any_binding_for_trigger(&self, trigger: TriggerButton) -> bool {
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

    /// Returns `true` when a binding exists for `trigger` in the matched app
    /// or in `"default"` fallback.
    pub(super) fn has_binding_for_trigger(
        &self,
        trigger: TriggerButton,
        matched_app: Option<&str>,
    ) -> bool {
        if let Some(app_id) = matched_app {
            if let Some(set) = self.binding_sets.get(app_id) {
                if set
                    .release_bindings
                    .iter()
                    .any(|binding| binding.trigger == trigger)
                    || set
                        .hold_bindings
                        .iter()
                        .any(|binding| binding.trigger == trigger)
                {
                    return true;
                }
            }
        }
        self.binding_sets.get("default").is_some_and(|set| {
            set.release_bindings
                .iter()
                .any(|binding| binding.trigger == trigger)
                || set
                    .hold_bindings
                    .iter()
                    .any(|binding| binding.trigger == trigger)
        })
    }

    /// Resolve exact gesture binding using app-specific set first, then `"default"`.
    pub(super) fn resolve_binding(
        &self,
        trigger: TriggerButton,
        sequence: &[GestureStep],
        matched_app: Option<&str>,
    ) -> Option<&CompiledGestureBinding> {
        if let Some(app_id) = matched_app {
            if let Some(set) = self.binding_sets.get(app_id) {
                if let Some(binding) = set
                    .release_bindings
                    .iter()
                    .find(|binding| binding.trigger == trigger && binding.sequence == sequence)
                {
                    return Some(binding);
                }
            }
        }

        self.binding_sets.get("default").and_then(|set| {
            set.release_bindings
                .iter()
                .find(|binding| binding.trigger == trigger && binding.sequence == sequence)
        })
    }

    /// Resolve exact hold binding using app-specific set first, then `"default"`.
    pub(super) fn resolve_hold_binding(
        &self,
        trigger: TriggerButton,
        sequence: &[GestureStep],
        step: GestureStep,
        matched_app: Option<&str>,
    ) -> Option<&CompiledHoldBinding> {
        if let Some(binding) = resolve_hold_from_set(
            matched_app.and_then(|app_id| self.binding_sets.get(app_id)),
            trigger,
            sequence,
            step,
        ) {
            return Some(binding);
        }

        resolve_hold_from_set(self.binding_sets.get("default"), trigger, sequence, step)
    }

    /// Resolve label for the current sequence, respecting app-specific precedence.
    pub(super) fn resolve_label(
        &self,
        trigger: TriggerButton,
        sequence: &[GestureStep],
        matched_app: Option<&str>,
    ) -> Option<&String> {
        self.resolve_binding(trigger, sequence, matched_app)
            .map(|binding| &binding.label)
    }
}

/// State machine that drives gesture capture.
///
/// Each non-`Idle` variant stores an `entered_tick` value from
/// [`GetTickCount`](windows_sys::Win32::System::SystemInformation::GetTickCount)
/// so the safety timer can detect stuck sessions.
pub(super) enum GestureState {
    /// Waiting for a trigger-button press.
    Idle,
    /// Actively capturing a gesture sequence.
    Gesturing {
        /// Tick when the gesture session started.
        entered_tick: u32,
        /// Trigger button that started this gesture session.
        trigger: TriggerButton,
        /// Movement/input recognizer accumulating the in-progress sequence.
        recognizer: GestureRecognizer,
        /// Cursor position where the trigger button was initially pressed.
        origin: (i32, i32),
        /// Last observed cursor position during this gesture session.
        last_point: (i32, i32),
        /// Total cursor travel distance during this gesture session, in pixels.
        travel_distance_px: f64,
        /// Last label sent to the overlay (used for change detection).
        last_label: Option<String>,
        /// Matched app ID for per-app bindings, if any.
        matched_app: Option<String>,
        /// Whether a hold wheel action fired in this session.
        used_hold_wheel_action: bool,
    },
}

/// Abstract mouse event, decoupled from Win32 message constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MouseEvent {
    /// Any mouse button down event.
    ButtonDown(TriggerButton),
    /// Any mouse button up event.
    ButtonUp(TriggerButton),
    /// Pointer movement.
    MouseMove,
    /// Mouse wheel delta > 0 with notch count.
    WheelUp(u16),
    /// Mouse wheel delta < 0 with notch count.
    WheelDown(u16),
    /// Any event ignored by the state machine.
    Other,
}

/// Stack-allocated collection of up to `N` overlay commands.
///
/// Avoids heap allocation in the hot path of [`process_event_pure`].
pub(super) struct OverlayCommands<const N: usize> {
    buf: [std::mem::MaybeUninit<OverlayCommand>; N],
    len: usize,
}

impl<const N: usize> OverlayCommands<N> {
    pub(super) fn new() -> Self {
        Self {
            // SAFETY: An array of `MaybeUninit` does not require initialization.
            buf: unsafe { std::mem::MaybeUninit::uninit().assume_init() },
            len: 0,
        }
    }

    pub(super) fn push(&mut self, cmd: OverlayCommand) {
        assert!(self.len < N, "OverlayCommands overflow");
        self.buf[self.len].write(cmd);
        self.len += 1;
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.len
    }

    #[cfg(test)]
    pub(super) fn last(&self) -> Option<&OverlayCommand> {
        if self.len == 0 {
            None
        } else {
            // SAFETY: elements at indices 0..self.len are initialized.
            Some(unsafe { self.buf[self.len - 1].assume_init_ref() })
        }
    }
}

impl<const N: usize> std::ops::Index<usize> for OverlayCommands<N> {
    type Output = OverlayCommand;

    fn index(&self, idx: usize) -> &OverlayCommand {
        assert!(idx < self.len, "index out of bounds");
        // SAFETY: elements at indices 0..self.len are initialized.
        unsafe { self.buf[idx].assume_init_ref() }
    }
}

impl<const N: usize> Drop for OverlayCommands<N> {
    fn drop(&mut self) {
        for i in 0..self.len {
            // SAFETY: elements at indices 0..self.len are initialized.
            unsafe { self.buf[i].assume_init_drop() };
        }
    }
}

impl<const N: usize> IntoIterator for OverlayCommands<N> {
    type Item = OverlayCommand;
    type IntoIter = OverlayCommandsIntoIter<N>;

    fn into_iter(self) -> Self::IntoIter {
        let iter = OverlayCommandsIntoIter {
            // SAFETY: transfer ownership without dropping `self`.
            buf: unsafe { std::ptr::read(&self.buf) },
            len: self.len,
            pos: 0,
        };
        std::mem::forget(self);
        iter
    }
}

pub(super) struct OverlayCommandsIntoIter<const N: usize> {
    buf: [std::mem::MaybeUninit<OverlayCommand>; N],
    len: usize,
    pos: usize,
}

impl<const N: usize> Iterator for OverlayCommandsIntoIter<N> {
    type Item = OverlayCommand;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.len {
            None
        } else {
            let value = unsafe { self.buf[self.pos].assume_init_read() };
            self.pos += 1;
            Some(value)
        }
    }
}

impl<const N: usize> Drop for OverlayCommandsIntoIter<N> {
    fn drop(&mut self) {
        for i in self.pos..self.len {
            unsafe { self.buf[i].assume_init_drop() };
        }
    }
}

/// Side effects produced by [`process_event_pure`], applied by the caller.
pub(super) struct EventEffect {
    /// Whether the event should be suppressed (swallowed by the hook).
    pub(super) suppress: bool,
    /// Overlay commands to send (stack-allocated, max 4).
    pub(super) overlay_commands: OverlayCommands<4>,
    /// If set, replay the original trigger-button operation.
    pub(super) request_replay: Option<ReplayRequest>,
    /// If set, the given action should be executed.
    pub(super) request_execute: Option<ExecuteRequest>,
}

/// Mouse operation to replay when gesture matching fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReplayRequest {
    /// Trigger button to replay.
    pub(super) trigger: TriggerButton,
    /// Cursor position where the trigger was initially pressed.
    pub(super) down_at: (i32, i32),
    /// Cursor position where the trigger was released.
    pub(super) up_at: (i32, i32),
}

/// Action execution request produced by the state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExecuteRequest {
    /// Action to execute.
    pub(super) action: Action,
    /// Number of times to execute the action.
    pub(super) repeat: u16,
}

/// Pure-logic core of the gesture state machine.
///
/// Evaluates an incoming [`MouseEvent`] and mouse position against the current
/// [`GestureState`], and returns an [`EventEffect`] describing side effects.
/// The caller applies those side effects (overlay updates, deferred execute).
///
/// Transitions:
/// - `Idle` -> `Gesturing` on `ButtonDown(trigger)` when that trigger has a
///   matching binding for the matched app (or `"default"` fallback).
/// - `Gesturing` -> `Idle` on `ButtonUp(trigger)` for the same trigger.
pub(super) fn process_event_pure(
    state: &mut GestureState,
    config: &HookConfig,
    event: MouseEvent,
    pt: (i32, i32),
    tick: u32,
    matched_app: Option<String>,
) -> EventEffect {
    let mut effect = EventEffect {
        suppress: false,
        overlay_commands: OverlayCommands::new(),
        request_replay: None,
        request_execute: None,
    };

    match state {
        GestureState::Idle => {
            if let MouseEvent::ButtonDown(trigger) = event {
                if config.has_binding_for_trigger(trigger, matched_app.as_deref()) {
                    debug!(
                        "Idle → Gesturing, trigger={:?}, app={:?}",
                        trigger, matched_app
                    );
                    let mut recognizer = GestureRecognizer::new(
                        config.min_segment_px,
                        config.direction_switch_confirm_px,
                        config.axis_ambiguity_deadzone_px,
                        config.max_gesture_steps,
                    );
                    recognizer.add_point(pt.0, pt.1);

                    effect.overlay_commands.push(OverlayCommand::StartGesture);
                    effect
                        .overlay_commands
                        .push(OverlayCommand::TrackPoint { x: pt.0, y: pt.1 });
                    effect.suppress = true;

                    *state = GestureState::Gesturing {
                        entered_tick: tick,
                        trigger,
                        recognizer,
                        origin: pt,
                        last_point: pt,
                        travel_distance_px: 0.0,
                        last_label: None,
                        matched_app,
                        used_hold_wheel_action: false,
                    };
                }
            }
        }
        GestureState::Gesturing {
            trigger,
            recognizer,
            origin,
            last_point,
            travel_distance_px,
            last_label,
            matched_app: gesture_app,
            used_hold_wheel_action,
            ..
        } => {
            match event {
                MouseEvent::MouseMove => {
                    trace!("Gesturing mouse move at ({}, {})", pt.0, pt.1);
                    *travel_distance_px += segment_distance(*last_point, pt);
                    *last_point = pt;
                    recognizer.add_point(pt.0, pt.1);
                    effect
                        .overlay_commands
                        .push(OverlayCommand::TrackPoint { x: pt.0, y: pt.1 });
                    update_label_if_needed(
                        &mut effect,
                        config,
                        *trigger,
                        recognizer,
                        gesture_app.as_deref(),
                        last_label,
                    );
                }
                MouseEvent::WheelUp(steps) => {
                    process_wheel_input(
                        WheelInputContext {
                            effect: &mut effect,
                            config,
                            trigger: *trigger,
                            recognizer,
                            matched_app: gesture_app.as_deref(),
                            last_label,
                            used_hold_wheel_action,
                        },
                        GestureStep::WheelUp,
                        steps,
                    );
                }
                MouseEvent::WheelDown(steps) => {
                    process_wheel_input(
                        WheelInputContext {
                            effect: &mut effect,
                            config,
                            trigger: *trigger,
                            recognizer,
                            matched_app: gesture_app.as_deref(),
                            last_label,
                            used_hold_wheel_action,
                        },
                        GestureStep::WheelDown,
                        steps,
                    );
                }
                MouseEvent::ButtonDown(button) => {
                    recognizer.add_input_step(button.to_step());
                    effect.suppress = true;
                    update_label_if_needed(
                        &mut effect,
                        config,
                        *trigger,
                        recognizer,
                        gesture_app.as_deref(),
                        last_label,
                    );
                }
                MouseEvent::ButtonUp(button) => {
                    // Suppress all button-up events during gesture capture.
                    effect.suppress = true;

                    if button == *trigger {
                        debug!("Gesturing → Idle (finalize)");
                        let mut matched = false;
                        if let Some(sequence) = recognizer.finalize_sequence() {
                            if let Some(binding) =
                                config.resolve_binding(*trigger, &sequence, gesture_app.as_deref())
                            {
                                info!("Gesture matched sequence: {:?}", sequence);
                                matched = true;
                                effect.request_execute = Some(ExecuteRequest {
                                    action: binding.action.clone(),
                                    repeat: 1,
                                });
                            }
                        }
                        if !matched
                            && !*used_hold_wheel_action
                            && should_replay_unmatched(*travel_distance_px, config)
                        {
                            effect.request_replay = Some(ReplayRequest {
                                trigger: *trigger,
                                down_at: *origin,
                                up_at: pt,
                            });
                        }
                        effect.overlay_commands.push(OverlayCommand::EndGesture);
                        *state = GestureState::Idle;
                    }
                }
                MouseEvent::Other => {}
            }
        }
    }

    effect
}

fn should_replay_unmatched(travel_distance_px: f64, config: &HookConfig) -> bool {
    let threshold = config.replay_distance_threshold_px;
    travel_distance_px <= f64::from(threshold)
}

fn segment_distance(a: (i32, i32), b: (i32, i32)) -> f64 {
    let dx = f64::from(b.0 - a.0);
    let dy = f64::from(b.1 - a.1);
    dx.hypot(dy)
}

struct WheelInputContext<'a> {
    effect: &'a mut EventEffect,
    config: &'a HookConfig,
    trigger: TriggerButton,
    recognizer: &'a mut GestureRecognizer,
    matched_app: Option<&'a str>,
    last_label: &'a mut Option<String>,
    used_hold_wheel_action: &'a mut bool,
}

fn process_wheel_input(ctx: WheelInputContext<'_>, step: GestureStep, steps: u16) {
    if steps == 0 {
        return;
    }

    let current_sequence = ctx.recognizer.current_sequence().unwrap_or_default();
    if let Some(binding) =
        ctx.config
            .resolve_hold_binding(ctx.trigger, &current_sequence, step, ctx.matched_app)
    {
        ctx.effect.suppress = true;
        *ctx.used_hold_wheel_action = true;
        ctx.effect.request_execute = Some(ExecuteRequest {
            action: binding.action.clone(),
            repeat: steps,
        });
        update_label_direct(ctx.effect, ctx.last_label, Some(binding.label.clone()));
        ctx.recognizer.reset_sequence();
        return;
    }

    for _ in 0..usize::from(steps) {
        ctx.recognizer.add_input_step(step);
    }
    ctx.effect.suppress = true;
    update_label_if_needed(
        ctx.effect,
        ctx.config,
        ctx.trigger,
        ctx.recognizer,
        ctx.matched_app,
        ctx.last_label,
    );
}

fn resolve_hold_from_set<'a>(
    set: Option<&'a AppBindingSet>,
    trigger: TriggerButton,
    sequence: &[GestureStep],
    step: GestureStep,
) -> Option<&'a CompiledHoldBinding> {
    let set = set?;
    if let Some(exact) = set.hold_bindings.iter().find(|binding| {
        binding.trigger == trigger && binding.step == step && binding.sequence == sequence
    }) {
        return Some(exact);
    }

    set.hold_bindings.iter().find(|binding| {
        binding.trigger == trigger && binding.step == step && binding.sequence.is_empty()
    })
}

fn update_label_if_needed(
    effect: &mut EventEffect,
    config: &HookConfig,
    trigger: TriggerButton,
    recognizer: &GestureRecognizer,
    matched_app: Option<&str>,
    last_label: &mut Option<String>,
) {
    let next_label = recognizer.current_sequence().and_then(|sequence| {
        config
            .resolve_label(trigger, &sequence, matched_app)
            .cloned()
    });

    if *last_label != next_label {
        effect
            .overlay_commands
            .push(OverlayCommand::UpdateLabel(next_label.clone()));
        *last_label = next_label;
    }
}

fn update_label_direct(
    effect: &mut EventEffect,
    last_label: &mut Option<String>,
    next_label: Option<String>,
) {
    if *last_label != next_label {
        effect
            .overlay_commands
            .push(OverlayCommand::UpdateLabel(next_label.clone()));
        *last_label = next_label;
    }
}

/// Check whether the safety timer should reset the state machine.
///
/// Returns `true` when elapsed time since entering `Gesturing` exceeds
/// `timeout_ms`, using wrapping tick arithmetic.
pub(super) fn check_safety_timeout(state: &GestureState, tick: u32, timeout_ms: u32) -> bool {
    match state {
        GestureState::Idle => false,
        GestureState::Gesturing { entered_tick, .. } => {
            tick.wrapping_sub(*entered_tick) > timeout_ms
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::Action;
    use crate::hook::app_match::{CompiledApp, CompiledHoldBinding};

    fn key_action(key: &str) -> Action {
        Action::Keyboard {
            keys: vec![key.to_string()],
        }
    }

    fn binding(
        trigger: TriggerButton,
        sequence: Vec<GestureStep>,
        action: Action,
        label: &str,
    ) -> CompiledGestureBinding {
        CompiledGestureBinding {
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
    ) -> CompiledHoldBinding {
        CompiledHoldBinding {
            trigger,
            sequence,
            step,
            action,
            label: label.to_string(),
        }
    }

    fn test_config(default_bindings: Vec<CompiledGestureBinding>) -> HookConfig {
        test_config_with_hold(default_bindings, Vec::new())
    }

    fn test_config_with_hold(
        default_bindings: Vec<CompiledGestureBinding>,
        hold_bindings: Vec<CompiledHoldBinding>,
    ) -> HookConfig {
        HookConfig {
            safety_timeout_ms: 2000,
            min_segment_px: 1,
            direction_switch_confirm_px: 1,
            axis_ambiguity_deadzone_px: 0,
            replay_distance_threshold_px: 8,
            gesture_activation_mode: crate::config::GestureActivationMode::Element,
            max_gesture_steps: 8,
            apps: Vec::<CompiledApp>::new(),
            binding_sets: HashMap::from([(
                "default".to_string(),
                AppBindingSet {
                    release_bindings: default_bindings,
                    hold_bindings,
                },
            )]),
        }
    }

    #[test]
    fn idle_starts_gesture_on_configured_trigger() {
        let config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "A",
        )]);
        let mut state = GestureState::Idle;

        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 200),
            1000,
            None,
        );

        assert!(effect.suppress);
        assert_eq!(effect.overlay_commands.len(), 2);
        assert!(matches!(
            effect.overlay_commands[0],
            OverlayCommand::StartGesture
        ));
        assert!(matches!(state, GestureState::Gesturing { .. }));
    }

    #[test]
    fn idle_ignores_unconfigured_trigger() {
        let config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "A",
        )]);
        let mut state = GestureState::Idle;

        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Left),
            (100, 200),
            1000,
            None,
        );

        assert!(!effect.suppress);
        assert!(effect.overlay_commands.is_empty());
        assert!(matches!(state, GestureState::Idle));
    }

    #[test]
    fn executes_action_on_trigger_up_when_sequence_matches() {
        let action = Action::Keyboard {
            keys: vec!["ctrl".to_string(), "r".to_string()],
        };
        let config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Right, GestureStep::Down],
            action.clone(),
            "Reload",
        )]);
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (150, 100),
            1010,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (150, 150),
            1020,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Right),
            (150, 150),
            1030,
            None,
        );

        assert!(effect.suppress);
        assert!(effect.request_replay.is_none());
        assert_eq!(
            effect.request_execute,
            Some(ExecuteRequest { action, repeat: 1 })
        );
        assert!(matches!(state, GestureState::Idle));
        assert!(matches!(
            effect.overlay_commands.last(),
            Some(OverlayCommand::EndGesture)
        ));
    }

    #[test]
    fn trigger_click_without_matching_sequence_requests_replay() {
        let config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "A",
        )]);
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Right),
            (100, 100),
            1010,
            None,
        );

        assert!(effect.suppress);
        assert!(effect.request_execute.is_none());
        assert_eq!(
            effect.request_replay,
            Some(ReplayRequest {
                trigger: TriggerButton::Right,
                down_at: (100, 100),
                up_at: (100, 100),
            })
        );
    }

    #[test]
    fn unmatched_sequence_with_short_move_requests_replay() {
        let config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (105, 103),
            1010,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Right),
            (106, 104),
            1020,
            None,
        );

        assert!(effect.suppress);
        assert!(effect.request_execute.is_none());
        assert_eq!(
            effect.request_replay,
            Some(ReplayRequest {
                trigger: TriggerButton::Right,
                down_at: (100, 100),
                up_at: (106, 104),
            })
        );
    }

    #[test]
    fn unmatched_sequence_with_long_move_does_not_request_replay() {
        let config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (150, 100),
            1010,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Right),
            (160, 120),
            1020,
            None,
        );

        assert!(effect.suppress);
        assert!(effect.request_execute.is_none());
        assert!(effect.request_replay.is_none());
    }

    #[test]
    fn unmatched_sequence_with_small_displacement_but_large_travel_does_not_request_replay() {
        let config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (160, 100),
            1010,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (100, 100),
            1020,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Right),
            (102, 100),
            1030,
            None,
        );

        assert!(effect.suppress);
        assert!(effect.request_execute.is_none());
        assert!(effect.request_replay.is_none());
    }

    #[test]
    fn unmatched_sequence_replay_threshold_is_configurable() {
        let mut config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        config.replay_distance_threshold_px = 32;
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (119, 100),
            1010,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Right),
            (120, 100),
            1020,
            None,
        );

        assert!(effect.suppress);
        assert!(effect.request_execute.is_none());
        assert_eq!(
            effect.request_replay,
            Some(ReplayRequest {
                trigger: TriggerButton::Right,
                down_at: (100, 100),
                up_at: (120, 100),
            })
        );
    }

    #[test]
    fn replay_threshold_is_not_coupled_to_recognition_thresholds() {
        let mut config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Down],
            key_action("a"),
            "A",
        )]);
        config.min_segment_px = 30;
        config.direction_switch_confirm_px = 24;
        config.replay_distance_threshold_px = 6;
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (107, 100),
            1005,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Right),
            (107, 100),
            1010,
            None,
        );

        assert!(effect.suppress);
        assert!(effect.request_execute.is_none());
        assert!(effect.request_replay.is_none());
    }

    #[test]
    fn supports_wheel_input_in_sequence() {
        let action = key_action("pageup");
        let config = test_config(vec![binding(
            TriggerButton::Middle,
            vec![GestureStep::WheelUp],
            action.clone(),
            "PageUp",
        )]);
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Middle),
            (0, 0),
            100,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::WheelUp(1),
            (0, 0),
            110,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Middle),
            (0, 0),
            120,
            None,
        );

        assert_eq!(
            effect.request_execute,
            Some(ExecuteRequest { action, repeat: 1 })
        );
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
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (0, 0),
            100,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::WheelUp(2),
            (0, 0),
            110,
            None,
        );

        assert!(effect.suppress);
        assert_eq!(
            effect.request_execute,
            Some(ExecuteRequest { action, repeat: 2 })
        );
    }

    #[test]
    fn hold_wheel_usage_disables_unmatched_trigger_replay() {
        let config = test_config_with_hold(
            Vec::new(),
            vec![hold_binding(
                TriggerButton::Right,
                Vec::new(),
                GestureStep::WheelDown,
                key_action("pagedown"),
                "PageDown",
            )],
        );
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::WheelDown(1),
            (100, 100),
            1010,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Right),
            (100, 100),
            1020,
            None,
        );

        assert!(effect.suppress);
        assert!(effect.request_execute.is_none());
        assert!(effect.request_replay.is_none());
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
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (150, 100),
            1010,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::WheelDown(1),
            (150, 100),
            1020,
            None,
        );

        assert!(effect.suppress);
        assert_eq!(
            effect.request_execute,
            Some(ExecuteRequest { action, repeat: 1 })
        );
    }

    #[test]
    fn hold_wheel_specific_sequence_overrides_wildcard_binding() {
        let wildcard_action = key_action("a");
        let specific_action = key_action("b");
        let config = test_config_with_hold(
            Vec::new(),
            vec![
                hold_binding(
                    TriggerButton::Right,
                    Vec::new(),
                    GestureStep::WheelDown,
                    wildcard_action,
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
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (150, 100),
            1010,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::WheelDown(1),
            (150, 100),
            1020,
            None,
        );

        assert!(effect.suppress);
        assert_eq!(
            effect.request_execute,
            Some(ExecuteRequest {
                action: specific_action,
                repeat: 1
            })
        );
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
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (100, 100),
            1000,
            None,
        );
        process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (150, 100),
            1010,
            None,
        );
        let first = process_event_pure(
            &mut state,
            &config,
            MouseEvent::WheelUp(1),
            (150, 100),
            1020,
            None,
        );
        let second = process_event_pure(
            &mut state,
            &config,
            MouseEvent::WheelUp(1),
            (150, 100),
            1030,
            None,
        );

        assert_eq!(
            first.request_execute,
            Some(ExecuteRequest {
                action: specific_action,
                repeat: 1
            })
        );
        assert_eq!(
            second.request_execute,
            Some(ExecuteRequest {
                action: wildcard_action,
                repeat: 1
            })
        );
    }

    #[test]
    fn non_trigger_button_up_does_not_end_gesture() {
        let config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("x"),
            "X",
        )]);
        let mut state = GestureState::Idle;

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (0, 0),
            100,
            None,
        );
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Left),
            (0, 0),
            110,
            None,
        );

        assert!(effect.suppress);
        assert!(matches!(state, GestureState::Gesturing { .. }));
    }

    #[test]
    fn resolve_binding_prefers_app_specific_then_fallback() {
        let default_action = key_action("a");
        let app_action = key_action("b");
        let config = HookConfig {
            safety_timeout_ms: 2000,
            min_segment_px: 1,
            direction_switch_confirm_px: 1,
            axis_ambiguity_deadzone_px: 0,
            replay_distance_threshold_px: 8,
            gesture_activation_mode: crate::config::GestureActivationMode::Element,
            max_gesture_steps: 8,
            apps: Vec::<CompiledApp>::new(),
            binding_sets: HashMap::from([
                (
                    "default".to_string(),
                    AppBindingSet {
                        release_bindings: vec![binding(
                            TriggerButton::Right,
                            vec![GestureStep::Right],
                            default_action.clone(),
                            "Default",
                        )],
                        hold_bindings: Vec::new(),
                    },
                ),
                (
                    "explorer".to_string(),
                    AppBindingSet {
                        release_bindings: vec![binding(
                            TriggerButton::Right,
                            vec![GestureStep::Right],
                            app_action.clone(),
                            "App",
                        )],
                        hold_bindings: Vec::new(),
                    },
                ),
            ]),
        };

        let app_binding = config
            .resolve_binding(
                TriggerButton::Right,
                &[GestureStep::Right],
                Some("explorer"),
            )
            .expect("app binding");
        assert_eq!(app_binding.action, app_action);

        let fallback = config
            .resolve_binding(TriggerButton::Right, &[GestureStep::Right], Some("unknown"))
            .expect("default fallback");
        assert_eq!(fallback.action, default_action);
    }

    #[test]
    fn safety_timeout_works_with_wrapping_ticks() {
        let mut state = GestureState::Idle;
        let config = test_config(vec![binding(
            TriggerButton::Right,
            vec![GestureStep::Right],
            key_action("a"),
            "A",
        )]);

        process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonDown(TriggerButton::Right),
            (0, 0),
            u32::MAX - 500,
            None,
        );

        assert!(check_safety_timeout(&state, 2500, 2000));
    }
}
