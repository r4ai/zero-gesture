//! Low-level mouse hook thread.
//!
//! Runs a dedicated thread that will install a Windows mouse hook
//! (`SetWindowsHookExW`) and pump the Win32 message loop. Currently
//! the hook logic is a TODO stub.

use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};

use crate::overlay::OverlayCommand;
use crate::SharedConfig;

/// Messages sent from the main thread to the hook thread.
pub enum HookControl {
    /// Request the hook thread to stop and exit.
    Shutdown,
}

/// Spawns the hook thread and returns a channel sender and a join handle.
///
/// The returned [`Sender`] can be used to send [`HookControl`] messages
/// (e.g. [`HookControl::Shutdown`]) to the thread. The [`JoinHandle`]
/// can be used to wait for the thread to finish.
///
/// # Examples
///
/// ```no_run
/// use mouse_gesture_lib::config::AppConfig;
/// use mouse_gesture_lib::SharedConfig;
/// # // hook::spawn is crate-private, so this is illustrative.
/// ```
pub fn spawn(
    shared_config: SharedConfig,
    overlay_tx: Sender<OverlayCommand>,
) -> (Sender<HookControl>, JoinHandle<()>) {
    let (control_tx, control_rx) = unbounded();
    let handle = thread::Builder::new()
        .name("hook-thread".to_string())
        .spawn(move || run_loop(shared_config, overlay_tx, control_rx))
        .expect("failed to spawn hook thread");

    (control_tx, handle)
}

/// Main loop for the hook thread.
///
/// Polls `control_rx` with a 100 ms timeout. When a [`HookControl::Shutdown`]
/// message is received (or the channel disconnects), the loop exits.
fn run_loop(
    shared_config: SharedConfig,
    overlay_tx: Sender<OverlayCommand>,
    control_rx: Receiver<HookControl>,
) {
    let _ = shared_config;
    let _ = overlay_tx;

    loop {
        match control_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(HookControl::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                // TODO: SetWindowsHookExW + Win32 message loop.
            }
        }
    }
}
