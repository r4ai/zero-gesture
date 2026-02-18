//! Gesture action execution.
//!
//! Maps recognised gestures to user-defined actions and executes them via
//! Win32 APIs.

use log::{debug, warn};
use serde::{Deserialize, Serialize};

/// An action that can be triggered by a gesture.
///
/// Uses serde's internally-tagged representation so that the JSON looks like:
///
/// ```json
/// { "type": "keyboard", "keys": ["alt", "left"] }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Action {
    /// Simulate a keyboard shortcut by pressing the given keys simultaneously.
    Keyboard { keys: Vec<String> },
    /// Scroll the focused surface to its bottom using Win32 messages with
    /// keyboard fallback.
    #[serde(rename = "scroll_to_bottom")]
    ScrollToBottom,
}

/// Parse a human-readable key name into a Win32 virtual-key code.
///
/// Supports:
/// - Modifier keys: `ctrl`, `alt`, `shift`, `win`
/// - Navigation: `left`, `right`, `up`, `down`, `tab`, `enter`, `escape`,
///   `backspace`, `delete`, `home`, `end`, `pageup`, `pagedown`
/// - Function keys: `f1` – `f24`
/// - Single characters: `a`–`z`, `0`–`9`
///
/// Returns `None` for unrecognised names.
pub fn parse_key(name: &str) -> Option<u16> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

        let lower = name.to_ascii_lowercase();
        let lower = lower.as_str();

        if let Some(function_number) = lower.strip_prefix('f') {
            if !function_number.is_empty() && function_number.bytes().all(|ch| ch.is_ascii_digit())
            {
                let function_number = function_number.parse::<u16>().ok()?;
                if (1..=24).contains(&function_number) {
                    return Some(VK_F1 + (function_number - 1));
                }
                return None;
            }
        }

        match lower {
            // Modifiers
            "ctrl" | "control" => Some(VK_CONTROL),
            "alt" | "menu" => Some(VK_MENU),
            "shift" => Some(VK_SHIFT),
            "win" | "lwin" | "super" => Some(VK_LWIN),

            // Navigation / editing
            "left" => Some(VK_LEFT),
            "right" => Some(VK_RIGHT),
            "up" => Some(VK_UP),
            "down" => Some(VK_DOWN),
            "tab" => Some(VK_TAB),
            "enter" | "return" => Some(VK_RETURN),
            "escape" | "esc" => Some(VK_ESCAPE),
            "backspace" => Some(VK_BACK),
            "delete" | "del" => Some(VK_DELETE),
            "home" => Some(VK_HOME),
            "end" => Some(VK_END),
            "pageup" | "pgup" => Some(VK_PRIOR),
            "pagedown" | "pgdn" => Some(VK_NEXT),
            "space" => Some(VK_SPACE),

            // Single character keys (a-z → VK 0x41..0x5A, 0-9 → VK 0x30..0x39)
            s if s.len() == 1 => {
                let ch = s.as_bytes()[0];
                match ch {
                    b'a'..=b'z' => Some((ch - b'a' + 0x41) as u16),
                    b'0'..=b'9' => Some((ch - b'0' + 0x30) as u16),
                    _ => None,
                }
            }

            _ => None,
        }
    }

    #[cfg(not(windows))]
    {
        let _ = name;
        None
    }
}

