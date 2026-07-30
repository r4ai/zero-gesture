use crate::capture;
use crate::config;
use crate::{tray, ConfigDir, SettingsEngineState};
use log::{debug, info};
use std::fs;
use std::sync::Mutex;
use tauri_plugin_opener::OpenerExt;

/// Tauri managed state holding an active [`capture::win32::CaptureHandle`].
///
/// `None` means no capture is in progress.
pub struct CaptureState(pub Mutex<Option<capture::win32::CaptureHandle>>);

/// Tauri command that opens (or focuses) the settings window.
#[tauri::command]
pub fn show_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    tray::show_settings_window(&app).map_err(|e| e.to_string())
}

/// Tauri command that retrieves the current configuration.
#[tauri::command]
pub(crate) fn get_config(
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<config::ConfigDocument, String> {
    engine
        .control()?
        .current_config()
        .map_err(|error| error.to_string())
}

/// Tauri command that routes the mutation to the Engine config owner.
#[tauri::command]
pub(crate) fn update_config(
    new_config: config::ConfigDocument,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<(), String> {
    engine
        .control()?
        .apply_config(new_config)
        .map_err(|error| error.to_string())
}

/// Tauri command that reads a JSON file and applies it as the new configuration.
#[tauri::command]
pub(crate) fn import_config(
    file_path: String,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<(), String> {
    let raw = fs::read(&file_path).map_err(|e| format!("failed to read file: {e}"))?;
    engine
        .control()?
        .apply_config_bytes(raw)
        .map_err(|error| error.to_string())
}

/// Tauri command that writes the current configuration as JSON to the given path.
#[tauri::command]
pub(crate) fn export_config(
    file_path: String,
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<(), String> {
    let document = engine
        .control()?
        .current_config()
        .map_err(|error| error.to_string())?;
    config::export(&document, std::path::Path::new(&file_path)).map_err(|error| error.to_string())
}

/// Tauri command that opens the config directory in the system file manager.
///
/// Creates the directory if it does not yet exist, then opens it via the
/// opener plugin from the Rust side (bypassing JS-side path scope restrictions).
#[tauri::command]
pub fn open_config_dir(
    app: tauri::AppHandle,
    config_dir: tauri::State<'_, ConfigDir>,
) -> Result<(), String> {
    let path = &config_dir.0;
    fs::create_dir_all(path).map_err(|e| format!("failed to create config dir: {e}"))?;
    app.opener()
        .open_path(path.to_string_lossy().as_ref(), None::<&str>)
        .map_err(|e| format!("failed to open config dir: {e}"))
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

/// Tauri command that starts a one-shot window capture.
///
/// Installs a global `WH_MOUSE_LL` hook on a background thread.  When the user
/// clicks anywhere on screen the hook resolves the window under the cursor and
/// emits a `window-captured` event carrying [`crate::window_info::ForegroundWindowInfo`].
///
/// If a capture is already in progress it is silently replaced by a new one.
///
/// # Example
///
/// ```ignore
/// start_window_capture(app, capture_state);
/// // … frontend listens for "window-captured" event
/// ```
#[tauri::command]
pub fn start_window_capture(
    app: tauri::AppHandle,
    capture_state: tauri::State<'_, CaptureState>,
) -> Result<(), String> {
    let handle = capture::win32::start(app);
    let mut guard = capture_state
        .0
        .lock()
        .map_err(|_| "capture state mutex poisoned".to_string())?;
    // Cancel any existing capture first.
    if let Some(existing) = guard.take() {
        existing.cancel();
    }
    *guard = Some(handle);
    info!("start_window_capture: capture started");
    Ok(())
}

/// Tauri command that cancels an in-progress window capture.
///
/// No-op if no capture is currently active.
///
/// # Example
///
/// ```ignore
/// stop_window_capture(capture_state);
/// ```
#[tauri::command]
pub fn stop_window_capture(capture_state: tauri::State<'_, CaptureState>) -> Result<(), String> {
    let mut guard = capture_state
        .0
        .lock()
        .map_err(|_| "capture state mutex poisoned".to_string())?;
    if let Some(handle) = guard.take() {
        handle.cancel();
        info!("stop_window_capture: capture cancelled");
    } else {
        debug!("stop_window_capture: no active capture");
    }
    Ok(())
}
