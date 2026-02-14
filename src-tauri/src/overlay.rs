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
use log::{debug, error, info, trace};

#[cfg(not(windows))]
use log::warn;

use crate::SharedConfig;

#[cfg(windows)]
use std::cell::RefCell;

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreatePen,
        CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, GetDC,
        GetMonitorInfoW, InvalidateRect, MonitorFromPoint, Polyline, ReleaseDC, SelectObject,
        SetBkMode, SetTextColor, DT_CALCRECT, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER,
        HBITMAP, HBRUSH, HFONT, HDC, HPEN, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT,
        PS_SOLID, SRCCOPY, TRANSPARENT,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        GetSystemMetrics, PostQuitMessage, PostThreadMessageW, RegisterClassExW,
        SetLayeredWindowAttributes, SetWindowPos, ShowWindow, UnregisterClassW, HWND_TOPMOST,
        LWA_ALPHA, LWA_COLORKEY, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
        SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SW_HIDE, SW_SHOWNOACTIVATE,
        WM_APP, WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED,
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

/// Custom message: update the gesture label text.
///
/// `wParam` carries a raw pointer to a `Box<String>` (via `Box::into_raw`)
/// when showing a label, or `0` when hiding it.
#[cfg(windows)]
const WM_OVERLAY_LABEL: u32 = WM_APP + 5;

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
    /// Cached memory DC for the back buffer (full virtual-screen size).
    mem_dc: HDC,
    /// Cached bitmap selected into `mem_dc`.
    mem_bmp: HBITMAP,
    /// Previous bitmap originally selected in `mem_dc` before `mem_bmp`.
    old_mem_bmp: HBITMAP,
    /// Cached solid-black brush for clearing the back buffer.
    black_brush: HBRUSH,
    /// Back-buffer width in pixels.
    back_buffer_width: i32,
    /// Back-buffer height in pixels.
    back_buffer_height: i32,
    /// Virtual screen origin X — subtracted from screen coordinates to get
    /// client coordinates.
    origin_x: i32,
    /// Virtual screen origin Y.
    origin_y: i32,
    /// Snapshotted configuration (retained for future use, e.g. dynamic pen recreation).
    config: OverlayConfig,
    /// The label overlay window handle.
    label_hwnd: HWND,
    /// Font used for label text drawing.
    label_font: HFONT,
    /// Current label text (stored for WM_PAINT).
    label_text: Option<String>,
    /// Last known trail point (used to determine which monitor to show the label on).
    last_track_pt: Option<(i32, i32)>,
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
    info!("starting overlay thread");
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
    info!("overlay thread spawned");

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

/// Window class name for the label overlay (UTF-16, null-terminated).
#[cfg(windows)]
const LABEL_CLASS_NAME: &[u16] = &[
    b'Z' as u16, b'e' as u16, b'r' as u16, b'o' as u16, b'G' as u16, b'e' as u16, b's' as u16,
    b't' as u16, b'u' as u16, b'r' as u16, b'e' as u16, b'L' as u16, b'a' as u16, b'b' as u16,
    b'e' as u16, b'l' as u16, b'O' as u16, b'v' as u16, b'e' as u16, b'r' as u16, b'l' as u16,
    b'a' as u16, b'y' as u16, 0,
];

