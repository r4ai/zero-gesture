//! Low-level mouse hook thread.
//!
//! Installs a [`WH_MOUSE_LL`] hook and runs a Win32 message loop on a
//! dedicated thread.
//!
//! The hook starts a gesture immediately when a configured trigger button is
//! pressed, captures directional movement plus mouse-input steps, and executes
//! the bound action when the trigger button is released.
//!
//! # Architecture
//!
//! This module is the **Sensor** layer described in `docs/architecture.md`.
//! It runs on a dedicated OS thread so the low-level callback never blocks the
//! Tauri main thread or the overlay renderer.
//!
//! ## State Machine
//!
//! The runtime state machine has two states (`Idle`, `Gesturing`) with all
//! transitions defined by [`state::process_event_pure`] plus the safety timer
//! handler in `win32`.
//!
//! ```text
//! Initial: Idle
//!
//! Idle
//!   ├─ ButtonDown(trigger) and binding exists for (matched app or default)
//!   │    -> Gesturing
//!   │       side effects: suppress=true, StartGesture, TrackPoint(origin)
//!   └─ any other event
//!        -> Idle (no transition)
//!
//! Gesturing
//!   ├─ MouseMove
//!   │    -> Gesturing
//!   │       side effects: TrackPoint, optional UpdateLabel, suppress=false
//!   ├─ WheelUp / WheelDown
//!   │    -> Gesturing
//!   │       side effects: add input step, optional UpdateLabel, suppress=true
//!   ├─ ButtonDown(any button)
//!   │    -> Gesturing
//!   │       side effects: add input step, optional UpdateLabel, suppress=true
//!   ├─ ButtonUp(non-trigger button)
//!   │    -> Gesturing
//!   │       side effects: suppress=true
//!   ├─ ButtonUp(same trigger that started gesture)
//!   │    -> Idle
//!   │       side effects: suppress=true, EndGesture,
//!   │       - if sequence matched: request_execute
//!   │       - else if unmatched and release distance <= replay threshold:
//!   │         request_replay
//!   ├─ Other
//!   │    -> Gesturing (no transition, suppress=false)
//!   └─ safety timeout elapsed (`WM_TIMER`)
//!        -> Idle
//!           side effects: EndGesture (cleanup)
//! ```
//!
//! During `Gesturing`, directional movement and input steps are accumulated
//! into one sequence and resolved against app-specific bindings first, then
//! `"default"` fallback bindings.
//!
//! **Key invariant:** `WM_MOUSEMOVE` is never suppressed, so pointer tracking
//! stays natural while a gesture session is active.

mod app_match;
mod state;
mod trigger;
#[cfg(windows)]
mod win32;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::Sender;
use log::{info, warn};

use crate::config::AppConfig;
use crate::executor::generate_label;
use crate::overlay::OverlayCommand;
use crate::SharedConfig;

use app_match::{
    compile_matcher, AppBindingSet, CompiledApp, CompiledGestureBinding, CompiledMatcher,
};
use state::HookConfig;
use trigger::TriggerButton;

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

/// Messages sent from the main thread to the hook thread.
pub enum HookControl {
    /// Request the hook thread to stop and exit.
    Shutdown,
}

// ---------------------------------------------------------------------------
// Config resolution helpers
// ---------------------------------------------------------------------------

/// Resolves `min_segment_px` from config with a safe fallback.
///
/// Values less than or equal to zero are invalid and replaced by
/// [`AppConfig::DEFAULT_MIN_SEGMENT_PX`].
fn resolve_min_segment_px(value: i32) -> i32 {
    if value > 0 {
        value
    } else {
        warn!(
            "Invalid min_segment_px={} in config, falling back to {}",
            value,
            AppConfig::DEFAULT_MIN_SEGMENT_PX
        );
        AppConfig::DEFAULT_MIN_SEGMENT_PX
    }
}

