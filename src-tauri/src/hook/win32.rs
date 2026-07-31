use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{Sender, TrySendError};
use log::{debug, error, info, warn};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_VIRTUALDESK,
            MOUSEINPUT,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetAncestor, GetCursorPos, GetMessageW, KillTimer,
            PostThreadMessageW, SetForegroundWindow, SetTimer, SetWindowsHookExW,
            UnhookWindowsHookEx, WindowFromPoint, GA_ROOT, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT,
            WH_MOUSE_LL, WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
            WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_TIMER,
        },
    },
};

use crate::config::{ConfigSnapshotReader, RuntimeConfig};
use crate::domain::input::{ConfigGeneration, TargetToken};
use crate::domain::{BindingSetId, Disposition, MouseEvent, Point, TriggerButton};
use crate::executor;
use crate::overlay::{self, OverlayCommand};

use super::owner::{ActionWork, ContextView, NativeInputOwner, RenderWork};

const WM_ACTION_READY: u32 = WM_APP + 1;
const WM_RENDER_READY: u32 = WM_APP + 2;
const WM_CONTEXT_READY: u32 = WM_APP + 3;
const SAFETY_TIMER_ID: usize = 1;
const SAFETY_TIMER_PERIOD_MS: u32 = 100;
const CONTEXT_SAMPLE_PERIOD_MS: u64 = 4;
const CONTEXT_PUBLISH_PERIOD_MS: u32 = 25;
const RENDERER_SHUTDOWN_TIMEOUT_MS: u64 = 500;

struct HookThreadState {
    owner: NativeInputOwner,
    owner_tid: u32,
    context: Arc<ContextMailbox>,
    renderer: RendererWorker,
}

thread_local! {
    static HOOK_STATE: RefCell<Option<HookThreadState>> = const { RefCell::new(None) };
}

struct ContextMailbox {
    sequence: AtomicU64,
    valid: AtomicBool,
    generation: AtomicU64,
    binding_set: AtomicU64,
    target: AtomicU64,
    point: AtomicU64,
    tick: AtomicU32,
}

impl ContextMailbox {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            valid: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            binding_set: AtomicU64::new(0),
            target: AtomicU64::new(0),
            point: AtomicU64::new(0),
            tick: AtomicU32::new(0),
        }
    }

    fn publish(&self, context: Option<ContextView>) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        if let Some(context) = context {
            self.generation.store(context.generation, Ordering::Relaxed);
            self.binding_set
                .store(context.binding_set.index() as u64, Ordering::Relaxed);
            self.target.store(context.target.0, Ordering::Relaxed);
            self.point
                .store(pack_point(context.point), Ordering::Relaxed);
            self.tick.store(context.updated_tick, Ordering::Relaxed);
            self.valid.store(true, Ordering::Relaxed);
        } else {
            self.valid.store(false, Ordering::Relaxed);
        }
        self.sequence.fetch_add(1, Ordering::Release);
    }

    fn read(&self) -> Option<ContextView> {
        for _ in 0..4 {
            let before = self.sequence.load(Ordering::Acquire);
            if before & 1 != 0 {
                continue;
            }
            let valid = self.valid.load(Ordering::Relaxed);
            let generation = self.generation.load(Ordering::Relaxed);
            let binding_set = self.binding_set.load(Ordering::Relaxed);
            let target = self.target.load(Ordering::Relaxed);
            let point = self.point.load(Ordering::Relaxed);
            let updated_tick = self.tick.load(Ordering::Relaxed);
            if self.sequence.load(Ordering::Acquire) != before {
                continue;
            }
            if !valid {
                return None;
            }
            return BindingSetId::from_index(binding_set as usize).map(|binding_set| ContextView {
                generation,
                binding_set,
                target: TargetToken(target),
                point: unpack_point(point),
                updated_tick,
            });
        }
        None
    }
}

struct RendererWorker {
    generation: Option<ConfigGeneration>,
    sender: Option<Sender<OverlayCommand>>,
    handle: Option<JoinHandle<()>>,
}

impl RendererWorker {
    fn new() -> Self {
        Self {
            generation: None,
            sender: None,
            handle: None,
        }
    }

