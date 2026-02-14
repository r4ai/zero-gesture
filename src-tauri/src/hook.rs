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

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender};
use log::{debug, error, info, trace, warn};

use crate::config::{AppConfig, GestureBinding, MatchMethod, MatchTarget};
use crate::executor::{self, generate_label, Action};
use crate::gesture::{GestureKind, GestureRecognizer};
use crate::overlay::OverlayCommand;
use crate::window_info::ForegroundWindowInfo;
use crate::SharedConfig;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MIDDLEDOWN,
            MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
            MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW, SetTimer,
            SetWindowsHookExW, UnhookWindowsHookEx, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
            WH_MOUSE_LL, WM_APP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN,
            WM_RBUTTONUP, WM_TIMER,
        },
    },
};

#[cfg(windows)]
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

/// Custom message used to replay a suppressed click outside the hook callback.
///
/// Defined as `WM_APP + 1` to avoid collision with system-defined messages.
/// Posted to the hook thread's message queue via [`PostThreadMessageW`] when
/// the trigger button is released without a gesture, so that [`SendInput`]
/// can be called safely outside the hook callback (avoiding re-entrancy).
#[cfg(windows)]
const WM_REPLAY_CLICK: u32 = WM_APP + 1;

/// Custom message used to execute a gesture-bound action outside the hook callback.
///
/// Defined as `WM_APP + 2`. Posted when a gesture is recognised and has a
/// matching binding, so that [`executor::execute`] runs safely in the message
/// loop (avoiding re-entrancy with the low-level hook).
#[cfg(windows)]
const WM_EXECUTE_ACTION: u32 = WM_APP + 2;

/// Timer ID for the safety timeout.
#[cfg(windows)]
const SAFETY_TIMER_ID: usize = 1;

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

// ---------------------------------------------------------------------------
// Per-app matching
// ---------------------------------------------------------------------------

/// Compiled matching logic for a single [`AppMatcher`](crate::config::AppMatcher).
///
/// Pre-processes the match value at startup so the hot path only does
/// simple string operations or regex matching.
#[derive(Debug, Clone)]
enum CompiledMatchLogic {
    /// Exact match (case-insensitive): value is pre-lowercased.
    ExactCaseInsensitive(String),
    /// Exact match (case-sensitive): value as-is.
    ExactCaseSensitive(String),
    /// Substring match (case-insensitive): value is pre-lowercased.
    Contains(String),
    /// Regex pattern match.
    Regex(regex::Regex),
}

/// A compiled matcher combining a target and pre-processed matching logic.
#[derive(Debug, Clone)]
struct CompiledMatcher {
    target: MatchTarget,
    logic: CompiledMatchLogic,
}

impl CompiledMatcher {
    /// Test whether this matcher matches the given window info.
    fn matches(&self, info: &ForegroundWindowInfo) -> bool {
        let target_value = match &self.target {
            MatchTarget::ProcessName => info.process_name.as_deref(),
            MatchTarget::WindowClass => info.window_class.as_deref(),
            MatchTarget::Title => info.title.as_deref(),
        };
        let target_value = match target_value {
            Some(v) => v,
            None => return false,
        };
        match &self.logic {
            CompiledMatchLogic::ExactCaseInsensitive(pattern) => {
                target_value.to_ascii_lowercase() == *pattern
            }
            CompiledMatchLogic::ExactCaseSensitive(pattern) => target_value == pattern,
            CompiledMatchLogic::Contains(pattern) => {
                target_value.to_ascii_lowercase().contains(pattern.as_str())
            }
            CompiledMatchLogic::Regex(re) => re.is_match(target_value),
        }
    }
}

/// A compiled app definition with its ID and matchers.
#[derive(Debug, Clone)]
struct CompiledApp {
    id: String,
    matchers: Vec<CompiledMatcher>,
}

/// Pre-parsed bindings and labels for one app (or the "default" set).
#[derive(Debug, Clone)]
struct AppBindingSet {
    bindings: HashMap<GestureKind, Action>,
    labels: HashMap<GestureKind, String>,
}

