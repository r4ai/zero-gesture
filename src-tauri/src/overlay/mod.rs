//! Gesture overlay thread.
//!
//! Manages a transparent overlay window that draws the mouse gesture trail
//! using a pluggable rendering backend on a near-fullscreen layered window.
//! Commands are received via a crossbeam channel from the hook thread and
//! bridged into the Win32 message loop via custom `WM_APP` messages.
//!
//! # Architecture
//!
//! The overlay runs on a dedicated OS thread with its own Win32 message loop.
//! A bridge thread reads [`OverlayCommand`] from the crossbeam channel and
//! posts corresponding `WM_APP+N` messages to the overlay thread via
//! `PostThreadMessageW`, decoupling the channel-based API from the Win32
//! message pump.
//!
//! The window uses `WS_EX_LAYERED` with `LWA_COLORKEY` (black = transparent)
//! so that only the drawn trail is visible. `WS_EX_TRANSPARENT` ensures mouse
//! events pass through to applications beneath.
//!
//! # Rendering
//!
//! Trail rendering is abstracted behind the [`TrailRenderer`] trait so that
//! the backend can be swapped (e.g. from GDI to Direct2D) without touching
//! the window management or message-loop code.

#[cfg(windows)]
mod gdi;
#[cfg(windows)]
mod window;

#[cfg(windows)]
mod direct2d;

use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{unbounded, Sender};
use log::info;

#[cfg(not(windows))]
use log::warn;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{POINT, RECT},
    Graphics::Gdi::HDC,
};

use crate::config::RuntimeConfig;

// ---------------------------------------------------------------------------
// TrailRenderer trait
// ---------------------------------------------------------------------------

/// Abstraction over the trail-drawing backend (GDI, Direct2D, …).
///
/// Implementations are stored as `Box<dyn TrailRenderer>` in `OverlayState`
/// and called from the Win32 message handlers.
#[cfg(windows)]
pub trait TrailRenderer {
    /// Draw one line segment (in client coordinates) into the back buffer.
    fn draw_segment(&mut self, from: POINT, to: POINT);
    /// Clear the entire back buffer (all-black = fully transparent via color key).
    fn clear(&mut self);
    /// Blit the dirty rectangle from the back buffer to the given paint DC.
    fn paint(&self, hdc: HDC, dirty: &RECT);
    /// Return the pen width used for dirty-rect padding calculations.
    fn pen_width(&self) -> i32;
    /// Release GDI / GPU resources. Must be idempotent.
    fn cleanup(&mut self);
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

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
    /// Update the gesture label text.
    ///
    /// `Some(text)` shows the label with the given text; `None` hides it.
    UpdateLabel(Option<String>),
    /// Shut down the overlay thread.
    Shutdown,
}

// ---------------------------------------------------------------------------
// Color parsing
// ---------------------------------------------------------------------------

/// Default trail color (deep sky blue) used when parsing fails.
pub const DEFAULT_COLOR: (u8, u8, u8) = (0, 191, 255);

/// Parses a CSS-style hex color string to an `(R, G, B)` tuple.
///
/// Accepts `#RRGGBB` or `#RGB` formats (the leading `#` is optional).
/// Returns [`DEFAULT_COLOR`] (deep sky blue) on invalid input.
///
/// # Examples
///
/// ```
/// use zero_gesture_lib::overlay::parse_hex_color;
///
/// assert_eq!(parse_hex_color("#00BFFF"), (0, 191, 255));
/// assert_eq!(parse_hex_color("#ABC"), (0xAA, 0xBB, 0xCC));
/// assert_eq!(parse_hex_color("invalid"), (0, 191, 255));
/// ```
pub fn parse_hex_color(s: &str) -> (u8, u8, u8) {
    let hex = s.strip_prefix('#').unwrap_or(s);

    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16);
            let g = u8::from_str_radix(&hex[2..4], 16);
            let b = u8::from_str_radix(&hex[4..6], 16);
            match (r, g, b) {
                (Ok(r), Ok(g), Ok(b)) => (r, g, b),
                _ => DEFAULT_COLOR,
            }
        }
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16);
            let g = u8::from_str_radix(&hex[1..2], 16);
            let b = u8::from_str_radix(&hex[2..3], 16);
            match (r, g, b) {
                (Ok(r), Ok(g), Ok(b)) => (r * 17, g * 17, b * 17),
                _ => DEFAULT_COLOR,
            }
        }
        _ => DEFAULT_COLOR,
    }
}