/// Generate a human-readable label for an action by capitalizing key names
/// and joining them with ` + `.
///
/// # Examples
///
/// ```
/// use zero_gesture_lib::executor::{Action, generate_label};
///
/// let action = Action::Keyboard {
///     keys: vec!["alt".into(), "left".into()],
/// };
/// assert_eq!(generate_label(&action), "Alt + Left");
/// ```
pub fn generate_label(action: &Action) -> String {
    match action {
        Action::Keyboard { keys } => keys
            .iter()
            .map(|k| {
                let mut chars = k.chars();
                match chars.next() {
                    Some(c) => {
                        let mut s = c.to_uppercase().to_string();
                        s.push_str(chars.as_str());
                        s
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" + "),
        Action::ScrollToBottom => "Scroll To Bottom".to_string(),
    }
}

/// Execute an [`Action`] by synthesising OS input.
///
/// For [`Action::Keyboard`], this presses all keys in order (modifiers first
/// by convention in the config), then releases them in reverse order. Each
/// key name is resolved via [`parse_key`]; unrecognised names are logged and
/// skipped.
///
/// # Safety
///
/// Calls `SendInput` which requires no special privileges but will inject
/// real keyboard events into the focused window.
pub fn execute(action: &Action) {
    match action {
        Action::Keyboard { keys } => execute_keyboard(keys),
        Action::ScrollToBottom => execute_scroll_to_bottom(),
    }
}

#[cfg(windows)]
fn execute_scroll_to_bottom() {
    use std::mem::size_of;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetScrollInfo, GetWindowThreadProcessId,
        SendMessageTimeoutW, GUITHREADINFO, SB_BOTTOM, SB_VERT, SCROLLINFO, SIF_PAGE, SIF_POS,
        SIF_RANGE, SMTO_ABORTIFHUNG, WM_VSCROLL,
    };

    unsafe fn preferred_scroll_target(foreground: HWND) -> HWND {
        if foreground.is_null() {
            return null_mut();
        }

        let thread_id = unsafe { GetWindowThreadProcessId(foreground, null_mut()) };
        if thread_id == 0 {
            return foreground;
        }

        let mut thread_info: GUITHREADINFO = unsafe { std::mem::zeroed() };
        thread_info.cbSize = size_of::<GUITHREADINFO>() as u32;
        if unsafe { GetGUIThreadInfo(thread_id, &mut thread_info) } != 0 {
            if !thread_info.hwndFocus.is_null() {
                return thread_info.hwndFocus;
            }
            if !thread_info.hwndActive.is_null() {
                return thread_info.hwndActive;
            }
        }
        foreground
    }

    unsafe fn try_scroll_bottom_with_vscroll(hwnd: HWND) -> bool {
        if hwnd.is_null() {
            return false;
        }

        let mut before: SCROLLINFO = unsafe { std::mem::zeroed() };
        before.cbSize = size_of::<SCROLLINFO>() as u32;
        before.fMask = (SIF_RANGE | SIF_PAGE | SIF_POS) as u32;
        if unsafe { GetScrollInfo(hwnd, SB_VERT, &mut before) } == 0 {
            return false;
        }

        // No usable vertical scroll range.
        if before.nMax <= before.nMin {
            return false;
        }

        let mut result: usize = 0;
        if unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_VSCROLL,
                SB_BOTTOM as WPARAM,
                0 as LPARAM,
                SMTO_ABORTIFHUNG,
                100,
                &mut result,
            )
        } == 0
        {
            return false;
        }

        let mut after: SCROLLINFO = unsafe { std::mem::zeroed() };
        after.cbSize = size_of::<SCROLLINFO>() as u32;
        after.fMask = (SIF_RANGE | SIF_PAGE | SIF_POS) as u32;
        if unsafe { GetScrollInfo(hwnd, SB_VERT, &mut after) } == 0 {
            return false;
        }

        let page = i32::try_from(after.nPage).unwrap_or(i32::MAX);
        let effective_bottom = if page > 0 {
            after.nMax.saturating_sub(page.saturating_sub(1))
        } else {
            after.nMax
        };
        after.nPos >= effective_bottom || after.nPos > before.nPos
    }

    let foreground = unsafe { GetForegroundWindow() };
    let target = unsafe { preferred_scroll_target(foreground) };
    if unsafe { try_scroll_bottom_with_vscroll(target) }
        || (foreground != target && unsafe { try_scroll_bottom_with_vscroll(foreground) })
    {
        debug!("ScrollToBottom handled via WM_VSCROLL/SB_BOTTOM");
        return;
    }

    warn!("ScrollToBottom fallback to keyboard shortcut (Ctrl+End), WM_VSCROLL was not effective");
    let fallback_keys = [String::from("ctrl"), String::from("end")];
    execute_keyboard(&fallback_keys);
}

#[cfg(not(windows))]
fn execute_scroll_to_bottom() {
    warn!("ScrollToBottom action execution is only supported on Windows");
}

#[cfg(windows)]
fn execute_keyboard(keys: &[String]) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, KEYEVENTF_KEYUP};

    let vks: Vec<u16> = keys
        .iter()
        .filter_map(|k| {
            let vk = parse_key(k);
            if vk.is_none() {
                warn!("Unknown key name in binding: {:?}", k);
            }
            vk
        })
        .collect();

    if vks.is_empty() {
        return;
    }

    // Build input array: key-downs in order, then key-ups in reverse.
    let mut inputs: Vec<INPUT> = Vec::with_capacity(vks.len() * 2);

    for &vk in &vks {
        inputs.push(make_keyboard_input(vk, 0));
    }
    for &vk in vks.iter().rev() {
        inputs.push(make_keyboard_input(vk, KEYEVENTF_KEYUP));
    }

    debug!(
        "Sending keyboard input: {} key(s), {} events",
        vks.len(),
        inputs.len()
    );

    let expected_events = inputs.len() as u32;
    let sent_events = unsafe {
        SendInput(
            expected_events,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };

    if sent_events == 0 {
        warn!(
            "SendInput failed to inject any keyboard events (expected {} events)",
            expected_events
        );
    } else if sent_events < expected_events {
        warn!(
            "SendInput injected only {} of {} keyboard events",
            sent_events, expected_events
        );
    }
}

