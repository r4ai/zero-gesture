//! Gesture overlay thread.
//!
//! Manages a transparent overlay window that draws the mouse gesture trail
//! using GDI on a full-screen layered window. Commands are received via a
//! crossbeam channel from the hook thread and bridged into the Win32 message
//! loop via custom `WM_APP` messages.
//!
//! # Architecture
//!
//! The overlay runs on a dedicated OS thread with its own Win32 message loop.
//! A bridge thread reads [`OverlayCommand`] from the crossbeam channel and
//! posts corresponding `WM_APP+N` messages to the overlay thread via
//! [`PostThreadMessageW`], decoupling the channel-based API from the Win32
//! message pump.
//!
//! The window uses `WS_EX_LAYERED` with `LWA_COLORKEY` (black = transparent)
//! so that only the drawn trail is visible. `WS_EX_TRANSPARENT` ensures mouse
//! events pass through to applications beneath.
//!
//! # Rendering
//!
//! Currently uses GDI (`CreatePen` + `Polyline`) for simplicity, since the
//! project uses `windows-sys` (raw FFI) which makes COM-based Direct2D
//! extremely verbose.
//!
// TODO: Upgrade to Direct2D for anti-aliased rendering once a lightweight
// COM wrapper is available or the project switches to the `windows` crate.

use std::thread::{self, JoinHandle};

use crossbeam_channel::{unbounded, Receiver, Sender};
use log::{debug, error, trace};

#[cfg(not(windows))]
use log::warn;

use crate::SharedConfig;

#[cfg(windows)]
use std::cell::RefCell;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen,
        CreateSolidBrush, DeleteDC, DeleteObject, EndPaint, FillRect, InvalidateRect, Polyline,
        SelectObject, HPEN, PAINTSTRUCT, PS_SOLID, SRCCOPY,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        GetSystemMetrics, PostQuitMessage, PostThreadMessageW, RegisterClassExW,
        SetLayeredWindowAttributes, ShowWindow, UnregisterClassW, LWA_COLORKEY, MSG,
        SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE,
        SW_SHOWNOACTIVATE, WM_APP, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED,
        WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    },
};

/// Custom message: begin a new gesture trail (show window).
#[cfg(windows)]
const WM_OVERLAY_START: u32 = WM_APP + 1;

/// Custom message: append a trail point (x in wParam, y in lParam).
#[cfg(windows)]
const WM_OVERLAY_TRACK: u32 = WM_APP + 2;

/// Custom message: end the gesture trail (hide window).
#[cfg(windows)]
const WM_OVERLAY_END: u32 = WM_APP + 3;

/// Custom message: shut down the overlay thread.
#[cfg(windows)]
const WM_OVERLAY_SHUTDOWN: u32 = WM_APP + 4;

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
    /// Shut down the overlay thread.
    Shutdown,
}

// ---------------------------------------------------------------------------
// Color parsing
// ---------------------------------------------------------------------------

/// Default trail color (deep sky blue) used when parsing fails.
const DEFAULT_COLOR: (u8, u8, u8) = (0, 191, 255);

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
struct OverlayConfig {
    /// Trail color as `(R, G, B)`.
    color: (u8, u8, u8),
    /// Trail pen width in pixels.
    pen_width: i32,
}

// ---------------------------------------------------------------------------
// Thread-local state
// ---------------------------------------------------------------------------

/// All mutable state for the overlay thread, stored in a [`thread_local!`].
///
/// Mirrors the `HookThreadState` pattern from `hook.rs`. The WndProc callback
/// is a C-style function pointer with no user-data parameter, so thread-local
/// storage is used to access state.
#[cfg(windows)]
struct OverlayState {
    /// The overlay window handle.
    hwnd: HWND,
    /// Accumulated trail points for the current gesture.
    ///
    /// Points are stored in **client coordinates** (screen coordinate minus
    /// the virtual screen origin) so that `Polyline` draws at the correct
    /// position.
    trail: Vec<POINT>,
    /// GDI pen used to draw the trail.
    pen: HPEN,
    /// Virtual screen origin X — subtracted from screen coordinates to get
    /// client coordinates.
    origin_x: i32,
    /// Virtual screen origin Y.
    origin_y: i32,
    /// Snapshotted configuration (retained for future use, e.g. dynamic pen recreation).
    #[allow(dead_code)]
    config: OverlayConfig,
}

