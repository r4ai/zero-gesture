use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver, RecvTimeoutError, Sender, TryRecvError};

use crate::config::Action;
use crate::domain::input::SessionId;
use crate::domain::{Point, TriggerButton};

mod keymap;
#[cfg(target_os = "macos")]
mod native;

const ACTION_MAILBOX_CAPACITY: usize = 8;
const RESULT_MAILBOX_CAPACITY: usize = 8;
const WORKER_POLL: Duration = Duration::from_millis(10);
const SHUTDOWN_WAIT: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionOutcome {
    Posted,
    FailedBeforeInjection,
    FailedAfterInjection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExecutorResult {
    pub(crate) session: SessionId,
    pub(crate) kind: ExecutorWorkKind,
    pub(crate) outcome: ExecutionOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutorWorkKind {
    Action,
    Replay,
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) enum ExecutorWork {
    Keyboard {
        action: Action,
        repeat: u16,
    },
    Replay {
        trigger: TriggerButton,
        down_at: Point,
        up_at: Point,
    },
}

impl ExecutorWork {
    fn kind(&self) -> ExecutorWorkKind {
        match self {
            Self::Keyboard { .. } => ExecutorWorkKind::Action,
            Self::Replay { .. } => ExecutorWorkKind::Replay,
        }
    }
}

struct ExecutorCommand {
    session: SessionId,
    work: ExecutorWork,
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
    completed: Receiver<()>,
    handle: Option<JoinHandle<()>>,
    kpis: Arc<ExecutorKpis>,
}

impl MacosActionExecutor {
    #[cfg(target_os = "macos")]
    pub(crate) fn spawn(marker: i64) -> io::Result<Self> {
        Self::spawn_with(marker, execute)
    }

    pub(crate) fn spawn_with<P>(marker: i64, post: P) -> io::Result<Self>
    where
        P: Fn(&ExecutorWork, i64) -> ExecutionOutcome + Send + 'static,
    {
        let (commands, command_rx) = bounded(ACTION_MAILBOX_CAPACITY);
        let (result_tx, results) = bounded(RESULT_MAILBOX_CAPACITY);
        let (completion_tx, completed) = bounded(1);
        let kpis = Arc::new(ExecutorKpis::default());
        let thread_kpis = Arc::clone(&kpis);
        let handle = thread::Builder::new()
            .name("macos-action".to_string())
            .spawn(move || {
                worker_loop(marker, command_rx, result_tx, thread_kpis, post);
                let _ = completion_tx.send(());
            })?;
        Ok(Self {
            commands: Some(commands),
            results,
            completed,
            handle: Some(handle),
            kpis,
        })
    }

    pub(crate) fn try_dispatch(&self, session: SessionId, action: Action, repeat: u16) -> bool {
        let command = ExecutorCommand {
            session,
            work: ExecutorWork::Keyboard { action, repeat },
        };
        self.try_send(command)
    }

    pub(crate) fn try_replay(
        &self,
        session: SessionId,
        trigger: TriggerButton,
        down_at: Point,
        up_at: Point,
    ) -> bool {
        self.try_send(ExecutorCommand {
            session,
            work: ExecutorWork::Replay {
                trigger,
                down_at,
                up_at,
            },
        })
    }

    fn try_send(&self, command: ExecutorCommand) -> bool {
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
    commands: Receiver<ExecutorCommand>,
    results: Sender<ExecutorResult>,
    kpis: Arc<ExecutorKpis>,
    post: P,
) where
    P: Fn(&ExecutorWork, i64) -> ExecutionOutcome,
{
    loop {
        let command = match commands.recv_timeout(WORKER_POLL) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let kind = command.work.kind();
        let outcome = post(&command.work, marker);
        kpis.note(outcome);
        if results
            .send(ExecutorResult {
                session: command.session,
                kind,
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

#[cfg(target_os = "macos")]
fn execute(work: &ExecutorWork, marker: i64) -> ExecutionOutcome {
    match work {
        ExecutorWork::Keyboard { action, repeat } => execute_with(
            action,
            marker,
            *repeat,
            native::post_access_allowed,
            native::create_tag_and_post_repeat,
        ),
        ExecutorWork::Replay {
            trigger,
            down_at,
            up_at,
        } => execute_replay_with(
            *trigger,
            *down_at,
            *up_at,
            marker,
            native::post_access_allowed,
            native::create_tag_and_post_replay,
        ),
    }
}

fn execute_replay_with<A, P>(
    trigger: TriggerButton,
    down_at: Point,
    up_at: Point,
    marker: i64,
    post_access_allowed: A,
    post_replay: P,
) -> ExecutionOutcome
where
    A: FnOnce() -> bool,
    P: FnOnce(TriggerButton, Point, Point, i64) -> bool,
{
    if !post_access_allowed() || !post_replay(trigger, down_at, up_at, marker) {
        ExecutionOutcome::FailedBeforeInjection
    } else {
        ExecutionOutcome::Posted
    }
}

fn execute_with<A, P>(
    action: &Action,
    marker: i64,
    repeat: u16,
    post_access_allowed: A,
    mut post_repeat: P,
) -> ExecutionOutcome
where
    A: FnOnce() -> bool,
    P: FnMut(&[u16], i64) -> bool,
{
    let Action::Keyboard { keys } = action;
    let Some(key_codes) = keys
        .iter()
        .map(|key| keymap::macos_key_code(key))
        .collect::<Option<Vec<_>>>()
    else {
        return ExecutionOutcome::FailedBeforeInjection;
    };
    if repeat == 0 || !post_access_allowed() {
        return ExecutionOutcome::FailedBeforeInjection;
    }
    for completed in 0..repeat {
        if !post_repeat(&key_codes, marker) {
            return if completed == 0 {
                ExecutionOutcome::FailedBeforeInjection
            } else {
                ExecutionOutcome::FailedAfterInjection
            };
        }
    }
    ExecutionOutcome::Posted
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use super::*;

    fn keyboard(keys: &[&str]) -> Action {
        Action::Keyboard {
            keys: keys.iter().map(|key| (*key).to_string()).collect(),
        }
    }

    #[test]
    fn unavailable_post_access_fails_before_injection() {
        let post_called = Cell::new(false);

        assert_eq!(
            execute_with(
                &keyboard(&["a"]),
                7,
                1,
                || false,
                |_, _| {
                    post_called.set(true);
                    true
                },
            ),
            ExecutionOutcome::FailedBeforeInjection
        );
        assert!(!post_called.get());
    }

    #[test]
    fn replay_permission_or_creation_failure_is_before_injection() {
        let down = Point::new(1, 2);
        let up = Point::new(3, 4);
        for (access, created) in [(false, true), (true, false)] {
            assert_eq!(
                execute_replay_with(
                    TriggerButton::Right,
                    down,
                    up,
                    7,
                    || access,
                    |trigger, actual_down, actual_up, marker| {
                        assert_eq!(
                            (trigger, actual_down, actual_up, marker),
                            (TriggerButton::Right, down, up, 7)
                        );
                        created
                    },
                ),
                ExecutionOutcome::FailedBeforeInjection
            );
        }
    }

    #[test]
    fn later_repeat_generation_failure_is_failed_after_injection() {
        let repeats = Cell::new(0);

        assert_eq!(
            execute_with(
                &keyboard(&["a"]),
                7,
                2,
                || true,
                |_, _| {
                    repeats.set(repeats.get() + 1);
                    repeats.get() == 1
                },
            ),
            ExecutionOutcome::FailedAfterInjection
        );
        assert_eq!(repeats.get(), 2);
    }

    #[test]
    fn unsupported_macos_keys_fail_before_generating_an_event() {
        for key in ["f21", "f22", "f23", "f24", "unknown"] {
            let preflight_called = Cell::new(false);
            let post_called = Cell::new(false);
            assert_eq!(
                execute_with(
                    &keyboard(&[key]),
                    7,
                    1,
                    || {
                        preflight_called.set(true);
                        true
                    },
                    |_, _| {
                        post_called.set(true);
                        true
                    },
                ),
                ExecutionOutcome::FailedBeforeInjection
            );
            assert!(!preflight_called.get());
            assert!(!post_called.get());
        }
    }

    #[test]
    fn executor_mailbox_overload_is_bounded_and_preserves_fifo_order() {
        let blocked = Arc::new(AtomicBool::new(false));
        let release_block = Arc::new(AtomicBool::new(false));
        let worker_blocked = Arc::clone(&blocked);
        let worker_release = Arc::clone(&release_block);
        let executor = MacosActionExecutor::spawn_with(7, move |_, _| {
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
            MacosActionExecutor::spawn_with(7, |_, _| panic!("injected worker stop")).unwrap();
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
        let executor = MacosActionExecutor::spawn_with(7, move |_, _| {
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

        executor.shutdown();
        assert!(!release_block.load(Ordering::Acquire));
        assert_eq!(SHUTDOWN_WAIT, Duration::from_millis(100));
        release_block.store(true, Ordering::Release);
    }
}
