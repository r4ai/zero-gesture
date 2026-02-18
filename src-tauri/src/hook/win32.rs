use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Receiver, Sender};
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
            WH_MOUSE_LL, WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
            WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER,
        },
    },
};

use crate::executor;
use crate::executor::Action;
use crate::overlay::OverlayCommand;

use super::app_match::match_app;
use super::state::{
    check_safety_timeout, process_event_pure, GestureState, HookConfig, MouseEvent, ReplayRequest,
};
use super::trigger::TriggerButton;
use super::HookControl;

/// Custom message used to replay trigger-button operation outside hook callback.
///
/// Defined as `WM_APP + 1` so we can call `SendInput` from the message loop
/// and avoid callback re-entrancy.
const WM_REPLAY_OPERATION: u32 = WM_APP + 1;

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
    /// Deferred trigger-button replay request from hook callback.
    pending_replay: Option<ReplayRequest>,
    /// Deferred action to execute from the message loop.
    pending_actions: VecDeque<Action>,
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
                pending_replay: None,
                pending_actions: VecDeque::new(),
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

            if msg.message == WM_REPLAY_OPERATION {
                handle_replay_operation();
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
    let matched_app = precompute_matched_app(msg, info);
    let suppress = HOOK_STATE.with(|cell| {
        let mut borrow = match cell.try_borrow_mut() {
            Ok(borrow) => borrow,
            Err(_) => {
                warn!("low_level_mouse_proc: HOOK_STATE already borrowed, skipping event");
                return false;
            }
        };
        let hs = match borrow.as_mut() {
            Some(hs) => hs,
            None => return false,
        };
        process_event(hs, msg, info, matched_app)
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
fn process_event(
    hs: &mut HookThreadState,
    msg: u32,
    info: &MSLLHOOKSTRUCT,
    matched_app: Option<String>,
) -> bool {
    let event = to_mouse_event(msg, info);
    let tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
    let pt = (info.pt.x, info.pt.y);

    let effect = process_event_pure(&mut hs.state, &hs.config, event, pt, tick, matched_app);

    for cmd in effect.overlay_commands {
        let _ = hs.overlay_tx.send(cmd);
    }

    if let Some(replay) = effect.request_replay {
        hs.pending_replay = Some(replay);
        unsafe {
            PostThreadMessageW(GetCurrentThreadId(), WM_REPLAY_OPERATION, 0, 0);
        }
    }

    if let Some(execute_request) = effect.request_execute {
        let should_post = hs.pending_actions.is_empty();
        for _ in 0..usize::from(execute_request.repeat) {
            hs.pending_actions.push_back(execute_request.action.clone());
        }
        if should_post {
            unsafe {
                PostThreadMessageW(GetCurrentThreadId(), WM_EXECUTE_ACTION, 0, 0);
            }
        }
    }

    effect.suppress
}

/// Resolve app match at gesture start, while avoiding mutable borrow of hook state.
fn precompute_matched_app(msg: u32, info: &MSLLHOOKSTRUCT) -> Option<String> {
    let MouseEvent::ButtonDown(trigger) = to_mouse_event(msg, info) else {
        return None;
    };
    let pt = (info.pt.x, info.pt.y);

    let activation_mode = HOOK_STATE.with(|cell| {
        let borrow = cell.borrow();
        let hs = borrow.as_ref()?;
        if !matches!(hs.state, GestureState::Idle) {
            return None;
        }
        if !hs.config.has_any_binding_for_trigger(trigger) {
            return None;
        }
        Some(hs.config.gesture_activation_mode)
    })?;

    let activated_target = activate_window_at_point(pt.0, pt.1, activation_mode);
    let window_info = if let Some(hwnd) = activated_target {
        crate::window_info::get_window_info_by_hwnd(hwnd)
    } else {
        crate::window_info::get_foreground_window_info()
    };
    debug!("window info: {:?}", window_info);

    HOOK_STATE.with(|cell| {
        let borrow = cell.borrow();
        let hs = borrow.as_ref()?;
        match_app(&hs.config.apps, &window_info).map(|id| id.to_owned())
    })
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
            let steps = wheel_steps(delta);
            if delta > 0 {
                MouseEvent::WheelUp(steps)
            } else if delta < 0 {
                MouseEvent::WheelDown(steps)
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

/// Convert wheel delta into positive notch count (`>= 1` when delta != 0).
fn wheel_steps(delta: i16) -> u16 {
    const WHEEL_DELTA: u16 = 120;
    if delta == 0 {
        return 0;
    }
    let raw = delta.unsigned_abs();
    let steps = raw / WHEEL_DELTA;
    steps.max(1)
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
    let actions = HOOK_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let Some(hs) = borrow.as_mut() else {
            return Vec::new();
        };
        hs.pending_actions.drain(..).collect::<Vec<_>>()
    });

    for action in actions {
        debug!("Executing bound action: {:?}", action);
        executor::execute(&action);
    }
}

/// Replay suppressed trigger-button operation via `SendInput`.
///
/// Replays:
/// - trigger button down at captured press position
/// - trigger button up at captured release position
///
/// Both events use absolute virtual-desktop coordinates so behavior remains
/// correct on multi-monitor setups.
fn handle_replay_operation() {
    let replay = HOOK_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        borrow.as_mut().and_then(|hs| hs.pending_replay.take())
    });

    if let Some(replay) = replay {
        let (down_x, down_y) = screen_to_absolute(replay.down_at.0, replay.down_at.1);
        let (up_x, up_y) = screen_to_absolute(replay.up_at.0, replay.up_at.1);
        let base = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        let inputs = [
            make_mouse_input(down_x, down_y, base | replay.trigger.send_input_down_flag()),
            make_mouse_input(up_x, up_y, base | replay.trigger.send_input_up_flag()),
        ];

        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if sent < inputs.len() as u32 {
            warn!(
                "SendInput replay was partial (sent {sent}/{}) for trigger {:?}",
                inputs.len(),
                replay.trigger
            );
        } else {
            debug!(
                "Replayed trigger operation {:?} down_at=({}, {}), up_at=({}, {})",
                replay.trigger, replay.down_at.0, replay.down_at.1, replay.up_at.0, replay.up_at.1
            );
        }
    }
}

/// Convert screen coordinates to `SendInput` absolute virtual-desktop space.
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

/// Build one mouse `INPUT` event at absolute coordinates with custom flags.
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
/// Uses [`WindowFromPoint`] to locate the window under cursor, then applies
/// the configured activation mode:
/// - `window`: activate the root window only (legacy behavior)
/// - `element`: activate root window and attempt to focus exact element window
fn activate_window_at_point(
    x: i32,
    y: i32,
    mode: crate::config::GestureActivationMode,
) -> Option<windows_sys::Win32::Foundation::HWND> {
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
        let root_target = if root.is_null() { hwnd } else { root };

        match mode {
            crate::config::GestureActivationMode::Window => {
                debug!(
                    "activate_window_at_point(window): activating window {root_target:?} at ({x}, {y})"
                );
                let _ = SetForegroundWindow(root_target);
            }
            crate::config::GestureActivationMode::Element => {
                debug!(
                    "activate_window_at_point(element): root={root_target:?}, leaf={hwnd:?} at ({x}, {y})"
                );
                activate_element_window(root_target, hwnd);
            }
        }

        Some(root_target)
    }
}

/// Activate a top-level window and attempt to focus the specific child window.
fn activate_element_window(
    root: windows_sys::Win32::Foundation::HWND,
    leaf: windows_sys::Win32::Foundation::HWND,
) {
    use windows_sys::Win32::System::Threading::AttachThreadInput;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowThreadProcessId, SetForegroundWindow,
    };

    unsafe {
        let _ = SetForegroundWindow(root);

        let focus_target = if leaf.is_null() { root } else { leaf };
        if focus_target.is_null() {
            return;
        }

        let current_tid = GetCurrentThreadId();
        let target_tid = GetWindowThreadProcessId(focus_target, std::ptr::null_mut());

        let mut attached = false;
        if target_tid != 0 && target_tid != current_tid {
            attached = AttachThreadInput(current_tid, target_tid, 1) != 0;
            if !attached {
                debug!(
                    "activate_element_window: failed to attach thread input (current_tid={current_tid}, target_tid={target_tid})"
                );
            }
        }

        let focused = SetFocus(focus_target);
        if focused.is_null() && focus_target != root {
            let _ = SetFocus(root);
        }

        if attached {
            let _ = AttachThreadInput(current_tid, target_tid, 0);
        }
    }
}
