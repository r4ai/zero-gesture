//! Low-level mouse hook thread.
//!
//! The hook consumes one immutable, prevalidated runtime configuration. JSON
//! decoding, semantic validation, application selector compilation, and
//! binding compilation happen in [`crate::config`] before this thread starts.

mod owner;
#[cfg(windows)]
mod win32;

use std::io;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use log::info;
#[cfg(not(windows))]
use log::warn;

use crate::config::ConfigSnapshotReader;

/// Spawns the native input owner with the Engine publication reader.
pub fn spawn(reader: ConfigSnapshotReader) -> io::Result<(Arc<AtomicU32>, JoinHandle<()>)> {
    info!("starting hook thread");
    let tid = Arc::new(AtomicU32::new(0));
    let tid_clone = tid.clone();

    let handle = thread::Builder::new()
        .name("hook-thread".to_string())
        .spawn(move || {
            #[cfg(windows)]
            win32::run_loop_win32(reader, tid_clone);
            #[cfg(not(windows))]
            {
                let _ = (reader, tid_clone);
                warn!("Mouse hook is only supported on Windows");
            }
        })?;
    for _ in 0..1_000 {
        if tid.load(Ordering::Acquire) != 0 {
            info!("hook thread spawned");
            return Ok((tid, handle));
        }
        if handle.is_finished() {
            let _ = handle.join();
            return Err(io::Error::other(
                "native input owner exited before publishing readiness",
            ));
        }
        thread::sleep(Duration::from_millis(1));
    }
    drop(handle);
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "native input owner readiness timed out",
    ))
}