/// Linear scan to find the first app whose matchers match the given window info.
///
/// Returns the app ID if a match is found, `None` otherwise.
/// Each app's matchers use OR logic — any single matcher matching is sufficient.
fn match_app<'a>(apps: &'a [CompiledApp], info: &ForegroundWindowInfo) -> Option<&'a str> {
    for app in apps {
        if app.matchers.iter().any(|m| m.matches(info)) {
            return Some(&app.id);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

/// Messages sent from the main thread to the hook thread.
pub enum HookControl {
    /// Request the hook thread to stop and exit.
    Shutdown,
}

/// Which mouse button triggers gestures.
///
/// Maps to the corresponding Win32 `WM_*BUTTONDOWN` / `WM_*BUTTONUP` message
/// constants and [`SendInput`] flags. Defaults to [`TriggerButton::Right`]
/// for unrecognised configuration values.
///
/// # Examples
///
/// ```ignore
/// let btn = TriggerButton::from_config("middle");
/// assert_eq!(btn, TriggerButton::Middle);
///
/// let btn = TriggerButton::from_config("unknown");
/// assert_eq!(btn, TriggerButton::Right); // fallback
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerButton {
    Right,
    Middle,
}

impl TriggerButton {
    /// Parse a trigger button name from the user configuration string.
    ///
    /// Recognised values (case-insensitive): `"middle"`. Everything else
    /// (including `"right"`) maps to [`TriggerButton::Right`].
    fn from_config(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "middle" => TriggerButton::Middle,
            _ => TriggerButton::Right,
        }
    }

    /// Return the Win32 `WM_*BUTTONDOWN` message constant for this trigger.
    #[cfg(windows)]
    fn down_msg(self) -> u32 {
        match self {
            TriggerButton::Right => WM_RBUTTONDOWN,
            TriggerButton::Middle => WM_MBUTTONDOWN,
        }
    }

    /// Return the Win32 `WM_*BUTTONUP` message constant for this trigger.
    #[cfg(windows)]
    fn up_msg(self) -> u32 {
        match self {
            TriggerButton::Right => WM_RBUTTONUP,
            TriggerButton::Middle => WM_MBUTTONUP,
        }
    }

    /// Return the [`SendInput`] `MOUSEEVENTF_*DOWN` flag for this trigger.
    #[cfg(windows)]
    fn send_input_down_flag(self) -> u32 {
        match self {
            TriggerButton::Right => MOUSEEVENTF_RIGHTDOWN,
            TriggerButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
        }
    }

    /// Return the [`SendInput`] `MOUSEEVENTF_*UP` flag for this trigger.
    #[cfg(windows)]
    fn send_input_up_flag(self) -> u32 {
        match self {
            TriggerButton::Right => MOUSEEVENTF_RIGHTUP,
            TriggerButton::Middle => MOUSEEVENTF_MIDDLEUP,
        }
    }
}

/// Snapshot of configuration relevant to the hook, taken once at startup.
///
/// By copying the needed values out of [`SharedConfig`] before entering the
/// hook thread, we avoid taking any locks inside the latency-critical hook
/// callback. Changes to the live config require restarting the hook thread.
#[derive(Debug, Clone)]
struct HookConfig {
    trigger: TriggerButton,
    gesture_threshold: i32,
    safety_timeout_ms: u32,
    min_segment_px: i32,
    direction_switch_confirm_px: i32,
    axis_ambiguity_deadzone_px: i32,
    /// Compiled app definitions for per-app matching.
    apps: Vec<CompiledApp>,
    /// Per-app bindings, keyed by app ID. Includes `"default"`.
    binding_sets: HashMap<String, AppBindingSet>,
}

impl HookConfig {
    /// Look up the action for a gesture, checking app-specific bindings first,
    /// then falling back to `"default"`.
    fn resolve_binding(&self, kind: &GestureKind, matched_app: Option<&str>) -> Option<&Action> {
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
    fn resolve_label(&self, kind: &GestureKind, matched_app: Option<&str>) -> Option<&String> {
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
/// See the [module-level documentation](self) for the full transition table
/// and ASCII diagram.
///
/// Each non-`Idle` variant stores an `entered_tick` timestamp (from
/// [`GetTickCount`]) so the safety timer can detect stuck states.
///
/// [`GetTickCount`]: windows_sys::Win32::System::SystemInformation::GetTickCount
enum GestureState {
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
enum MouseEvent {
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
struct OverlayCommands<const N: usize> {
    buf: [std::mem::MaybeUninit<OverlayCommand>; N],
    len: usize,
}

impl<const N: usize> OverlayCommands<N> {
    fn new() -> Self {
        Self {
            // SAFETY: An array of `MaybeUninit` does not require initialisation.
            buf: unsafe { std::mem::MaybeUninit::uninit().assume_init() },
            len: 0,
        }
    }

    fn push(&mut self, cmd: OverlayCommand) {
        assert!(self.len < N, "OverlayCommands overflow");
        self.buf[self.len].write(cmd);
        self.len += 1;
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.len
    }

    #[cfg(test)]
    fn last(&self) -> Option<&OverlayCommand> {
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

struct OverlayCommandsIntoIter<const N: usize> {
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
struct EventEffect {
    /// Whether the event should be suppressed (swallowed by the hook).
    suppress: bool,
    /// Overlay commands to send (stack-allocated, max 4).
    overlay_commands: OverlayCommands<4>,
    /// If set, a click should be replayed at these screen coordinates.
    request_replay: Option<(i32, i32)>,
    /// If set, the given action should be executed.
    request_execute: Option<Action>,
    /// If set, the window at these screen coordinates should be activated
    /// (brought to the foreground).
    activate_window_at: Option<(i32, i32)>,
}

/// Info needed to replay a suppressed click via [`SendInput`].
///
/// Stored in [`HookThreadState::pending_replay`] by the hook callback and
/// consumed by [`handle_replay_click`] in the message loop.
#[cfg(windows)]
#[derive(Clone, Copy)]
struct ReplayInfo {
    /// Screen X coordinate where the original button-down occurred.
    origin_x: i32,
    /// Screen Y coordinate where the original button-down occurred.
    origin_y: i32,
}

// ---------------------------------------------------------------------------
// Thread-local state (accessible from the C-style hook callback)
// ---------------------------------------------------------------------------

/// All mutable state for the hook thread, stored in a [`thread_local!`].
///
/// A `WH_MOUSE_LL` callback is an `extern "system"` function with no
/// user-data pointer, so we must use thread-local storage to access our
/// state. This is safe because the hook callback is always invoked on the
/// thread that installed it, and the message loop is single-threaded.
#[cfg(windows)]
struct HookThreadState {
    /// Current position in the gesture state machine.
    state: GestureState,
    /// Snapshotted configuration (no locks taken in the callback).
    config: HookConfig,
    /// Channel to the overlay thread for drawing commands.
    overlay_tx: Sender<OverlayCommand>,
    /// Set by the hook callback when a click needs to be replayed;
    /// consumed by [`handle_replay_click`] in the message loop.
    pending_replay: Option<ReplayInfo>,
    /// Set by the hook callback when a gesture has a matching binding;
    /// consumed by the `WM_EXECUTE_ACTION` handler in the message loop.
    pending_action: Option<Action>,
}

// Thread-local storage for the hook state.
//
// Initialised to `Some(…)` when the hook thread starts and set back to
// `None` during shutdown. The `RefCell` is borrowed mutably inside the
// hook callback and the message loop handlers; this is safe because
// Win32 guarantees that the `WH_MOUSE_LL` callback runs synchronously
// on this thread (it is not called re-entrantly).
#[cfg(windows)]
thread_local! {
    static HOOK_STATE: RefCell<Option<HookThreadState>> = const { RefCell::new(None) };
}

// ---------------------------------------------------------------------------
// spawn / run_loop
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
        let apps: Vec<CompiledApp> = cfg
            .apps
            .iter()
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

            let parsed: Vec<(GestureKind, &GestureBinding)> = gesture_map
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

            let bindings: HashMap<GestureKind, Action> =
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
            run_loop_win32(hook_config, overlay_tx, tid_clone, control_rx);
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

/// Main loop of the hook thread (Windows implementation).
///
/// Performs the following steps in order:
///
/// 1. **Publish thread ID** — stores [`GetCurrentThreadId`] into `tid_arc`
///    so that the main thread (or watchdog) can post `WM_QUIT`.
/// 2. **Spawn watchdog** — a helper thread that blocks on `control_rx` and
///    posts `WM_QUIT` when [`HookControl::Shutdown`] arrives, ensuring
///    reliable shutdown even if the main thread races with hook startup.
/// 3. **Install hook** — calls [`SetWindowsHookExW`] with `WH_MOUSE_LL`
///    and our [`low_level_mouse_proc`] callback. Passing `null` for the
///    module handle and `0` for the thread ID makes this a global hook.
/// 4. **Start safety timer** — [`SetTimer`] fires `WM_TIMER` at the
///    configured timeout interval for stuck-state recovery.
/// 5. **Run message loop** — [`GetMessageW`] blocks until a message arrives.
///    - `WM_REPLAY_CLICK` → [`handle_replay_click`]
///    - `WM_TIMER` → [`handle_safety_timer`]
///    - Other → [`DispatchMessageW`]
///    - `WM_QUIT` or error → break.
/// 6. **Cleanup** — kill timer, unhook, clear thread-local state, join
///    watchdog thread.
///
/// # Safety
///
/// This function uses `unsafe` for all Win32 FFI calls. The key invariants
/// are documented inline. Notably, `SetWindowsHookExW` requires a valid
/// callback that follows the `LowLevelMouseProc` calling convention, and
/// [`GetMessageW`] must be called on the same thread that installed the hook.
#[cfg(windows)]
fn run_loop_win32(
    hook_config: HookConfig,
    overlay_tx: Sender<OverlayCommand>,
    tid_arc: Arc<AtomicU32>,
    control_rx: Receiver<HookControl>,
) {
    unsafe {
        let safety_timeout_ms = hook_config.safety_timeout_ms;

        // Publish our thread ID so the main thread can post WM_QUIT.
        let tid = GetCurrentThreadId();
        tid_arc.store(tid, Ordering::Release);
        info!("hook thread started (tid={tid})");

        // Spawn a watchdog thread that monitors the control channel and
        // posts WM_QUIT if HookControl::Shutdown is received. This ensures
        // a reliable shutdown even if PostThreadMessageW from the main
        // thread races with thread startup.
        let watchdog = thread::Builder::new()
            .name("hook-watchdog".to_string())
            .spawn(move || {
                // Block until Shutdown or channel disconnect.
                let _ = control_rx.recv();
                // SAFETY: PostThreadMessageW is safe to call from any thread.
                PostThreadMessageW(tid, 0x0012 /* WM_QUIT */, 0, 0);
            })
            .expect("failed to spawn hook watchdog thread");

        // Initialize thread-local state.
        HOOK_STATE.with(|cell| {
            *cell.borrow_mut() = Some(HookThreadState {
                state: GestureState::Idle,
                config: hook_config,
                overlay_tx,
                pending_replay: None,
                pending_action: None,
            });
        });

        // Install the low-level mouse hook.
        let hook = SetWindowsHookExW(
            WH_MOUSE_LL,
            Some(low_level_mouse_proc),
            std::ptr::null_mut(),
            0,
        );
        if hook.is_null() {
            error!("SetWindowsHookExW failed");
            return;
        }
        debug!("WH_MOUSE_LL hook installed (tid={tid})");

        // Set a safety timer.
        SetTimer(
            std::ptr::null_mut(),
            SAFETY_TIMER_ID,
            safety_timeout_ms,
            None,
        );

        // Win32 message loop — exits on WM_QUIT.
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if ret == 0 || ret == -1 {
                break;
            }

            if msg.message == WM_REPLAY_CLICK {
                handle_replay_click();
            } else if msg.message == WM_EXECUTE_ACTION {
                handle_execute_action();
            } else if msg.message == WM_TIMER {
                handle_safety_timer();
            } else {
                DispatchMessageW(&msg);
            }
        }

        KillTimer(std::ptr::null_mut(), SAFETY_TIMER_ID);
        UnhookWindowsHookEx(hook);
        debug!("WH_MOUSE_LL hook removed");

        // Clean up thread-local state.
        HOOK_STATE.with(|cell| {
            *cell.borrow_mut() = None;
        });

        // The watchdog thread will exit once the control channel is dropped
        // or after it posts WM_QUIT. Join it to avoid detached threads.
        let _ = watchdog.join();
        info!("hook thread stopped (tid={tid})");
    }
}

// ---------------------------------------------------------------------------
// Hook callback
// ---------------------------------------------------------------------------

/// Win32 low-level mouse hook callback (`LowLevelMouseProc`).
///
/// Called by the OS on every mouse event system-wide. The parameters are:
///
/// - `n_code` — if negative, we must call [`CallNextHookEx`] without
///   processing. Otherwise it is `HC_ACTION` (0).
/// - `w_param` — the mouse message identifier (e.g. `WM_MOUSEMOVE`,
///   `WM_RBUTTONDOWN`).
/// - `l_param` — pointer to an [`MSLLHOOKSTRUCT`] containing coordinates,
///   timestamp, and flags (notably [`LLMHF_INJECTED`] for synthetic events).
///
/// # Return value
///
/// - **Non-zero** (we return `1`): the event is swallowed — no application
///   receives it. Used to suppress the trigger button press/release.
/// - **[`CallNextHookEx`]**: passes the event to the next hook in the chain
///   and ultimately to the target application.
///
/// # Re-entrancy
///
/// This callback must not call [`SendInput`] directly, because the injected
/// event would re-enter this callback synchronously. Instead we store
/// replay info in thread-local state and post [`WM_REPLAY_CLICK`] to the
/// message loop.
///
/// # Safety
///
/// - `l_param` is cast to `*const MSLLHOOKSTRUCT`; this is valid as long
///   as `n_code >= 0`, which we guard above.
/// - Thread-local access via [`HOOK_STATE`] is safe because the callback
///   runs on the same thread that installed the hook.
#[cfg(windows)]
unsafe extern "system" fn low_level_mouse_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
    }

    let info = &*(l_param as *const MSLLHOOKSTRUCT);

    // Always pass through injected events (our own SendInput replays).
    if info.flags & LLMHF_INJECTED != 0 {
        return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
    }

    let msg = w_param as u32;
    let suppress = HOOK_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let hs = match borrow.as_mut() {
            Some(hs) => hs,
            None => return false,
        };
        process_event(hs, msg, info)
    });

    if suppress {
        1 // non-zero = swallow the message
    } else {
        CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
    }
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
/// See the [module-level documentation](self) for the full transition table.
fn process_event_pure(
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
        activate_window_at: None,
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
                let dx = pt.0 - ox;
                let dy = pt.1 - oy;
                let dist_squared = i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy);
                let threshold = i64::from(config.gesture_threshold);
                if dist_squared > threshold * threshold {
                    debug!("ButtonDown → Gesturing (app={:?})", matched_app);
                    effect.activate_window_at = Some((ox, oy));
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
fn check_safety_timeout(state: &GestureState, tick: u32, timeout_ms: u32) -> bool {
    match state {
        GestureState::Idle => false,
        GestureState::ButtonDown { entered_tick, .. }
        | GestureState::Gesturing { entered_tick, .. } => {
            tick.wrapping_sub(*entered_tick) > timeout_ms
        }
    }
}

/// Process a mouse event against the gesture state machine (Win32 wrapper).
///
/// Converts Win32 types into platform-independent representations, calls
/// [`process_event_pure`], and applies the resulting [`EventEffect`] by
/// sending overlay commands, posting replay/execute messages, etc.
///
/// # Returns
///
/// `true` if the event should be **suppressed** (swallowed by the hook so
/// that no application receives it). `WM_MOUSEMOVE` is never suppressed.
#[cfg(windows)]
fn process_event(hs: &mut HookThreadState, msg: u32, info: &MSLLHOOKSTRUCT) -> bool {
    let event = if msg == hs.config.trigger.down_msg() {
        MouseEvent::TriggerDown
    } else if msg == hs.config.trigger.up_msg() {
        MouseEvent::TriggerUp
    } else if msg == WM_MOUSEMOVE {
        MouseEvent::MouseMove
    } else {
        MouseEvent::Other
    };

    let tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
    let pt = (info.pt.x, info.pt.y);

    // Detect the foreground window when transitioning from ButtonDown to
    // Gesturing (i.e. on the first MouseMove that exceeds the threshold).
    // This ensures window detection happens at most once per gesture.
    let matched_app =
        if matches!(hs.state, GestureState::ButtonDown { .. }) && event == MouseEvent::MouseMove {
            let window_info = crate::window_info::get_foreground_window_info();
            match_app(&hs.config.apps, &window_info).map(|s| s.to_owned())
        } else {
            None
        };

    let effect = process_event_pure(&mut hs.state, &hs.config, event, pt, tick, matched_app);

    if let Some((x, y)) = effect.activate_window_at {
        activate_window_at_point(x, y);
    }

    for cmd in effect.overlay_commands {
        let _ = hs.overlay_tx.send(cmd);
    }
    if let Some((rx, ry)) = effect.request_replay {
        hs.pending_replay = Some(ReplayInfo {
            origin_x: rx,
            origin_y: ry,
        });
        unsafe {
            PostThreadMessageW(GetCurrentThreadId(), WM_REPLAY_CLICK, 0, 0);
        }
    }
    if let Some(action) = effect.request_execute {
        hs.pending_action = Some(action);
        unsafe {
            PostThreadMessageW(GetCurrentThreadId(), WM_EXECUTE_ACTION, 0, 0);
        }
    }

    effect.suppress
}

// ---------------------------------------------------------------------------
// Message loop handlers
// ---------------------------------------------------------------------------

/// Replay a suppressed click via [`SendInput`].
///
/// Called from the message loop (on `WM_REPLAY_CLICK`), **not** from the
/// hook callback, to avoid re-entrancy — if we called [`SendInput`] inside
/// [`low_level_mouse_proc`], the injected event would re-enter our callback
/// synchronously before the first invocation returns.
///
/// The replay uses `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK` so
/// that the click lands at exactly the original screen position, even on
/// multi-monitor setups. The injected events carry [`LLMHF_INJECTED`] and
/// are therefore passed through by our hook (see the early-return check in
/// [`low_level_mouse_proc`]).
#[cfg(windows)]
fn handle_replay_click() {
    // Drop the RefCell borrow before calling SendInput. SendInput can
    // synchronously re-enter low_level_mouse_proc on this same thread.
    let replay = HOOK_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let hs = match borrow.as_mut() {
            Some(hs) => hs,
            None => return None,
        };

        hs.pending_replay.take().map(|replay| {
            (
                replay,
                hs.config.trigger.send_input_down_flag(),
                hs.config.trigger.send_input_up_flag(),
            )
        })
    });

    if let Some((replay, down_flag, up_flag)) = replay {
        let (vx, vy) = screen_to_absolute(replay.origin_x, replay.origin_y);
        let base_flags = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        let inputs = [
            make_mouse_input(vx, vy, base_flags | down_flag),
            make_mouse_input(vx, vy, base_flags | up_flag),
        ];

        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            );
        }
        debug!(
            "Replayed click at ({}, {}) → virtual ({vx}, {vy})",
            replay.origin_x, replay.origin_y
        );
    }
}

/// Safety timer handler — resets the state machine if stuck.
///
/// Called every configured timeout interval from the message loop on `WM_TIMER`.
/// Compares the current [`GetTickCount`] against the `entered_tick` stored
/// in `ButtonDown` or `Gesturing`. If the elapsed time exceeds the timeout,
/// the state is conservatively reset to `Idle`.
///
/// This guards against edge cases where the trigger button-up event is
/// lost (e.g. the window loses focus, a remote desktop session disconnects,
/// or another hook swallows the event).
///
/// If we were in `Gesturing`, an [`OverlayCommand::EndGesture`] is sent
/// so the overlay cleans up its trail rendering.
///
/// [`GetTickCount`]: windows_sys::Win32::System::SystemInformation::GetTickCount
#[cfg(windows)]
fn handle_safety_timer() {
    HOOK_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let hs = match borrow.as_mut() {
            Some(hs) => hs,
            None => return,
        };

        let tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
        let stuck = check_safety_timeout(&hs.state, tick, hs.config.safety_timeout_ms);

        if stuck {
            warn!("Safety timer: resetting stuck state to Idle");
            // If we were gesturing, tell the overlay to clean up.
            if matches!(hs.state, GestureState::Gesturing { .. }) {
                let _ = hs.overlay_tx.send(OverlayCommand::EndGesture);
            }
            hs.state = GestureState::Idle;
        }
    });
}

