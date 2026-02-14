//! Direct2D trail renderer (stub).
//!
//! This module will eventually provide anti-aliased trail rendering via
//! Direct2D. For now it serves as a placeholder to validate the adapter
//! pattern wiring.

use windows_sys::Win32::Foundation::{POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::HDC;

use super::{OverlayConfig, TrailRenderer};
use windows_sys::Win32::Foundation::HWND;

/// Stub Direct2D renderer — not yet implemented.
#[allow(dead_code)]
pub(super) struct Direct2dRenderer;

#[allow(dead_code)]
impl Direct2dRenderer {
    pub fn new(_hwnd: HWND, _config: &OverlayConfig, _vw: i32, _vh: i32) -> Result<Self, String> {
        Err("Direct2D renderer is not yet implemented".to_string())
    }
}

impl TrailRenderer for Direct2dRenderer {
    fn draw_segment(&mut self, _from: POINT, _to: POINT) {
        // Not yet implemented.
    }

    fn clear(&mut self) {
        // Not yet implemented.
    }

    fn paint(&self, _hdc: HDC, _dirty: &RECT) {
        // Not yet implemented.
    }

    fn pen_width(&self) -> i32 {
        0
    }

    fn cleanup(&mut self) {
        // Nothing to clean up.
    }
}