#[cfg(not(windows))]
fn execute_keyboard(keys: &[String]) {
    let _ = keys;
    warn!("Keyboard action execution is only supported on Windows");
}

/// Returns `true` if the given virtual-key code is an extended key that
/// requires the `KEYEVENTF_EXTENDEDKEY` flag for `SendInput`.
#[cfg(windows)]
fn is_extended_key(vk: u16) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

    matches!(
        vk,
        VK_UP
            | VK_DOWN
            | VK_LEFT
            | VK_RIGHT
            | VK_HOME
            | VK_END
            | VK_PRIOR
            | VK_NEXT
            | VK_INSERT
            | VK_DELETE
            | VK_LWIN
            | VK_RWIN
    )
}

/// Build an [`INPUT`] struct for a keyboard event.
///
/// Automatically sets `KEYEVENTF_EXTENDEDKEY` for navigation and other
/// extended keys so that applications recognise them correctly.
#[cfg(windows)]
fn make_keyboard_input(
    vk: u16,
    flags: u32,
) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

    let flags = if is_extended_key(vk) {
        flags | KEYEVENTF_EXTENDEDKEY
    } else {
        flags
    };

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_keyboard_serialization_roundtrip() {
        let action = Action::Keyboard {
            keys: vec!["alt".to_string(), "left".to_string()],
        };
        let json = serde_json::to_string(&action).unwrap();
        let deserialized: Action = serde_json::from_str(&json).unwrap();
        assert_eq!(action, deserialized);
    }

    #[test]
    fn action_keyboard_deserialize_from_json() {
        let json = r#"{"type": "keyboard", "keys": ["ctrl", "w"]}"#;
        let action: Action = serde_json::from_str(json).unwrap();
        assert_eq!(
            action,
            Action::Keyboard {
                keys: vec!["ctrl".to_string(), "w".to_string()]
            }
        );
    }

    #[test]
    fn action_scroll_to_bottom_deserialize_from_json() {
        let json = r#"{"type": "scroll_to_bottom"}"#;
        let action: Action = serde_json::from_str(json).unwrap();
        assert_eq!(action, Action::ScrollToBottom);
    }

    #[cfg(windows)]
    mod windows_tests {
        use super::*;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_F1;

        #[test]
        fn parse_key_modifiers() {
            assert!(parse_key("ctrl").is_some());
            assert!(parse_key("alt").is_some());
            assert!(parse_key("shift").is_some());
            assert!(parse_key("win").is_some());
        }

        #[test]
        fn parse_key_navigation() {
            assert!(parse_key("left").is_some());
            assert!(parse_key("right").is_some());
            assert!(parse_key("up").is_some());
            assert!(parse_key("down").is_some());
            assert!(parse_key("tab").is_some());
            assert!(parse_key("enter").is_some());
            assert!(parse_key("escape").is_some());
        }

        #[test]
        fn parse_key_function_keys() {
            for i in 1..=24 {
                assert!(
                    parse_key(&format!("f{i}")).is_some(),
                    "f{i} should be recognised"
                );
            }
            assert!(parse_key("f0").is_none());
            assert!(parse_key("f25").is_none());
        }

        #[test]
        fn parse_key_function_keys_use_contiguous_vk_codes() {
            for i in 1..=24_u16 {
                let key = format!("f{i}");
                assert_eq!(
                    parse_key(&key),
                    Some(VK_F1 + (i - 1)),
                    "{key} should map to VK_F1 + {}",
                    i - 1
                );
            }
        }

        #[test]
        fn parse_key_characters() {
            for ch in b'a'..=b'z' {
                let name = String::from(ch as char);
                assert!(parse_key(&name).is_some(), "{name} should be recognised");
            }
            for ch in b'0'..=b'9' {
                let name = String::from(ch as char);
                assert!(parse_key(&name).is_some(), "{name} should be recognised");
            }
        }

        #[test]
        fn parse_key_unknown_returns_none() {
            assert!(parse_key("nonexistent").is_none());
            assert!(parse_key("").is_none());
        }

        #[test]
        fn parse_key_case_insensitive() {
            assert_eq!(parse_key("Ctrl"), parse_key("ctrl"));
            assert_eq!(parse_key("ALT"), parse_key("alt"));
            assert_eq!(parse_key("Shift"), parse_key("shift"));
        }
    }
}
