use std::ffi::c_void;
use std::io;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TryRecvError};

use crate::config::Action;
use crate::domain::input::SessionId;

const ACTION_MAILBOX_CAPACITY: usize = 8;
const RESULT_MAILBOX_CAPACITY: usize = 8;
const WORKER_POLL: Duration = Duration::from_millis(10);
const SHUTDOWN_WAIT: Duration = Duration::from_millis(100);
const SESSION_EVENT_TAP: u32 = 1;
const EVENT_FIELD_SOURCE_USER_DATA: u32 = 55;

type CGEventRef = *mut c_void;

#[derive(Clone, Copy)]
struct CgFunctions {
    preflight_post_access: unsafe extern "C" fn() -> bool,
    create_keyboard_event: unsafe extern "C" fn(*const c_void, u16, bool) -> CGEventRef,
    set_integer_value: unsafe extern "C" fn(CGEventRef, u32, i64),
    post_event: unsafe extern "C" fn(u32, CGEventRef),
    release: unsafe extern "C" fn(*const c_void),
}

#[cfg(target_os = "macos")]
const SYSTEM_CG_FUNCTIONS: CgFunctions = CgFunctions {
    preflight_post_access: CGPreflightPostEventAccess,
    create_keyboard_event: CGEventCreateKeyboardEvent,
    set_integer_value: CGEventSetIntegerValueField,
    post_event: CGEventPost,
    release: CFRelease,
};

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGPreflightPostEventAccess() -> bool;
    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> CGEventRef;
    fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    fn CGEventPost(tap: u32, event: CGEventRef);
}

#[cfg(target_os = "macos")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: *const c_void);
}

struct OwnedEvent {
    event: NonNull<c_void>,
    release: unsafe extern "C" fn(*const c_void),
}

impl OwnedEvent {
    unsafe fn from_create(
        event: CGEventRef,
        release: unsafe extern "C" fn(*const c_void),
    ) -> Option<Self> {
        NonNull::new(event).map(|event| Self { event, release })
    }

    fn as_ptr(&self) -> CGEventRef {
        self.event.as_ptr()
    }
}

