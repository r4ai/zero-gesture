use std::thread::{self, JoinHandle};

use crossbeam_channel::{unbounded, Receiver, Sender};

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum OverlayCommand {
    StartGesture,
    TrackPoint { x: i32, y: i32 },
    EndGesture,
    Shutdown,
}

pub fn spawn() -> (Sender<OverlayCommand>, JoinHandle<()>) {
    let (overlay_tx, overlay_rx) = unbounded();
    let handle = thread::Builder::new()
        .name("overlay-thread".to_string())
        .spawn(move || run_loop(overlay_rx))
        .expect("failed to spawn overlay thread");

    (overlay_tx, handle)
}

fn run_loop(overlay_rx: Receiver<OverlayCommand>) {
    while let Ok(command) = overlay_rx.recv() {
        match command {
            OverlayCommand::StartGesture => {
                // TODO: Show transparent overlay window.
            }
            OverlayCommand::TrackPoint { x: _, y: _ } => {
                // TODO: Draw trail point.
            }
            OverlayCommand::EndGesture => {
                // TODO: Hide overlay and clear trail.
            }
            OverlayCommand::Shutdown => break,
        }
    }
}
