//! Win32 window management for the gesture overlay.
//!
//! Contains the message loop, bridge thread, window creation, WndProcs,
//! and all message handlers. The actual trail rendering is delegated to
//! a [`TrailRenderer`](super::TrailRenderer) implementation.

use std::cell::RefCell;
use std::thread;

use crossbeam_channel::Receiver;
use log::{debug, error, info, trace};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreateFontW, CreateRoundRectRgn, CreateSolidBrush, DeleteObject, DrawTextW,
        EndPaint, FillRect, GetDC, GetMonitorInfoW, InvalidateRect, MonitorFromPoint, ReleaseDC,
        SelectObject, SetBkMode, SetTextColor, SetWindowRgn, CLEARTYPE_QUALITY,
        CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DT_CALCRECT, DT_CENTER, DT_NOPREFIX,
        DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, HFONT, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        GetSystemMetrics, PostQuitMessage, PostThreadMessageW, RegisterClassExW,
        SetLayeredWindowAttributes, SetWindowPos, ShowWindow, UnregisterClassW, HWND_TOP,
        LWA_ALPHA, LWA_COLORKEY, MSG, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, WM_APP,
        WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    },
};

use super::gdi::GdiRenderer;
use super::{OverlayCommand, OverlayConfig, TrailRenderer};

// ---------------------------------------------------------------------------
// Custom messages
// ---------------------------------------------------------------------------

const WM_OVERLAY_START: u32 = WM_APP + 1;
const WM_OVERLAY_TRACK: u32 = WM_APP + 2;
const WM_OVERLAY_END: u32 = WM_APP + 3;
const WM_OVERLAY_SHUTDOWN: u32 = WM_APP + 4;
const WM_OVERLAY_LABEL: u32 = WM_APP + 5;

/// Tiny inset applied to overlay bounds to avoid "exact fullscreen window"
/// classification by parts of the Windows shell.
///
/// Why this exists:
/// - A borderless, monitor-sized popup can be treated similarly to fullscreen UI.
/// - In that state, taskbar behavior and shell indicators (e.g. transient
///   "do not disturb / focus assist"-like icon changes) can become unstable.
/// - We do not need true pixel-perfect edge coverage for a gesture trail.
///
/// Trade-off:
/// - A 1px transparent margin may remain at monitor edges.
/// - In exchange, shell side effects are reduced and z-order behavior becomes
///   more predictable.
const OVERLAY_FULLSCREEN_AVOID_MARGIN_PX: i32 = 1;

// ---------------------------------------------------------------------------
// Window class names
// ---------------------------------------------------------------------------

/// Window class name for the overlay (UTF-16, null-terminated).
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
const LABEL_CLASS_NAME: &[u16] = &[
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
    b'L' as u16,
    b'a' as u16,
    b'b' as u16,
    b'e' as u16,
    b'l' as u16,
    b'O' as u16,
    b'v' as u16,
    b'e' as u16,
    b'r' as u16,
    b'l' as u16,
    b'a' as u16,
    b'y' as u16,
    0,
];

// ---------------------------------------------------------------------------
// Thread-local state
// ---------------------------------------------------------------------------

/// All mutable state for the overlay thread, stored in a [`thread_local!`].
///
/// The WndProc callback is a C-style function pointer with no user-data
/// parameter, so thread-local storage is used to access state.
struct OverlayState {
    hwnd: HWND,
    trail: Vec<POINT>,
    origin_x: i32,
    origin_y: i32,
    renderer: Box<dyn TrailRenderer>,
    #[allow(dead_code)] // retained for future use (e.g. dynamic pen recreation)
    config: OverlayConfig,
    label_hwnd: HWND,
    label_font: HFONT,
    label_text: Option<String>,
    last_track_pt: Option<(i32, i32)>,
    label_padding: i32,
}

thread_local! {
    static OVERLAY_STATE: RefCell<Option<OverlayState>> = const { RefCell::new(None) };
}

// ---------------------------------------------------------------------------
// Main loop
// ---------------------------------------------------------------------------

