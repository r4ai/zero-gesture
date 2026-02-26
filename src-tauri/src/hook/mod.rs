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
//!   │       side effects:
//!   │       - if hold binding exists: request_execute immediately, suppress=true
//!   │       - else: add input step, optional UpdateLabel, suppress=true
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
//!   │       - else if hold wheel action fired in this session: no replay
//!   │       - else if unmatched and travel distance <= replay threshold:
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

use std::collections::HashMap;
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
    compile_matcher, AppBindingSet, CompiledApp, CompiledGestureBinding, CompiledHoldBinding,
    CompiledMatcher,
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
// Binding compilation helpers
// ---------------------------------------------------------------------------

/// Compile one app's validated gesture bindings.
fn compile_bindings_for_app(
    app_id: &str,
    app_bindings: &[crate::config::GestureBinding],
) -> AppBindingSet {
    let mut release_bindings: Vec<CompiledGestureBinding> = Vec::new();
    let mut hold_bindings: Vec<CompiledHoldBinding> = Vec::new();

    for binding in app_bindings {
        let trigger = TriggerButton::from_config(&binding.gesture.trigger);
        let label = binding
            .label
            .clone()
            .unwrap_or_else(|| generate_label(&binding.action));

        match binding.gesture.mode {
            crate::config::GestureMode::Release => {
                let sequence = binding.gesture.sequence.clone();
                release_bindings.push(CompiledGestureBinding {
                    trigger,
                    sequence,
                    action: binding.action.clone(),
                    label,
                });
            }
            crate::config::GestureMode::Hold => {
                let Some(step) = binding.gesture.step else {
                    warn!(
                        "Hold gesture is missing step in app {:?}; this should be prevented by config validation, skipping",
                        app_id
                    );
                    continue;
                };
                hold_bindings.push(CompiledHoldBinding {
                    trigger,
                    sequence: binding.gesture.sequence.clone(),
                    step,
                    action: binding.action.clone(),
                    label,
                });
            }
        }
    }

    AppBindingSet {
        release_bindings,
        hold_bindings,
    }
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
        let cfg = shared_config.0.read().unwrap().clone().validated();

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
            let binding_set = compile_bindings_for_app(app_id, app_bindings);

            total_bindings += binding_set.release_bindings.len();
            total_bindings += binding_set.hold_bindings.len();
            binding_sets.insert(app_id.clone(), binding_set);
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
            safety_timeout_ms: cfg.safety_timeout_ms,
            min_segment_px: cfg.min_segment_px,
            direction_switch_confirm_px: cfg.direction_switch_confirm_px,
            axis_ambiguity_deadzone_px: cfg.axis_ambiguity_deadzone_px,
            replay_distance_threshold_px: cfg.replay_distance_threshold_px,
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
            sequence: Vec::new(),
        }
    }

    fn binding(
        trigger: crate::config::TriggerButton,
        sequence: Vec<crate::config::GestureStep>,
        key: &str,
        label: Option<&str>,
    ) -> crate::config::GestureBinding {
        crate::config::GestureBinding {
            id: format!("release-{key}"),
            label: label.map(ToString::to_string),
            gesture: crate::config::GesturePattern {
                trigger,
                mode: crate::config::GestureMode::Release,
                sequence,
                step: None,
            },
            action: keyboard_action(key),
        }
    }

    fn hold_binding(
        trigger: crate::config::TriggerButton,
        sequence: Vec<crate::config::GestureStep>,
        step: Option<crate::config::GestureStep>,
        key: &str,
        label: Option<&str>,
    ) -> crate::config::GestureBinding {
        crate::config::GestureBinding {
            id: format!("hold-{key}"),
            label: label.map(ToString::to_string),
            gesture: crate::config::GesturePattern {
                trigger,
                mode: crate::config::GestureMode::Hold,
                sequence,
                step,
            },
            action: keyboard_action(key),
        }
    }

    #[test]
    fn compile_bindings_for_app_compiles_validated_bindings() {
        let mut cfg = crate::config::AppConfig {
            bindings: HashMap::from([(
                crate::config::AppConfig::DEFAULT_APP_ID.to_string(),
                vec![
                    binding(
                        crate::config::TriggerButton::RightClick,
                        vec![crate::config::GestureStep::Up],
                        "a",
                        Some("Up"),
                    ),
                    hold_binding(
                        crate::config::TriggerButton::RightClick,
                        vec![crate::config::GestureStep::Up],
                        Some(crate::config::GestureStep::WheelDown),
                        "pagedown",
                        Some("Wheel Down"),
                    ),
                    // Duplicate that should be removed by config validation.
                    binding(
                        crate::config::TriggerButton::RightClick,
                        vec![crate::config::GestureStep::Up],
                        "b",
                        Some("Duplicate"),
                    ),
                ],
            )]),
            ..crate::config::AppConfig::default()
        };
        cfg.validate();

        let compiled = compile_bindings_for_app(
            crate::config::AppConfig::DEFAULT_APP_ID,
            cfg.bindings
                .get(crate::config::AppConfig::DEFAULT_APP_ID)
                .expect("default bindings must exist"),
        );
        assert_eq!(compiled.release_bindings.len(), 1);
        assert_eq!(compiled.hold_bindings.len(), 1);
        assert_eq!(compiled.release_bindings[0].label, "Up");
        assert_eq!(compiled.hold_bindings[0].label, "Wheel Down");
    }

    #[test]
    fn compile_bindings_for_app_defensively_skips_hold_binding_without_step() {
        let raw = vec![hold_binding(
            crate::config::TriggerButton::RightClick,
            Vec::new(),
            None,
            "a",
            Some("Invalid"),
        )];
        let compiled = compile_bindings_for_app("default", &raw);
        assert!(compiled.hold_bindings.is_empty());
    }
}