/// Resolves `direction_switch_confirm_px` from config with a safe fallback.
///
/// Values less than or equal to zero are invalid and replaced by
/// [`AppConfig::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX`].
fn resolve_direction_switch_confirm_px(value: i32) -> i32 {
    if value > 0 {
        value
    } else {
        warn!(
            "Invalid direction_switch_confirm_px={} in config, falling back to {}",
            value,
            AppConfig::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX
        );
        AppConfig::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX
    }
}

/// Resolves `axis_ambiguity_deadzone_px` from config with a safe fallback.
///
/// Negative values are invalid and replaced by
/// [`AppConfig::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX`].
fn resolve_axis_ambiguity_deadzone_px(value: i32) -> i32 {
    if value >= 0 {
        value
    } else {
        warn!(
            "Invalid axis_ambiguity_deadzone_px={} in config, falling back to {}",
            value,
            AppConfig::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX
        );
        AppConfig::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX
    }
}

/// Resolves `replay_distance_threshold_px` from config with a safe fallback.
///
/// Values less than or equal to zero are invalid and replaced by
/// [`AppConfig::DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX`].
fn resolve_replay_distance_threshold_px(value: i32) -> i32 {
    if value > 0 {
        value
    } else {
        warn!(
            "Invalid replay_distance_threshold_px={} in config, falling back to {}",
            value,
            AppConfig::DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX
        );
        AppConfig::DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX
    }
}

/// Resolves `safety_timeout_ms` from config with a safe fallback.
///
/// A value of zero is invalid and replaced by
/// [`AppConfig::DEFAULT_SAFETY_TIMEOUT_MS`].
fn resolve_safety_timeout_ms(value: u32) -> u32 {
    if value > 0 {
        value
    } else {
        warn!(
            "Invalid safety_timeout_ms={} in config, falling back to {}",
            value,
            AppConfig::DEFAULT_SAFETY_TIMEOUT_MS
        );
        AppConfig::DEFAULT_SAFETY_TIMEOUT_MS
    }
}

/// Compile and validate one app's raw gesture bindings.
///
/// Validation rules:
/// - sequence must not be empty
/// - sequence length must be `<= AppConfig::MAX_GESTURE_STEPS`
/// - sequence must not include the trigger click step itself
/// - sequence must not contain consecutive identical directional move steps
/// - duplicate `(trigger, sequence)` entries are ignored after the first
fn has_consecutive_same_move_steps(sequence: &[crate::config::GestureStep]) -> bool {
    sequence.windows(2).any(|pair| {
        pair[0] == pair[1]
            && matches!(
                pair[0],
                crate::config::GestureStep::Up
                    | crate::config::GestureStep::Down
                    | crate::config::GestureStep::Left
                    | crate::config::GestureStep::Right
            )
    })
}

fn compile_bindings_for_app(
    app_id: &str,
    app_bindings: &[crate::config::GestureBinding],
) -> Vec<CompiledGestureBinding> {
    let mut bindings: Vec<CompiledGestureBinding> = Vec::new();
    let mut seen: HashSet<(TriggerButton, Vec<crate::config::GestureStep>)> = HashSet::new();

    for binding in app_bindings {
        if binding.gesture.sequence.is_empty() {
            warn!(
                "Empty gesture sequence in bindings for app {:?}, skipping",
                app_id
            );
            continue;
        }
        if binding.gesture.sequence.len() > AppConfig::MAX_GESTURE_STEPS {
            warn!(
                "Gesture sequence too long ({} > {}) in app {:?}, skipping",
                binding.gesture.sequence.len(),
                AppConfig::MAX_GESTURE_STEPS,
                app_id
            );
            continue;
        }
        if has_consecutive_same_move_steps(&binding.gesture.sequence) {
            warn!(
                "Gesture sequence contains consecutive identical directional moves {:?} in app {:?}, skipping",
                binding.gesture.sequence, app_id
            );
            continue;
        }

        let trigger = TriggerButton::from_config(&binding.gesture.trigger);
        let trigger_step = trigger.to_step();
        if binding.gesture.sequence.contains(&trigger_step) {
            warn!(
                "Gesture sequence contains its own trigger step {:?} in app {:?}, skipping",
                trigger_step, app_id
            );
            continue;
        }

        let sequence = binding.gesture.sequence.clone();
        if !seen.insert((trigger, sequence.clone())) {
            warn!(
                "Duplicate gesture binding for trigger={:?}, sequence={:?} in app {:?}, skipping",
                trigger, sequence, app_id
            );
            continue;
        }

        bindings.push(CompiledGestureBinding {
            trigger,
            sequence,
            action: binding.action.clone(),
            label: binding
                .label
                .clone()
                .unwrap_or_else(|| generate_label(&binding.action)),
        });
    }

    bindings
}

