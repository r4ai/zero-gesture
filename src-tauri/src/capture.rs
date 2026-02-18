//! One-shot window capture via a global low-level mouse hook.
//!
//! When [`start`] is called, a dedicated OS thread installs a `WH_MOUSE_LL`
//! hook.  The first left-button-down event that is **not** injected causes the
//! hook to:
//!
//! 1. Suppress the click (return non-zero from the hook callback).
//! 2. Resolve the window under the cursor with `WindowFromPoint`.
//! 3. Collect [`ForegroundWindowInfo`] for that window.
//! 4. Emit a `window-captured` Tauri event carrying the info.
//! 5. Post `WM_QUIT` to shut the hook thread down.
//!
//! Calling [`stop`] at any time causes the hook thread to exit without emitting
//! an event (cancel path).

#[cfg(windows)]
pub mod win32 {
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;

    use log::{debug, error, info};
    use tauri::Emitter;
    use windows_sys::Win32::{
        Foundation::{LPARAM, LRESULT, WPARAM},
        UI::WindowsAndMessaging::{
            CallNextHookEx, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
            UnhookWindowsHookEx, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_MOUSE_LL, WM_LBUTTONDOWN,
            WM_QUIT,
        },
    };

    use crate::window_info::{get_window_info_by_hwnd, ForegroundWindowInfo};

    // ---------------------------------------------------------------------------
    // Thread-local callback state
    // ---------------------------------------------------------------------------

    struct CaptureState {
        /// Tauri app handle used to emit events.
        app: tauri::AppHandle,
        /// Set to `true` once a capture is consumed (prevents double-emit).
        captured: bool,
    }

    thread_local! {
        static CAPTURE_STATE: RefCell<Option<CaptureState>> = const { RefCell::new(None) };
    }

    // ---------------------------------------------------------------------------
    // Hook callback
    // ---------------------------------------------------------------------------

    /// Win32 low-level mouse hook callback for one-shot window capture.
    ///
    /// Suppresses the first real left-button-down event, resolves the window
    /// under the cursor, emits `window-captured`, then posts `WM_QUIT` to exit
    /// the hook thread.
    ///
    /// # Safety
    ///
    /// `l_param` is cast to `*const MSLLHOOKSTRUCT` per `WH_MOUSE_LL` contract.
    unsafe extern "system" fn capture_proc(
        n_code: i32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> LRESULT {
        if n_code < 0 {
            return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
        }

        if w_param as u32 != WM_LBUTTONDOWN {
            return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
        }

        let info = &*(l_param as *const MSLLHOOKSTRUCT);

        // Ignore injected (synthetic) clicks.
        if info.flags & LLMHF_INJECTED != 0 {
            return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
        }

        debug!(
            "capture_proc: left-button-down at ({}, {})",
            info.pt.x, info.pt.y
        );

        let suppressed = CAPTURE_STATE.with(|cell| {
            let mut borrow = cell.borrow_mut();
            let state = match borrow.as_mut() {
                Some(s) if !s.captured => s,
                _ => return false,
            };
            state.captured = true;

            // Resolve window under cursor position.
            let hwnd = windows_sys::Win32::UI::WindowsAndMessaging::WindowFromPoint(info.pt);
            debug!("capture_proc: WindowFromPoint -> hwnd={:?}", hwnd);

            let window_info: ForegroundWindowInfo = if hwnd.is_null() {
                debug!("capture_proc: hwnd is null, using default ForegroundWindowInfo");
                ForegroundWindowInfo::default()
            } else {
                get_window_info_by_hwnd(hwnd)
            };

            debug!("capture_proc: window_info = {:?}", window_info);

            // Emit event to the frontend.
            if let Err(err) = state.app.emit("window-captured", &window_info) {
                error!("capture_proc: failed to emit window-captured: {err}");
            } else {
                info!("capture_proc: window-captured emitted");
            }

            // Ask message loop to exit.
            let tid = windows_sys::Win32::System::Threading::GetCurrentThreadId();
            PostThreadMessageW(tid, WM_QUIT, 0, 0);

            true
        });

        if suppressed {
            // Suppress the click so it doesn't reach the target window.
            1
        } else {
            CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
        }
    }

    // ---------------------------------------------------------------------------
    // Public API
    // ---------------------------------------------------------------------------

    /// Handle returned by [`start`] that can cancel an in-progress capture.
    pub struct CaptureHandle {
        tid: Arc<std::sync::atomic::AtomicU32>,
        cancelled: Arc<AtomicBool>,
    }

    impl CaptureHandle {
        /// Cancel the capture.  Safe to call even after the capture has already
        /// completed (no-op in that case).
        pub fn cancel(&self) {
            if self
                .cancelled
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let tid = self.tid.load(Ordering::Acquire);
                if tid != 0 {
                    unsafe {
                        PostThreadMessageW(tid, WM_QUIT, 0, 0);
                    }
                }
            }
        }
    }