/// Execute a pending gesture-bound action via [`executor::execute`].
///
/// Called from the message loop (on `WM_EXECUTE_ACTION`), **not** from the
/// hook callback, following the same deferred-execution pattern as click
/// replay to avoid re-entrancy.
#[cfg(windows)]
fn handle_execute_action() {
    let action = HOOK_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        borrow.as_mut().and_then(|hs| hs.pending_action.take())
    });

    if let Some(action) = action {
        debug!("Executing bound action: {:?}", action);
        executor::execute(&action);
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

/// Compile an [`AppMatcher`](crate::config::AppMatcher) into a [`CompiledMatcher`].
///
/// Returns `None` if the matcher is invalid (e.g. invalid regex pattern).
fn compile_matcher(m: &crate::config::AppMatcher) -> Option<CompiledMatcher> {
    let logic = match (&m.target, &m.method) {
        // Exact on process_name/title → case-insensitive
        (MatchTarget::ProcessName | MatchTarget::Title, MatchMethod::Exact) => {
            CompiledMatchLogic::ExactCaseInsensitive(m.value.to_ascii_lowercase())
        }
        // Exact on window_class → case-sensitive
        (MatchTarget::WindowClass, MatchMethod::Exact) => {
            CompiledMatchLogic::ExactCaseSensitive(m.value.clone())
        }
        // Contains → always case-insensitive
        (_, MatchMethod::Contains) => CompiledMatchLogic::Contains(m.value.to_ascii_lowercase()),
        // Regex
        (_, MatchMethod::Regex) => match regex::Regex::new(&m.value) {
            Ok(re) => CompiledMatchLogic::Regex(re),
            Err(err) => {
                warn!(
                    "Invalid regex pattern {:?} for {:?} matcher: {}",
                    m.value, m.target, err
                );
                return None;
            }
        },
    };
    Some(CompiledMatcher {
        target: m.target.clone(),
        logic,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert screen coordinates to the normalised absolute coordinate space
/// used by [`SendInput`] with `MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK`.
///
/// The virtual desktop spans all monitors. [`GetSystemMetrics`] gives us:
/// - `SM_XVIRTUALSCREEN` / `SM_YVIRTUALSCREEN` — top-left origin
/// - `SM_CXVIRTUALSCREEN` / `SM_CYVIRTUALSCREEN` — total width/height
///
/// The formula maps `(x, y)` into the `0..65535` range that `SendInput`
/// expects:
///
/// ```text
/// abs_x = (x - virt_left) * 65536 / virt_width
/// abs_y = (y - virt_top)  * 65536 / virt_height
/// ```
///
/// Returns `(0, 0)` if the virtual desktop size is zero (should not happen
/// in practice).
///
/// [`GetSystemMetrics`]: windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics
#[cfg(windows)]
fn screen_to_absolute(x: i32, y: i32) -> (i32, i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        if vw == 0 || vh == 0 {
            return (0, 0);
        }

        let ax = ((x - vx) as i64 * 65536 / vw as i64) as i32;
        let ay = ((y - vy) as i64 * 65536 / vh as i64) as i32;
        (ax, ay)
    }
}

/// Build an [`INPUT`] struct for a mouse event with the given absolute
/// coordinates and flags.
///
/// The returned struct has `type = INPUT_MOUSE` and all other fields
/// (`mouseData`, `time`, `dwExtraInfo`) zeroed. The caller is expected to
/// combine position flags (`MOUSEEVENTF_ABSOLUTE`, `MOUSEEVENTF_VIRTUALDESK`)
/// with action flags (`MOUSEEVENTF_RIGHTDOWN`, etc.) in `flags`.
#[cfg(windows)]
fn make_mouse_input(x: i32, y: i32, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: x,
                dy: y,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Activate (bring to foreground) the top-level window at the given screen
/// coordinates.
///
/// Uses [`WindowFromPoint`] to find the window under `(x, y)`, then
/// [`GetAncestor`] with `GA_ROOT` to resolve it to a top-level window,
/// and finally [`SetForegroundWindow`] to make it the active window.
///
/// This is called at the start of a gesture so that subsequent keyboard
/// actions dispatched by the executor target the correct window.
///
/// [`WindowFromPoint`]: windows_sys::Win32::UI::WindowsAndMessaging::WindowFromPoint
/// [`GetAncestor`]: windows_sys::Win32::UI::WindowsAndMessaging::GetAncestor
/// [`SetForegroundWindow`]: windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow
#[cfg(windows)]
fn activate_window_at_point(x: i32, y: i32) {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, SetForegroundWindow, WindowFromPoint, GA_ROOT,
    };

    unsafe {
        let hwnd = WindowFromPoint(POINT { x, y });
        if hwnd.is_null() {
            debug!("activate_window_at_point: no window at ({x}, {y})");
            return;
        }

        let root = GetAncestor(hwnd, GA_ROOT);
        let target = if root.is_null() { hwnd } else { root };

        debug!("activate_window_at_point: activating window {target:?} at ({x}, {y})");
        SetForegroundWindow(target);
    }
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
        assert!(effect.activate_window_at.is_none());
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
        assert_eq!(
            effect.activate_window_at,
            Some((100, 200)),
            "should activate window at origin when gesture starts"
        );
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

    // ── Per-app matching tests ───────────────────────────────────────────

    #[test]
    fn match_app_process_name_exact() {
        let apps = vec![CompiledApp {
            id: "browser".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::ProcessName,
                logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: Some("chrome.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));

        // Case insensitive
        let info = ForegroundWindowInfo {
            process_name: Some("Chrome.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));

        // No match
        let info = ForegroundWindowInfo {
            process_name: Some("firefox.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), None);
    }

    #[test]
    fn match_app_window_class_exact_case_sensitive() {
        let apps = vec![CompiledApp {
            id: "explorer".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::WindowClass,
                logic: CompiledMatchLogic::ExactCaseSensitive("CabinetWClass".to_string()),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: Some("CabinetWClass".to_string()),
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("explorer"));

        // Case mismatch → no match (case-sensitive)
        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: Some("cabinetwclass".to_string()),
            title: None,
        };
        assert_eq!(match_app(&apps, &info), None);
    }

    #[test]
    fn match_app_title_contains() {
        let apps = vec![CompiledApp {
            id: "browser".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::Title,
                logic: CompiledMatchLogic::Contains("google chrome".to_string()),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: None,
            title: Some("My Page - Google Chrome".to_string()),
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));

        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: None,
            title: Some("Firefox".to_string()),
        };
        assert_eq!(match_app(&apps, &info), None);
    }

    #[test]
    fn match_app_regex() {
        let apps = vec![CompiledApp {
            id: "terminals".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::ProcessName,
                logic: CompiledMatchLogic::Regex(
                    regex::Regex::new(r"^(windowsterminal|cmd|powershell)\.exe$").unwrap(),
                ),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: Some("windowsterminal.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("terminals"));

        let info = ForegroundWindowInfo {
            process_name: Some("notepad.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), None);
    }

    #[test]
    fn match_app_or_logic() {
        let apps = vec![CompiledApp {
            id: "browser".to_string(),
            matchers: vec![
                CompiledMatcher {
                    target: MatchTarget::ProcessName,
                    logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
                },
                CompiledMatcher {
                    target: MatchTarget::ProcessName,
                    logic: CompiledMatchLogic::ExactCaseInsensitive("firefox.exe".to_string()),
                },
            ],
        }];

        // Matches second matcher
        let info = ForegroundWindowInfo {
            process_name: Some("firefox.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));
    }

    #[test]
    fn match_app_first_match_wins() {
        let apps = vec![
            CompiledApp {
                id: "browser".to_string(),
                matchers: vec![CompiledMatcher {
                    target: MatchTarget::ProcessName,
                    logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
                }],
            },
            CompiledApp {
                id: "google".to_string(),
                matchers: vec![CompiledMatcher {
                    target: MatchTarget::ProcessName,
                    logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
                }],
            },
        ];

        let info = ForegroundWindowInfo {
            process_name: Some("chrome.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));
    }

    #[test]
    fn match_app_none_field_no_panic() {
        let apps = vec![CompiledApp {
            id: "browser".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::ProcessName,
                logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), None);
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