// ---------------------------------------------------------------------------
// spawn
// ---------------------------------------------------------------------------

/// Spawns the hook thread.
///
/// Configuration is read from `shared_config` once (snapshotted) so that no
/// locks are held inside the hook callback.
///
/// # Returns
///
/// A tuple of:
/// - A [`Sender`] for [`HookControl`] messages (send [`HookControl::Shutdown`]
///   to request an orderly exit).
/// - An [`Arc<AtomicU32>`] that will be populated with the Win32 thread ID
///   (via [`GetCurrentThreadId`]) once the hook thread starts. The caller can
///   use this to post `WM_QUIT` directly via [`PostThreadMessageW`].
/// - A [`JoinHandle`] for the hook thread.
///
/// # Example
///
/// ```ignore
/// let (control_tx, tid, handle) = hook::spawn(shared_config, overlay_tx);
///
/// // … later, to shut down:
/// let _ = control_tx.send(HookControl::Shutdown);
/// handle.join().unwrap();
/// ```
pub fn spawn(
    shared_config: SharedConfig,
    overlay_tx: Sender<OverlayCommand>,
) -> (Sender<HookControl>, Arc<AtomicU32>, JoinHandle<()>) {
    info!("starting hook thread");
    let (control_tx, control_rx) = crossbeam_channel::unbounded();
    let tid = Arc::new(AtomicU32::new(0));
    let tid_clone = tid.clone();

    // Snapshot config before entering the thread.
    let hook_config = {
        let cfg = shared_config.0.read().unwrap();

        // Compile app matchers.
        // Sort by app_id for deterministic matching order, since HashMap
        // iteration is non-deterministic and match_app returns first match.
        let mut sorted_apps: Vec<_> = cfg.apps.iter().collect();
        sorted_apps.sort_by_key(|(app_id, _)| (*app_id).clone());

        let apps: Vec<CompiledApp> = sorted_apps
            .into_iter()
            .filter_map(|(app_id, app_def)| {
                let matchers: Vec<CompiledMatcher> = app_def
                    .matchers
                    .iter()
                    .filter_map(compile_matcher)
                    .collect();
                if matchers.is_empty() {
                    warn!("App {:?} has no valid matchers, skipping", app_id);
                    return None;
                }
                Some(CompiledApp {
                    id: app_id.clone(),
                    matchers,
                })
            })
            .collect();

        // Parse per-app bindings.
        let mut binding_sets: HashMap<String, AppBindingSet> = HashMap::new();
        let mut total_bindings = 0;

        for (app_id, app_bindings) in &cfg.bindings {
            if app_id != "default" && !cfg.apps.contains_key(app_id) {
                warn!(
                    "Bindings reference app {:?} which is not defined in apps, skipping",
                    app_id
                );
                continue;
            }

            let bindings = compile_bindings_for_app(app_id, app_bindings);

            total_bindings += bindings.len();
            binding_sets.insert(app_id.clone(), AppBindingSet { bindings });
        }

        // Ensure a "default" binding set is always present, since resolution
        // logic falls back to it. If the user did not define one, insert an
        // empty set and warn so gestures do not silently stop working.
        if !binding_sets.contains_key("default") {
            warn!("No \"default\" bindings defined in configuration; inserting empty default set");
            binding_sets.insert(
                "default".to_string(),
                AppBindingSet {
                    bindings: Vec::new(),
                },
            );
        }

        if total_bindings > 0 {
            info!(
                "Loaded {} gesture binding(s) across {} app set(s)",
                total_bindings,
                binding_sets.len()
            );
        }
        if !apps.is_empty() {
            info!("Compiled {} app definition(s)", apps.len());
        }

        HookConfig {
            safety_timeout_ms: resolve_safety_timeout_ms(cfg.safety_timeout_ms),
            min_segment_px: resolve_min_segment_px(cfg.min_segment_px),
            direction_switch_confirm_px: resolve_direction_switch_confirm_px(
                cfg.direction_switch_confirm_px,
            ),
            axis_ambiguity_deadzone_px: resolve_axis_ambiguity_deadzone_px(
                cfg.axis_ambiguity_deadzone_px,
            ),
            replay_distance_threshold_px: resolve_replay_distance_threshold_px(
                cfg.replay_distance_threshold_px,
            ),
            max_gesture_steps: AppConfig::MAX_GESTURE_STEPS,
            apps,
            binding_sets,
        }
    };

    let handle = thread::Builder::new()
        .name("hook-thread".to_string())
        .spawn(move || {
            #[cfg(windows)]
            win32::run_loop_win32(hook_config, overlay_tx, tid_clone, control_rx);
            #[cfg(not(windows))]
            {
                let _ = (hook_config, overlay_tx, tid_clone, control_rx);
                warn!("Mouse hook is only supported on Windows");
            }
        })
        .expect("failed to spawn hook thread");
    info!("hook thread spawned");

    (control_tx, tid, handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keyboard_action(key: &str) -> crate::executor::Action {
        crate::executor::Action::Keyboard {
            keys: vec![key.to_string()],
        }
    }

    fn binding(
        trigger: crate::config::TriggerButton,
        sequence: Vec<crate::config::GestureStep>,
        key: &str,
        label: Option<&str>,
    ) -> crate::config::GestureBinding {
        crate::config::GestureBinding {
            label: label.map(ToString::to_string),
            gesture: crate::config::GesturePattern { trigger, sequence },
            action: keyboard_action(key),
        }
    }

    #[test]
    fn compile_bindings_for_app_skips_duplicate_trigger_and_sequence() {
        let raw = vec![
            binding(
                crate::config::TriggerButton::RightClick,
                vec![crate::config::GestureStep::Up],
                "a",
                Some("First"),
            ),
            binding(
                crate::config::TriggerButton::RightClick,
                vec![crate::config::GestureStep::Up],
                "b",
                Some("Second"),
            ),
        ];

        let compiled = compile_bindings_for_app("default", &raw);
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled[0].label, "First");
        assert_eq!(compiled[0].action, keyboard_action("a"));
    }

    #[test]
    fn compile_bindings_for_app_skips_sequence_containing_trigger_step() {
        let raw = vec![binding(
            crate::config::TriggerButton::RightClick,
            vec![crate::config::GestureStep::RightClick],
            "a",
            Some("Invalid"),
        )];

        let compiled = compile_bindings_for_app("default", &raw);
        assert!(compiled.is_empty());
    }

    #[test]
    fn compile_bindings_for_app_skips_sequence_with_consecutive_same_mouse_move() {
        let raw = vec![
            binding(
                crate::config::TriggerButton::RightClick,
                vec![
                    crate::config::GestureStep::Left,
                    crate::config::GestureStep::Left,
                ],
                "a",
                Some("Invalid"),
            ),
            binding(
                crate::config::TriggerButton::RightClick,
                vec![
                    crate::config::GestureStep::Left,
                    crate::config::GestureStep::Up,
                ],
                "b",
                Some("Valid"),
            ),
        ];

        let compiled = compile_bindings_for_app("default", &raw);
        assert_eq!(compiled.len(), 1);
        assert_eq!(compiled[0].label, "Valid");
    }
}