#[cfg(windows)]
thread_local! {
    static OVERLAY_STATE: RefCell<Option<OverlayState>> = const { RefCell::new(None) };
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
/// # Examples
///
/// ```no_run
/// use zero_gesture_lib::overlay::{self, OverlayCommand};
/// use zero_gesture_lib::SharedConfig;
/// use zero_gesture_lib::config::AppConfig;
///
/// let config = SharedConfig::new(AppConfig::default());
/// let (tx, handle) = overlay::spawn(config);
/// tx.send(OverlayCommand::Shutdown).unwrap();
/// handle.join().unwrap();
/// ```
pub fn spawn(shared_config: SharedConfig) -> (Sender<OverlayCommand>, JoinHandle<()>) {
    let (overlay_tx, overlay_rx) = unbounded();

    // Snapshot config before entering the thread.
    let overlay_config = {
        let cfg = shared_config.0.read().unwrap();
        OverlayConfig {
            color: parse_hex_color(&cfg.trail_color),
            pen_width: cfg.trail_thickness.round() as i32,
        }
    };

    let handle = thread::Builder::new()
        .name("overlay-thread".to_string())
        .spawn(move || {
            #[cfg(windows)]
            run_loop_win32(overlay_config, overlay_rx);
            #[cfg(not(windows))]
            {
                let _ = (overlay_config, overlay_rx);
                warn!("Overlay is only supported on Windows");
            }
        })
        .expect("failed to spawn overlay thread");

    (overlay_tx, handle)
}

// ---------------------------------------------------------------------------
// Win32 implementation
// ---------------------------------------------------------------------------

/// Window class name for the overlay (UTF-16, null-terminated).
#[cfg(windows)]
const CLASS_NAME: &[u16] = &[
    b'Z' as u16,
    b'e' as u16,
    b'r' as u16,
    b'o' as u16,
    b'G' as u16,
    b'e' as u16,
    b's' as u16,
    b't' as u16,
    b'u' as u16,
    b'r' as u16,
    b'e' as u16,
    b'O' as u16,
    b'v' as u16,
    b'e' as u16,
    b'r' as u16,
    b'l' as u16,
    b'a' as u16,
    b'y' as u16,
    0,
];

/// Main loop of the overlay thread (Windows implementation).
///
/// 1. Gets the current thread ID for message posting.
/// 2. Spawns a bridge thread that reads [`OverlayCommand`] from the crossbeam
///    channel and posts corresponding `WM_APP+N` messages.
/// 3. Registers a window class and creates a full-screen layered window.
/// 4. Creates a GDI pen and stores state in thread-local [`OVERLAY_STATE`].
/// 5. Runs the Win32 message loop until `WM_QUIT`.
/// 6. Cleans up: deletes GDI objects, destroys window, unregisters class,
///    joins the bridge thread.
#[cfg(windows)]
fn run_loop_win32(config: OverlayConfig, overlay_rx: Receiver<OverlayCommand>) {
    unsafe {
        // Set per-monitor DPI awareness for this thread so that
        // GetSystemMetrics / CreateWindowExW use physical pixels,
        // matching the coordinates from WH_MOUSE_LL (which always
        // reports per-monitor-aware physical screen coordinates).
        windows_sys::Win32::UI::HiDpi::SetThreadDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );

        let tid = GetCurrentThreadId();

        // Bridge thread: crossbeam channel → Win32 messages.
        let bridge = thread::Builder::new()
            .name("overlay-bridge".to_string())
            .spawn(move || {
                while let Ok(cmd) = overlay_rx.recv() {
                    let posted = match cmd {
                        OverlayCommand::StartGesture => {
                            PostThreadMessageW(tid, WM_OVERLAY_START, 0, 0)
                        }
                        OverlayCommand::TrackPoint { x, y } => {
                            PostThreadMessageW(tid, WM_OVERLAY_TRACK, x as WPARAM, y as LPARAM)
                        }
                        OverlayCommand::EndGesture => PostThreadMessageW(tid, WM_OVERLAY_END, 0, 0),
                        OverlayCommand::Shutdown => {
                            PostThreadMessageW(tid, WM_OVERLAY_SHUTDOWN, 0, 0);
                            break;
                        }
                    };
                    if posted == 0 {
                        break;
                    }
                }
            })
            .expect("failed to spawn overlay bridge thread");

        // Register window class.
        let hinstance = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(overlay_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: CLASS_NAME.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };

        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            error!("RegisterClassExW failed for overlay window");
            let _ = bridge.join();
            return;
        }

        // Full virtual screen geometry (all monitors).
        let vx = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let vy = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let vw = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let vh = GetSystemMetrics(SM_CYVIRTUALSCREEN);

        let ex_style =
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;

        let hwnd = CreateWindowExW(
            ex_style,
            CLASS_NAME.as_ptr(),
            std::ptr::null(), // no title
            WS_POPUP,
            vx,
            vy,
            vw,
            vh,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );

        if hwnd.is_null() {
            error!("CreateWindowExW failed for overlay window");
            UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
            let _ = bridge.join();
            return;
        }

        // Black = fully transparent via color key.
        SetLayeredWindowAttributes(hwnd, 0x00000000, 0, LWA_COLORKEY);

        // Create GDI pen for trail drawing.
        let (r, g, b) = config.color;
        let colorref = (r as u32) | ((g as u32) << 8) | ((b as u32) << 16);
        let pen = CreatePen(PS_SOLID, config.pen_width, colorref);

        debug!(
            "Overlay window created: hwnd={:?}, size={}x{} at ({},{}), color=#{:02X}{:02X}{:02X}, width={}",
            hwnd, vw, vh, vx, vy, r, g, b, config.pen_width
        );

        // Store state in thread-local.
        OVERLAY_STATE.with(|cell| {
            *cell.borrow_mut() = Some(OverlayState {
                hwnd,
                trail: Vec::with_capacity(256),
                pen,
                origin_x: vx,
                origin_y: vy,
                config,
            });
        });

        // Win32 message loop.
        let mut msg: MSG = std::mem::zeroed();
        loop {
            let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if ret == 0 || ret == -1 {
                break;
            }

            match msg.message {
                WM_OVERLAY_START => handle_start(),
                WM_OVERLAY_TRACK => handle_track(msg.wParam as i32, msg.lParam as i32),
                WM_OVERLAY_END => handle_end(),
                WM_OVERLAY_SHUTDOWN => {
                    PostQuitMessage(0);
                }
                _ => {
                    DispatchMessageW(&msg);
                }
            }
        }

        // Cleanup.
        OVERLAY_STATE.with(|cell| {
            if let Some(state) = cell.borrow_mut().take() {
                DeleteObject(state.pen as *mut _);
            }
        });
        DestroyWindow(hwnd);
        UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
        debug!("Overlay thread exiting");

        let _ = bridge.join();
    }
}

