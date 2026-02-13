//! Low-level mouse hook thread.
//!
//! Installs a `WH_MOUSE_LL` hook and runs a Win32 message loop on a
//! dedicated thread. A state machine distinguishes "click" (no movement)
//! from "gesture" (movement past a pixel threshold) for the configured
//! trigger button, replaying the original click when no gesture is detected.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender};
use log::{debug, error, warn, trace};

use crate::overlay::OverlayCommand;
use crate::SharedConfig;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, POINT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE,
            MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP,
            MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PostThreadMessageW,
            SetTimer, SetWindowsHookExW, UnhookWindowsHookEx, LLMHF_INJECTED, MSLLHOOKSTRUCT,
            MSG, WH_MOUSE_LL, WM_APP, WM_MBUTTONDOWN,
            WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER,
        },
    },
};

#[cfg(windows)]
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

/// Custom message used to replay a suppressed click outside the hook callback.
#[cfg(windows)]
const WM_REPLAY_CLICK: u32 = WM_APP + 1;

/// Pixel distance threshold before a held button becomes a gesture.
const GESTURE_THRESHOLD: i32 = 10;

/// Safety timeout in milliseconds — if state is stuck, reset to Idle.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TriggerButton {
    Right,
    Middle,
}

impl TriggerButton {
    fn from_config(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "middle" => TriggerButton::Middle,
            _ => TriggerButton::Right,
        }
    }

    #[cfg(windows)]
    fn down_msg(self) -> u32 {
        match self {
            TriggerButton::Right => WM_RBUTTONDOWN,
            TriggerButton::Middle => WM_MBUTTONDOWN,
        }
    }

    #[cfg(windows)]
    fn up_msg(self) -> u32 {
        match self {
            TriggerButton::Right => WM_RBUTTONUP,
            TriggerButton::Middle => WM_MBUTTONUP,
        }
    }

    #[cfg(windows)]
    fn send_input_down_flag(self) -> u32 {
        match self {
            TriggerButton::Right => MOUSEEVENTF_RIGHTDOWN,
            TriggerButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
        }
    }

    #[cfg(windows)]
    fn send_input_up_flag(self) -> u32 {
        match self {
            TriggerButton::Right => MOUSEEVENTF_RIGHTUP,
            TriggerButton::Middle => MOUSEEVENTF_MIDDLEUP,
        }
    }
}

/// Snapshot of config relevant to the hook, taken once at startup.
#[derive(Debug, Clone)]
struct HookConfig {
    trigger: TriggerButton,
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

#[cfg(windows)]
enum GestureState {
    /// Waiting for the trigger button press.
    Idle,
    /// Trigger button is held; no significant movement yet.
    ButtonDown {
        origin: POINT,
        /// Tick count when we entered this state.
        entered_tick: u32,
    },
    /// Actively gesturing (movement exceeded threshold).
    Gesturing {
        entered_tick: u32,
    },
}

/// Info needed to replay a suppressed click.
#[cfg(windows)]
#[derive(Clone, Copy)]
struct ReplayInfo {
    origin: POINT,
}

// ---------------------------------------------------------------------------
// Thread-local state (accessible from the C-style hook callback)
// ---------------------------------------------------------------------------

#[cfg(windows)]
struct HookThreadState {
    state: GestureState,
    config: HookConfig,
    overlay_tx: Sender<OverlayCommand>,
    pending_replay: Option<ReplayInfo>,
}

#[cfg(windows)]
thread_local! {
    static HOOK_STATE: RefCell<Option<HookThreadState>> = const { RefCell::new(None) };
}

// ---------------------------------------------------------------------------
// spawn / run_loop
// ---------------------------------------------------------------------------

/// Spawns the hook thread.
///
/// Returns:
/// - A [`Sender`] for [`HookControl`] messages (currently only `Shutdown`).
/// - An `Arc<AtomicU32>` that will contain the Win32 thread ID once set.
/// - A [`JoinHandle`] for the thread.
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
        let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(low_level_mouse_proc), std::ptr::null_mut(), 0);
        if hook.is_null() {
            error!("SetWindowsHookExW failed");
            return;
        }
        debug!("WH_MOUSE_LL hook installed (tid={tid})");

        // Set a safety timer.
        SetTimer(std::ptr::null_mut(), SAFETY_TIMER_ID, SAFETY_TIMEOUT_MS, None);

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

/// Process a mouse event against the state machine.
/// Returns `true` if the event should be suppressed.
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
                    hs.state = GestureState::Gesturing {
                        entered_tick: tick,
                    };
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

/// Replay a suppressed click via `SendInput` (called from the message loop,
/// not from the hook callback, to avoid re-entrancy issues).
#[cfg(windows)]
fn handle_replay_click() {
    HOOK_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let hs = match borrow.as_mut() {
            Some(hs) => hs,
            None => return,
        };

        if let Some(replay) = hs.pending_replay.take() {
            let (vx, vy) = screen_to_absolute(replay.origin.x, replay.origin.y);
            let base_flags = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
            let down_flag = hs.config.trigger.send_input_down_flag();
            let up_flag = hs.config.trigger.send_input_up_flag();

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
    });
}

/// If the state machine has been stuck in a non-Idle state for too long,
/// reset to Idle as a conservative recovery.
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
/// used by `SendInput` (`0..65535` mapped to virtual desktop).
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

/// Build an `INPUT` struct for a mouse event.
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
