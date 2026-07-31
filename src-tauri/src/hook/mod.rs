//! Low-level mouse hook thread.
//!
//! The hook reads prevalidated immutable runtime snapshots from the Engine
//! publication reader. An active gesture pins one generation while a context
//! worker resolves window/application state outside the callback.

mod owner;
#[cfg(windows)]
mod win32;

use std::io;
use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{bounded, Receiver};
use log::info;
#[cfg(not(windows))]
use log::warn;

use crate::config::ConfigSnapshotReader;

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

/// Spawns the native input owner with the Engine publication reader.
pub fn spawn(
    reader: ConfigSnapshotReader,
) -> io::Result<(Arc<AtomicU32>, JoinHandle<()>, Receiver<HookEvent>)> {
    info!("starting hook thread");
    let tid = Arc::new(AtomicU32::new(0));
    let (event_tx, event_rx) = bounded(1);

    let handle = thread::Builder::new()
        .name("hook-thread".to_string())
        .spawn(move || {
            #[cfg(windows)]
            let result = panic::catch_unwind(AssertUnwindSafe(|| {
                win32::run_loop_win32(reader, event_tx.clone())
            }))
            .unwrap_or_else(|_| Err(HookFailure::new("hook", "panicked")));
            #[cfg(not(windows))]
            {
                let _ = reader;
                warn!("Mouse hook is only supported on Windows");
                let _ = event_tx.send(HookEvent::Ready(1));
                let result = Ok(());
            }
            if let Err(failure) = result {
                let _ = event_tx.send(HookEvent::Fatal(failure));
            }
        })?;
    match event_rx.recv_timeout(Duration::from_secs(1)) {
        Ok(HookEvent::Ready(thread_id)) => {
            tid.store(thread_id, Ordering::Release);
            info!("hook thread spawned");
            Ok((tid, handle, event_rx))
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
