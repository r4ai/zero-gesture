use std::collections::HashMap;

use log::{debug, info, trace};

use crate::executor::Action;
use crate::gesture::{GestureKind, GestureRecognizer};
use crate::overlay::OverlayCommand;

use super::app_match::AppBindingSet;
use super::trigger::TriggerButton;

// ---------------------------------------------------------------------------
// HookConfig
// ---------------------------------------------------------------------------

/// Snapshot of configuration relevant to the hook, taken once at startup.
///
/// By copying the needed values out of [`SharedConfig`](crate::SharedConfig) before entering the
/// hook thread, we avoid taking any locks inside the latency-critical hook
/// callback. Changes to the live config require restarting the hook thread.
#[derive(Debug, Clone)]
pub(super) struct HookConfig {
    pub(super) trigger: TriggerButton,
    pub(super) gesture_threshold: i32,
    pub(super) safety_timeout_ms: u32,
    pub(super) min_segment_px: i32,
    pub(super) direction_switch_confirm_px: i32,
    pub(super) axis_ambiguity_deadzone_px: i32,
    /// Compiled app definitions for per-app matching.
    pub(super) apps: Vec<super::app_match::CompiledApp>,
    /// Per-app bindings, keyed by app ID. Includes `"default"`.
    pub(super) binding_sets: HashMap<String, AppBindingSet>,
}

impl HookConfig {
    /// Look up the action for a gesture, checking app-specific bindings first,
    /// then falling back to `"default"`.
    pub(super) fn resolve_binding(
        &self,
        kind: &GestureKind,
        matched_app: Option<&str>,
    ) -> Option<&Action> {
        if let Some(app_id) = matched_app {
            if let Some(set) = self.binding_sets.get(app_id) {
                if let Some(action) = set.bindings.get(kind) {
                    return Some(action);
                }
            }
        }
        self.binding_sets
            .get("default")
            .and_then(|set| set.bindings.get(kind))
    }

    /// Look up the label for a gesture, checking app-specific labels first,
    /// then falling back to `"default"`.
    pub(super) fn resolve_label(
        &self,
        kind: &GestureKind,
        matched_app: Option<&str>,
    ) -> Option<&String> {
        if let Some(app_id) = matched_app {
            if let Some(set) = self.binding_sets.get(app_id) {
                if let Some(label) = set.labels.get(kind) {
                    return Some(label);
                }
            }
        }
        self.binding_sets
            .get("default")
            .and_then(|set| set.labels.get(kind))
    }
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// Three-state machine that drives gesture recognition.
///
/// See the [module-level documentation](super) for the full transition table
/// and ASCII diagram.
///
/// Each non-`Idle` variant stores an `entered_tick` timestamp (from
/// [`GetTickCount`]) so the safety timer can detect stuck states.
///
/// [`GetTickCount`]: windows_sys::Win32::System::SystemInformation::GetTickCount
pub(super) enum GestureState {
    /// Waiting for the trigger button press. This is the resting state.
    Idle,
    /// Trigger button is held; no significant movement yet.
    ///
    /// If the user releases the button without exceeding the configured
    /// gesture threshold,
    /// the click is replayed to the target application. If movement exceeds the
    /// threshold, we transition to [`Gesturing`](GestureState::Gesturing).
    ButtonDown {
        /// Screen X coordinate where the trigger button was pressed.
        origin_x: i32,
        /// Screen Y coordinate where the trigger button was pressed.
        origin_y: i32,
        /// [`GetTickCount`](windows_sys::Win32::System::SystemInformation::GetTickCount)
        /// value when we entered this state (for safety timeout).
        entered_tick: u32,
    },
    /// Actively gesturing — movement has exceeded the configured gesture
    /// threshold.
    ///
    /// Mouse move events are forwarded to the overlay as [`OverlayCommand::TrackPoint`].
    /// When the trigger button is released, [`OverlayCommand::EndGesture`] is
    /// sent and the state returns to [`Idle`](GestureState::Idle).
    Gesturing {
        /// [`GetTickCount`](windows_sys::Win32::System::SystemInformation::GetTickCount)
        /// value when we entered this state (for safety timeout).
        entered_tick: u32,
        /// Recognizer for converting mouse movement into gesture patterns.
        recognizer: GestureRecognizer,
        /// Last gesture recognized during this gesture session (for change detection).
        last_recognized: Option<GestureKind>,
        /// The matched app ID for this gesture session (for per-app bindings).
        matched_app: Option<String>,
    },
}

/// Abstract mouse event, decoupled from Win32 message constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MouseEvent {
    /// The configured trigger button was pressed.
    TriggerDown,
    /// The configured trigger button was released.
    TriggerUp,
    /// The mouse cursor moved.
    MouseMove,
    /// Any other mouse message (ignored by the state machine).
    Other,
}