// ---------------------------------------------------------------------------
// Message handlers
// ---------------------------------------------------------------------------

/// Handle `WM_OVERLAY_START`: clear trail, show window.
///
/// The `RefCell` borrow is dropped **before** calling Win32 APIs that may
/// synchronously dispatch `WM_PAINT` (e.g. `ShowWindow`), because
/// `overlay_wnd_proc` borrows the same `RefCell` to read the trail.
#[cfg(windows)]
fn handle_start() {
    let hwnd = OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return None,
        };

        state.trail.clear();
        debug!("Overlay: StartGesture — showing window");
        Some(state.hwnd)
    });

    if let Some(hwnd) = hwnd {
        unsafe {
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
}

/// Handle `WM_OVERLAY_TRACK`: append point, request repaint.
#[cfg(windows)]
fn handle_track(x: i32, y: i32) {
    OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return,
        };

        trace!("Overlay: TrackPoint ({x}, {y})");
        // Convert screen coordinates to client coordinates by subtracting
        // the virtual screen origin (window top-left).
        state.trail.push(POINT {
            x: x - state.origin_x,
            y: y - state.origin_y,
        });
        unsafe {
            // bErase = FALSE (0) — don't erase background, we redraw fully in WM_PAINT.
            InvalidateRect(state.hwnd, std::ptr::null(), 0);
        }
    });
}

