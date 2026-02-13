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
//! | `ButtonDown` | `WM_MOUSEMOVE` | dist > [`GESTURE_THRESHOLD`] | `Gesturing` | Send `StartGesture` + `TrackPoint`s; **pass through** |
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
//! A [`SetTimer`]-based timer fires every [`SAFETY_TIMEOUT_MS`] (2 s). If the
//! state machine has been stuck in `ButtonDown` or `Gesturing` for longer than
//! that, it is conservatively reset to `Idle` to recover from edge cases such
//! as a lost button-up event.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender};
use log::{debug, error, trace, warn};

use crate::overlay::OverlayCommand;
use crate::SharedConfig;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, POINT, WPARAM},
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

/// Pixel distance threshold before a held button becomes a gesture.
///
/// Compared against the squared Euclidean distance from the button-down
/// origin to avoid a `sqrt` call in the hot path. A value of 10 means the
/// user must move at least ~10 px in any direction before a gesture begins.
const GESTURE_THRESHOLD: i32 = 10;

/// Safety timeout in milliseconds — if state is stuck, reset to Idle.
///
/// If the state machine remains in `ButtonDown` or `Gesturing` for longer
/// than this duration, [`handle_safety_timer`] will force-reset it to `Idle`.
/// This guards against lost button-up events (e.g. focus changes, remote
/// desktop disconnects) that would otherwise leave the hook in a stuck state.
const SAFETY_TIMEOUT_MS: u32 = 2000;

/// Timer ID for the safety timeout.
#[cfg(windows)]
const SAFETY_TIMER_ID: usize = 1;

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
#[cfg(windows)]
enum GestureState {
    /// Waiting for the trigger button press. This is the resting state.
    Idle,
    /// Trigger button is held; no significant movement yet.
    ///
    /// If the user releases the button without exceeding [`GESTURE_THRESHOLD`],
    /// the click is replayed to the target application. If movement exceeds the
    /// threshold, we transition to [`Gesturing`](GestureState::Gesturing).
    ButtonDown {
        /// Screen coordinates where the trigger button was pressed.
        origin: POINT,
        /// [`GetTickCount`](windows_sys::Win32::System::SystemInformation::GetTickCount)
        /// value when we entered this state (for safety timeout).
        entered_tick: u32,
    },
    /// Actively gesturing — movement has exceeded [`GESTURE_THRESHOLD`].
    ///
    /// Mouse move events are forwarded to the overlay as [`OverlayCommand::TrackPoint`].
    /// When the trigger button is released, [`OverlayCommand::EndGesture`] is
    /// sent and the state returns to [`Idle`](GestureState::Idle).
    Gesturing {
        /// [`GetTickCount`](windows_sys::Win32::System::SystemInformation::GetTickCount)
        /// value when we entered this state (for safety timeout).
        entered_tick: u32,
    },
}

/// Info needed to replay a suppressed click via [`SendInput`].
///
/// Stored in [`HookThreadState::pending_replay`] by the hook callback and
/// consumed by [`handle_replay_click`] in the message loop.
#[cfg(windows)]
#[derive(Clone, Copy)]
struct ReplayInfo {
    /// Screen coordinates where the original button-down occurred.
    origin: POINT,
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
    let (control_tx, control_rx) = crossbeam_channel::unbounded();
    let tid = Arc::new(AtomicU32::new(0));
    let tid_clone = tid.clone();