/// Main loop of the overlay thread (Windows implementation).
///
/// 1. Gets the current thread ID for message posting.
/// 2. Spawns a bridge thread that reads [`OverlayCommand`] from the crossbeam
///    channel and posts corresponding `WM_APP+N` messages.
/// 3. Registers a window class and creates a full-screen layered window.
/// 4. Creates a renderer and stores state in thread-local [`OVERLAY_STATE`].
/// 5. Runs the Win32 message loop until `WM_QUIT`.
/// 6. Cleans up: renderer resources, destroys window, unregisters class,
///    joins the bridge thread.
pub(super) fn run_loop_win32(config: OverlayConfig, overlay_rx: Receiver<OverlayCommand>) {
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
                    let (posted, label_payload, should_break) = match cmd {
                        OverlayCommand::StartGesture => {
                            (PostThreadMessageW(tid, WM_OVERLAY_START, 0, 0), 0, false)
                        }
                        OverlayCommand::TrackPoint { x, y } => (
                            PostThreadMessageW(tid, WM_OVERLAY_TRACK, x as WPARAM, y as LPARAM),
                            0,
                            false,
                        ),
                        OverlayCommand::EndGesture => {
                            (PostThreadMessageW(tid, WM_OVERLAY_END, 0, 0), 0, false)
                        }
                        OverlayCommand::UpdateLabel(text) => {
                            let w_param = match text {
                                Some(s) => Box::into_raw(Box::new(s)) as WPARAM,
                                None => 0,
                            };
                            (
                                PostThreadMessageW(tid, WM_OVERLAY_LABEL, w_param, 0),
                                w_param,
                                false,
                            )
                        }
                        OverlayCommand::Shutdown => {
                            (PostThreadMessageW(tid, WM_OVERLAY_SHUTDOWN, 0, 0), 0, true)
                        }
                    };
                    if posted == 0 {
                        if label_payload != 0 {
                            drop(Box::from_raw(label_payload as *mut String));
                        }
                        break;
                    }
                    if should_break {
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

        // Intentionally avoid WS_EX_TOPMOST here.
        //
        // The overlay is only a visual aid; it must not "own" global top-most
        // order because that can disturb taskbar z-order and shell heuristics.
        // We still keep:
        // - WS_EX_NOACTIVATE: never steal keyboard focus.
        // - WS_EX_TRANSPARENT: pass mouse through.
        // - WS_EX_TOOLWINDOW: stay out of Alt-Tab/taskbar as a normal app window.
        let ex_style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;

        // Apply a small inset so the window is not exactly virtual-screen-sized.
        // Keep a fallback to full size if dimensions are too small to inset.
        let (wx, wy, ww, wh) = if vw > OVERLAY_FULLSCREEN_AVOID_MARGIN_PX * 2
            && vh > OVERLAY_FULLSCREEN_AVOID_MARGIN_PX * 2
        {
            (
                vx + OVERLAY_FULLSCREEN_AVOID_MARGIN_PX,
                vy + OVERLAY_FULLSCREEN_AVOID_MARGIN_PX,
                vw - OVERLAY_FULLSCREEN_AVOID_MARGIN_PX * 2,
                vh - OVERLAY_FULLSCREEN_AVOID_MARGIN_PX * 2,
            )
        } else {
            (vx, vy, vw, vh)
        };

        let hwnd = CreateWindowExW(
            ex_style,
            CLASS_NAME.as_ptr(),
            std::ptr::null(), // no title
            WS_POPUP,
            wx,
            wy,
            ww,
            wh,
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

        // Create the trail renderer.
        let renderer: Box<dyn TrailRenderer> = match GdiRenderer::new(hwnd, &config, ww, wh) {
            Ok(r) => Box::new(r),
            Err(e) => {
                error!("failed to create GDI renderer: {e}");
                DestroyWindow(hwnd);
                UnregisterClassW(CLASS_NAME.as_ptr(), hinstance);
                let _ = bridge.join();
                return;
            }
        };

        let (r, g, b) = config.color;
        debug!(
            "Overlay window created: hwnd={:?}, size={}x{} at ({},{}), color=#{:02X}{:02X}{:02X}, width={}",
            hwnd, ww, wh, wx, wy, r, g, b, config.pen_width
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

        // Same policy as the trail window: non-activating overlay UI that does
        // not enforce global top-most ownership.
        let label_ex_style =
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;

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

        // Create font for the label.
        let font_name: Vec<u16> = format!("{}\0", config.label_font_family)
            .encode_utf16()
            .collect();
        let font_weight = config.label_font_weight.clamp(0, 1000);
        let label_font = CreateFontW(
            config.label_font_size, // height
            0,                      // width (auto)
            0,                      // escapement
            0,                      // orientation
            font_weight,            // weight
            0,                      // italic
            0,                      // underline
            0,                      // strikeout
            DEFAULT_CHARSET as u32, // charset
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH | FF_DONTCARE) as u32,
            font_name.as_ptr(),
        );

        // Store state in thread-local.
        OVERLAY_STATE.with(|cell| {
            *cell.borrow_mut() = Some(OverlayState {
                hwnd,
                trail: Vec::with_capacity(256),
                origin_x: wx,
                origin_y: wy,
                renderer,
                label_padding: config.label_padding,
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
            if let Some(mut state) = cell.borrow_mut().take() {
                state.renderer.cleanup();
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

        state.renderer.clear();

        debug!("Overlay: StartGesture — showing window");
        (Some(state.hwnd), state.label_hwnd)
    });

    if let Some(hwnd) = hwnd {
        unsafe {
            if !label_hwnd.is_null() {
                ShowWindow(label_hwnd, SW_HIDE);
            }
            // Use SetWindowPos with HWND_TOP (not HWND_TOPMOST) so we can bring
            // the overlay in front of regular windows without pinning it above
            // all system UI. SWP_SHOWWINDOW avoids a separate ShowWindow call.
            SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
    }
}

/// Handle `WM_OVERLAY_TRACK`: draw new segment into back buffer, invalidate dirty rect.
fn handle_track(x: i32, y: i32) {
    OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return,
        };

        trace!("Overlay: TrackPoint ({x}, {y})");
        state.last_track_pt = Some((x, y));
        let new_pt = POINT {
            x: x - state.origin_x,
            y: y - state.origin_y,
        };

        let prev = state.trail.last().copied();
        state.trail.push(new_pt);

        if let Some(prev) = prev {
            state.renderer.draw_segment(prev, new_pt);

            let pad = state.renderer.pen_width() / 2 + 1;
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
fn handle_end() {
    let (hwnd, label_hwnd) = OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return (None, std::ptr::null_mut()),
        };

        state.trail.clear();
        state.label_text = None;

        state.renderer.clear();

        debug!("Overlay: EndGesture — hiding window");
        (Some(state.hwnd), state.label_hwnd)
    });
    // Borrow is now released — safe to call Win32 APIs that trigger WndProc.

    if let Some(hwnd) = hwnd {
        unsafe {
            if !label_hwnd.is_null() {
                ShowWindow(label_hwnd, SW_HIDE);
            }
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
fn handle_label(text_ptr: WPARAM) {
    let text = if text_ptr == 0 {
        None
    } else {
        Some(unsafe { *Box::from_raw(text_ptr as *mut String) })
    };
    let wide_text = text.as_ref().map(|t| {
        t.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    });

    let (label_hwnd, label_font, track_pt, label_padding) = OVERLAY_STATE.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let state = match borrow.as_mut() {
            Some(s) => s,
            None => return (std::ptr::null_mut(), std::ptr::null_mut(), None, 0),
        };
        state.label_text = text;
        (
            state.label_hwnd,
            state.label_font,
            state.last_track_pt,
            state.label_padding,
        )
    });

    if label_hwnd.is_null() {
        return;
    }

    match wide_text {
        None => {
            unsafe { ShowWindow(label_hwnd, SW_HIDE) };
        }
        Some(wide) => unsafe {
            let dc = GetDC(label_hwnd);
            let old_font = SelectObject(dc, label_font as *mut _);

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
            let win_w = text_w + label_padding * 2;
            let win_h = text_h + label_padding * 2;
            let corner_radius = (label_padding + 6).clamp(8, 24);

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
                HWND_TOP,
                win_x,
                win_y,
                win_w,
                win_h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            // Rounded region is owned by the window after SetWindowRgn succeeds.
            // Only delete it on failure to avoid double-free.
            let rgn = CreateRoundRectRgn(0, 0, win_w + 1, win_h + 1, corner_radius, corner_radius);
            if !rgn.is_null() && SetWindowRgn(label_hwnd, rgn, 1) == 0 {
                DeleteObject(rgn as *mut _);
            }
            InvalidateRect(label_hwnd, std::ptr::null(), 1);
        },
    }
}

// ---------------------------------------------------------------------------
// Label WndProc
// ---------------------------------------------------------------------------

/// Window procedure for the label overlay window.
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

            let black_brush = CreateSolidBrush(0x00000000);
            FillRect(hdc, &ps.rcPaint, black_brush);
            DeleteObject(black_brush as *mut _);

            OVERLAY_STATE.with(|cell| {
                let borrow = cell.borrow();
                if let Some(state) = borrow.as_ref() {
                    if let Some(text) = &state.label_text {
                        let old_font = SelectObject(hdc, state.label_font as *mut _);
                        SetBkMode(hdc, TRANSPARENT as i32);
                        SetTextColor(hdc, 0x00FFFFFF); // white

                        let wide: Vec<u16> =
                            text.encode_utf16().chain(std::iter::once(0)).collect();

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
// Overlay WndProc
// ---------------------------------------------------------------------------

/// Window procedure for the overlay window.
unsafe extern "system" fn overlay_wnd_proc(
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

            OVERLAY_STATE.with(|cell| {
                let borrow = cell.borrow();
                if let Some(state) = borrow.as_ref() {
                    state.renderer.paint(hdc, &ps.rcPaint);
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
