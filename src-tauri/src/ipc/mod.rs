mod protocol;

pub use protocol::EngineStatus;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::{ConfigApplyResult, ConfigObservation};
#[cfg(windows)]
pub use windows::{ControlError, EngineControl, EngineServer, ServerExit};

#[cfg(not(windows))]
mod unsupported {
    use super::EngineStatus;
    use crate::config::{ActiveConfig, ConfigDocument, ConfigOwner};
    use std::fmt;
    use std::path::Path;
    use std::sync::{atomic::AtomicBool, Arc};

    #[derive(Debug)]
    pub struct ControlError;

    impl fmt::Display for ControlError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("Engine IPC is not implemented on this platform")
        }
    }

    impl std::error::Error for ControlError {}

    #[derive(Clone)]
    pub struct EngineControl;

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

    impl EngineControl {
        pub fn connect_or_start(
            _executable: &Path,
            _config_dir: &Path,
        ) -> Result<Self, ControlError> {
            Err(ControlError)
        }

        pub fn ping(&self) -> Result<(), ControlError> {
            Err(ControlError)
        }

        pub fn status(&self) -> Result<EngineStatus, ControlError> {
            Err(ControlError)
        }

        pub fn shutdown(&self) -> Result<bool, ControlError> {
            Err(ControlError)
        }

        pub(crate) fn current_config(&self) -> Result<ConfigObservation, ControlError> {
            Err(ControlError)
        }

        pub(crate) fn apply_config(
            &self,
            _document: ConfigDocument,
            _expected_revision: u64,
        ) -> Result<ConfigApplyResult, ControlError> {
            Err(ControlError)
        }

        pub(crate) fn apply_config_bytes(
            &self,
            _bytes: Vec<u8>,
            _expected_revision: u64,
        ) -> Result<ConfigApplyResult, ControlError> {
            Err(ControlError)
        }
    }

    pub struct EngineServer;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub enum ServerExit {
        Shutdown,
        Stopped,
    }

    impl EngineServer {
        pub fn new(_config_dir: &Path) -> Result<Option<Self>, ControlError> {
            Err(ControlError)
        }

        pub fn run<F>(
            self,
            _stop: Arc<AtomicBool>,
            _config_owner: ConfigOwner,
            _on_applied: F,
        ) -> Result<ServerExit, ControlError>
        where
            F: FnMut(&ActiveConfig, u64),
        {
            Err(ControlError)
        }
    }
}

#[cfg(not(windows))]
pub(crate) use unsupported::{ConfigApplyResult, ConfigObservation};
#[cfg(not(windows))]
pub use unsupported::{ControlError, EngineControl, EngineServer, ServerExit};
