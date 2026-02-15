use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Receiver, Sender};
use log::{debug, error, info, warn};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW, SetTimer,
        SetWindowsHookExW, UnhookWindowsHookEx, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL,
        WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE,
        WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER,
    },
};

use crate::executor;
use crate::executor::Action;
use crate::overlay::OverlayCommand;

use super::app_match::match_app;
use super::state::{
    check_safety_timeout, process_event_pure, GestureState, HookConfig, MouseEvent,
};
use super::trigger::TriggerButton;
use super::HookControl;

/// Custom message used to execute a gesture-bound action outside the hook callback.
///
/// Defined as `WM_APP + 2`. Posted when a gesture has a matching binding so
/// [`executor::execute`] runs in the message loop rather than in the low-level
/// hook callback (avoids re-entrancy hazards).
const WM_EXECUTE_ACTION: u32 = WM_APP + 2;

/// Timer ID for the safety timeout.
const SAFETY_TIMER_ID: usize = 1;

/// All mutable state for the hook thread, stored in [`HOOK_STATE`].
struct HookThreadState {
    /// Current position in the gesture state machine.
    state: GestureState,
    /// Snapshotted configuration (no locks in callback hot path).
    config: HookConfig,
    /// Channel to the overlay thread.
    overlay_tx: Sender<OverlayCommand>,
    /// Deferred action to execute from the message loop.
    pending_action: Option<Action>,
}

// Thread-local storage is required because `WH_MOUSE_LL` callback signature
// does not provide user-data.
thread_local! {
    static HOOK_STATE: RefCell<Option<HookThreadState>> = const { RefCell::new(None) };
}

/// Main loop of the hook thread (Windows implementation).
///
/// Performs:
/// 1. Publish thread ID.
/// 2. Spawn watchdog listening for [`HookControl::Shutdown`].
/// 3. Install `WH_MOUSE_LL` hook.
/// 4. Start safety timer.
/// 5. Process Win32 messages until `WM_QUIT`.
/// 6. Cleanup (timer, hook, TLS, watchdog).
///
/// # Safety
///
/// Uses Win32 FFI (`unsafe`) and must keep callback/message-loop thread
/// affinity intact.
pub(super) fn run_loop_win32(
    hook_config: HookConfig,
    overlay_tx: Sender<OverlayCommand>,
    tid_arc: Arc<AtomicU32>,
    control_rx: Receiver<HookControl>,
) {
    unsafe {
        let safety_timeout_ms = hook_config.safety_timeout_ms;

        let tid = GetCurrentThreadId();
        tid_arc.store(tid, Ordering::Release);
        info!("hook thread started (tid={tid})");

        let watchdog = thread::Builder::new()
            .name("hook-watchdog".to_string())
            .spawn(move || {
                let _ = control_rx.recv();
                PostThreadMessageW(
                    tid,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT,
                    0,
                    0,
                );
            })
            .expect("failed to spawn hook watchdog thread");

        HOOK_STATE.with(|cell| {
            *cell.borrow_mut() = Some(HookThreadState {
                state: GestureState::Idle,
                config: hook_config,
                overlay_tx,
                pending_action: None,
            });
        });

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

        SetTimer(
            std::ptr::null_mut(),
            SAFETY_TIMER_ID,
            safety_timeout_ms,
            None,
        );

        let mut msg: MSG = std::mem::zeroed();
        loop {
            let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if ret == 0 || ret == -1 {
                break;
            }

            if msg.message == WM_EXECUTE_ACTION {
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

        HOOK_STATE.with(|cell| {
            *cell.borrow_mut() = None;
        });

        let _ = watchdog.join();
        info!("hook thread stopped (tid={tid})");
    }
}

/// Win32 low-level mouse hook callback (`LowLevelMouseProc`).
///
/// Called by the OS for each mouse event. Returns non-zero to swallow an
/// event, or delegates via [`CallNextHookEx`] to pass through.
///
/// # Safety
///
/// `l_param` is cast to `*const MSLLHOOKSTRUCT` per `WH_MOUSE_LL` contract.
unsafe extern "system" fn low_level_mouse_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
    }

    let info = &*(l_param as *const MSLLHOOKSTRUCT);
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
        1
    } else {
        CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
    }
}