// ---------------------------------------------------------------------------
// Config snapshot
// ---------------------------------------------------------------------------

/// Snapshotted overlay configuration, taken once at thread start.
///
/// Same pattern as `HookConfig` in `hook.rs` — no lock in the hot path.
#[derive(Debug, Clone)]
pub(super) struct OverlayConfig {
    /// Trail color as `(R, G, B)`.
    pub color: (u8, u8, u8),
    /// Trail pen width in pixels.
    pub pen_width: i32,
    /// Font family for the gesture label.
    pub label_font_family: String,
    /// Font size in pixels for the gesture label.
    pub label_font_size: i32,
    /// Font weight for the gesture label (Win32 range: 0..=1000).
    pub label_font_weight: i32,
    /// Padding in pixels around the gesture label text.
    pub label_padding: i32,
}

// ---------------------------------------------------------------------------
// Public interface
// ---------------------------------------------------------------------------

/// Spawns the overlay thread and returns a channel sender and a join handle.
///
/// Configuration is read from `shared_config` once (snapshotted) so that no
/// locks are held in the rendering path.
///
/// Send [`OverlayCommand`] messages through the returned [`Sender`] to
/// control the overlay. The [`JoinHandle`] can be used to wait for the
/// thread to finish.
///
pub(crate) fn spawn(
    runtime: Arc<RuntimeConfig>,
) -> std::io::Result<(Sender<OverlayCommand>, JoinHandle<()>)> {
    info!("starting overlay thread");
    let (overlay_tx, overlay_rx) = unbounded();

    // Snapshot config before entering the thread.
    let overlay_config = {
        let cfg = &runtime.appearance;
        OverlayConfig {
            color: parse_hex_color(&cfg.trail_color),
            pen_width: cfg.trail_thickness.round() as i32,
            label_font_family: cfg.label_font_family.clone(),
            label_font_size: cfg.label_font_size.round() as i32,
            label_font_weight: cfg.label_font_weight,
            label_padding: cfg.label_padding.round() as i32,
        }
    };

    let handle = thread::Builder::new()
        .name("overlay-thread".to_string())
        .spawn(move || {
            #[cfg(windows)]
            window::run_loop_win32(overlay_config, overlay_rx);
            #[cfg(not(windows))]
            {
                let _ = (overlay_config, overlay_rx);
                warn!("Overlay is only supported on Windows");
            }
        })?;
    info!("overlay thread spawned");

    Ok((overlay_tx, handle))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::parse_hex_color;

    #[test]
    fn parse_hex_color_6_digit() {
        assert_eq!(parse_hex_color("#00BFFF"), (0x00, 0xBF, 0xFF));
        assert_eq!(parse_hex_color("#FF0000"), (0xFF, 0x00, 0x00));
        assert_eq!(parse_hex_color("#ffffff"), (0xFF, 0xFF, 0xFF));
    }

    #[test]
    fn parse_hex_color_6_digit_no_hash() {
        assert_eq!(parse_hex_color("00BFFF"), (0x00, 0xBF, 0xFF));
    }

    #[test]
    fn parse_hex_color_3_digit() {
        assert_eq!(parse_hex_color("#ABC"), (0xAA, 0xBB, 0xCC));
        assert_eq!(parse_hex_color("#FFF"), (0xFF, 0xFF, 0xFF));
        assert_eq!(parse_hex_color("#000"), (0x00, 0x00, 0x00));
    }

    #[test]
    fn parse_hex_color_invalid_fallback() {
        let default = (0, 191, 255);
        assert_eq!(parse_hex_color("invalid"), default);
        assert_eq!(parse_hex_color(""), default);
        assert_eq!(parse_hex_color("#GGGGGG"), default);
        assert_eq!(parse_hex_color("#12345"), default); // wrong length
    }
}
