use std::cell::RefCell;
use std::sync::atomic::{fence, AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TryRecvError, TrySendError};
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
use crate::overlay::{self, OverlayClient, OverlayCommand, OverlayDelivery};

use super::owner::{ActionWork, ContextView, NativeInputOwner, RenderWork, CONTEXT_MAX_AGE_MS};
use super::{HookEvent, HookFailure};

const WM_ACTION_READY: u32 = WM_APP + 1;
const WM_RENDER_READY: u32 = WM_APP + 2;
const WM_CONTEXT_READY: u32 = WM_APP + 3;
const SAFETY_TIMER_ID: usize = 1;
const SAFETY_TIMER_PERIOD_MS: u32 = 100;
const CONTEXT_SAMPLE_PERIOD_MS: u64 = 4;
const CONTEXT_PUBLISH_PERIOD_MS: u32 = 25;
const RENDERER_SHUTDOWN_TIMEOUT_MS: u64 = 500;
const RENDERER_QUEUE_CAPACITY: usize = 64;

struct HookThreadState {
    owner: NativeInputOwner,
    capture: Arc<crate::capture::WindowCapture>,
    owner_tid: u32,
    context: Arc<ContextMailbox>,
    context_stop: Arc<AtomicBool>,
    context_handle: Option<JoinHandle<Result<(), HookFailure>>>,
    renderer: RendererClient,
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
        let writing = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        debug_assert_eq!(writing & 1, 1);
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
        self.sequence.store(writing + 1, Ordering::Release);
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
            fence(Ordering::Acquire);
            if self.sequence.load(Ordering::Relaxed) != before {
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
    runtime: Option<Arc<RuntimeConfig>>,
    sender: Option<OverlayClient>,
    handle: Option<JoinHandle<()>>,
}

impl RendererWorker {
    fn new() -> Self {
        Self {
            generation: None,
            runtime: None,
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
        match overlay::spawn(Arc::clone(&runtime)) {
            Ok((sender, handle)) => {
                self.generation = Some(generation);
                self.runtime = Some(runtime);
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

    fn send(&mut self, command: OverlayCommand) -> bool {
        self.sender
            .as_mut()
            .is_some_and(|sender| matches!(sender.try_send(command), OverlayDelivery::Accepted))
    }

    fn send_lossy(&mut self, command: OverlayCommand) -> bool {
        self.sender
            .as_mut()
            .is_some_and(|sender| match sender.try_send(command) {
                OverlayDelivery::Accepted | OverlayDelivery::Full => true,
                OverlayDelivery::Fault => false,
            })
    }

    fn has_unexpected_exit(&self) -> bool {
        self.handle.as_ref().is_some_and(JoinHandle::is_finished)
    }

    fn label(&self, action: Option<crate::domain::ActionId>) -> Option<String> {
        action.map(|action| {
            self.runtime
                .as_ref()
                .expect("renderer runtime must exist while rendering")
                .action_label(action)
                .to_string()
        })
    }

    fn shutdown(&mut self) -> bool {
        let mut clean = true;
        if let Some(sender) = self.sender.take() {
            clean = matches!(
                sender.send_timeout(
                    OverlayCommand::Shutdown,
                    Duration::from_millis(RENDERER_SHUTDOWN_TIMEOUT_MS),
                ),
                OverlayDelivery::Accepted
            );
        }
        if let Some(handle) = self.handle.take() {
            for _ in 0..RENDERER_SHUTDOWN_TIMEOUT_MS / 10 {
                if handle.is_finished() {
                    clean &= handle.join().is_ok();
                    self.generation = None;
                    self.runtime = None;
                    return clean;
                }
                thread::sleep(Duration::from_millis(10));
            }
            clean = false;
        }
        self.generation = None;
        self.runtime = None;
        clean
    }
}

enum RendererRequest {
    Start {
        generation: ConfigGeneration,
        runtime: Arc<RuntimeConfig>,
    },
    Command {
        command: OverlayCommand,
        lossy: bool,
    },
    Label {
        action: Option<crate::domain::ActionId>,
    },
    Shutdown,
}

enum RendererStatus {
    Fatal,
}

struct RendererClient {
    sender: Sender<RendererRequest>,
    status: Receiver<RendererStatus>,
    handle: Option<JoinHandle<()>>,
    terminal_reserved: bool,
}

impl RendererClient {
    fn spawn() -> std::io::Result<Self> {
        let (sender, requests) = bounded(RENDERER_QUEUE_CAPACITY);
        let (status_tx, status) = bounded(1);
        let handle = thread::Builder::new()
            .name("renderer-owner".to_string())
            .spawn(move || run_renderer_owner(requests, status_tx))?;
        Ok(Self {
            sender,
            status,
            handle: Some(handle),
            terminal_reserved: false,
        })
    }

    fn try_send(&mut self, request: RendererRequest, lossy: bool) -> bool {
        let start = matches!(&request, RendererRequest::Start { .. });
        let end = matches!(
            &request,
            RendererRequest::Command {
                command: OverlayCommand::EndGesture,
                ..
            }
        );
        if start
            && (self.terminal_reserved
                || self.sender.len().saturating_add(2) > RENDERER_QUEUE_CAPACITY)
        {
            return false;
        }
        if lossy
            && self
                .sender
                .len()
                .saturating_add(usize::from(self.terminal_reserved))
                >= RENDERER_QUEUE_CAPACITY
        {
            return true;
        }
        if end && !self.terminal_reserved {
            return false;
        }
        match self.sender.try_send(request) {
            Ok(()) => {
                if start {
                    self.terminal_reserved = true;
                } else if end {
                    self.terminal_reserved = false;
                }
                true
            }
            Err(TrySendError::Full(_)) if lossy => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        }
    }

    fn has_failed(&self) -> bool {
        matches!(
            self.status.try_recv(),
            Ok(RendererStatus::Fatal) | Err(TryRecvError::Disconnected)
        ) || self.handle.as_ref().is_some_and(JoinHandle::is_finished)
    }

    fn shutdown(&mut self) -> bool {
        let requested = self
            .sender
            .send_timeout(
                RendererRequest::Shutdown,
                Duration::from_millis(RENDERER_SHUTDOWN_TIMEOUT_MS),
            )
            .is_ok();
        let joined = self
            .handle
            .take()
            .is_none_or(|handle| handle.join().is_ok());
        requested && joined
    }
}

fn run_renderer_owner(requests: Receiver<RendererRequest>, status: Sender<RendererStatus>) {
    let mut worker = RendererWorker::new();
    let mut clean = true;
    loop {
        #[cfg(debug_assertions)]
        if test_failure_marker_exists("ZG_P03C_TEST_RENDERER_FAILURE_MARKER") {
            clean = false;
            break;
        }
        match requests.recv_timeout(Duration::from_millis(25)) {
            Ok(RendererRequest::Start {
                generation,
                runtime,
            }) => {
                if !worker.ensure(generation, runtime) || !worker.send(OverlayCommand::StartGesture)
                {
                    clean = false;
                    break;
                }
            }
            Ok(RendererRequest::Command { command, lossy }) => {
                let delivered = if lossy {
                    worker.send_lossy(command)
                } else {
                    worker.send(command)
                };
                if !delivered {
                    clean = false;
                    break;
                }
            }
            Ok(RendererRequest::Label { action }) => {
                let label = worker.label(action);
                if !worker.send_lossy(OverlayCommand::UpdateLabel(label)) {
                    clean = false;
                    break;
                }
            }
            Ok(RendererRequest::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                if worker.has_unexpected_exit() {
                    clean = false;
                    break;
                }
            }
        }
    }
    clean &= worker.shutdown();
    if !clean {
        let _ = status.try_send(RendererStatus::Fatal);
    }
}

pub(super) fn run_loop_win32(
    reader: ConfigSnapshotReader,
    capture: Arc<crate::capture::WindowCapture>,
    events: Sender<HookEvent>,
) -> Result<(), HookFailure> {
    unsafe {
        let tid = GetCurrentThreadId();
        let context = Arc::new(ContextMailbox::new());
        let context_stop = Arc::new(AtomicBool::new(false));
        let context_handle = spawn_context_worker(
            reader.clone(),
            tid,
            Arc::clone(&context),
            Arc::clone(&context_stop),
        )
        .map_err(|_| HookFailure::new("context", "failed to start"))?;
        let renderer =
            RendererClient::spawn().map_err(|_| HookFailure::new("renderer", "failed to start"))?;
        HOOK_STATE.with(|cell| {
            *cell.borrow_mut() = Some(HookThreadState {
                owner: NativeInputOwner::new(reader),
                capture,
                owner_tid: tid,
                context,
                context_stop,
                context_handle: Some(context_handle),
                renderer,
            });
        });

        #[cfg(debug_assertions)]
        let inject_install_failure =
            std::env::var_os("ZG_P03C_TEST_HOOK_INSTALL_FAILURE").is_some();
        #[cfg(not(debug_assertions))]
        let inject_install_failure = false;
        let hook = if inject_install_failure {
            std::ptr::null_mut()
        } else {
            SetWindowsHookExW(
                WH_MOUSE_LL,
                Some(low_level_mouse_proc),
                std::ptr::null_mut(),
                0,
            )
        };
        if hook.is_null() {
            shutdown_hook_state();
            return Err(HookFailure::new("hook", "installation failed"));
        }
        if SetTimer(
            std::ptr::null_mut(),
            SAFETY_TIMER_ID,
            SAFETY_TIMER_PERIOD_MS,
            None,
        ) == 0
        {
            UnhookWindowsHookEx(hook);
            shutdown_hook_state();
            return Err(HookFailure::new("hook", "safety timer setup failed"));
        }
        if events.send(HookEvent::Ready(tid)).is_err() {
            KillTimer(std::ptr::null_mut(), SAFETY_TIMER_ID);
            UnhookWindowsHookEx(hook);
            shutdown_hook_state();
            return Err(HookFailure::new("hook", "readiness receiver disappeared"));
        }
        debug!("WH_MOUSE_LL hook installed (tid={tid})");

        let mut msg: MSG = std::mem::zeroed();
        let mut failure = None;
        loop {
            let result = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if result == 0 {
                break;
            }
            if result == -1 {
                failure = Some(HookFailure::new("hook", "message loop failed"));
                break;
            }
            match msg.message {
                WM_ACTION_READY => drain_actions(),
                WM_RENDER_READY => drain_renderer(),
                WM_CONTEXT_READY => update_context(),
                WM_TIMER => {
                    handle_safety_timer();
                    if let Some(worker_failure) = poll_worker_failure() {
                        failure = Some(worker_failure);
                        break;
                    }
                }
                _ => {
                    DispatchMessageW(&msg);
                }
            }
        }

        KillTimer(std::ptr::null_mut(), SAFETY_TIMER_ID);
        UnhookWindowsHookEx(hook);
        failure = failure.or_else(shutdown_hook_state);
        debug!("WH_MOUSE_LL hook removed");
        info!("hook thread stopped (tid={tid})");
        failure.map_or(Ok(()), Err)
    }
}

fn poll_worker_failure() -> Option<HookFailure> {
    HOOK_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        let state = state.as_mut()?;
        if state
            .context_handle
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
        {
            return Some(HookFailure::new("context", "terminated unexpectedly"));
        }
        if !state.renderer.has_failed() {
            return None;
        }
        state.owner.renderer_terminated();
        while let Some(work) = state.owner.pop_action() {
            execute_action_work(state, work);
        }
        Some(HookFailure::new("renderer", "terminated unexpectedly"))
    })
}

fn shutdown_hook_state() -> Option<HookFailure> {
    HOOK_STATE.with(|cell| {
        let mut state = cell.borrow_mut().take()?;
        state.owner.shutdown();
        state.context_stop.store(true, Ordering::Release);
        let context_failed = state
            .context_handle
            .take()
            .is_some_and(|handle| !matches!(handle.join(), Ok(Ok(()))));
        let renderer_failed = !state.renderer.shutdown();
        if context_failed {
            Some(HookFailure::new("context", "terminated unexpectedly"))
        } else if renderer_failed {
            Some(HookFailure::new("renderer", "terminated unexpectedly"))
        } else {
            None
        }
    })
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
        let event = to_mouse_event(w_param as u32, info.mouseData);
        let point = Point::new(info.pt.x, info.pt.y);
        let outcome =
            process_native_callback(&state.capture, &mut state.owner, event, point, info.time);
        signal_wakeups(
            outcome.action_wakeup,
            outcome.render_wakeup,
            state.owner_tid,
        );
        outcome.disposition
    });
    if disposition == Disposition::Suppress {
        1
    } else {
        CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CallbackOutcome {
    disposition: Disposition,
    action_wakeup: bool,
    render_wakeup: bool,
}

/// Synchronous Windows callback core.
///
/// Its concrete inputs expose only the atomic capture gate and fixed-capacity
/// native owner. OS queries, IPC, file I/O, logging, blocking sends, and
/// unbounded work remain outside this structural boundary.
fn process_native_callback(
    capture: &crate::capture::WindowCapture,
    owner: &mut NativeInputOwner,
    event: MouseEvent,
    point: Point,
    tick: u32,
) -> CallbackOutcome {
    if super::record_window_capture(capture, event, point) {
        return CallbackOutcome {
            disposition: Disposition::Suppress,
            action_wakeup: false,
            render_wakeup: false,
        };
    }
    let disposition = owner.callback(event, point, tick);
    let (action_wakeup, render_wakeup) = owner.take_wakeups();
    CallbackOutcome {
        disposition,
        action_wakeup,
        render_wakeup,
    }
}

fn signal_work(owner: &mut NativeInputOwner, owner_tid: u32) {
    let (actions, renderer) = owner.take_wakeups();
    signal_wakeups(actions, renderer, owner_tid);
}

fn signal_wakeups(actions: bool, renderer: bool, owner_tid: u32) {
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
                    state.renderer.try_send(
                        RendererRequest::Start {
                            generation,
                            runtime,
                        },
                        false,
                    )
                }),
                RenderWork::Point { point, .. } => state.renderer.try_send(
                    RendererRequest::Command {
                        command: OverlayCommand::TrackPoint {
                            x: point.x,
                            y: point.y,
                        },
                        lossy: true,
                    },
                    true,
                ),
                RenderWork::Label { action, .. } => state
                    .renderer
                    .try_send(RendererRequest::Label { action }, true),
                RenderWork::End { .. } => state.renderer.try_send(
                    RendererRequest::Command {
                        command: OverlayCommand::EndGesture,
                        lossy: false,
                    },
                    false,
                ),
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
) -> std::io::Result<JoinHandle<Result<(), HookFailure>>> {
    thread::Builder::new()
        .name("input-context".to_string())
        .spawn(move || {
            let mut last_signature = None;
            let mut last_publish_tick = 0_u32;
            while !stop.load(Ordering::Acquire) {
                #[cfg(debug_assertions)]
                if test_failure_marker_exists("ZG_P03C_TEST_CONTEXT_FAILURE_MARKER") {
                    return Err(HookFailure::new("context", "fault injected"));
                }
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
            Ok(())
        })
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
    let sampled_tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
    let target = top_level_window(cursor)?;
    let info = crate::window_info::get_window_info_by_hwnd(target);
    let binding_set = snapshot
        .match_windows_app(&info)
        .unwrap_or_else(|| snapshot.default_binding_set());
    let resolved_tick = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() };
    if !context_resolution_is_fresh(sampled_tick, resolved_tick) {
        return None;
    }
    Some(ContextView {
        generation: snapshot.generation(),
        binding_set,
        target: TargetToken(target as usize as u64),
        point: Point::new(cursor.x, cursor.y),
        updated_tick: sampled_tick,
    })
}

fn context_resolution_is_fresh(sampled_tick: u32, resolved_tick: u32) -> bool {
    resolved_tick.wrapping_sub(sampled_tick) <= CONTEXT_MAX_AGE_MS
}

#[cfg(debug_assertions)]
fn test_failure_marker_exists(variable: &str) -> bool {
    std::env::var_os(variable).is_some_and(|path| std::path::Path::new(&path).exists())
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
    use super::super::owner::{ACTION_CAPACITY, RENDER_CAPACITY};
    use super::*;
    use crate::config::{self, ConfigDocument, ConfigOwner};
    use crate::domain::input::tests::count_allocations;
    use crate::domain::input::TargetToken;
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

    #[test]
    fn delayed_context_resolution_is_rejected_as_stale() {
        assert!(context_resolution_is_fresh(1_000, 1_100));
        assert!(!context_resolution_is_fresh(1_000, 1_101));
        assert!(context_resolution_is_fresh(u32::MAX - 50, 49));
    }

    #[test]
    fn concurrent_context_mailbox_reads_never_observe_torn_fields() {
        let mailbox = Arc::new(ContextMailbox::new());
        let first = ContextView {
            generation: 11,
            binding_set: BindingSetId::from_index(1).unwrap(),
            target: TargetToken(101),
            point: Point::new(1_001, -1_001),
            updated_tick: 1_011,
        };
        let second = ContextView {
            generation: 22,
            binding_set: BindingSetId::from_index(2).unwrap(),
            target: TargetToken(202),
            point: Point::new(2_002, -2_002),
            updated_tick: 2_022,
        };
        mailbox.publish(Some(first));
        let writer_mailbox = Arc::clone(&mailbox);
        let writer = thread::spawn(move || {
            for index in 0..50_000 {
                writer_mailbox.publish(Some(if index % 2 == 0 { second } else { first }));
            }
        });
        for _ in 0..50_000 {
            if let Some(value) = mailbox.read() {
                let complete_first = value.generation == first.generation
                    && value.binding_set == first.binding_set
                    && value.target == first.target
                    && value.point == first.point
                    && value.updated_tick == first.updated_tick;
                let complete_second = value.generation == second.generation
                    && value.binding_set == second.binding_set
                    && value.target == second.target
                    && value.point == second.point
                    && value.updated_tick == second.updated_tick;
                assert!(complete_first || complete_second);
            }
        }
        writer.join().unwrap();
    }

    #[test]
    fn renderer_lifecycle_backpressure_never_blocks_the_hook_thread() {
        let (sender, requests) = bounded(1);
        assert!(sender.try_send(RendererRequest::Shutdown).is_ok());
        let (_status_tx, status) = bounded(1);
        let client = RendererClient {
            sender,
            status,
            handle: None,
            terminal_reserved: false,
        };
        let started = std::time::Instant::now();

        let mut client = client;
        assert!(!client.try_send(RendererRequest::Shutdown, false));
        assert!(started.elapsed() < Duration::from_millis(50));
        drop(requests);
    }

    #[test]
    fn renderer_owner_queue_reserves_terminal_during_lossy_saturation() {
        let (sender, requests) = bounded(RENDERER_QUEUE_CAPACITY);
        let (_status_tx, status) = bounded(1);
        let mut client = RendererClient {
            sender,
            status,
            handle: None,
            terminal_reserved: false,
        };
        let runtime =
            crate::config::ActiveConfig::from_document(crate::config::ConfigDocument::default())
                .unwrap()
                .runtime();

        assert!(client.try_send(
            RendererRequest::Start {
                generation: ConfigGeneration(1),
                runtime,
            },
            false,
        ));
        for index in 0..RENDERER_QUEUE_CAPACITY {
            assert!(client.try_send(
                RendererRequest::Command {
                    command: OverlayCommand::TrackPoint {
                        x: index as i32,
                        y: 0,
                    },
                    lossy: true,
                },
                true,
            ));
        }
        assert_eq!(requests.len(), RENDERER_QUEUE_CAPACITY - 1);
        assert!(client.try_send(
            RendererRequest::Command {
                command: OverlayCommand::EndGesture,
                lossy: false,
            },
            false,
        ));
        assert_eq!(requests.len(), RENDERER_QUEUE_CAPACITY);
        assert!(matches!(
            requests.try_iter().last(),
            Some(RendererRequest::Command {
                command: OverlayCommand::EndGesture,
                ..
            })
        ));
    }

    #[test]
    fn production_callback_core_is_allocation_free_bounded_and_fail_open_under_saturation() {
        let directory = tempfile::tempdir().unwrap();
        config::save_atomic(
            &config::ActiveConfig::from_document(ConfigDocument::default()).unwrap(),
            directory.path(),
        )
        .unwrap();
        let (writer, _) = ConfigOwner::startup(directory.path());
        let mut owner = NativeInputOwner::new(writer.reader());
        owner.set_context(Some(ContextView {
            generation: 1,
            binding_set: BindingSetId::from_index(0).unwrap(),
            target: TargetToken(17),
            point: Point::new(0, 0),
            updated_tick: 1,
        }));
        let capture = crate::capture::WindowCapture::new();
        let capturing = crate::capture::WindowCapture::new();
        let capture_deadline = std::time::Instant::now() + Duration::from_secs(1);
        let capture_epoch = capturing.begin(41, capture_deadline).unwrap();
        let captured = process_native_callback(
            &capturing,
            &mut owner,
            MouseEvent::ButtonDown(TriggerButton::Left),
            Point::new(12, 34),
            1,
        );
        assert_eq!(
            captured,
            CallbackOutcome {
                disposition: Disposition::Suppress,
                action_wakeup: false,
                render_wakeup: false,
            }
        );
        assert_eq!(
            capturing.poll(41, capture_epoch, capture_deadline),
            Ok(crate::capture::CapturePoll::Captured(Point::new(12, 34)))
        );

        let started = process_native_callback(
            &capture,
            &mut owner,
            MouseEvent::ButtonDown(TriggerButton::Right),
            Point::new(0, 0),
            1,
        );
        assert_eq!(started.disposition, Disposition::Suppress);
        assert!(started.action_wakeup);
        assert!(started.render_wakeup);

        let (pass_count, allocations) = count_allocations(|| {
            let mut pass_count = 0;
            for tick in 2..100_002 {
                let outcome = process_native_callback(
                    &capture,
                    &mut owner,
                    MouseEvent::MouseMove,
                    Point::new(tick as i32, -(tick as i32)),
                    tick,
                );
                pass_count += usize::from(outcome.disposition == Disposition::Pass);
            }
            pass_count
        });
        assert_eq!(pass_count, 100_000);
        assert_eq!(allocations, 0);
        assert!(std::iter::from_fn(|| owner.pop_action()).count() <= ACTION_CAPACITY);
        assert!(std::iter::from_fn(|| owner.pop_render()).count() <= RENDER_CAPACITY);
    }
}