/// Stack-allocated collection of up to `N` overlay commands.
///
/// Avoids heap allocation in the hot path of [`process_event_pure`].
/// The maximum number of commands produced per event is 3 (StartGesture +
/// two TrackPoints when transitioning to Gesturing).
pub(super) struct OverlayCommands<const N: usize> {
    buf: [std::mem::MaybeUninit<OverlayCommand>; N],
    len: usize,
}

impl<const N: usize> OverlayCommands<N> {
    pub(super) fn new() -> Self {
        Self {
            // SAFETY: An array of `MaybeUninit` does not require initialisation.
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
            // SAFETY: elements at indices 0..self.len are initialised.
            Some(unsafe { self.buf[self.len - 1].assume_init_ref() })
        }
    }
}

impl<const N: usize> std::ops::Index<usize> for OverlayCommands<N> {
    type Output = OverlayCommand;
    fn index(&self, idx: usize) -> &OverlayCommand {
        assert!(idx < self.len, "index out of bounds");
        // SAFETY: elements at indices 0..self.len are initialised.
        unsafe { self.buf[idx].assume_init_ref() }
    }
}

impl<const N: usize> Drop for OverlayCommands<N> {
    fn drop(&mut self) {
        for i in 0..self.len {
            // SAFETY: elements at indices 0..self.len are initialised.
            unsafe { self.buf[i].assume_init_drop() };
        }
    }
}

