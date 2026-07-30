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

use crate::domain::{
    ActionId, Decision, Disposition, GestureInput, GestureMachine, GestureTransition, MouseEvent,
    Point, RenderEffect, ReplayRequest, TriggerButton,
};
use crate::executor;
use crate::overlay::OverlayCommand;

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
    /// Portable gesture recognition and session decisions.
    machine: GestureMachine,
    /// Windows application matchers used before a gesture starts.
    runtime: Arc<crate::config::RuntimeConfig>,
    /// Channel to the overlay thread.
    overlay_tx: Sender<OverlayCommand>,
    /// Deferred trigger-button replay request from hook callback.
    pending_replay: Option<ReplayRequest>,
    /// Deferred action to execute from the message loop.
    pending_actions: VecDeque<(ActionId, u16)>,
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
    runtime: Arc<crate::config::RuntimeConfig>,
    overlay_tx: Sender<OverlayCommand>,
    tid_arc: Arc<AtomicU32>,
    control_rx: Receiver<HookControl>,
) {
    unsafe {
        let safety_timeout_ms = runtime.gesture.safety_timeout_ms;
        let gesture = runtime.gesture.clone();

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
                machine: GestureMachine::new(gesture),
                runtime,
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
    let point = Point::new(info.pt.x, info.pt.y);

    let binding_set = match event {
        MouseEvent::ButtonDown(trigger) if hs.machine.can_start(trigger) => {
            let activated_target = activate_window_at_point(point.x, point.y);
            let window_info = if let Some(hwnd) = activated_target {
                crate::window_info::get_window_info_by_hwnd(hwnd)
            } else {
                crate::window_info::get_foreground_window_info()
            };
            debug!("window info: {:?}", window_info);
            hs.runtime.match_windows_app(&window_info)
        }
        _ => None,
    };

    let decision = hs.machine.handle(GestureInput::Pointer {
        event,
        point,
        tick,
        binding_set,
    });
    apply_decision(hs, decision)
}

fn apply_decision(hs: &mut HookThreadState, decision: Decision) -> bool {
    for effect in decision.render {
        let command = match effect {
            RenderEffect::StartGesture => OverlayCommand::StartGesture,
            RenderEffect::TrackPoint(point) => OverlayCommand::TrackPoint {
                x: point.x,
                y: point.y,
            },
            RenderEffect::UpdateLabel(label) => OverlayCommand::UpdateLabel(
                label.map(|action| hs.runtime.action_label(action).to_string()),
            ),
            RenderEffect::EndGesture => OverlayCommand::EndGesture,
        };
        let _ = hs.overlay_tx.send(command);
    }

    match decision.transition {
        GestureTransition::Continue | GestureTransition::Complete | GestureTransition::Cancel => {}
        GestureTransition::ContinueWithAction { action, repeat } => {
            queue_action(hs, action, repeat);
        }
        GestureTransition::FinishWithAction { action } => {
            queue_action(hs, action, 1);
        }
        GestureTransition::Replay(replay) => {
            hs.pending_replay = Some(replay);
            unsafe {
                PostThreadMessageW(GetCurrentThreadId(), WM_REPLAY_OPERATION, 0, 0);
            }
        }
    }

    decision.disposition == Disposition::Suppress
}

fn queue_action(hs: &mut HookThreadState, action: ActionId, repeat: u16) {
    let should_post = hs.pending_actions.is_empty();
    hs.pending_actions.push_back((action, repeat));
    if should_post {
        unsafe {
            PostThreadMessageW(GetCurrentThreadId(), WM_EXECUTE_ACTION, 0, 0);
        }
    }
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
        let decision = hs.machine.handle(GestureInput::SafetyTimer { tick });
        if decision.transition == GestureTransition::Cancel {
            warn!("Safety timer: resetting stuck state to Idle");
        }
        apply_decision(hs, decision);
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

    for (action, repeat) in actions {
        for _ in 0..usize::from(repeat) {
            let action = HOOK_STATE.with(|cell| {
                let borrow = cell.borrow();
                borrow
                    .as_ref()
                    .map(|state| state.runtime.action(action).clone())
            });
            if let Some(action) = action {
                debug!("Executing bound action: {:?}", action);
                executor::execute(&action);
            }
        }
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
        let (down_x, down_y) = screen_to_absolute(replay.down_at.x, replay.down_at.y);
        let (up_x, up_y) = screen_to_absolute(replay.up_at.x, replay.up_at.y);
        let base = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        let inputs = [
            make_mouse_input(down_x, down_y, base | trigger_down_flag(replay.trigger)),
            make_mouse_input(up_x, up_y, base | trigger_up_flag(replay.trigger)),
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
                replay.trigger, replay.down_at.x, replay.down_at.y, replay.up_at.x, replay.up_at.y
            );
        }
    }
}

fn trigger_down_flag(trigger: TriggerButton) -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_RIGHTDOWN,
    };

    match trigger {
        TriggerButton::Left => MOUSEEVENTF_LEFTDOWN,
        TriggerButton::Right => MOUSEEVENTF_RIGHTDOWN,
        TriggerButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
    }
}

fn trigger_up_flag(trigger: TriggerButton) -> u32 {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTUP,
    };

    match trigger {
        TriggerButton::Left => MOUSEEVENTF_LEFTUP,
        TriggerButton::Right => MOUSEEVENTF_RIGHTUP,
        TriggerButton::Middle => MOUSEEVENTF_MIDDLEUP,
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
        MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    };

    #[test]
    fn wheel_translation_preserves_sign_and_notch_count() {
        assert_eq!(wheel_delta((240_u32) << 16), 240);
        assert_eq!(wheel_steps(240), 2);
        assert_eq!(wheel_delta(((-120_i16) as u16 as u32) << 16), -120);
        assert_eq!(wheel_steps(-120), 1);
    }

    #[test]
    fn trigger_replay_flags_match_win32_buttons() {
        assert_eq!(trigger_down_flag(TriggerButton::Left), MOUSEEVENTF_LEFTDOWN);
        assert_eq!(trigger_up_flag(TriggerButton::Left), MOUSEEVENTF_LEFTUP);
        assert_eq!(
            trigger_down_flag(TriggerButton::Right),
            MOUSEEVENTF_RIGHTDOWN
        );
        assert_eq!(trigger_up_flag(TriggerButton::Right), MOUSEEVENTF_RIGHTUP);
        assert_eq!(
            trigger_down_flag(TriggerButton::Middle),
            MOUSEEVENTF_MIDDLEDOWN
        );
        assert_eq!(trigger_up_flag(TriggerButton::Middle), MOUSEEVENTF_MIDDLEUP);
    }
}
