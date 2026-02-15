//! Low-level mouse hook thread.
//!
//! Installs a [`WH_MOUSE_LL`] hook and runs a Win32 message loop on a
//! dedicated thread.
//!
//! The hook starts a gesture immediately when a configured trigger button is
//! pressed, captures directional movement plus mouse-input steps, and executes
//! the bound action when the trigger button is released.

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

            let bindings: Vec<CompiledGestureBinding> = app_bindings
                .iter()
                .filter_map(|binding| {
                    if binding.sequence.is_empty() {
                        warn!(
                            "Empty gesture sequence in bindings for app {:?}, skipping",
                            app_id
                        );
                        return None;
                    }
                    if binding.sequence.len() > AppConfig::MAX_GESTURE_STEPS {
                        warn!(
                            "Gesture sequence too long ({} > {}) in app {:?}, skipping",
                            binding.sequence.len(),
                            AppConfig::MAX_GESTURE_STEPS,
                            app_id
                        );
                        return None;
                    }

                    Some(CompiledGestureBinding {
                        trigger: TriggerButton::from_config(&binding.trigger),
                        sequence: binding.sequence.clone(),
                        action: binding.action.clone(),
                        label: binding
                            .label
                            .clone()
                            .unwrap_or_else(|| generate_label(&binding.action)),
                    })
                })
                .collect();

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