impl<const N: usize> IntoIterator for OverlayCommands<N> {
    type Item = OverlayCommand;
    type IntoIter = OverlayCommandsIntoIter<N>;
    fn into_iter(self) -> Self::IntoIter {
        let iter = OverlayCommandsIntoIter {
            // SAFETY: We transfer ownership without dropping `self`.
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
    fn next(&mut self) -> Option<OverlayCommand> {
        if self.pos >= self.len {
            None
        } else {
            let val = unsafe { self.buf[self.pos].assume_init_read() };
            self.pos += 1;
            Some(val)
        }
    }
}

impl<const N: usize> Drop for OverlayCommandsIntoIter<N> {
    fn drop(&mut self) {
        // Drop remaining un-consumed elements.
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
    /// If set, a click should be replayed at these screen coordinates.
    pub(super) request_replay: Option<(i32, i32)>,
    /// If set, the given action should be executed.
    pub(super) request_execute: Option<Action>,
}

/// Pure-logic core of the gesture state machine.
///
/// Evaluates the incoming [`MouseEvent`] and mouse coordinates against the
/// current [`GestureState`], returning an [`EventEffect`] that describes the
/// side effects to apply. The caller is responsible for actually performing
/// those side effects (sending overlay commands, replaying clicks, etc.).
///
/// # State transitions
///
/// See the [module-level documentation](super) for the full transition table.
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
            if event == MouseEvent::TriggerDown {
                debug!("Idle → ButtonDown at ({}, {})", pt.0, pt.1);
                *state = GestureState::ButtonDown {
                    origin_x: pt.0,
                    origin_y: pt.1,
                    entered_tick: tick,
                };
                effect.suppress = true;
            }
        }
        GestureState::ButtonDown {
            origin_x, origin_y, ..
        } => {
            let (ox, oy) = (*origin_x, *origin_y);
            if event == MouseEvent::MouseMove {
                if exceeds_gesture_threshold((ox, oy), pt, config.gesture_threshold) {
                    debug!("ButtonDown → Gesturing (app={:?})", matched_app);
                    effect.overlay_commands.push(OverlayCommand::StartGesture);
                    effect
                        .overlay_commands
                        .push(OverlayCommand::TrackPoint { x: ox, y: oy });
                    effect
                        .overlay_commands
                        .push(OverlayCommand::TrackPoint { x: pt.0, y: pt.1 });
                    let mut recognizer = GestureRecognizer::new(
                        config.min_segment_px,
                        config.direction_switch_confirm_px,
                        config.axis_ambiguity_deadzone_px,
                    );
                    recognizer.add_point(ox, oy);
                    recognizer.add_point(pt.0, pt.1);
                    let initial_gesture = recognizer.recognize();
                    let label = initial_gesture
                        .as_ref()
                        .and_then(|k| config.resolve_label(k, matched_app.as_deref()))
                        .cloned();
                    effect
                        .overlay_commands
                        .push(OverlayCommand::UpdateLabel(label));
                    *state = GestureState::Gesturing {
                        entered_tick: tick,
                        recognizer,
                        last_recognized: initial_gesture,
                        matched_app,
                    };
                }
                // never suppress mouse move
            } else if event == MouseEvent::TriggerUp {
                debug!("ButtonDown → Idle (replay click)");
                effect.request_replay = Some((ox, oy));
                effect.suppress = true;
                *state = GestureState::Idle;
            }
        }
        GestureState::Gesturing {
            recognizer,
            last_recognized,
            matched_app: gesture_app,
            ..
        } => {
            if event == MouseEvent::MouseMove {
                trace!("Gesturing → Gesturing at ({}, {})", pt.0, pt.1);
                recognizer.add_point(pt.0, pt.1);
                effect
                    .overlay_commands
                    .push(OverlayCommand::TrackPoint { x: pt.0, y: pt.1 });
                let current_gesture = recognizer.recognize();
                if current_gesture != *last_recognized {
                    let label = current_gesture
                        .as_ref()
                        .and_then(|k| config.resolve_label(k, gesture_app.as_deref()))
                        .cloned();
                    effect
                        .overlay_commands
                        .push(OverlayCommand::UpdateLabel(label));
                    *last_recognized = current_gesture;
                }
                // never suppress mouse move
            } else if event == MouseEvent::TriggerUp {
                debug!("Gesturing → Idle (end gesture)");
                recognizer.add_point(pt.0, pt.1);
                let gesture = recognizer.recognize();
                if let Some(kind) = gesture {
                    info!("Gesture recognized: {:?}", kind);
                    if let Some(action) = config.resolve_binding(&kind, gesture_app.as_deref()) {
                        debug!("Gesture {:?} matched binding: {:?}", kind, action);
                        effect.request_execute = Some(action.clone());
                    }
                }
                effect.overlay_commands.push(OverlayCommand::EndGesture);
                effect.suppress = true;
                *state = GestureState::Idle;
            }
        }
    }

    effect
}

/// Check whether the safety timer should reset the state machine.
///
/// Returns `true` if the state machine has been stuck in `ButtonDown` or
/// `Gesturing` for longer than `timeout_ms` (based on wrapping tick
/// arithmetic).
pub(super) fn check_safety_timeout(state: &GestureState, tick: u32, timeout_ms: u32) -> bool {
    match state {
        GestureState::Idle => false,
        GestureState::ButtonDown { entered_tick, .. }
        | GestureState::Gesturing { entered_tick, .. } => {
            tick.wrapping_sub(*entered_tick) > timeout_ms
        }
    }
}

