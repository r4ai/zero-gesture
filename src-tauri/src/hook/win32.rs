use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

use crossbeam_channel::Receiver;
use crossbeam_channel::Sender;
use log::{debug, error, info, warn};

use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_VIRTUALDESK,
            MOUSEINPUT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW, SetTimer,
            SetWindowsHookExW, UnhookWindowsHookEx, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
            WH_MOUSE_LL, WM_APP, WM_MOUSEMOVE, WM_TIMER,
        },
    },
};

use crate::executor;
use crate::executor::Action;
use crate::overlay::OverlayCommand;

use super::app_match::match_app;
use super::state::{
    check_safety_timeout, exceeds_gesture_threshold, process_event_pure, GestureState, HookConfig,
    MouseEvent,
};
use super::HookControl;

/// Custom message used to replay a suppressed click outside the hook callback.
///
/// Defined as `WM_APP + 1` to avoid collision with system-defined messages.
/// Posted to the hook thread's message queue via [`PostThreadMessageW`] when
/// the trigger button is released without a gesture, so that [`SendInput`]
/// can be called safely outside the hook callback (avoiding re-entrancy).
const WM_REPLAY_CLICK: u32 = WM_APP + 1;

/// Custom message used to execute a gesture-bound action outside the hook callback.
///
/// Defined as `WM_APP + 2`. Posted when a gesture is recognised and has a
/// matching binding, so that [`executor::execute`] runs safely in the message
/// loop (avoiding re-entrancy with the low-level hook).
const WM_EXECUTE_ACTION: u32 = WM_APP + 2;

/// Timer ID for the safety timeout.
const SAFETY_TIMER_ID: usize = 1;

/// Info needed to replay a suppressed click via [`SendInput`].
///
/// Stored in [`HookThreadState::pending_replay`] by the hook callback and
/// consumed by [`handle_replay_click`] in the message loop.
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
thread_local! {
    static HOOK_STATE: RefCell<Option<HookThreadState>> = const { RefCell::new(None) };
}

// ---------------------------------------------------------------------------
// run_loop_win32
// ---------------------------------------------------------------------------

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
pub(super) fn run_loop_win32(
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
/// event would re-enter this callback synchronously before the first invocation
/// returns. Instead we store replay info in thread-local state and post
/// [`WM_REPLAY_CLICK`] to the message loop.
///
/// # Safety
///
/// - `l_param` is cast to `*const MSLLHOOKSTRUCT`; this is valid as long
///   as `n_code >= 0`, which we guard above.
/// - Thread-local access via [`HOOK_STATE`] is safe because the callback
///   runs on the same thread that installed the hook.
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

    // On ButtonDown -> Gesturing transition, activate the window at the
    // gesture origin first, then read window info from the activated target
    // handle (falling back to foreground lookup only if no target exists).
    let matched_app = if let (
        GestureState::ButtonDown {
            origin_x, origin_y, ..
        },
        MouseEvent::MouseMove,
    ) = (&hs.state, event)
    {
        if exceeds_gesture_threshold((*origin_x, *origin_y), pt, hs.config.gesture_threshold) {
            let activated_target = activate_window_at_point(*origin_x, *origin_y);
            let window_info = if let Some(hwnd) = activated_target {
                crate::window_info::get_window_info_by_hwnd(hwnd)
            } else {
                crate::window_info::get_foreground_window_info()
            };
            debug!("window info: {:?}", window_info);
            match_app(&hs.config.apps, &window_info).map(|s| s.to_owned())
        } else {
            None
        }
    } else {
        None
    };

    let effect = process_event_pure(&mut hs.state, &hs.config, event, pt, tick, matched_app);

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
fn activate_window_at_point(x: i32, y: i32) -> Option<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, SetForegroundWindow, WindowFromPoint, GA_ROOT,
    };

    unsafe {
        let hwnd = WindowFromPoint(POINT { x, y });
        if hwnd.is_null() {
            debug!("activate_window_at_point: no window at ({x}, {y})");
            return None;
        }

        let root = GetAncestor(hwnd, GA_ROOT);
        let target = if root.is_null() { hwnd } else { root };

        debug!("activate_window_at_point: activating window {target:?} at ({x}, {y})");
        let _ = SetForegroundWindow(target);
        Some(target)
    }
}
