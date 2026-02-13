//! Gesture action execution.
//!
//! Maps recognised gestures to user-defined actions (currently keyboard
//! shortcuts) and executes them by synthesising input via the Win32
//! [`SendInput`] API.

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
}

/// Parse a human-readable key name into a Win32 virtual-key code.
///
/// Supports:
/// - Modifier keys: `ctrl`, `alt`, `shift`, `win`
/// - Navigation: `left`, `right`, `up`, `down`, `tab`, `enter`, `escape`,
///   `backspace`, `delete`, `home`, `end`, `pageup`, `pagedown`
/// - Function keys: `f1` – `f12`
/// - Single characters: `a`–`z`, `0`–`9`
///
/// Returns `None` for unrecognised names.
pub fn parse_key(name: &str) -> Option<u16> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;

        match name.to_ascii_lowercase().as_str() {
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

            // Function keys
            "f1" => Some(VK_F1),
            "f2" => Some(VK_F2),
            "f3" => Some(VK_F3),
            "f4" => Some(VK_F4),
            "f5" => Some(VK_F5),
            "f6" => Some(VK_F6),
            "f7" => Some(VK_F7),
            "f8" => Some(VK_F8),
            "f9" => Some(VK_F9),
            "f10" => Some(VK_F10),
            "f11" => Some(VK_F11),
            "f12" => Some(VK_F12),

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

/// Execute an [`Action`] by synthesising OS input.
///
/// For [`Action::Keyboard`], this presses all keys in order (modifiers first
/// by convention in the config), then releases them in reverse order. Each
/// key name is resolved via [`parse_key`]; unrecognised names are logged and
/// skipped.
///
/// # Safety
///
/// Calls [`SendInput`] which requires no special privileges but will inject
/// real keyboard events into the focused window.
pub fn execute(action: &Action) {
    match action {
        Action::Keyboard { keys } => execute_keyboard(keys),
    }
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
/// requires the `KEYEVENTF_EXTENDEDKEY` flag for [`SendInput`].
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

    #[cfg(windows)]
    mod windows_tests {
        use super::*;

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
            for i in 1..=12 {
                assert!(
                    parse_key(&format!("f{i}")).is_some(),
                    "f{i} should be recognised"
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
