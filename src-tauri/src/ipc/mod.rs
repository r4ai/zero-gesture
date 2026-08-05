mod protocol;

pub use protocol::EngineStatus;
pub(crate) use protocol::ErrorCode;

#[cfg(any(windows, target_os = "macos"))]
mod core;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;
#[cfg(windows)]
pub(crate) use windows::{acquire_settings_launch_gate, SettingsLaunchGate};
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, target_os = "macos"))]
pub(crate) use core::{
    ConfigApplyError, ConfigApplyPhase, ConfigApplyResult, ConfigObservation,
    WindowCaptureObservation, WindowCaptureStarted,
};
#[cfg(any(windows, target_os = "macos"))]
pub use core::{ControlError, EngineControl, EngineServer, ServerExit};
#[cfg(target_os = "macos")]
use macos as platform;

#[cfg(not(any(windows, target_os = "macos")))]
mod unsupported {
    use super::EngineStatus;
    use crate::config::ConfigDocument;
    use std::fmt;
    use std::path::Path;

    #[derive(Debug)]
    pub struct ControlError;

    impl fmt::Display for ControlError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("Engine IPC is not implemented on this platform")
        }
    }

    impl std::error::Error for ControlError {}

    impl ControlError {
        pub(crate) fn projection(_error: impl fmt::Display) -> Self {
            Self
        }
    }

    #[derive(Clone)]
    pub(crate) struct EngineControl;

    #[derive(Clone, Debug, serde::Serialize)]
    pub(crate) struct ConfigObservation {
        pub(crate) revision: u64,
        pub(crate) generation: u64,
        pub(crate) config: Option<ConfigDocument>,
    }

    #[derive(Clone, Debug, serde::Serialize)]
    pub(crate) struct ConfigApplyResult {
        pub(crate) current: ConfigObservation,
        pub(crate) durability_warning: bool,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum ConfigApplyPhase {
        Prepare,
        Commit,
        Query,
    }

    #[derive(Debug)]
    pub(crate) struct ConfigApplyError {
        pub(crate) phase: ConfigApplyPhase,
        pub(crate) source: ControlError,
    }

    impl EngineControl {
        pub(crate) fn connect_or_start(
            _executable: &Path,
            _config_dir: &Path,
        ) -> Result<Self, ControlError> {
            Err(ControlError)
        }

        pub(crate) fn ping(&self) -> Result<(), ControlError> {
            Err(ControlError)
        }

        pub(crate) fn status(&self) -> Result<EngineStatus, ControlError> {
            Err(ControlError)
        }

        pub(crate) fn shutdown(&self) -> Result<bool, ControlError> {
            Err(ControlError)
        }

        pub(crate) fn current_config(&self) -> Result<ConfigObservation, ControlError> {
            Err(ControlError)
        }

        pub(crate) fn apply_config(
            &self,
            _document: ConfigDocument,
            _expected_revision: u64,
        ) -> Result<ConfigApplyResult, ConfigApplyError> {
            Err(ConfigApplyError {
                phase: ConfigApplyPhase::Prepare,
                source: ControlError,
            })
        }

        pub(crate) fn apply_config_bytes(
            &self,
            _bytes: Vec<u8>,
            _expected_revision: u64,
        ) -> Result<ConfigApplyResult, ConfigApplyError> {
            Err(ConfigApplyError {
                phase: ConfigApplyPhase::Prepare,
                source: ControlError,
            })
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) use unsupported::EngineControl;
#[cfg(not(any(windows, target_os = "macos")))]
pub(crate) use unsupported::{ConfigApplyResult, ConfigObservation};