    /// Starts a one-shot window capture.
    ///
    /// Spawns a dedicated OS thread that installs `WH_MOUSE_LL`.  Returns a
    /// [`CaptureHandle`] that can be used to cancel the capture before the user
    /// clicks.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let handle = start(app_handle);
    /// // … later, if user presses Escape:
    /// handle.cancel();
    /// ```
    pub fn start(app: tauri::AppHandle) -> CaptureHandle {
        let tid_arc = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));

        let tid_arc_clone = tid_arc.clone();
        let cancelled_clone = cancelled.clone();

        thread::Builder::new()
            .name("window-capture".to_string())
            .spawn(move || {
                run_capture_loop(app, tid_arc_clone, cancelled_clone);
            })
            .expect("failed to spawn window-capture thread");

        CaptureHandle {
            tid: tid_arc,
            cancelled,
        }
    }

    /// Main loop of the capture thread.
    fn run_capture_loop(
        app: tauri::AppHandle,
        tid_arc: Arc<std::sync::atomic::AtomicU32>,
        _cancelled: Arc<AtomicBool>,
    ) {
        unsafe {
            use windows_sys::Win32::System::Threading::GetCurrentThreadId;

            let tid = GetCurrentThreadId();
            tid_arc.store(tid, Ordering::Release);
            info!("capture thread started (tid={tid})");

            CAPTURE_STATE.with(|cell| {
                *cell.borrow_mut() = Some(CaptureState {
                    app,
                    captured: false,
                });
            });

            let hook = SetWindowsHookExW(WH_MOUSE_LL, Some(capture_proc), std::ptr::null_mut(), 0);
            if hook.is_null() {
                error!("SetWindowsHookExW failed for capture thread");
                return;
            }
            debug!("capture WH_MOUSE_LL hook installed (tid={tid})");

            let mut msg: MSG = std::mem::zeroed();
            loop {
                let ret = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                if ret == 0 || ret == -1 {
                    break;
                }
                windows_sys::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
            }

            UnhookWindowsHookEx(hook);
            debug!("capture WH_MOUSE_LL hook removed");

            CAPTURE_STATE.with(|cell| {
                *cell.borrow_mut() = None;
            });

            info!("capture thread stopped (tid={tid})");
        }
    }
}

/// Stub for non-Windows builds.
#[cfg(not(windows))]
pub mod win32 {
    use std::sync::atomic::AtomicU32;
    use std::sync::Arc;

    pub struct CaptureHandle {
        pub(crate) tid: Arc<AtomicU32>,
        pub(crate) cancelled: Arc<std::sync::atomic::AtomicBool>,
    }

    impl CaptureHandle {
        pub fn cancel(&self) {}
    }

    pub fn start(_app: tauri::AppHandle) -> CaptureHandle {
        CaptureHandle {
            tid: Arc::new(AtomicU32::new(0)),
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }
}