    fn ensure(&mut self, generation: ConfigGeneration, runtime: Arc<RuntimeConfig>) -> bool {
        if self.generation == Some(generation) {
            return self.sender.is_some();
        }
        if !self.shutdown() {
            return false;
        }
        match overlay::spawn(runtime) {
            Ok((sender, handle)) => {
                self.generation = Some(generation);
                self.sender = Some(sender);
                self.handle = Some(handle);
                true
            }
            Err(error) => {
                error!("failed to start renderer worker: {error}");
                false
            }
        }
    }

    fn send(&self, command: OverlayCommand) -> bool {
        self.sender
            .as_ref()
            .is_some_and(|sender| sender.try_send(command).is_ok())
    }

    fn send_lossy(&self, command: OverlayCommand) -> bool {
        self.sender
            .as_ref()
            .is_some_and(|sender| match sender.try_send(command) {
                Ok(()) | Err(TrySendError::Full(_)) => true,
                Err(TrySendError::Disconnected(_)) => false,
            })
    }

    fn shutdown(&mut self) -> bool {
        let mut clean = true;
        if let Some(sender) = self.sender.take() {
            clean = sender
                .send_timeout(
                    OverlayCommand::Shutdown,
                    Duration::from_millis(RENDERER_SHUTDOWN_TIMEOUT_MS),
                )
                .is_ok();
        }
        if let Some(handle) = self.handle.take() {
            for _ in 0..RENDERER_SHUTDOWN_TIMEOUT_MS / 10 {
                if handle.is_finished() {
                    clean &= handle.join().is_ok();
                    self.generation = None;
                    return clean;
                }
                thread::sleep(Duration::from_millis(10));
            }
            clean = false;
        }
        self.generation = None;
        clean
    }
}

pub(super) fn run_loop_win32(reader: ConfigSnapshotReader, tid_arc: Arc<AtomicU32>) {
    unsafe {
        let tid = GetCurrentThreadId();
        tid_arc.store(tid, Ordering::Release);
        let context = Arc::new(ContextMailbox::new());
        let context_stop = Arc::new(AtomicBool::new(false));
        let context_handle = spawn_context_worker(
            reader.clone(),
            tid,
            Arc::clone(&context),
            Arc::clone(&context_stop),
        );
        HOOK_STATE.with(|cell| {
            *cell.borrow_mut() = Some(HookThreadState {
                owner: NativeInputOwner::new(reader),
                owner_tid: tid,
                context,
                renderer: RendererWorker::new(),
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
            context_stop.store(true, Ordering::Release);
            let _ = context_handle.join();
            HOOK_STATE.with(|cell| *cell.borrow_mut() = None);
            return;
        }
        SetTimer(
            std::ptr::null_mut(),
            SAFETY_TIMER_ID,
            SAFETY_TIMER_PERIOD_MS,
            None,
        );
        debug!("WH_MOUSE_LL hook installed (tid={tid})");

        let mut msg: MSG = std::mem::zeroed();
        loop {
            let result = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if result == 0 || result == -1 {
                break;
            }
            match msg.message {
                WM_ACTION_READY => drain_actions(),
                WM_RENDER_READY => drain_renderer(),
                WM_CONTEXT_READY => update_context(),
                WM_TIMER => handle_safety_timer(),
                _ => {
                    DispatchMessageW(&msg);
                }
            }
        }

        KillTimer(std::ptr::null_mut(), SAFETY_TIMER_ID);
        UnhookWindowsHookEx(hook);
        context_stop.store(true, Ordering::Release);
        let _ = context_handle.join();
        HOOK_STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().as_mut() {
                state.owner.shutdown();
                let _ = state.renderer.shutdown();
            }
            *cell.borrow_mut() = None;
        });
        debug!("WH_MOUSE_LL hook removed");
        info!("hook thread stopped (tid={tid})");
    }
}

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

    let disposition = HOOK_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let Some(state) = state.as_mut() else {
            return Disposition::Pass;
        };
        let disposition = state.owner.callback(
            to_mouse_event(w_param as u32, info.mouseData),
            Point::new(info.pt.x, info.pt.y),
            info.time,
        );
        signal_work(&mut state.owner, state.owner_tid);
        disposition
    });
    if disposition == Disposition::Suppress {
        1
    } else {
        CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
    }
}

