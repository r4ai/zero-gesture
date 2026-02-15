use crate::config;
use crate::config::GestureStep;

/// Mouse button used by the hook state machine.
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

    /// Convert this button to the equivalent gesture step.
    pub(super) fn to_step(self) -> GestureStep {
        match self {
            TriggerButton::Left => GestureStep::LeftClick,
            TriggerButton::Right => GestureStep::RightClick,
            TriggerButton::Middle => GestureStep::MiddleClick,
        }
    }
}
