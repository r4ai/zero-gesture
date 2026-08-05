use crate::config;
use crate::{tray, ConfigDir, SettingsEngineState};
use log::debug;
use std::fs;
use tauri_plugin_opener::OpenerExt;

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SettingsErrorCode {
    RevisionConflict,
    EngineUnavailable,
    EngineDisconnected,
    ValidationFailed,
    RequestRejected,
    FilesystemFailed,
    PlatformFailed,
    BackendFailed,
    CaptureStale,
    CaptureUnavailable,
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SettingsOperation {
    Query,
    Prepare,
    Commit,
    EnableDisable,
    Import,
    Export,
    OpenConfigDir,
    WindowCapture,
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct SettingsCommandError {
    code: SettingsErrorCode,
    operation: SettingsOperation,
    message: String,
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    current: Option<Box<crate::ipc::ConfigObservation>>,
}

impl SettingsCommandError {
    fn new(
        code: SettingsErrorCode,
        operation: SettingsOperation,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self {
            code,
            operation,
            message: message.into(),
            retryable,
            current: None,
        }
    }

    fn engine_unavailable(operation: SettingsOperation, message: impl Into<String>) -> Self {
        Self::new(
            SettingsErrorCode::EngineUnavailable,
            operation,
            message,
            true,
        )
    }

    fn filesystem(operation: SettingsOperation, message: impl Into<String>) -> Self {
        Self::new(
            SettingsErrorCode::FilesystemFailed,
            operation,
            message,
            true,
        )
    }

    fn platform(operation: SettingsOperation, message: impl Into<String>) -> Self {
        Self::new(SettingsErrorCode::PlatformFailed, operation, message, true)
    }

    fn from_control(
        error: crate::ipc::ControlError,
        current: Option<crate::ipc::ConfigObservation>,
        operation: SettingsOperation,
    ) -> Self {
        use crate::ipc::{ControlError, ErrorCode};
        let code = match &error {
            ControlError::Unavailable
            | ControlError::EndpointBusy
            | ControlError::Timeout
            | ControlError::Io(_) => SettingsErrorCode::EngineDisconnected,
            ControlError::Rejected(ErrorCode::ConfigRevisionConflict) => {
                SettingsErrorCode::RevisionConflict
            }
            ControlError::Rejected(
                ErrorCode::ConfigPayloadTooLarge | ErrorCode::ConfigValidationFailed,
            ) => SettingsErrorCode::ValidationFailed,
            ControlError::Rejected(ErrorCode::CaptureStale) => SettingsErrorCode::CaptureStale,
            ControlError::Rejected(
                ErrorCode::CaptureUnavailable | ErrorCode::CaptureBackendFailed,
            ) => SettingsErrorCode::CaptureUnavailable,
            ControlError::Rejected(ErrorCode::ConfigPersistenceFailed) => {
                SettingsErrorCode::BackendFailed
            }
            ControlError::Rejected(_) => SettingsErrorCode::RequestRejected,
            ControlError::SpawnFailed(_)
            | ControlError::Security(_)
            | ControlError::Protocol(_)
            | ControlError::ProjectionFailed(_) => SettingsErrorCode::BackendFailed,
        };
        Self {
            code,
            operation,
            message: error.to_string(),
            retryable: !matches!(
                code,
                SettingsErrorCode::ValidationFailed | SettingsErrorCode::RequestRejected
            ),
            current: current.map(Box::new),
        }
    }
}

fn control(
    engine: &SettingsEngineState,
    operation: SettingsOperation,
) -> Result<&crate::ipc::EngineControl, SettingsCommandError> {
    match engine {
        SettingsEngineState::Connected(control) => Ok(control),
        SettingsEngineState::Unavailable(message) => Err(SettingsCommandError::engine_unavailable(
            operation,
            message.clone(),
        )),
    }
}

fn control_error(
    control: &crate::ipc::EngineControl,
    error: crate::ipc::ControlError,
    operation: SettingsOperation,
) -> SettingsCommandError {
    let operation = match &error {
        crate::ipc::ControlError::Rejected(
            crate::ipc::ErrorCode::ConfigTokenMismatch
            | crate::ipc::ErrorCode::NoPreparedConfig
            | crate::ipc::ErrorCode::ConfigPersistenceFailed,
        ) => SettingsOperation::Commit,
        _ => operation,
    };
    let current = matches!(
        error,
        crate::ipc::ControlError::Rejected(crate::ipc::ErrorCode::ConfigRevisionConflict)
    )
    .then(|| control.current_config().ok())
    .flatten();
    SettingsCommandError::from_control(error, current, operation)
}

fn config_apply_error(
    control: &crate::ipc::EngineControl,
    error: crate::ipc::ConfigApplyError,
) -> SettingsCommandError {
    let operation = match error.phase {
        crate::ipc::ConfigApplyPhase::Prepare => SettingsOperation::Prepare,
        crate::ipc::ConfigApplyPhase::Commit => SettingsOperation::Commit,
        crate::ipc::ConfigApplyPhase::Query => SettingsOperation::Query,
    };
    control_error(control, error.source, operation)
}

/// Tauri command that opens (or focuses) the settings window.
#[tauri::command]
pub fn show_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    tray::show_settings_window(&app).map_err(|e| e.to_string())
}

/// Tauri command that retrieves the current configuration.
#[tauri::command]
pub(crate) fn get_config(
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<crate::ipc::ConfigObservation, SettingsCommandError> {
    let control = control(engine.inner(), SettingsOperation::Query)?;
    control
        .current_config()
        .map_err(|error| SettingsCommandError::from_control(error, None, SettingsOperation::Query))
}

/// Tauri command that routes the mutation to the Engine config owner.
#[tauri::command]
pub(crate) fn update_config(
    new_config: config::ConfigDocument,
    expected_revision: u64,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<crate::ipc::ConfigApplyResult, SettingsCommandError> {
    let control = control(engine.inner(), SettingsOperation::Prepare)?;
    control
        .apply_config(new_config, expected_revision)
        .map_err(|error| config_apply_error(control, error))
}

/// Tauri command that reads a JSON file and applies it as the new configuration.
#[tauri::command]
pub(crate) fn import_config(
    file_path: String,
    expected_revision: u64,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<crate::ipc::ConfigApplyResult, SettingsCommandError> {
    let raw = fs::read(&file_path).map_err(|error| {
        SettingsCommandError::filesystem(
            SettingsOperation::Import,
            format!("failed to read file: {error}"),
        )
    })?;
    let control = control(engine.inner(), SettingsOperation::Prepare)?;
    control
        .apply_config_bytes(raw, expected_revision)
        .map_err(|error| config_apply_error(control, error))
}

/// Tauri command that writes the current configuration as JSON to the given path.
#[tauri::command]
pub(crate) fn export_config(
    file_path: String,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<(), SettingsCommandError> {
    let control = control(engine.inner(), SettingsOperation::Export)?;
    let current = control.current_config().map_err(|error| {
        SettingsCommandError::from_control(error, None, SettingsOperation::Export)
    })?;
    let document = current.config.ok_or_else(|| {
        SettingsCommandError::new(
            SettingsErrorCode::RequestRejected,
            SettingsOperation::Export,
            "configuration is unavailable; repair it before export",
            false,
        )
    })?;
    config::export(&document, std::path::Path::new(&file_path)).map_err(|error| {
        SettingsCommandError::filesystem(SettingsOperation::Export, error.to_string())
    })
}

/// Tauri command that opens the config directory in the system file manager.
///
/// Creates the directory if it does not yet exist, then opens it via the
/// opener plugin from the Rust side (bypassing JS-side path scope restrictions).
#[tauri::command]
pub(crate) fn open_config_dir(
    app: tauri::AppHandle,
    config_dir: tauri::State<'_, ConfigDir>,
) -> Result<(), SettingsCommandError> {
    let path = &config_dir.0;
    fs::create_dir_all(path).map_err(|error| {
        SettingsCommandError::filesystem(
            SettingsOperation::OpenConfigDir,
            format!("failed to create config dir: {error}"),
        )
    })?;
    app.opener()
        .open_path(path.to_string_lossy().as_ref(), None::<&str>)
        .map_err(|error| {
            SettingsCommandError::platform(
                SettingsOperation::OpenConfigDir,
                format!("failed to open config dir: {error}"),
            )
        })
}

#[tauri::command]
pub(crate) fn set_enabled(
    enabled: bool,
    expected_revision: u64,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<crate::ipc::ConfigApplyResult, SettingsCommandError> {
    let control = control(engine.inner(), SettingsOperation::EnableDisable)?;
    control
        .set_enabled(enabled, expected_revision)
        .map_err(|error| control_error(control, error, SettingsOperation::EnableDisable))
}

/// Tauri command that retrieves foreground window information.
///
/// Returns the process name, Win32 window class, and title of the window
/// that was in the foreground at the time of the call. All fields may be
/// `None` if the corresponding information is unavailable.
///
/// # Example
///
/// ```ignore
/// let info = get_foreground_window_info();
/// println!("process: {:?}", info.process_name);
/// ```
#[tauri::command]
pub fn get_foreground_window_info() -> crate::window_info::ForegroundWindowInfo {
    let info = crate::window_info::get_foreground_window_info();
    debug!("get_foreground_window_info: {:?}", info);
    info
}

#[tauri::command]
pub(crate) fn start_window_capture(
    capture_id: u64,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<crate::ipc::WindowCaptureStarted, SettingsCommandError> {
    let control = control(engine.inner(), SettingsOperation::WindowCapture)?;
    control
        .begin_window_capture(capture_id)
        .map_err(|error| control_error(control, error, SettingsOperation::WindowCapture))
}

#[tauri::command]
pub(crate) fn poll_window_capture(
    capture_id: u64,
    epoch: u64,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<crate::ipc::WindowCaptureObservation, SettingsCommandError> {
    let control = control(engine.inner(), SettingsOperation::WindowCapture)?;
    control
        .poll_window_capture(capture_id, epoch)
        .map_err(|error| control_error(control, error, SettingsOperation::WindowCapture))
}

#[tauri::command]
pub(crate) fn stop_window_capture(
    capture_id: u64,
    epoch: u64,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<(), SettingsCommandError> {
    let control = control(engine.inner(), SettingsOperation::WindowCapture)?;
    control
        .cancel_window_capture(capture_id, epoch)
        .map_err(|error| control_error(control, error, SettingsOperation::WindowCapture))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{ConfigObservation, ControlError, ErrorCode};

    #[test]
    fn revision_conflict_is_a_typed_retryable_error_with_current_observation() {
        let current = ConfigObservation {
            revision: 9,
            generation: 9,
            config: Some(config::ConfigDocument::default()),
        };
        let error = SettingsCommandError::from_control(
            ControlError::Rejected(ErrorCode::ConfigRevisionConflict),
            Some(current),
            SettingsOperation::Prepare,
        );

        let value = serde_json::to_value(error).unwrap();
        assert_eq!(value["code"], "revision-conflict");
        assert_eq!(value["operation"], "prepare");
        assert_eq!(value["retryable"], true);
        assert_eq!(value["current"]["revision"], 9);
    }

    #[test]
    fn control_failures_keep_unavailable_disconnected_and_rejected_distinct() {
        let cases = [
            (ControlError::Unavailable, "engine-disconnected"),
            (
                ControlError::Rejected(ErrorCode::ConfigValidationFailed),
                "validation-failed",
            ),
            (
                ControlError::Rejected(ErrorCode::ConfigTokenMismatch),
                "request-rejected",
            ),
            (
                ControlError::Security("invalid peer".to_string()),
                "backend-failed",
            ),
        ];

        for (error, expected) in cases {
            let value = serde_json::to_value(SettingsCommandError::from_control(
                error,
                None,
                SettingsOperation::Prepare,
            ))
            .unwrap();
            assert_eq!(value["code"], expected);
        }
    }

    #[test]
    fn persistence_failure_identifies_the_commit_phase() {
        let value = serde_json::to_value(SettingsCommandError::from_control(
            ControlError::Rejected(ErrorCode::ConfigPersistenceFailed),
            None,
            SettingsOperation::Commit,
        ))
        .unwrap();
        assert_eq!(value["code"], "backend-failed");
        assert_eq!(value["operation"], "commit");
    }

    #[test]
    fn enable_disable_conflict_keeps_its_typed_operation() {
        let value = serde_json::to_value(SettingsCommandError::from_control(
            ControlError::Rejected(ErrorCode::ConfigRevisionConflict),
            None,
            SettingsOperation::EnableDisable,
        ))
        .unwrap();
        assert_eq!(value["code"], "revision-conflict");
        assert_eq!(value["operation"], "enable-disable");
    }
}
