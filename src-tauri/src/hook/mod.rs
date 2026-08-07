//! Low-level mouse hook thread.
//!
//! The hook reads prevalidated immutable runtime snapshots from the Engine
//! publication reader. An active gesture pins one generation while a context
//! worker resolves window/application state outside the callback.

#[cfg(any(target_os = "macos", test))]
mod macos;
mod owner;
#[cfg(windows)]
mod win32;

use std::io;
#[cfg(any(windows, target_os = "macos"))]
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::Ordering;
use std::sync::atomic::{AtomicBool, AtomicU32};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver};
use log::info;
#[cfg(not(any(windows, target_os = "macos")))]
use log::warn;

use crate::config::ConfigSnapshotReader;
use crate::domain::{MouseEvent, Point};

pub(crate) fn record_window_capture(
    capture: &crate::capture::WindowCapture,
    event: MouseEvent,
    point: Point,
) -> bool {
    capture.try_record(event, point)
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HookFailure {
    worker: &'static str,
    reason: &'static str,
}

impl HookFailure {
    pub(super) const fn new(worker: &'static str, reason: &'static str) -> Self {
        Self { worker, reason }
    }
}

impl std::fmt::Display for HookFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} worker {}", self.worker, self.reason)
    }
}

pub(crate) enum HookEvent {
    Ready(u32),
    Fatal(HookFailure),
}

type HookSpawn = (
    Arc<AtomicU32>,
    Arc<AtomicBool>,
    JoinHandle<()>,
    Receiver<HookEvent>,
);

/// Spawns the native input owner with the Engine publication reader.
pub fn spawn(
    reader: ConfigSnapshotReader,
    capture: Arc<crate::capture::WindowCapture>,
) -> io::Result<HookSpawn> {
    info!("starting hook thread");
    let tid = Arc::new(AtomicU32::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let (event_tx, event_rx) = bounded(1);

    let handle = thread::Builder::new()
        .name("hook-thread".to_string())
        .spawn(move || {
            #[cfg(windows)]
            let _ = thread_stop;
            #[cfg(windows)]
            let result = {
                panic::catch_unwind(AssertUnwindSafe(|| {
                    win32::run_loop_win32(reader, capture, event_tx.clone())
                }))
                .unwrap_or_else(|_| Err(HookFailure::new("hook", "panicked")))
            };
            #[cfg(target_os = "macos")]
            let result = {
                let _ = capture;
                panic::catch_unwind(AssertUnwindSafe(|| {
                    macos::run_loop_macos(reader, thread_stop, event_tx.clone())
                }))
                .unwrap_or_else(|_| Err(HookFailure::new("event tap", "panicked")))
            };
            #[cfg(not(any(windows, target_os = "macos")))]
            let result = {
                let _ = reader;
                let _ = capture;
                let _ = thread_stop;
                warn!("Mouse hook is only supported on Windows");
                let _ = event_tx.send(HookEvent::Ready(1));
                Ok(())
            };
            if let Err(failure) = result {
                let _ = event_tx.send(HookEvent::Fatal(failure));
            }
        })?;
    match event_rx.recv_timeout(Duration::from_secs(1)) {
        Ok(HookEvent::Ready(thread_id)) => {
            tid.store(thread_id, Ordering::Release);
            info!("hook thread spawned");
            Ok((tid, stop, handle, event_rx))
        }
        Ok(HookEvent::Fatal(failure)) => {
            let _ = handle.join();
            Err(io::Error::other(failure.to_string()))
        }
        Err(crossbeam_channel::RecvTimeoutError::Timeout) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "native input owner readiness timed out",
        )),
        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
            let _ = handle.join();
            Err(io::Error::other(
                "native input owner exited before publishing readiness",
            ))
        }
    }
}