/// Handle `WM_OVERLAY_END`: clear trail, repaint, then hide window.
///
/// The repaint is forced synchronously via [`UpdateWindow`] **before**
/// hiding so that the window surface is cleared. Without this, the next
/// `ShowWindow` would briefly display the stale trail from the previous
/// gesture.
///
/// The `RefCell` borrow is dropped **before** calling `UpdateWindow`,
/// which synchronously dispatches `WM_PAINT`. If the borrow were still
/// held, `overlay_wnd_proc` would panic trying to borrow the same
/// `RefCell`.
#[cfg(windows)]
fn handle_end() {
    let hwnd = OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return None,
        };

        state.trail.clear();
        debug!("Overlay: EndGesture — hiding window");
        Some(state.hwnd)
    });
    // Borrow is now released — safe to call Win32 APIs that trigger WndProc.

    if let Some(hwnd) = hwnd {
        unsafe {
            // Force a synchronous repaint (empty trail → all-black → transparent)
            // while the window is still visible, so the surface is clean when
            // the window is shown again for the next gesture.
            InvalidateRect(hwnd, std::ptr::null(), 1);
            windows_sys::Win32::Graphics::Gdi::UpdateWindow(hwnd);
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

// ---------------------------------------------------------------------------
// WndProc
// ---------------------------------------------------------------------------

/// Window procedure for the overlay window.
///
/// Handles:
/// - `WM_ERASEBKGND` — fills with black (the transparent color key).
/// - `WM_PAINT` — fills black background, then draws the trail with `Polyline`.
/// - `WM_DESTROY` — posts `WM_QUIT` to exit the message loop.
///
/// All other messages are passed to [`DefWindowProcW`].
#[cfg(windows)]
unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // Return 1 to tell Windows we handled it — actual clearing is
            // done in WM_PAINT via the back buffer, avoiding flicker.
            1
        }
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            let rc = &ps.rcPaint;
            let w = rc.right - rc.left;
            let h = rc.bottom - rc.top;

            // Double-buffer: draw into an off-screen bitmap, then blit once.
            let mem_dc = CreateCompatibleDC(hdc);
            let mem_bmp = CreateCompatibleBitmap(hdc, w, h);
            let old_bmp = SelectObject(mem_dc, mem_bmp as *mut _);

            // Fill back buffer with black (the transparent color key).
            // Offset the fill rect to (0,0) in the memory DC.
            let fill_rc = windows_sys::Win32::Foundation::RECT {
                left: 0,
                top: 0,
                right: w,
                bottom: h,
            };
            let black_brush = CreateSolidBrush(0x00000000);
            FillRect(mem_dc, &fill_rc, black_brush);
            DeleteObject(black_brush as *mut _);

            // Draw the trail into the back buffer.
            OVERLAY_STATE.with(|cell| {
                let borrow = cell.borrow();
                if let Some(state) = borrow.as_ref() {
                    if state.trail.len() >= 2 {
                        // Shift the coordinate origin so that screen-relative
                        // client coordinates map correctly into the memory DC
                        // whose origin is the dirty rect's top-left corner.
                        let shifted: Vec<POINT> = state
                            .trail
                            .iter()
                            .map(|p| POINT {
                                x: p.x - rc.left,
                                y: p.y - rc.top,
                            })
                            .collect();
                        let old_pen = SelectObject(mem_dc, state.pen as *mut _);
                        Polyline(mem_dc, shifted.as_ptr(), shifted.len() as i32);
                        SelectObject(mem_dc, old_pen);
                    }
                }
            });

            // Single
            BitBlt(hdc, rc.left, rc.top, w, h, mem_dc, 0, 0, SRCCOPY);

            // Cleanup back buffer.
            SelectObject(mem_dc, old_bmp);
            DeleteObject(mem_bmp as *mut _);
            DeleteDC(mem_dc);

            EndPaint(hwnd, &ps);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w_param, l_param),
    }
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