fn signal_work(owner: &mut NativeInputOwner, owner_tid: u32) {
    let (actions, renderer) = owner.take_wakeups();
    if actions {
        unsafe {
            PostThreadMessageW(owner_tid, WM_ACTION_READY, 0, 0);
        }
    }
    if renderer {
        unsafe {
            PostThreadMessageW(owner_tid, WM_RENDER_READY, 0, 0);
        }
    }
}

fn drain_actions() {
    HOOK_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        while let Some(work) = state.owner.pop_action() {
            execute_action_work(state, work);
        }
        signal_work(&mut state.owner, state.owner_tid);
    });
}

fn execute_action_work(state: &mut HookThreadState, work: ActionWork) {
    match work {
        ActionWork::Activate { session, target } => {
            state
                .owner
                .activation_result(session, activate_target(target));
        }
        ActionWork::Dispatch {
            session,
            generation,
            action,
            repeat,
        } => {
            if repeat == 0 {
                state.owner.action_failed_before_injection(session);
                return;
            }
            let Some(runtime) = state.owner.runtime(generation) else {
                state.owner.executor_fault();
                return;
            };
            state.owner.injection_started(session);
            let complete = (0..repeat).all(|_| executor::execute(runtime.action(action)));
            if complete {
                state.owner.action_completed(session);
            } else {
                state.owner.action_failed_after_injection(session);
            }
        }
        ActionWork::Replay {
            trigger,
            down_at,
            up_at,
            ..
        } => replay_trigger(trigger, down_at, up_at),
    }
}

fn drain_renderer() {
    HOOK_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let Some(state) = state.as_mut() else {
            return;
        };
        while let Some(work) = state.owner.pop_render() {
            let delivered = match work {
                RenderWork::Start {
                    generation,
                    session: _,
                } => state.owner.runtime(generation).is_some_and(|runtime| {
                    state.renderer.ensure(generation, runtime)
                        && state.renderer.send(OverlayCommand::StartGesture)
                }),
                RenderWork::Point { point, .. } => {
                    state.renderer.send_lossy(OverlayCommand::TrackPoint {
                        x: point.x,
                        y: point.y,
                    })
                }
                RenderWork::Label { action, .. } => {
                    let label = action.and_then(|action| {
                        state
                            .renderer
                            .generation
                            .and_then(|generation| state.owner.runtime(generation))
                            .map(|runtime| runtime.action_label(action).to_string())
                    });
                    state
                        .renderer
                        .send_lossy(OverlayCommand::UpdateLabel(label))
                }
                RenderWork::End { .. } => state.renderer.send(OverlayCommand::EndGesture),
            };
            if !delivered {
                state.owner.renderer_fault();
                break;
            }
        }
        signal_work(&mut state.owner, state.owner_tid);
    });
}

fn update_context() {
    HOOK_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if let Some(state) = state.as_mut() {
            state.owner.set_context(state.context.read());
        }
    });
}

fn handle_safety_timer() {
    HOOK_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        if let Some(state) = state.as_mut() {
            let tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
            state.owner.safety_timer(tick);
            signal_work(&mut state.owner, state.owner_tid);
        }
    });
    drain_actions();
    drain_renderer();
}

fn spawn_context_worker(
    reader: ConfigSnapshotReader,
    owner_tid: u32,
    mailbox: Arc<ContextMailbox>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("input-context".to_string())
        .spawn(move || {
            let mut last_signature = None;
            let mut last_publish_tick = 0_u32;
            while !stop.load(Ordering::Acquire) {
                let context = resolve_context(&reader);
                let tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
                let signature = context.map(|context| {
                    (
                        context.generation,
                        context.binding_set.index(),
                        context.target.0,
                        context.point,
                    )
                });
                if signature != last_signature
                    || tick.wrapping_sub(last_publish_tick) >= CONTEXT_PUBLISH_PERIOD_MS
                {
                    mailbox.publish(context);
                    unsafe {
                        PostThreadMessageW(owner_tid, WM_CONTEXT_READY, 0, 0);
                    }
                    last_signature = signature;
                    last_publish_tick = tick;
                }
                thread::sleep(Duration::from_millis(CONTEXT_SAMPLE_PERIOD_MS));
            }
        })
        .expect("failed to spawn input context worker")
}

