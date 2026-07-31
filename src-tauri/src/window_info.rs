//! Foreground window detection.
//!
//! Provides information about the currently focused window — process name,
//! Win32 window class, and title — for use in per-application gesture bindings.

/// Information about the current foreground window.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ForegroundWindowInfo {
    /// Executable file name (e.g., "chrome.exe"), lowercased. `None` if unavailable.
    pub process_name: Option<String>,
    /// Win32 window class name (e.g., "CabinetWClass"). `None` if unavailable.
    pub window_class: Option<String>,
    /// Window title text. `None` if unavailable.
    pub title: Option<String>,
    /// macOS application bundle identifier. `None` if unavailable.
    pub bundle_identifier: Option<String>,
}

/// Retrieves information about the current foreground window.
///
/// Uses Win32 APIs to inspect the foreground window. All failures produce
/// `None` for the corresponding field (no panics).
#[cfg(windows)]
pub fn get_foreground_window_info() -> ForegroundWindowInfo {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    unsafe {
        let hwnd = GetForegroundWindow();
        get_window_info_by_hwnd(hwnd)
    }
}

/// Retrieves information about the specified window handle.
///
/// All failures produce `None` for the corresponding field (no panics).
#[cfg(windows)]
pub(crate) fn get_window_info_by_hwnd(
    hwnd: windows_sys::Win32::Foundation::HWND,
) -> ForegroundWindowInfo {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, SetLastError};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    };

    unsafe {
        if hwnd.is_null() {
            return ForegroundWindowInfo::default();
        }

        // Get process name
        let process_name = {
            let mut pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid != 0 {
                let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if !handle.is_null() {
                    let mut buf = [0u16; 1024];
                    let mut len = buf.len() as u32;
                    let ok = QueryFullProcessImageNameW(handle, 0, buf.as_mut_ptr(), &mut len);
                    CloseHandle(handle);
                    if ok != 0 && len > 0 {
                        let path = String::from_utf16_lossy(&buf[..len as usize]);
                        // Extract just the filename and lowercase it
                        path.rsplit('\\').next().map(|s| s.to_lowercase())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        // Get window class name
        let window_class = {
            let mut buf = [0u16; 256];
            let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
            if len > 0 {
                Some(String::from_utf16_lossy(&buf[..len as usize]))
            } else {
                None
            }
        };

        // Get window title
        let title = {
            // Distinguish "empty title" from Win32 API failure.
            SetLastError(0);
            let text_len = GetWindowTextLengthW(hwnd);
            if text_len > 0 {
                let mut buf = vec![0u16; (text_len + 1) as usize];
                SetLastError(0);
                let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
                if len > 0 {
                    Some(String::from_utf16_lossy(&buf[..len as usize]))
                } else if GetLastError() == 0 {
                    Some(String::new())
                } else {
                    None
                }
            } else if GetLastError() == 0 {
                Some(String::new())
            } else {
                None
            }
        };

        ForegroundWindowInfo {
            process_name,
            window_class,
            title,
            bundle_identifier: None,
        }
    }
}

/// Stub for non-Windows platforms.
#[cfg(not(windows))]
pub fn get_foreground_window_info() -> ForegroundWindowInfo {
    ForegroundWindowInfo::default()
}
