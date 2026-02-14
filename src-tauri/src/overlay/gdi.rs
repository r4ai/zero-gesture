//! GDI-based trail renderer.
//!
//! Uses `CreatePen` + `Polyline` for simplicity. No anti-aliasing.

use windows_sys::Win32::{
    Foundation::HWND,
    Foundation::{POINT, RECT},
    Graphics::Gdi::{
        BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen, CreateSolidBrush, DeleteDC,
        DeleteObject, FillRect, GetDC, Polyline, ReleaseDC, SelectObject, HBITMAP, HBRUSH, HDC,
        HPEN, PS_SOLID, SRCCOPY,
    },
};

use super::{OverlayConfig, TrailRenderer};

/// GDI trail renderer — draws into a persistent back buffer using `Polyline`.
pub(super) struct GdiRenderer {
    pen: HPEN,
    pen_width_px: i32,
    mem_dc: HDC,
    mem_bmp: HBITMAP,
    old_mem_bmp: HBITMAP,
    black_brush: HBRUSH,
    back_buffer_width: i32,
    back_buffer_height: i32,
}

impl GdiRenderer {
    /// Create a new GDI renderer with a full-screen back buffer.
    ///
    /// `hwnd` is the overlay window; `vw`/`vh` are the virtual-screen
    /// dimensions in pixels.
    pub fn new(hwnd: HWND, config: &OverlayConfig, vw: i32, vh: i32) -> Result<Self, String> {
        unsafe {
            let (r, g, b) = config.color;
            let colorref = (r as u32) | ((g as u32) << 8) | ((b as u32) << 16);
            let pen = CreatePen(PS_SOLID, config.pen_width, colorref);
            if pen.is_null() {
                return Err("CreatePen failed".to_string());
            }

            let screen_dc = GetDC(hwnd);
            if screen_dc.is_null() {
                DeleteObject(pen as *mut _);
                return Err("GetDC failed".to_string());
            }

            let mem_dc = CreateCompatibleDC(screen_dc);
            if mem_dc.is_null() {
                ReleaseDC(hwnd, screen_dc);
                DeleteObject(pen as *mut _);
                return Err("CreateCompatibleDC failed".to_string());
            }

            let mem_bmp = CreateCompatibleBitmap(screen_dc, vw, vh);
            if mem_bmp.is_null() {
                DeleteDC(mem_dc);
                ReleaseDC(hwnd, screen_dc);
                DeleteObject(pen as *mut _);
                return Err("CreateCompatibleBitmap failed".to_string());
            }

            let old_mem_bmp = SelectObject(mem_dc, mem_bmp as *mut _) as HBITMAP;
            if old_mem_bmp.is_null() {
                DeleteObject(mem_bmp as *mut _);
                DeleteDC(mem_dc);
                ReleaseDC(hwnd, screen_dc);
                DeleteObject(pen as *mut _);
                return Err("SelectObject failed for back buffer".to_string());
            }

            ReleaseDC(hwnd, screen_dc);

            let black_brush = CreateSolidBrush(0x00000000);
            if black_brush.is_null() {
                SelectObject(mem_dc, old_mem_bmp as *mut _);
                DeleteObject(mem_bmp as *mut _);
                DeleteDC(mem_dc);
                DeleteObject(pen as *mut _);
                return Err("CreateSolidBrush failed".to_string());
            }

            // Clear the back buffer initially.
            let full_rc = RECT {
                left: 0,
                top: 0,
                right: vw,
                bottom: vh,
            };
            FillRect(mem_dc, &full_rc, black_brush);

            Ok(Self {
                pen,
                pen_width_px: config.pen_width,
                mem_dc,
                mem_bmp,
                old_mem_bmp,
                black_brush,
                back_buffer_width: vw,
                back_buffer_height: vh,
            })
        }
    }
}

impl TrailRenderer for GdiRenderer {
    fn draw_segment(&mut self, from: POINT, to: POINT) {
        unsafe {
            let old_pen = SelectObject(self.mem_dc, self.pen as *mut _);
            let pts = [from, to];
            Polyline(self.mem_dc, pts.as_ptr(), 2);
            SelectObject(self.mem_dc, old_pen);
        }
    }

    fn clear(&mut self) {
        unsafe {
            let full_rc = RECT {
                left: 0,
                top: 0,
                right: self.back_buffer_width,
                bottom: self.back_buffer_height,
            };
            FillRect(self.mem_dc, &full_rc, self.black_brush);
        }
    }

    fn paint(&self, hdc: HDC, dirty: &RECT) {
        let w = dirty.right - dirty.left;
        let h = dirty.bottom - dirty.top;
        unsafe {
            BitBlt(
                hdc,
                dirty.left,
                dirty.top,
                w,
                h,
                self.mem_dc,
                dirty.left,
                dirty.top,
                SRCCOPY,
            );
        }
    }

    fn pen_width(&self) -> i32 {
        self.pen_width_px
    }

    fn cleanup(&mut self) {
        unsafe {
            if !self.pen.is_null() {
                DeleteObject(self.pen as *mut _);
                self.pen = std::ptr::null_mut();
            }
            if !self.black_brush.is_null() {
                DeleteObject(self.black_brush as *mut _);
                self.black_brush = std::ptr::null_mut();
            }
            if !self.old_mem_bmp.is_null() {
                SelectObject(self.mem_dc, self.old_mem_bmp as *mut _);
                self.old_mem_bmp = std::ptr::null_mut();
            }
            if !self.mem_bmp.is_null() {
                DeleteObject(self.mem_bmp as *mut _);
                self.mem_bmp = std::ptr::null_mut();
            }
            if !self.mem_dc.is_null() {
                DeleteDC(self.mem_dc);
                self.mem_dc = std::ptr::null_mut();
            }
        }
    }
}

impl Drop for GdiRenderer {
    fn drop(&mut self) {
        self.cleanup();
    }
}
