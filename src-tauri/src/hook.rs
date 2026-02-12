use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};

use crate::overlay::OverlayCommand;
use crate::SharedConfig;

pub enum HookControl {
    Shutdown,
}

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