/// Process one Win32 mouse message against the pure state machine.
///
/// Converts Win32 event data into [`MouseEvent`], resolves matched app context,
/// applies produced side effects, and returns whether the original event should
/// be suppressed.
fn process_event(hs: &mut HookThreadState, msg: u32, info: &MSLLHOOKSTRUCT) -> bool {
    let event = to_mouse_event(msg, info);
    let tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
    let pt = (info.pt.x, info.pt.y);

    let matched_app = match (&hs.state, event) {
        (GestureState::Idle, MouseEvent::ButtonDown(trigger))
            if hs.config.has_any_binding_for_trigger(trigger) =>
        {
            let activated_target = activate_window_at_point(pt.0, pt.1);
            let window_info = if let Some(hwnd) = activated_target {
                crate::window_info::get_window_info_by_hwnd(hwnd)
            } else {
                crate::window_info::get_foreground_window_info()
            };
            debug!("window info: {:?}", window_info);
            match_app(&hs.config.apps, &window_info).map(|id| id.to_owned())
        }
        _ => None,
    };

    let effect = process_event_pure(&mut hs.state, &hs.config, event, pt, tick, matched_app);

    for cmd in effect.overlay_commands {
        let _ = hs.overlay_tx.send(cmd);
    }

    if let Some(action) = effect.request_execute {
        hs.pending_action = Some(action);
        unsafe {
            PostThreadMessageW(GetCurrentThreadId(), WM_EXECUTE_ACTION, 0, 0);
        }
    }

    effect.suppress
}

/// Convert Win32 mouse message IDs into hook-level [`MouseEvent`].
fn to_mouse_event(msg: u32, info: &MSLLHOOKSTRUCT) -> MouseEvent {
    match msg {
        WM_LBUTTONDOWN => MouseEvent::ButtonDown(TriggerButton::Left),
        WM_RBUTTONDOWN => MouseEvent::ButtonDown(TriggerButton::Right),
        WM_MBUTTONDOWN => MouseEvent::ButtonDown(TriggerButton::Middle),
        WM_LBUTTONUP => MouseEvent::ButtonUp(TriggerButton::Left),
        WM_RBUTTONUP => MouseEvent::ButtonUp(TriggerButton::Right),
        WM_MBUTTONUP => MouseEvent::ButtonUp(TriggerButton::Middle),
        WM_MOUSEMOVE => MouseEvent::MouseMove,
        WM_MOUSEWHEEL => {
            let delta = wheel_delta(info.mouseData);
            if delta > 0 {
                MouseEvent::WheelUp
            } else if delta < 0 {
                MouseEvent::WheelDown
            } else {
                MouseEvent::Other
            }
        }
        _ => MouseEvent::Other,
    }
}

/// Extract signed wheel delta from `MSLLHOOKSTRUCT::mouseData`.
fn wheel_delta(mouse_data: u32) -> i16 {
    ((mouse_data >> 16) & 0xFFFF) as i16
}

/// Safety timer handler.
///
/// Resets stuck gesture state back to `Idle`. If a gesture was in progress,
/// sends [`OverlayCommand::EndGesture`] to ensure overlay cleanup.
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
            if matches!(hs.state, GestureState::Gesturing { .. }) {
                let _ = hs.overlay_tx.send(OverlayCommand::EndGesture);
            }
            hs.state = GestureState::Idle;
        }
    });
}

/// Execute pending action from `WM_EXECUTE_ACTION`.
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

/// Activate (bring to foreground) the top-level window at the given screen
/// coordinates.
///
/// Uses [`WindowFromPoint`] to locate the window under cursor, resolves a
/// top-level window via [`GetAncestor`] (`GA_ROOT`), and requests activation
/// with [`SetForegroundWindow`].
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