/// Horizontal and vertical padding (in pixels) around label text.
#[cfg(windows)]
const LABEL_PADDING: i32 = 12;

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
        info!("overlay thread started (tid={tid})");

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
                        OverlayCommand::UpdateLabel(text) => {
                            let w_param = match text {
                                Some(s) => Box::into_raw(Box::new(s)) as WPARAM,
                                None => 0,
                            };
                            PostThreadMessageW(tid, WM_OVERLAY_LABEL, w_param, 0)
                        }
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
        if pen.is_null() {
            error!("CreatePen failed for overlay window");
            DestroyWindow(hwnd);
            UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
            let _ = bridge.join();
            return;
        }

        // Create persistent back-buffer resources (full virtual-screen size).
        let screen_dc = GetDC(hwnd);
        if screen_dc.is_null() {
            error!("GetDC failed for overlay window");
            DeleteObject(pen as *mut _);
            DestroyWindow(hwnd);
            UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
            let _ = bridge.join();
            return;
        }

        let mem_dc = CreateCompatibleDC(screen_dc);
        if mem_dc.is_null() {
            error!("CreateCompatibleDC failed for overlay window");
            ReleaseDC(hwnd, screen_dc);
            DeleteObject(pen as *mut _);
            DestroyWindow(hwnd);
            UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
            let _ = bridge.join();
            return;
        }

        let mem_bmp = CreateCompatibleBitmap(screen_dc, vw, vh);
        if mem_bmp.is_null() {
            error!("CreateCompatibleBitmap failed for overlay window");
            DeleteDC(mem_dc);
            ReleaseDC(hwnd, screen_dc);
            DeleteObject(pen as *mut _);
            DestroyWindow(hwnd);
            UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
            let _ = bridge.join();
            return;
        }

        let old_mem_bmp = SelectObject(mem_dc, mem_bmp as *mut _) as HBITMAP;
        if old_mem_bmp.is_null() {
            error!("SelectObject failed for overlay back buffer");
            DeleteObject(mem_bmp as *mut _);
            DeleteDC(mem_dc);
            ReleaseDC(hwnd, screen_dc);
            DeleteObject(pen as *mut _);
            DestroyWindow(hwnd);
            UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
            let _ = bridge.join();
            return;
        }

        ReleaseDC(hwnd, screen_dc);
        let black_brush = CreateSolidBrush(0x00000000);
        if black_brush.is_null() {
            error!("CreateSolidBrush failed for overlay back buffer");
            SelectObject(mem_dc, old_mem_bmp as *mut _);
            DeleteObject(mem_bmp as *mut _);
            DeleteDC(mem_dc);
            DeleteObject(pen as *mut _);
            DestroyWindow(hwnd);
            UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
            let _ = bridge.join();
            return;
        }

        // Clear the back buffer initially.
        let full_rc = RECT {
            left: 0,
            top: 0,
            right: vw,
            bottom: vh,
        };
        FillRect(mem_dc, &full_rc, black_brush);

        debug!(
            "Overlay window created: hwnd={:?}, size={}x{} at ({},{}), color=#{:02X}{:02X}{:02X}, width={}",
            hwnd, vw, vh, vx, vy, r, g, b, config.pen_width
        );

        // Register label window class.
        let label_wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(label_wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinstance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: LABEL_CLASS_NAME.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        let label_atom = RegisterClassExW(&label_wc);
        if label_atom == 0 {
            error!("RegisterClassExW failed for label overlay window");
            // Continue without label support — trail overlay still works.
        }

        let label_ex_style =
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;

        let label_hwnd = if label_atom != 0 {
            CreateWindowExW(
                label_ex_style,
                LABEL_CLASS_NAME.as_ptr(),
                std::ptr::null(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                hinstance,
                std::ptr::null(),
            )
        } else {
            std::ptr::null_mut()
        };

        if !label_hwnd.is_null() {
            SetLayeredWindowAttributes(label_hwnd, 0, 200, LWA_ALPHA);
        }

        // Create font for the label (20px, system default).
        let font_name: Vec<u16> = "Segoe UI\0".encode_utf16().collect();
        let label_font = CreateFontW(
            20,                // height
            0,                 // width (auto)
            0,                 // escapement
            0,                 // orientation
            400,               // weight (FW_NORMAL)
            0,                 // italic
            0,                 // underline
            0,                 // strikeout
            1,                 // charset (DEFAULT_CHARSET)
            0,                 // out precision
            0,                 // clip precision
            5,                 // quality (CLEARTYPE_QUALITY)
            0,                 // pitch and family
            font_name.as_ptr(),
        );

        // Store state in thread-local.
        OVERLAY_STATE.with(|cell| {
            *cell.borrow_mut() = Some(OverlayState {
                hwnd,
                trail: Vec::with_capacity(256),
                pen,
                mem_dc,
                mem_bmp,
                old_mem_bmp,
                black_brush,
                back_buffer_width: vw,
                back_buffer_height: vh,
                origin_x: vx,
                origin_y: vy,
                config,
                label_hwnd,
                label_font,
                label_text: None,
                last_track_pt: None,
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
                WM_OVERLAY_LABEL => handle_label(msg.wParam),
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
                DeleteObject(state.black_brush as *mut _);
                if !state.old_mem_bmp.is_null() {
                    SelectObject(state.mem_dc, state.old_mem_bmp as *mut _);
                }
                DeleteObject(state.mem_bmp as *mut _);
                DeleteDC(state.mem_dc);
                if !state.label_font.is_null() {
                    DeleteObject(state.label_font as *mut _);
                }
                if !state.label_hwnd.is_null() {
                    DestroyWindow(state.label_hwnd);
                }
            }
        });
        DestroyWindow(hwnd);
        UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
        UnregisterClassW(LABEL_CLASS_NAME.as_ptr(), hinstance);
        info!("overlay thread stopped (tid={tid})");

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
    let (hwnd, label_hwnd) = OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return (None, std::ptr::null_mut()),
        };

        state.trail.clear();
        state.label_text = None;
        state.last_track_pt = None;

        // Clear the persistent back buffer so no stale trail is visible.
        unsafe {
            let full_rc = RECT {
                left: 0,
                top: 0,
                right: state.back_buffer_width,
                bottom: state.back_buffer_height,
            };
            FillRect(state.mem_dc, &full_rc, state.black_brush);
        }

        debug!("Overlay: StartGesture — showing window");
        (Some(state.hwnd), state.label_hwnd)
    });

    if let Some(hwnd) = hwnd {
        unsafe {
            if !label_hwnd.is_null() {
                ShowWindow(label_hwnd, SW_HIDE);
            }
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
}

/// Handle `WM_OVERLAY_TRACK`: draw new segment into back buffer, invalidate dirty rect.
///
/// The new line segment is drawn directly into the persistent back buffer
/// (`mem_dc`), so `WM_PAINT` only needs to `BitBlt`. Only the bounding box
/// from the previous point to the new point (padded by pen width) is
/// invalidated.
#[cfg(windows)]
fn handle_track(x: i32, y: i32) {
    OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return,
        };

        trace!("Overlay: TrackPoint ({x}, {y})");
        state.last_track_pt = Some((x, y));
        // Convert screen coordinates to client coordinates by subtracting
        // the virtual screen origin (window top-left).
        let new_pt = POINT {
            x: x - state.origin_x,
            y: y - state.origin_y,
        };

        let prev = state.trail.last().copied();
        state.trail.push(new_pt);

        // Draw the new segment into the persistent back buffer and
        // invalidate only the affected region.
        if let Some(prev) = prev {
            unsafe {
                let old_pen = SelectObject(state.mem_dc, state.pen as *mut _);
                let pts = [prev, new_pt];
                Polyline(state.mem_dc, pts.as_ptr(), 2);
                SelectObject(state.mem_dc, old_pen);
            }

            let pad = state.config.pen_width / 2 + 1;
            let dirty = RECT {
                left: prev.x.min(new_pt.x) - pad,
                top: prev.y.min(new_pt.y) - pad,
                right: prev.x.max(new_pt.x) + pad,
                bottom: prev.y.max(new_pt.y) + pad,
            };
            unsafe {
                InvalidateRect(state.hwnd, &dirty, 0);
            }
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
    let (hwnd, label_hwnd) = OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return (None, std::ptr::null_mut()),
        };

        state.trail.clear();
        state.label_text = None;

        // Clear the persistent back buffer so the next BitBlt shows black
        // (= transparent via color key).
        unsafe {
            let full_rc = RECT {
                left: 0,
                top: 0,
                right: state.back_buffer_width,
                bottom: state.back_buffer_height,
            };
            FillRect(state.mem_dc, &full_rc, state.black_brush);
        }

        debug!("Overlay: EndGesture — hiding window");
        (Some(state.hwnd), state.label_hwnd)
    });
    // Borrow is now released — safe to call Win32 APIs that trigger WndProc.

    if let Some(hwnd) = hwnd {
        unsafe {
            if !label_hwnd.is_null() {
                ShowWindow(label_hwnd, SW_HIDE);
            }
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
// Label handling
// ---------------------------------------------------------------------------

/// Handle `WM_OVERLAY_LABEL`: show or hide the gesture label.
///
/// `text_ptr` is either `0` (hide) or a raw pointer to a `Box<String>`
/// allocated by the bridge thread.
#[cfg(windows)]
fn handle_label(text_ptr: WPARAM) {
    let text = if text_ptr == 0 {
        None
    } else {
        Some(unsafe { *Box::from_raw(text_ptr as *mut String) })
    };

    // Store text and read label_hwnd + last_track_pt + label_font.
    let (label_hwnd, label_font, track_pt) = OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return (std::ptr::null_mut(), std::ptr::null_mut(), None),
        };
        state.label_text = text;
        (state.label_hwnd, state.label_font, state.last_track_pt)
    });

    if label_hwnd.is_null() {
        return;
    }

    // Read back whether we have text.
    let has_text = OVERLAY_STATE.with(|cell| {
        let borrow = cell.borrow();
        borrow.as_ref().and_then(|s| s.label_text.as_ref().map(|t| t.clone()))
    });

    match has_text {
        None => {
            unsafe { ShowWindow(label_hwnd, SW_HIDE) };
        }
        Some(text) => {
            unsafe {
                // Measure text size.
                let dc = GetDC(label_hwnd);
                let old_font = SelectObject(dc, label_font as *mut _);

                let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                let mut rc = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                DrawTextW(
                    dc,
                    wide.as_ptr(),
                    (wide.len() - 1) as i32,
                    &mut rc,
                    DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
                );
                SelectObject(dc, old_font);
                ReleaseDC(label_hwnd, dc);

                let text_w = rc.right - rc.left;
                let text_h = rc.bottom - rc.top;
                let win_w = text_w + LABEL_PADDING * 2;
                let win_h = text_h + LABEL_PADDING * 2;

                // Determine which monitor to use based on the last trail point.
                let pt = track_pt.unwrap_or((0, 0));
                let point = POINT { x: pt.0, y: pt.1 };
                let hmon = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
                let mut mi: MONITORINFO = std::mem::zeroed();
                mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                GetMonitorInfoW(hmon, &mut mi);

                let mon_cx = (mi.rcWork.left + mi.rcWork.right) / 2;
                let mon_cy = (mi.rcWork.top + mi.rcWork.bottom) / 2;
                let win_x = mon_cx - win_w / 2;
                let win_y = mon_cy - win_h / 2;

                SetWindowPos(
                    label_hwnd,
                    HWND_TOPMOST,
                    win_x,
                    win_y,
                    win_w,
                    win_h,
                    SWP_NOACTIVATE,
                );
                ShowWindow(label_hwnd, SW_SHOWNOACTIVATE);
                InvalidateRect(label_hwnd, std::ptr::null(), 1);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Label WndProc
// ---------------------------------------------------------------------------

/// Window procedure for the label overlay window.
///
/// Draws a black background with white text.
#[cfg(windows)]
unsafe extern "system" fn label_wnd_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let hdc = BeginPaint(hwnd, &mut ps);

            // Fill background with black.
            let black_brush = CreateSolidBrush(0x00000000);
            FillRect(hdc, &ps.rcPaint, black_brush);
            DeleteObject(black_brush as *mut _);

            // Draw text.
            OVERLAY_STATE.with(|cell| {
                let borrow = cell.borrow();
                if let Some(state) = borrow.as_ref() {
                    if let Some(text) = &state.label_text {
                        let old_font = SelectObject(hdc, state.label_font as *mut _);
                        SetBkMode(hdc, TRANSPARENT as i32);
                        SetTextColor(hdc, 0x00FFFFFF); // white

                        let wide: Vec<u16> =
                            text.encode_utf16().chain(std::iter::once(0)).collect();

                        // Get client rect for centered drawing.
                        let mut client_rc = RECT {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        };
                        windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect(
                            hwnd,
                            &mut client_rc,
                        );
                        DrawTextW(
                            hdc,
                            wide.as_ptr(),
                            (wide.len() - 1) as i32,
                            &mut client_rc,
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                        );
                        SelectObject(hdc, old_font);
                    }
                }
            });

            EndPaint(hwnd, &ps);
            0
        }
        WM_DESTROY => 0,
        _ => DefWindowProcW(hwnd, msg, w_param, l_param),
    }
}

// ---------------------------------------------------------------------------
// WndProc
// ---------------------------------------------------------------------------

/// Window procedure for the overlay window.
///
/// Handles:
/// - `WM_ERASEBKGND` — no-op (returns handled) to avoid redundant clears.
/// - `WM_PAINT` — blits only the dirty rectangle from the persistent back buffer.
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

            // Blit the dirty region from the persistent back buffer.
            OVERLAY_STATE.with(|cell| {
                let borrow = cell.borrow();
                if let Some(state) = borrow.as_ref() {
                    BitBlt(
                        hdc,
                        rc.left,
                        rc.top,
                        w,
                        h,
                        state.mem_dc,
                        rc.left,
                        rc.top,
                        SRCCOPY,
                    );
                }
            });

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
