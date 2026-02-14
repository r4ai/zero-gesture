/// Which mouse button triggers gestures.
///
/// Maps to the corresponding Win32 `WM_*BUTTONDOWN` / `WM_*BUTTONUP` message
/// constants and [`SendInput`] flags. Defaults to [`TriggerButton::Right`]
/// for unrecognised configuration values.
///
/// # Examples
///
/// ```ignore
/// let btn = TriggerButton::from_config("middle");
/// assert_eq!(btn, TriggerButton::Middle);
///
/// let btn = TriggerButton::from_config("unknown");
/// assert_eq!(btn, TriggerButton::Right); // fallback
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TriggerButton {
    Right,
    Middle,
}

impl TriggerButton {
    /// Parse a trigger button name from the user configuration string.
    ///
    /// Recognised values (case-insensitive): `"middle"`. Everything else
    /// (including `"right"`) maps to [`TriggerButton::Right`].
    pub(super) fn from_config(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "middle" => TriggerButton::Middle,
            _ => TriggerButton::Right,
        }
    }

    /// Return the Win32 `WM_*BUTTONDOWN` message constant for this trigger.
    #[cfg(windows)]
    pub(super) fn down_msg(self) -> u32 {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_MBUTTONDOWN, WM_RBUTTONDOWN};
        match self {
            TriggerButton::Right => WM_RBUTTONDOWN,
            TriggerButton::Middle => WM_MBUTTONDOWN,
        }
    }

    /// Return the Win32 `WM_*BUTTONUP` message constant for this trigger.
    #[cfg(windows)]
    pub(super) fn up_msg(self) -> u32 {
        use windows_sys::Win32::UI::WindowsAndMessaging::{WM_MBUTTONUP, WM_RBUTTONUP};
        match self {
            TriggerButton::Right => WM_RBUTTONUP,
            TriggerButton::Middle => WM_MBUTTONUP,
        }
    }

    /// Return the [`SendInput`] `MOUSEEVENTF_*DOWN` flag for this trigger.
    #[cfg(windows)]
    pub(super) fn send_input_down_flag(self) -> u32 {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_RIGHTDOWN,
        };
        match self {
            TriggerButton::Right => MOUSEEVENTF_RIGHTDOWN,
            TriggerButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
        }
    }

    /// Return the [`SendInput`] `MOUSEEVENTF_*UP` flag for this trigger.
    #[cfg(windows)]
    pub(super) fn send_input_up_flag(self) -> u32 {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTUP,
        };
        match self {
            TriggerButton::Right => MOUSEEVENTF_RIGHTUP,
            TriggerButton::Middle => MOUSEEVENTF_MIDDLEUP,
        }
    }
}