fn resolve_context(reader: &ConfigSnapshotReader) -> Option<ContextView> {
    let snapshot = reader.read()?;
    if !snapshot.enabled {
        return None;
    }
    let mut cursor = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut cursor) } == 0 {
        return None;
    }
    let target = top_level_window(cursor)?;
    let info = crate::window_info::get_window_info_by_hwnd(target);
    let binding_set = snapshot
        .match_windows_app(&info)
        .unwrap_or_else(|| snapshot.default_binding_set());
    Some(ContextView {
        generation: snapshot.generation(),
        binding_set,
        target: TargetToken(target as usize as u64),
        point: Point::new(cursor.x, cursor.y),
        updated_tick: unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() },
    })
}

fn top_level_window(point: POINT) -> Option<HWND> {
    let window = unsafe { WindowFromPoint(point) };
    if window.is_null() {
        return None;
    }
    let root = unsafe { GetAncestor(window, GA_ROOT) };
    Some(if root.is_null() { window } else { root })
}

fn activate_target(target: TargetToken) -> bool {
    let target = target.0 as usize as HWND;
    !target.is_null() && unsafe { SetForegroundWindow(target) } != 0
}

fn replay_trigger(trigger: TriggerButton, down_at: Point, up_at: Point) {
    let (down_x, down_y) = screen_to_absolute(down_at.x, down_at.y);
    let (up_x, up_y) = screen_to_absolute(up_at.x, up_at.y);
    let base = MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
    let inputs = [
        make_mouse_input(down_x, down_y, base | trigger_down_flag(trigger)),
        make_mouse_input(up_x, up_y, base | trigger_up_flag(trigger)),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        warn!("trigger replay was partial ({sent}/{})", inputs.len());
    }
}

fn to_mouse_event(message: u32, mouse_data: u32) -> MouseEvent {
    match message {
        WM_LBUTTONDOWN => MouseEvent::ButtonDown(TriggerButton::Left),
        WM_RBUTTONDOWN => MouseEvent::ButtonDown(TriggerButton::Right),
        WM_MBUTTONDOWN => MouseEvent::ButtonDown(TriggerButton::Middle),
        WM_LBUTTONUP => MouseEvent::ButtonUp(TriggerButton::Left),
        WM_RBUTTONUP => MouseEvent::ButtonUp(TriggerButton::Right),
        WM_MBUTTONUP => MouseEvent::ButtonUp(TriggerButton::Middle),
        WM_MOUSEMOVE => MouseEvent::MouseMove,
        WM_MOUSEWHEEL => {
            let delta = wheel_delta(mouse_data);
            match delta.cmp(&0) {
                std::cmp::Ordering::Greater => MouseEvent::WheelUp(wheel_steps(delta)),
                std::cmp::Ordering::Less => MouseEvent::WheelDown(wheel_steps(delta)),
                std::cmp::Ordering::Equal => MouseEvent::Other,
            }
        }
        _ => MouseEvent::Other,
    }
}

fn wheel_delta(mouse_data: u32) -> i16 {
    ((mouse_data >> 16) & 0xFFFF) as i16
}

fn wheel_steps(delta: i16) -> u16 {
    const WHEEL_DELTA: u16 = 120;
    if delta == 0 {
        return 0;
    }
    (delta.unsigned_abs() / WHEEL_DELTA).max(1)
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

fn screen_to_absolute(x: i32, y: i32) -> (i32, i32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if width == 0 || height == 0 {
            return (0, 0);
        }
        (
            ((x - vx) as i64 * 65_536 / width as i64) as i32,
            ((y - vy) as i64 * 65_536 / height as i64) as i32,
        )
    }
}

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

fn pack_point(point: Point) -> u64 {
    u64::from(point.x as u32) << 32 | u64::from(point.y as u32)
}

fn unpack_point(point: u64) -> Point {
    Point::new((point >> 32) as u32 as i32, point as u32 as i32)
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
        assert_eq!(wheel_delta(240_u32 << 16), 240);
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

    #[test]
    fn context_mailbox_keeps_only_the_latest_complete_value() {
        let mailbox = ContextMailbox::new();
        mailbox.publish(Some(ContextView {
            generation: 7,
            binding_set: BindingSetId::from_index(3).unwrap(),
            target: TargetToken(41),
            point: Point::new(-10, 20),
            updated_tick: 99,
        }));
        mailbox.publish(None);
        assert!(mailbox.read().is_none());
    }
}