    // Snapshot config before entering the thread.
    let hook_config = {
        let cfg = shared_config.0.read().unwrap();
        HookConfig {
            trigger: TriggerButton::from_config(&cfg.gesture_trigger_button),
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
/// 4. **Start safety timer** — [`SetTimer`] fires `WM_TIMER` every
///    [`SAFETY_TIMEOUT_MS`] for stuck-state recovery.
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
        // Publish our thread ID so the main thread can post WM_QUIT.
        let tid = GetCurrentThreadId();
        tid_arc.store(tid, Ordering::Release);

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
            SAFETY_TIMEOUT_MS,
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

/// Process a mouse event against the gesture state machine.
///
/// This is the pure-logic core of the hook — it reads the current
/// [`GestureState`], evaluates the incoming `msg` and mouse coordinates
/// from `info`, performs side effects (overlay commands, pending replay),
/// and returns whether the event should be suppressed.
///
/// # State transitions
///
/// See the [module-level documentation](self) for the full transition table.
///
/// # Returns
///
/// `true` if the event should be **suppressed** (swallowed by the hook so
/// that no application receives it). `WM_MOUSEMOVE` is never suppressed.
#[cfg(windows)]
fn process_event(hs: &mut HookThreadState, msg: u32, info: &MSLLHOOKSTRUCT) -> bool {
    let trigger_down = hs.config.trigger.down_msg();
    let trigger_up = hs.config.trigger.up_msg();
    let tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };

    match &hs.state {
        GestureState::Idle => {
            if msg == trigger_down {
                debug!("Idle → ButtonDown at ({}, {})", info.pt.x, info.pt.y);
                hs.state = GestureState::ButtonDown {
                    origin: info.pt,
                    entered_tick: tick,
                };
                return true; // suppress the button-down
            }
        }
        GestureState::ButtonDown { origin, .. } => {
            if msg == WM_MOUSEMOVE {
                let dx = info.pt.x - origin.x;
                let dy = info.pt.y - origin.y;
                if dx * dx + dy * dy > GESTURE_THRESHOLD * GESTURE_THRESHOLD {
                    debug!("ButtonDown → Gesturing");
                    let _ = hs.overlay_tx.send(OverlayCommand::StartGesture);
                    let _ = hs.overlay_tx.send(OverlayCommand::TrackPoint {
                        x: origin.x,
                        y: origin.y,
                    });
                    let _ = hs.overlay_tx.send(OverlayCommand::TrackPoint {
                        x: info.pt.x,
                        y: info.pt.y,
                    });
                    hs.state = GestureState::Gesturing { entered_tick: tick };
                }
                return false; // never suppress mouse move
            }
            if msg == trigger_up {
                debug!("ButtonDown → Idle (replay click)");
                let origin = *origin;
                hs.pending_replay = Some(ReplayInfo { origin });
                hs.state = GestureState::Idle;
                // Post WM_REPLAY_CLICK to the message loop (outside callback).
                unsafe {
                    PostThreadMessageW(GetCurrentThreadId(), WM_REPLAY_CLICK, 0, 0);
                }
                return true; // suppress the button-up
            }
        }
        GestureState::Gesturing { .. } => {
            if msg == WM_MOUSEMOVE {
                trace!("Gesturing → Gesturing at ({}, {})", info.pt.x, info.pt.y);
                let _ = hs.overlay_tx.send(OverlayCommand::TrackPoint {
                    x: info.pt.x,
                    y: info.pt.y,
                });
                return false; // never suppress mouse move
            }
            if msg == trigger_up {
                debug!("Gesturing → Idle (end gesture)");
                let _ = hs.overlay_tx.send(OverlayCommand::EndGesture);
                hs.state = GestureState::Idle;
                return true; // suppress the button-up
            }
        }
    }

    false
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
        let (vx, vy) = screen_to_absolute(replay.origin.x, replay.origin.y);
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
            replay.origin.x, replay.origin.y
        );
    }
}

/// Safety timer handler — resets the state machine if stuck.
///
/// Called every [`SAFETY_TIMEOUT_MS`] from the message loop on `WM_TIMER`.
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
        let stuck = match &hs.state {
            GestureState::Idle => false,
            GestureState::ButtonDown { entered_tick, .. }
            | GestureState::Gesturing { entered_tick } => {
                tick.wrapping_sub(*entered_tick) > SAFETY_TIMEOUT_MS
            }
        };

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
