//! Low-level mouse hook thread.
//!
//! Installs a [`WH_MOUSE_LL`] hook and runs a Win32 message loop on a
//! dedicated thread. A state machine distinguishes "click" (no movement)
//! from "gesture" (movement past a pixel threshold) for the configured
//! trigger button, replaying the original click when no gesture is detected.
//!
//! # Architecture
//!
//! This module is the **"Sensor"** layer described in `docs/architecture.md`.
//! It runs on a dedicated OS thread so that the hook callback
//! ([`low_level_mouse_proc`]) never blocks the Tauri main thread or the
//! overlay renderer.
//!
//! ## State Machine
//!
//! The core logic is a three-state machine driven by [`process_event`]:
//!
//! ```text
//! Idle ──[trigger DOWN]──► ButtonDown { origin }
//!                          │                    │
//!           [move > 10px]  │                    │ [trigger UP, no move]
//!                          ▼                    ▼
//!                     Gesturing            Idle + replay click
//!                          │
//!            [trigger UP]  │
//!                          ▼
//!                     Idle + EndGesture
//! ```
//!
//! | From | Event | Condition | To | Side Effects |
//! |---|---|---|---|---|
//! | `Idle` | trigger DOWN | — | `ButtonDown` | Record origin; **suppress event** |
//! | `ButtonDown` | `WM_MOUSEMOVE` | dist > configured threshold | `Gesturing` | Send `StartGesture` + `TrackPoint`s; **pass through** |
//! | `ButtonDown` | `WM_MOUSEMOVE` | dist ≤ threshold | `ButtonDown` | **pass through** |
//! | `ButtonDown` | trigger UP | — | `Idle` | Post [`WM_REPLAY_CLICK`]; **suppress event** |
//! | `Gesturing` | `WM_MOUSEMOVE` | — | `Gesturing` | Send `TrackPoint`; **pass through** |
//! | `Gesturing` | trigger UP | — | `Idle` | Send `EndGesture`; **suppress event** |
//!
//! **Key invariant:** `WM_MOUSEMOVE` is **never** suppressed — the mouse
//! pointer always tracks normally regardless of gesture state.
//!
//! ## Click Replay
//!
//! When the user presses and releases the trigger button without moving past
//! the threshold, no gesture has occurred and the original click must be
//! delivered to the target application. Because calling [`SendInput`] from
//! inside the hook callback would cause re-entrancy, we instead:
//!
//! 1. Store the click origin in [`HookThreadState::pending_replay`].
//! 2. Post [`WM_REPLAY_CLICK`] (a custom `WM_APP + 1` message) to our own
//!    message loop via [`PostThreadMessageW`].
//! 3. The message loop handler [`handle_replay_click`] calls [`SendInput`]
//!    with `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK` at the original
//!    screen coordinates.
//! 4. The injected events carry [`LLMHF_INJECTED`] and are passed through
//!    by our hook, avoiding infinite loops.
//!
//! ## Thread-Local State
//!
//! A `WH_MOUSE_LL` callback is a C-style function pointer (`extern "system"`)
//! with no user-data parameter. To access our state we use a [`thread_local!`]
//! [`RefCell`] ([`HOOK_STATE`]). This is safe because:
//!
//! - The hook callback is always invoked on the thread that installed it.
//! - The message loop is single-threaded, so no concurrent access occurs.
//! - Configuration is snapshotted once at startup ([`HookConfig`]) — no
//!   locks are taken inside the callback.
//!
//! ## Shutdown
//!
//! A watchdog thread blocks on the [`HookControl`] channel. When
//! [`HookControl::Shutdown`] is received (or the channel disconnects), it
//! posts `WM_QUIT` to the hook thread, which breaks the [`GetMessageW`] loop
//! and triggers orderly cleanup (unhook, kill timer, drop thread-local state).
//!
//! ## Safety Timer
//!
//! A [`SetTimer`]-based timer fires at a configured interval. If the state
//! machine has been stuck in `ButtonDown` or `Gesturing` for longer than that,
//! it is conservatively reset to `Idle` to recover from edge cases such as a
//! lost button-up event.

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
use crate::gesture::GestureKind;
use crate::overlay::OverlayCommand;
use crate::SharedConfig;

use app_match::{compile_matcher, AppBindingSet, CompiledApp, CompiledMatcher};
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

/// Resolves `gesture_threshold` from config with a safe fallback.
///
/// Values less than or equal to zero are invalid and replaced by
/// [`AppConfig::DEFAULT_GESTURE_THRESHOLD`].
fn resolve_gesture_threshold(value: i32) -> i32 {
    if value > 0 {
        value
    } else {
        warn!(
            "Invalid gesture_threshold={} in config, falling back to {}",
            value,
            AppConfig::DEFAULT_GESTURE_THRESHOLD
        );
        AppConfig::DEFAULT_GESTURE_THRESHOLD
    }
}

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

/// Parse a gesture kind name (e.g. `"Left"`, `"DownRight"`) into a [`GestureKind`].
///
/// Uses serde deserialization of the enum variant name for consistency with
/// the config format.
fn parse_gesture_kind(name: &str) -> Option<GestureKind> {
    // GestureKind derives Deserialize, so we can deserialize directly from a JSON string value
    // without manually formatting JSON.
    let value = serde_json::Value::String(name.to_owned());
    serde_json::from_value(value).ok()
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

        for (app_id, gesture_map) in &cfg.bindings {
            if app_id != "default" && !cfg.apps.contains_key(app_id) {
                warn!(
                    "Bindings reference app {:?} which is not defined in apps, skipping",
                    app_id
                );
                continue;
            }

            let parsed: Vec<(GestureKind, &crate::config::GestureBinding)> = gesture_map
                .iter()
                .filter_map(|(name, binding)| {
                    let kind = parse_gesture_kind(name);
                    if kind.is_none() {
                        warn!(
                            "Unknown gesture name {:?} in bindings for app {:?}",
                            name, app_id
                        );
                    }
                    kind.map(|k| (k, binding))
                })
                .collect();

            let bindings: HashMap<GestureKind, crate::executor::Action> =
                parsed.iter().map(|(k, b)| (*k, b.action.clone())).collect();

            let labels: HashMap<GestureKind, String> = parsed
                .iter()
                .map(|(k, b)| {
                    let label = b.label.clone().unwrap_or_else(|| generate_label(&b.action));
                    (*k, label)
                })
                .collect();

            total_bindings += bindings.len();
            binding_sets.insert(app_id.clone(), AppBindingSet { bindings, labels });
        }

        // Ensure a "default" binding set is always present, since resolution
        // logic falls back to it. If the user did not define one, insert an
        // empty set and warn so gestures do not silently stop working.
        if !binding_sets.contains_key("default") {
            warn!("No \"default\" bindings defined in configuration; inserting empty default set");
            binding_sets.insert(
                "default".to_string(),
                AppBindingSet {
                    bindings: HashMap::new(),
                    labels: HashMap::new(),
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
            trigger: TriggerButton::from_config(&cfg.gesture_trigger_button),
            gesture_threshold: resolve_gesture_threshold(cfg.gesture_threshold),
            safety_timeout_ms: resolve_safety_timeout_ms(cfg.safety_timeout_ms),
            min_segment_px: resolve_min_segment_px(cfg.min_segment_px),
            direction_switch_confirm_px: resolve_direction_switch_confirm_px(
                cfg.direction_switch_confirm_px,
            ),
            axis_ambiguity_deadzone_px: resolve_axis_ambiguity_deadzone_px(
                cfg.axis_ambiguity_deadzone_px,
            ),
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
