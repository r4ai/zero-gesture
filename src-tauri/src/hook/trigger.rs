use crate::config;
use crate::config::GestureStep;

/// Mouse buttons used by the hook state machine.
///
/// This hook-local enum normalizes configuration values into a compact form
/// used by matching and event processing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum TriggerButton {
    Left,
    Right,
    Middle,
}

impl TriggerButton {
    /// Convert config trigger enum into hook-local representation.
    pub(super) fn from_config(trigger: &config::TriggerButton) -> Self {
        match trigger {
            config::TriggerButton::LeftClick => TriggerButton::Left,
            config::TriggerButton::RightClick => TriggerButton::Right,
            config::TriggerButton::MiddleClick => TriggerButton::Middle,
        }
    }

    /// Convert this button into the equivalent click [`GestureStep`].
    pub(super) fn to_step(self) -> GestureStep {
        match self {
            TriggerButton::Left => GestureStep::LeftClick,
            TriggerButton::Right => GestureStep::RightClick,
            TriggerButton::Middle => GestureStep::MiddleClick,
        }
    }

    /// Convert this button into `SendInput` mouse-button-down flags.
    #[cfg(windows)]
    pub(super) fn send_input_down_flag(self) -> u32 {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_RIGHTDOWN,
        };

        match self {
            TriggerButton::Left => MOUSEEVENTF_LEFTDOWN,
            TriggerButton::Right => MOUSEEVENTF_RIGHTDOWN,
            TriggerButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
        }
    }

    /// Convert this button into `SendInput` mouse-button-up flags.
    #[cfg(windows)]
    pub(super) fn send_input_up_flag(self) -> u32 {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_RIGHTUP,
        };

        match self {
            TriggerButton::Left => MOUSEEVENTF_LEFTUP,
            TriggerButton::Right => MOUSEEVENTF_RIGHTUP,
            TriggerButton::Middle => MOUSEEVENTF_MIDDLEUP,
        }
    }
}
