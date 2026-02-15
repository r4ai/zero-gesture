use std::collections::HashMap;

use log::{debug, info, trace};

use crate::config::GestureStep;
use crate::executor::Action;
use crate::gesture::GestureRecognizer;
use crate::overlay::OverlayCommand;

use super::app_match::{AppBindingSet, CompiledGestureBinding};
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
            set.bindings
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
                    .bindings
                    .iter()
                    .any(|binding| binding.trigger == trigger)
                {
                    return true;
                }
            }
        }
        self.binding_sets.get("default").is_some_and(|set| {
            set.bindings
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
                    .bindings
                    .iter()
                    .find(|binding| binding.trigger == trigger && binding.sequence == sequence)
                {
                    return Some(binding);
                }
            }
        }

        self.binding_sets.get("default").and_then(|set| {
            set.bindings
                .iter()
                .find(|binding| binding.trigger == trigger && binding.sequence == sequence)
        })
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
        /// Last label sent to the overlay (used for change detection).
        last_label: Option<String>,
        /// Matched app ID for per-app bindings, if any.
        matched_app: Option<String>,
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
    /// Mouse wheel delta > 0.
    WheelUp,
    /// Mouse wheel delta < 0.
    WheelDown,
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
    /// If set, the given action should be executed.
    pub(super) request_execute: Option<Action>,
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
                        last_label: None,
                        matched_app,
                    };
                }
            }
        }
        GestureState::Gesturing {
            trigger,
            recognizer,
            last_label,
            matched_app: gesture_app,
            ..
        } => match event {
            MouseEvent::MouseMove => {
                trace!("Gesturing mouse move at ({}, {})", pt.0, pt.1);
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
            MouseEvent::WheelUp => {
                recognizer.add_input_step(GestureStep::WheelUp);
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
            MouseEvent::WheelDown => {
                recognizer.add_input_step(GestureStep::WheelDown);
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
                    if let Some(sequence) = recognizer.finalize_sequence() {
                        if let Some(binding) =
                            config.resolve_binding(*trigger, &sequence, gesture_app.as_deref())
                        {
                            info!("Gesture matched sequence: {:?}", sequence);
                            effect.request_execute = Some(binding.action.clone());
                        }
                    }
                    effect.overlay_commands.push(OverlayCommand::EndGesture);
                    *state = GestureState::Idle;
                }
            }
            MouseEvent::Other => {}
        },
    }

    effect
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
    use crate::hook::app_match::CompiledApp;

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

    fn test_config(default_bindings: Vec<CompiledGestureBinding>) -> HookConfig {
        HookConfig {
            safety_timeout_ms: 2000,
            min_segment_px: 1,
            direction_switch_confirm_px: 1,
            axis_ambiguity_deadzone_px: 0,
            max_gesture_steps: 8,
            apps: Vec::<CompiledApp>::new(),
            binding_sets: HashMap::from([(
                "default".to_string(),
                AppBindingSet {
                    bindings: default_bindings,
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
        assert_eq!(effect.request_execute, Some(action));
        assert!(matches!(state, GestureState::Idle));
        assert!(matches!(
            effect.overlay_commands.last(),
            Some(OverlayCommand::EndGesture)
        ));
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
        process_event_pure(&mut state, &config, MouseEvent::WheelUp, (0, 0), 110, None);
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::ButtonUp(TriggerButton::Middle),
            (0, 0),
            120,
            None,
        );

        assert_eq!(effect.request_execute, Some(action));
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
            max_gesture_steps: 8,
            apps: Vec::<CompiledApp>::new(),
            binding_sets: HashMap::from([
                (
                    "default".to_string(),
                    AppBindingSet {
                        bindings: vec![binding(
                            TriggerButton::Right,
                            vec![GestureStep::Right],
                            default_action.clone(),
                            "Default",
                        )],
                    },
                ),
                (
                    "explorer".to_string(),
                    AppBindingSet {
                        bindings: vec![binding(
                            TriggerButton::Right,
                            vec![GestureStep::Right],
                            app_action.clone(),
                            "App",
                        )],
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
