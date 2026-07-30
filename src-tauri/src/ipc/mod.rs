mod protocol;

pub use protocol::EngineStatus;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{ControlError, EngineControl, EngineServer, ServerExit};

#[cfg(not(windows))]
mod unsupported {
    use super::EngineStatus;
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

        pub fn run(self, _stop: Arc<AtomicBool>) -> Result<ServerExit, ControlError> {
            Err(ControlError)
        }
    }
}

#[cfg(not(windows))]
pub use unsupported::{ControlError, EngineControl, EngineServer, ServerExit};