/// Returns `true` when the cursor distance from `origin` exceeds the
/// configured gesture threshold.
pub(super) fn exceeds_gesture_threshold(
    origin: (i32, i32),
    pt: (i32, i32),
    threshold: i32,
) -> bool {
    let dx = i64::from(pt.0 - origin.0);
    let dy = i64::from(pt.1 - origin.1);
    let threshold = i64::from(threshold);
    dx * dx + dy * dy > threshold * threshold
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a default [`HookConfig`] for testing with a gesture threshold of 10px.
    fn test_config() -> HookConfig {
        HookConfig {
            trigger: TriggerButton::Right,
            gesture_threshold: 10,
            safety_timeout_ms: 2000,
            min_segment_px: 12,
            direction_switch_confirm_px: 8,
            axis_ambiguity_deadzone_px: 2,
            apps: Vec::new(),
            binding_sets: HashMap::new(),
        }
    }

    #[test]
    fn idle_to_button_down_on_trigger_down() {
        let mut state = GestureState::Idle;
        let config = test_config();

        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::TriggerDown,
            (100, 200),
            1000,
            None,
        );

        assert!(effect.suppress, "trigger down should be suppressed");
        assert!(effect.overlay_commands.is_empty());
        assert!(effect.request_replay.is_none());
        assert!(effect.request_execute.is_none());
        assert!(
            matches!(
                state,
                GestureState::ButtonDown {
                    origin_x: 100,
                    origin_y: 200,
                    entered_tick: 1000
                }
            ),
            "should transition to ButtonDown"
        );
    }

    #[test]
    fn button_down_to_idle_on_trigger_up_replays_click() {
        let mut state = GestureState::ButtonDown {
            origin_x: 100,
            origin_y: 200,
            entered_tick: 1000,
        };
        let config = test_config();

        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::TriggerUp,
            (101, 201),
            1050,
            None,
        );

        assert!(effect.suppress, "trigger up should be suppressed");
        assert_eq!(
            effect.request_replay,
            Some((100, 200)),
            "should request replay at origin"
        );
        assert!(effect.request_execute.is_none());
        assert!(matches!(state, GestureState::Idle));
    }

    #[test]
    fn button_down_to_gesturing_on_large_move() {
        let mut state = GestureState::ButtonDown {
            origin_x: 100,
            origin_y: 200,
            entered_tick: 1000,
        };
        let config = test_config();
        // Move 20px right — exceeds threshold of 10
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (120, 200),
            1010,
            None,
        );

        assert!(!effect.suppress, "mouse move is never suppressed");
        assert!(effect.request_replay.is_none());
        // Should have StartGesture + 2 TrackPoints + UpdateLabel
        assert_eq!(effect.overlay_commands.len(), 4);
        assert!(matches!(
            effect.overlay_commands[0],
            OverlayCommand::StartGesture
        ));
        assert!(matches!(
            effect.overlay_commands[1],
            OverlayCommand::TrackPoint { x: 100, y: 200 }
        ));
        assert!(matches!(
            effect.overlay_commands[2],
            OverlayCommand::TrackPoint { x: 120, y: 200 }
        ));
        assert!(matches!(
            effect.overlay_commands[3],
            OverlayCommand::UpdateLabel(_)
        ));
        assert!(matches!(state, GestureState::Gesturing { .. }));
    }

    #[test]
    fn button_down_stays_on_small_move() {
        let mut state = GestureState::ButtonDown {
            origin_x: 100,
            origin_y: 200,
            entered_tick: 1000,
        };
        let config = test_config();
        // Move 5px — below threshold of 10
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (105, 200),
            1010,
            None,
        );

        assert!(!effect.suppress);
        assert!(effect.overlay_commands.is_empty());
        assert!(matches!(state, GestureState::ButtonDown { .. }));
    }

    #[test]
    fn gesturing_to_idle_on_trigger_up_sends_end_gesture() {
        let mut state = GestureState::Gesturing {
            entered_tick: 1000,
            recognizer: GestureRecognizer::new(12, 8, 2),
            last_recognized: None,
            matched_app: None,
        };
        let config = test_config();

        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::TriggerUp,
            (200, 300),
            1100,
            None,
        );

        assert!(effect.suppress, "trigger up should be suppressed");
        assert!(effect.request_replay.is_none());
        // Last overlay command should be EndGesture
        assert!(!effect.overlay_commands.is_empty());
        assert!(matches!(
            effect.overlay_commands.last().unwrap(),
            OverlayCommand::EndGesture
        ));
        assert!(matches!(state, GestureState::Idle));
    }

    #[test]
    fn gesturing_tracks_mouse_move() {
        let mut state = GestureState::Gesturing {
            entered_tick: 1000,
            recognizer: GestureRecognizer::new(12, 8, 2),
            last_recognized: None,
            matched_app: None,
        };
        let config = test_config();

        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (150, 250),
            1050,
            None,
        );

        assert!(!effect.suppress, "mouse move is never suppressed");
        assert_eq!(effect.overlay_commands.len(), 1);
        assert!(matches!(
            effect.overlay_commands[0],
            OverlayCommand::TrackPoint { x: 150, y: 250 }
        ));
        assert!(matches!(state, GestureState::Gesturing { .. }));
    }

    #[test]
    fn mouse_move_never_suppressed_in_any_state() {
        let config = test_config();

        // Idle
        let mut state = GestureState::Idle;
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (50, 50),
            100,
            None,
        );
        assert!(!effect.suppress);

        // ButtonDown
        let mut state = GestureState::ButtonDown {
            origin_x: 100,
            origin_y: 200,
            entered_tick: 100,
        };
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (101, 200),
            110,
            None,
        );
        assert!(!effect.suppress);

        // Gesturing
        let mut state = GestureState::Gesturing {
            entered_tick: 100,
            recognizer: GestureRecognizer::new(12, 8, 2),
            last_recognized: None,
            matched_app: None,
        };
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::MouseMove,
            (150, 250),
            110,
            None,
        );
        assert!(!effect.suppress);
    }

    #[test]
    fn other_events_are_ignored() {
        let config = test_config();

        // Idle + Other
        let mut state = GestureState::Idle;
        let effect = process_event_pure(&mut state, &config, MouseEvent::Other, (0, 0), 100, None);
        assert!(!effect.suppress);
        assert!(effect.overlay_commands.is_empty());
        assert!(matches!(state, GestureState::Idle));

        // ButtonDown + Other
        let mut state = GestureState::ButtonDown {
            origin_x: 100,
            origin_y: 200,
            entered_tick: 100,
        };
        let effect = process_event_pure(&mut state, &config, MouseEvent::Other, (0, 0), 110, None);
        assert!(!effect.suppress);
        assert!(effect.overlay_commands.is_empty());
        assert!(matches!(state, GestureState::ButtonDown { .. }));
    }

    #[test]
    fn safety_timeout_idle_not_stuck() {
        let state = GestureState::Idle;
        assert!(!check_safety_timeout(&state, 5000, 2000));
    }

    #[test]
    fn safety_timeout_button_down_stuck() {
        let state = GestureState::ButtonDown {
            origin_x: 0,
            origin_y: 0,
            entered_tick: 1000,
        };
        // 3001ms elapsed > 2000ms timeout
        assert!(check_safety_timeout(&state, 4001, 2000));
    }

    #[test]
    fn safety_timeout_button_down_not_yet() {
        let state = GestureState::ButtonDown {
            origin_x: 0,
            origin_y: 0,
            entered_tick: 1000,
        };
        // 1500ms elapsed ≤ 2000ms timeout
        assert!(!check_safety_timeout(&state, 2500, 2000));
    }

    #[test]
    fn safety_timeout_gesturing_stuck() {
        let state = GestureState::Gesturing {
            entered_tick: 1000,
            recognizer: GestureRecognizer::new(12, 8, 2),
            last_recognized: None,
            matched_app: None,
        };
        assert!(check_safety_timeout(&state, 4000, 2000));
    }

    #[test]
    fn safety_timeout_wrapping_tick() {
        // Test wrapping arithmetic: entered_tick near u32::MAX, current tick wrapped around
        let state = GestureState::ButtonDown {
            origin_x: 0,
            origin_y: 0,
            entered_tick: u32::MAX - 500,
        };
        // Wrapped tick: 2500 - (MAX - 500) wrapping = 3001
        let current_tick = 2500;
        assert!(check_safety_timeout(&state, current_tick, 2000));
    }

    #[test]
    fn gesturing_with_binding_requests_execute() {
        let mut bindings = HashMap::new();
        let action = Action::Keyboard {
            keys: vec!["alt".to_string(), "left".to_string()],
        };
        bindings.insert(GestureKind::Left, action.clone());

        let mut labels = HashMap::new();
        labels.insert(GestureKind::Left, "Back".to_string());

        let mut binding_sets = HashMap::new();
        binding_sets.insert("default".to_string(), AppBindingSet { bindings, labels });

        let config = HookConfig {
            trigger: TriggerButton::Right,
            gesture_threshold: 10,
            safety_timeout_ms: 2000,
            min_segment_px: 12,
            direction_switch_confirm_px: 8,
            axis_ambiguity_deadzone_px: 2,
            apps: Vec::new(),
            binding_sets,
        };

        // Build a recognizer and feed it a clear leftward gesture
        let mut recognizer = GestureRecognizer::new(12, 8, 2);
        recognizer.add_point(500, 300);
        recognizer.add_point(400, 300);
        recognizer.add_point(300, 300);
        recognizer.add_point(200, 300);

        let mut state = GestureState::Gesturing {
            entered_tick: 1000,
            recognizer,
            last_recognized: Some(GestureKind::Left),
            matched_app: None,
        };

        // Trigger up at far-left point to finalize the gesture
        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::TriggerUp,
            (100, 300),
            1200,
            None,
        );

        assert!(effect.suppress);
        assert!(matches!(state, GestureState::Idle));
        assert_eq!(effect.request_execute, Some(action));
        assert!(matches!(
            effect.overlay_commands.last().unwrap(),
            OverlayCommand::EndGesture
        ));
    }

    // ── resolve_binding / resolve_label tests ────────────────────────────

    #[test]
    fn resolve_binding_app_specific_then_fallback() {
        let default_action = Action::Keyboard {
            keys: vec!["alt".to_string(), "left".to_string()],
        };
        let app_action = Action::Keyboard {
            keys: vec!["alt".to_string(), "up".to_string()],
        };

        let mut binding_sets = HashMap::new();
        binding_sets.insert(
            "default".to_string(),
            AppBindingSet {
                bindings: HashMap::from([(GestureKind::Left, default_action.clone())]),
                labels: HashMap::new(),
            },
        );
        binding_sets.insert(
            "explorer".to_string(),
            AppBindingSet {
                bindings: HashMap::from([(GestureKind::Left, app_action.clone())]),
                labels: HashMap::new(),
            },
        );

        let config = HookConfig {
            trigger: TriggerButton::Right,
            gesture_threshold: 10,
            safety_timeout_ms: 2000,
            min_segment_px: 12,
            direction_switch_confirm_px: 8,
            axis_ambiguity_deadzone_px: 2,
            apps: Vec::new(),
            binding_sets,
        };

        // App-specific binding found
        assert_eq!(
            config.resolve_binding(&GestureKind::Left, Some("explorer")),
            Some(&app_action)
        );

        // Fallback to default
        assert_eq!(
            config.resolve_binding(&GestureKind::Left, Some("unknown_app")),
            Some(&default_action)
        );

        // No matched app → default
        assert_eq!(
            config.resolve_binding(&GestureKind::Left, None),
            Some(&default_action)
        );

        // Gesture not in any set
        assert_eq!(
            config.resolve_binding(&GestureKind::Right, Some("explorer")),
            None
        );
    }

    #[test]
    fn resolve_label_app_specific_then_fallback() {
        let mut binding_sets = HashMap::new();
        binding_sets.insert(
            "default".to_string(),
            AppBindingSet {
                bindings: HashMap::new(),
                labels: HashMap::from([(GestureKind::Left, "Back".to_string())]),
            },
        );
        binding_sets.insert(
            "explorer".to_string(),
            AppBindingSet {
                bindings: HashMap::new(),
                labels: HashMap::from([(GestureKind::Left, "Up".to_string())]),
            },
        );

        let config = HookConfig {
            trigger: TriggerButton::Right,
            gesture_threshold: 10,
            safety_timeout_ms: 2000,
            min_segment_px: 12,
            direction_switch_confirm_px: 8,
            axis_ambiguity_deadzone_px: 2,
            apps: Vec::new(),
            binding_sets,
        };

        assert_eq!(
            config.resolve_label(&GestureKind::Left, Some("explorer")),
            Some(&"Up".to_string())
        );
        assert_eq!(
            config.resolve_label(&GestureKind::Left, None),
            Some(&"Back".to_string())
        );
    }

    #[test]
    fn process_event_pure_with_matched_app_uses_app_binding() {
        let default_action = Action::Keyboard {
            keys: vec!["alt".to_string(), "left".to_string()],
        };
        let app_action = Action::Keyboard {
            keys: vec!["alt".to_string(), "up".to_string()],
        };

        let mut binding_sets = HashMap::new();
        binding_sets.insert(
            "default".to_string(),
            AppBindingSet {
                bindings: HashMap::from([(GestureKind::Left, default_action.clone())]),
                labels: HashMap::from([(GestureKind::Left, "Back".to_string())]),
            },
        );
        binding_sets.insert(
            "explorer".to_string(),
            AppBindingSet {
                bindings: HashMap::from([(GestureKind::Left, app_action.clone())]),
                labels: HashMap::from([(GestureKind::Left, "Up".to_string())]),
            },
        );

        let config = HookConfig {
            trigger: TriggerButton::Right,
            gesture_threshold: 10,
            safety_timeout_ms: 2000,
            min_segment_px: 12,
            direction_switch_confirm_px: 8,
            axis_ambiguity_deadzone_px: 2,
            apps: Vec::new(),
            binding_sets,
        };

        // Build a clear leftward gesture recognizer
        let mut recognizer = GestureRecognizer::new(12, 8, 2);
        recognizer.add_point(500, 300);
        recognizer.add_point(400, 300);
        recognizer.add_point(300, 300);
        recognizer.add_point(200, 300);

        let mut state = GestureState::Gesturing {
            entered_tick: 1000,
            recognizer,
            last_recognized: Some(GestureKind::Left),
            matched_app: Some("explorer".to_string()),
        };

        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::TriggerUp,
            (100, 300),
            1200,
            None,
        );

        // Should use the explorer-specific binding
        assert_eq!(effect.request_execute, Some(app_action));
    }

    #[test]
    fn process_event_pure_with_matched_app_falls_back_to_default() {
        let default_action = Action::Keyboard {
            keys: vec!["alt".to_string(), "right".to_string()],
        };

        let mut binding_sets = HashMap::new();
        binding_sets.insert(
            "default".to_string(),
            AppBindingSet {
                bindings: HashMap::from([(GestureKind::Right, default_action.clone())]),
                labels: HashMap::from([(GestureKind::Right, "Forward".to_string())]),
            },
        );
        // explorer has no Right binding
        binding_sets.insert(
            "explorer".to_string(),
            AppBindingSet {
                bindings: HashMap::new(),
                labels: HashMap::new(),
            },
        );

        let config = HookConfig {
            trigger: TriggerButton::Right,
            gesture_threshold: 10,
            safety_timeout_ms: 2000,
            min_segment_px: 12,
            direction_switch_confirm_px: 8,
            axis_ambiguity_deadzone_px: 2,
            apps: Vec::new(),
            binding_sets,
        };

        // Build a clear rightward gesture recognizer
        let mut recognizer = GestureRecognizer::new(12, 8, 2);
        recognizer.add_point(100, 300);
        recognizer.add_point(200, 300);
        recognizer.add_point(300, 300);
        recognizer.add_point(400, 300);

        let mut state = GestureState::Gesturing {
            entered_tick: 1000,
            recognizer,
            last_recognized: Some(GestureKind::Right),
            matched_app: Some("explorer".to_string()),
        };

        let effect = process_event_pure(
            &mut state,
            &config,
            MouseEvent::TriggerUp,
            (500, 300),
            1200,
            None,
        );

        // Should fall back to default binding
        assert_eq!(effect.request_execute, Some(default_action));
    }

    // ── OverlayCommands unit tests ──────────────────────────────────────

    #[test]
    fn overlay_commands_new_is_empty() {
        let cmds: OverlayCommands<3> = OverlayCommands::new();
        assert!(cmds.is_empty());
        assert_eq!(cmds.len(), 0);
    }

    #[test]
    fn overlay_commands_push_and_len() {
        let mut cmds: OverlayCommands<3> = OverlayCommands::new();
        cmds.push(OverlayCommand::StartGesture);
        assert_eq!(cmds.len(), 1);
        assert!(!cmds.is_empty());

        cmds.push(OverlayCommand::TrackPoint { x: 10, y: 20 });
        cmds.push(OverlayCommand::EndGesture);
        assert_eq!(cmds.len(), 3);
    }

    #[test]
    #[should_panic(expected = "OverlayCommands overflow")]
    fn overlay_commands_push_overflow_panics() {
        let mut cmds: OverlayCommands<2> = OverlayCommands::new();
        cmds.push(OverlayCommand::StartGesture);
        cmds.push(OverlayCommand::EndGesture);
        cmds.push(OverlayCommand::StartGesture); // should panic
    }

    #[test]
    fn overlay_commands_index() {
        let mut cmds: OverlayCommands<3> = OverlayCommands::new();
        cmds.push(OverlayCommand::StartGesture);
        cmds.push(OverlayCommand::TrackPoint { x: 5, y: 15 });
        cmds.push(OverlayCommand::EndGesture);

        assert!(matches!(cmds[0], OverlayCommand::StartGesture));
        assert!(matches!(
            cmds[1],
            OverlayCommand::TrackPoint { x: 5, y: 15 }
        ));
        assert!(matches!(cmds[2], OverlayCommand::EndGesture));
    }

    #[test]
    #[should_panic(expected = "index out of bounds")]
    fn overlay_commands_index_out_of_bounds_panics() {
        let cmds: OverlayCommands<3> = OverlayCommands::new();
        let _ = &cmds[0];
    }

    #[test]
    fn overlay_commands_last() {
        let mut cmds: OverlayCommands<3> = OverlayCommands::new();
        assert!(cmds.last().is_none());

        cmds.push(OverlayCommand::StartGesture);
        assert!(matches!(cmds.last(), Some(OverlayCommand::StartGesture)));

        cmds.push(OverlayCommand::EndGesture);
        assert!(matches!(cmds.last(), Some(OverlayCommand::EndGesture)));
    }

    #[test]
    fn overlay_commands_into_iter() {
        let mut cmds: OverlayCommands<3> = OverlayCommands::new();
        cmds.push(OverlayCommand::StartGesture);
        cmds.push(OverlayCommand::TrackPoint { x: 1, y: 2 });
        cmds.push(OverlayCommand::EndGesture);

        let collected: Vec<_> = cmds.into_iter().collect();
        assert_eq!(collected.len(), 3);
        assert!(matches!(collected[0], OverlayCommand::StartGesture));
        assert!(matches!(
            collected[1],
            OverlayCommand::TrackPoint { x: 1, y: 2 }
        ));
        assert!(matches!(collected[2], OverlayCommand::EndGesture));
    }

    #[test]
    fn overlay_commands_into_iter_empty() {
        let cmds: OverlayCommands<3> = OverlayCommands::new();
        let collected: Vec<_> = cmds.into_iter().collect();
        assert!(collected.is_empty());
    }

    #[test]
    fn overlay_commands_into_iter_partial_consume() {
        let mut cmds: OverlayCommands<3> = OverlayCommands::new();
        cmds.push(OverlayCommand::StartGesture);
        cmds.push(OverlayCommand::TrackPoint { x: 0, y: 0 });
        cmds.push(OverlayCommand::EndGesture);

        let mut iter = cmds.into_iter();
        // Consume only the first element; dropping the iterator should
        // safely drop the remaining two.
        let first = iter.next().unwrap();
        assert!(matches!(first, OverlayCommand::StartGesture));
        drop(iter);
    }

    #[test]
    fn overlay_commands_drop_without_consume() {
        // Ensure dropping a non-empty OverlayCommands without iterating
        // does not leak or cause UB.
        let mut cmds: OverlayCommands<3> = OverlayCommands::new();
        cmds.push(OverlayCommand::StartGesture);
        cmds.push(OverlayCommand::TrackPoint { x: 42, y: 99 });
        drop(cmds);
    }
}
