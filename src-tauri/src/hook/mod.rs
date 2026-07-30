//! Low-level mouse hook thread.
//!
//! The hook consumes one immutable, prevalidated runtime configuration. JSON
//! decoding, semantic validation, application selector compilation, and
//! binding compilation happen in [`crate::config`] before this thread starts.

#[cfg(windows)]
mod win32;

use std::sync::atomic::AtomicU32;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::Sender;
use log::info;
#[cfg(not(windows))]
use log::warn;

use crate::config::RuntimeConfig;
use crate::overlay::OverlayCommand;

/// Messages sent from the main thread to the hook thread.
pub enum HookControl {
    /// Request the hook thread to stop and exit.
    Shutdown,
}

/// Spawns the hook thread with a compiled immutable configuration snapshot.
pub fn spawn(
    runtime: Arc<RuntimeConfig>,
    overlay_tx: Sender<OverlayCommand>,
) -> (Sender<HookControl>, Arc<AtomicU32>, JoinHandle<()>) {
    info!("starting hook thread");
    let (control_tx, control_rx) = crossbeam_channel::unbounded();
    let tid = Arc::new(AtomicU32::new(0));
    let tid_clone = tid.clone();

    let handle = thread::Builder::new()
        .name("hook-thread".to_string())
        .spawn(move || {
            #[cfg(windows)]
            win32::run_loop_win32(runtime, overlay_tx, tid_clone, control_rx);
            #[cfg(not(windows))]
            {
                let _ = (runtime, overlay_tx, tid_clone, control_rx);
                warn!("Mouse hook is only supported on Windows");
            }
        })
        .expect("failed to spawn hook thread");
    info!("hook thread spawned");

    (control_tx, tid, handle)
}