impl Drop for OwnedEvent {
    fn drop(&mut self) {
        unsafe {
            (self.release)(self.event.as_ptr().cast_const());
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionOutcome {
    Posted,
    FailedBeforeInjection,
    FailedAfterInjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExecutorResult {
    pub(crate) session: SessionId,
    pub(crate) outcome: ExecutionOutcome,
}

struct ExecutorCommand {
    session: SessionId,
    action: Action,
    repeat: u16,
}

#[derive(Default)]
struct ExecutorKpis {
    queued: AtomicU64,
    dropped: AtomicU64,
    posted: AtomicU64,
    failed_before: AtomicU64,
    failed_after: AtomicU64,
}

#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
struct ExecutorKpiSnapshot {
    queued: u64,
    dropped: u64,
    posted: u64,
    failed_before: u64,
    failed_after: u64,
}

impl ExecutorKpis {
    fn note(&self, outcome: ExecutionOutcome) {
        match outcome {
            ExecutionOutcome::Posted => self.posted.fetch_add(1, Ordering::Relaxed),
            ExecutionOutcome::FailedBeforeInjection => {
                self.failed_before.fetch_add(1, Ordering::Relaxed)
            }
            ExecutionOutcome::FailedAfterInjection => {
                self.failed_after.fetch_add(1, Ordering::Relaxed)
            }
        };
    }

    #[cfg(test)]
    fn snapshot(&self) -> ExecutorKpiSnapshot {
        ExecutorKpiSnapshot {
            queued: self.queued.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            posted: self.posted.load(Ordering::Relaxed),
            failed_before: self.failed_before.load(Ordering::Relaxed),
            failed_after: self.failed_after.load(Ordering::Relaxed),
        }
    }
}

pub(crate) struct MacosActionExecutor {
    commands: Option<Sender<ExecutorCommand>>,
    results: Receiver<ExecutorResult>,
    stop: Arc<AtomicBool>,
    completed: Receiver<()>,
    handle: Option<JoinHandle<()>>,
    kpis: Arc<ExecutorKpis>,
}

impl MacosActionExecutor {
    #[cfg(target_os = "macos")]
    pub(crate) fn spawn(marker: i64) -> io::Result<Self> {
        Self::spawn_with(marker, |action, marker, repeat| {
            execute_with(action, marker, repeat, SYSTEM_CG_FUNCTIONS)
        })
    }

    pub(crate) fn spawn_with<P>(marker: i64, post: P) -> io::Result<Self>
    where
        P: Fn(&Action, i64, u16) -> ExecutionOutcome + Send + 'static,
    {
        let (commands, command_rx) = bounded(ACTION_MAILBOX_CAPACITY);
        let (result_tx, results) = bounded(RESULT_MAILBOX_CAPACITY);
        let (completion_tx, completed) = bounded(1);
        let stop = Arc::new(AtomicBool::new(false));
        let kpis = Arc::new(ExecutorKpis::default());
        let thread_stop = Arc::clone(&stop);
        let thread_kpis = Arc::clone(&kpis);
        let handle = thread::Builder::new()
            .name("macos-action".to_string())
            .spawn(move || {
                worker_loop(
                    marker,
                    thread_stop,
                    command_rx,
                    result_tx,
                    thread_kpis,
                    post,
                );
                let _ = completion_tx.send(());
            })?;
        Ok(Self {
            commands: Some(commands),
            results,
            stop,
            completed,
            handle: Some(handle),
            kpis,
        })
    }

    pub(crate) fn try_dispatch(&self, session: SessionId, action: Action, repeat: u16) -> bool {
        let command = ExecutorCommand {
            session,
            action,
            repeat,
        };
        let delivered = self
            .commands
            .as_ref()
            .is_some_and(|commands| commands.try_send(command).is_ok());
        if delivered {
            self.kpis.queued.fetch_add(1, Ordering::Relaxed);
        } else {
            self.kpis.dropped.fetch_add(1, Ordering::Relaxed);
        }
        delivered
    }

    pub(crate) fn poll(&self) -> Result<Option<ExecutorResult>, ()> {
        match self.results.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(()),
        }
    }

    pub(crate) fn shutdown(mut self) {
        self.stop_worker();
    }

    fn stop_worker(&mut self) {
        if self.handle.is_none() {
            return;
        }
        self.stop.store(true, Ordering::Release);
        self.commands.take();
        let completed = self.completed.recv_timeout(SHUTDOWN_WAIT).is_ok();
        if let Some(handle) = self.handle.take() {
            if completed {
                let _ = handle.join();
            } else {
                drop(handle);
            }
        }
    }
}

impl Drop for MacosActionExecutor {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

fn worker_loop<P>(
    marker: i64,
    stop: Arc<AtomicBool>,
    commands: Receiver<ExecutorCommand>,
    results: Sender<ExecutorResult>,
    kpis: Arc<ExecutorKpis>,
    post: P,
) where
    P: Fn(&Action, i64, u16) -> ExecutionOutcome,
{
    while !stop.load(Ordering::Acquire) {
        let command = match commands.recv_timeout(WORKER_POLL) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let outcome = post(&command.action, marker, command.repeat);
        kpis.note(outcome);
        if results
            .send(ExecutorResult {
                session: command.session,
                outcome,
            })
            .is_err()
        {
            break;
        }
    }
    log::info!(
        "macOS action worker stopped (queued={}, dropped={}, posted={}, failed_before={}, failed_after={})",
        kpis.queued.load(Ordering::Relaxed),
        kpis.dropped.load(Ordering::Relaxed),
        kpis.posted.load(Ordering::Relaxed),
        kpis.failed_before.load(Ordering::Relaxed),
        kpis.failed_after.load(Ordering::Relaxed),
    );
}

fn execute_with(
    action: &Action,
    marker: i64,
    repeat: u16,
    functions: CgFunctions,
) -> ExecutionOutcome {
    let Action::Keyboard { keys } = action;
    let Some(key_codes) = keys
        .iter()
        .map(|key| macos_key_code(key))
        .collect::<Option<Vec<_>>>()
    else {
        return ExecutionOutcome::FailedBeforeInjection;
    };
    if repeat == 0 || unsafe { !(functions.preflight_post_access)() } {
        return ExecutionOutcome::FailedBeforeInjection;
    }
    for completed in 0..repeat {
        let Some(events) = (unsafe { create_key_events(&key_codes, marker, functions) }) else {
            return if completed == 0 {
                ExecutionOutcome::FailedBeforeInjection
            } else {
                ExecutionOutcome::FailedAfterInjection
            };
        };
        for event in &events {
            unsafe {
                (functions.post_event)(SESSION_EVENT_TAP, event.as_ptr());
            }
        }
    }
    ExecutionOutcome::Posted
}

unsafe fn create_key_events(
    key_codes: &[u16],
    marker: i64,
    functions: CgFunctions,
) -> Option<Vec<OwnedEvent>> {
    let mut events = Vec::with_capacity(key_codes.len() * 2);
    for (&key_code, key_down) in key_codes
        .iter()
        .map(|key| (key, true))
        .chain(key_codes.iter().rev().map(|key| (key, false)))
    {
        let event = OwnedEvent::from_create(
            (functions.create_keyboard_event)(std::ptr::null(), key_code, key_down),
            functions.release,
        )?;
        (functions.set_integer_value)(event.as_ptr(), EVENT_FIELD_SOURCE_USER_DATA, marker);
        events.push(event);
    }
    Some(events)
}

fn macos_key_code(key: &str) -> Option<u16> {
    Some(match key {
        "a" => 0x00,
        "s" => 0x01,
        "d" => 0x02,
        "f" => 0x03,
        "h" => 0x04,
        "g" => 0x05,
        "z" => 0x06,
        "x" => 0x07,
        "c" => 0x08,
        "v" => 0x09,
        "b" => 0x0b,
        "q" => 0x0c,
        "w" => 0x0d,
        "e" => 0x0e,
        "r" => 0x0f,
        "y" => 0x10,
        "t" => 0x11,
        "1" => 0x12,
        "2" => 0x13,
        "3" => 0x14,
        "4" => 0x15,
        "6" => 0x16,
        "5" => 0x17,
        "9" => 0x19,
        "7" => 0x1a,
        "8" => 0x1c,
        "0" => 0x1d,
        "o" => 0x1f,
        "u" => 0x20,
        "i" => 0x22,
        "p" => 0x23,
        "enter" => 0x24,
        "l" => 0x25,
        "j" => 0x26,
        "k" => 0x28,
        "n" => 0x2d,
        "m" => 0x2e,
        "tab" => 0x30,
        "space" => 0x31,
        "backspace" => 0x33,
        "escape" => 0x35,
        "command" => 0x37,
        "shift" => 0x38,
        "option" => 0x3a,
        "ctrl" => 0x3b,
        "f17" => 0x40,
        "f18" => 0x4f,
        "f19" => 0x50,
        "f20" => 0x5a,
        "f5" => 0x60,
        "f6" => 0x61,
        "f7" => 0x62,
        "f3" => 0x63,
        "f8" => 0x64,
        "f9" => 0x65,
        "f11" => 0x67,
        "f13" => 0x69,
        "f16" => 0x6a,
        "f14" => 0x6b,
        "f10" => 0x6d,
        "f12" => 0x6f,
        "f15" => 0x71,
        "home" => 0x73,
        "pageup" => 0x74,
        "delete" => 0x75,
        "f4" => 0x76,
        "end" => 0x77,
        "f2" => 0x78,
        "pagedown" => 0x79,
        "f1" => 0x7a,
        "left" => 0x7b,
        "right" => 0x7c,
        "down" => 0x7d,
        "up" => 0x7e,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum RecordedCall {
        Create(u16, bool, usize),
        Tag(usize, u32, i64),
        Post(u32, usize),
        Release(usize),
    }

    thread_local! {
        static CALLS: RefCell<Vec<RecordedCall>> = const { RefCell::new(Vec::new()) };
        static CREATE_COUNT: Cell<usize> = const { Cell::new(0) };
        static NULL_AT: Cell<usize> = const { Cell::new(usize::MAX) };
    }

    unsafe extern "C" fn preflight_allowed() -> bool {
        true
    }

    unsafe extern "C" fn preflight_denied() -> bool {
        false
    }

    unsafe extern "C" fn record_create(_: *const c_void, key: u16, down: bool) -> CGEventRef {
        let call = CREATE_COUNT.get();
        CREATE_COUNT.set(call + 1);
        if call == NULL_AT.get() {
            return std::ptr::null_mut();
        }
        let event = Box::into_raw(Box::new(call as u64)).cast::<c_void>();
        CALLS.with(|calls| {
            calls
                .borrow_mut()
                .push(RecordedCall::Create(key, down, call));
        });
        event
    }

    unsafe extern "C" fn record_tag(event: CGEventRef, field: u32, marker: i64) {
        let id = *(event.cast::<u64>()) as usize;
        CALLS.with(|calls| {
            calls
                .borrow_mut()
                .push(RecordedCall::Tag(id, field, marker));
        });
    }

    unsafe extern "C" fn record_post(tap: u32, event: CGEventRef) {
        let id = *(event.cast::<u64>()) as usize;
        CALLS.with(|calls| calls.borrow_mut().push(RecordedCall::Post(tap, id)));
    }

    unsafe extern "C" fn record_release(event: *const c_void) {
        let id = *(event.cast::<u64>()) as usize;
        CALLS.with(|calls| calls.borrow_mut().push(RecordedCall::Release(id)));
        drop(Box::from_raw(event.cast_mut().cast::<u64>()));
    }

    fn functions(preflight: unsafe extern "C" fn() -> bool) -> CgFunctions {
        CgFunctions {
            preflight_post_access: preflight,
            create_keyboard_event: record_create,
            set_integer_value: record_tag,
            post_event: record_post,
            release: record_release,
        }
    }

    fn reset_calls(null_at: usize) {
        CREATE_COUNT.set(0);
        NULL_AT.set(null_at);
        CALLS.with(|calls| calls.borrow_mut().clear());
    }

    fn keyboard(keys: &[&str]) -> Action {
        Action::Keyboard {
            keys: keys.iter().map(|key| (*key).to_string()).collect(),
        }
    }

    #[test]
    fn generated_keyboard_events_all_carry_process_marker() {
        reset_calls(usize::MAX);

        assert_eq!(
            execute_with(
                &keyboard(&["command", "a"]),
                0x1234,
                1,
                functions(preflight_allowed),
            ),
            ExecutionOutcome::Posted
        );

        let calls = CALLS.with(|calls| calls.borrow().clone());
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(
                    call,
                    RecordedCall::Tag(_, EVENT_FIELD_SOURCE_USER_DATA, 0x1234)
                ))
                .count(),
            4
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, RecordedCall::Post(SESSION_EVENT_TAP, _)))
                .count(),
            4
        );
    }

    #[test]
    fn keyboard_action_posts_key_downs_then_reverse_key_ups() {
        reset_calls(usize::MAX);

        assert_eq!(
            execute_with(
                &keyboard(&["command", "a"]),
                7,
                1,
                functions(preflight_allowed),
            ),
            ExecutionOutcome::Posted
        );

        let calls = CALLS.with(|calls| calls.borrow().clone());
        let created = calls
            .into_iter()
            .filter_map(|call| match call {
                RecordedCall::Create(key, down, _) => Some((key, down)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            created,
            [(0x37, true), (0x00, true), (0x00, false), (0x37, false)]
        );
    }

    #[test]
    fn unavailable_post_access_fails_before_injection() {
        reset_calls(usize::MAX);

        assert_eq!(
            execute_with(&keyboard(&["a"]), 7, 1, functions(preflight_denied),),
            ExecutionOutcome::FailedBeforeInjection
        );
        assert!(CALLS.with(|calls| calls.borrow().is_empty()));
    }

    #[test]
    fn nullable_event_creation_releases_only_owned_events_and_posts_nothing() {
        reset_calls(1);

        assert_eq!(
            execute_with(
                &keyboard(&["command", "a"]),
                7,
                1,
                functions(preflight_allowed),
            ),
            ExecutionOutcome::FailedBeforeInjection
        );

        let calls = CALLS.with(|calls| calls.borrow().clone());
        assert!(calls.contains(&RecordedCall::Release(0)));
        assert!(!calls
            .iter()
            .any(|call| matches!(call, RecordedCall::Post(_, _))));
    }

    #[test]
    fn later_repeat_generation_failure_is_failed_after_injection() {
        reset_calls(2);

        assert_eq!(
            execute_with(&keyboard(&["a"]), 7, 2, functions(preflight_allowed),),
            ExecutionOutcome::FailedAfterInjection
        );

        let calls = CALLS.with(|calls| calls.borrow().clone());
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, RecordedCall::Post(SESSION_EVENT_TAP, _)))
                .count(),
            2
        );
    }

    #[test]
    fn unsupported_macos_keys_fail_before_generating_an_event() {
        for key in ["f21", "f22", "f23", "f24", "unknown"] {
            reset_calls(usize::MAX);
            assert_eq!(
                execute_with(&keyboard(&[key]), 7, 1, functions(preflight_allowed),),
                ExecutionOutcome::FailedBeforeInjection
            );
            assert!(CALLS.with(|calls| calls.borrow().is_empty()));
        }
    }

    #[test]
    fn executor_mailbox_overload_is_bounded_and_preserves_fifo_order() {
        let blocked = Arc::new(AtomicBool::new(false));
        let release_block = Arc::new(AtomicBool::new(false));
        let worker_blocked = Arc::clone(&blocked);
        let worker_release = Arc::clone(&release_block);
        let executor = MacosActionExecutor::spawn_with(7, move |_, _, _| {
            worker_blocked.store(true, Ordering::Release);
            while !worker_release.load(Ordering::Acquire) {
                thread::yield_now();
            }
            ExecutionOutcome::Posted
        })
        .unwrap();

        assert!(executor.try_dispatch(SessionId(0), keyboard(&["a"]), 1));
        while !blocked.load(Ordering::Acquire) {
            thread::yield_now();
        }
        for session in 1..=ACTION_MAILBOX_CAPACITY as u64 {
            assert!(executor.try_dispatch(SessionId(session), keyboard(&["a"]), 1));
        }
        assert!(!executor.try_dispatch(SessionId(99), keyboard(&["a"]), 1));
        release_block.store(true, Ordering::Release);

        let mut sessions = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(1);
        while sessions.len() < ACTION_MAILBOX_CAPACITY + 1 && Instant::now() < deadline {
            if let Ok(Some(result)) = executor.poll() {
                sessions.push(result.session.0);
            } else {
                thread::yield_now();
            }
        }
        assert_eq!(
            sessions,
            (0..=ACTION_MAILBOX_CAPACITY as u64).collect::<Vec<_>>()
        );
        assert_eq!(executor.kpis.snapshot().dropped, 1);
        executor.shutdown();
    }

    #[test]
    fn worker_stop_rejects_new_actions_without_blocking_input_owner() {
        let executor =
            MacosActionExecutor::spawn_with(7, |_, _, _| panic!("injected worker stop")).unwrap();
        assert!(executor.try_dispatch(SessionId(1), keyboard(&["a"]), 1));
        let deadline = Instant::now() + Duration::from_secs(1);
        while executor.poll().is_ok() && Instant::now() < deadline {
            thread::yield_now();
        }

        let started = Instant::now();
        let mut rejected = false;
        while Instant::now() < deadline {
            if !executor.try_dispatch(SessionId(2), keyboard(&["a"]), 1) {
                rejected = true;
                break;
            }
            thread::yield_now();
        }
        assert!(rejected);
        assert!(started.elapsed() < Duration::from_millis(20));
        executor.shutdown();
    }

    #[test]
    fn executor_shutdown_is_bounded_while_os_leaf_is_in_flight() {
        let blocked = Arc::new(AtomicBool::new(false));
        let release_block = Arc::new(AtomicBool::new(false));
        let worker_blocked = Arc::clone(&blocked);
        let worker_release = Arc::clone(&release_block);
        let executor = MacosActionExecutor::spawn_with(7, move |_, _, _| {
            worker_blocked.store(true, Ordering::Release);
            while !worker_release.load(Ordering::Acquire) {
                thread::yield_now();
            }
            ExecutionOutcome::Posted
        })
        .unwrap();
        assert!(executor.try_dispatch(SessionId(1), keyboard(&["a"]), 1));
        while !blocked.load(Ordering::Acquire) {
            thread::yield_now();
        }

        let started = Instant::now();
        executor.shutdown();
        assert!(started.elapsed() < Duration::from_millis(200));
        release_block.store(true, Ordering::Release);
    }
}
