//! Gesture overlay thread.
//!
//! Manages a transparent overlay window that draws the mouse gesture trail.
//! Commands are received via a channel from the hook thread.

use std::thread::{self, JoinHandle};

use crossbeam_channel::{unbounded, Receiver, Sender};

/// Commands sent to the overlay thread to control the gesture trail.
///
/// # Examples
///
/// ```
/// use zero_gesture_lib::overlay::OverlayCommand;
///
/// let cmd = OverlayCommand::TrackPoint { x: 100, y: 200 };
/// println!("{cmd:?}");
/// ```
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum OverlayCommand {
    /// Begin a new gesture — show the overlay window.
    StartGesture,
    /// Append a point to the current gesture trail.
    TrackPoint {
        /// Screen X coordinate in pixels.
        x: i32,
        /// Screen Y coordinate in pixels.
        y: i32,
    },
    /// End the current gesture — hide the overlay and clear the trail.
    EndGesture,
    /// Shut down the overlay thread.
    Shutdown,
}

/// Spawns the overlay thread and returns a channel sender and a join handle.
///
/// Send [`OverlayCommand`] messages through the returned [`Sender`] to
/// control the overlay. The [`JoinHandle`] can be used to wait for the
/// thread to finish.
///
/// # Examples
///
/// ```no_run
/// use zero_gesture_lib::overlay::{self, OverlayCommand};
///
/// let (tx, handle) = overlay::spawn();
/// tx.send(OverlayCommand::Shutdown).unwrap();
/// handle.join().unwrap();
/// ```
pub fn spawn() -> (Sender<OverlayCommand>, JoinHandle<()>) {
    let (overlay_tx, overlay_rx) = unbounded();
    let handle = thread::Builder::new()
        .name("overlay-thread".to_string())
        .spawn(move || run_loop(overlay_rx))
        .expect("failed to spawn overlay thread");

    (overlay_tx, handle)
}

/// Main loop for the overlay thread.
///
/// Blocks on `overlay_rx` and processes each [`OverlayCommand`].
/// Exits when [`OverlayCommand::Shutdown`] is received or the channel is
/// disconnected.
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
