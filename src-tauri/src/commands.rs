use crate::capture;
use crate::config;
use crate::{tray, ConfigDir, SharedConfig, ThreadRuntime};
use log::{debug, info};
use std::fs;
use std::sync::Mutex;
use tauri::Emitter;
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
pub fn get_config(
    shared_config: tauri::State<'_, SharedConfig>,
) -> Result<config::ConfigDocument, String> {
    shared_config.document()
}

/// Persists and applies a new configuration.
///
/// Any effective config change restarts or stops worker threads depending on
/// the `enabled` field. This function is called by both the `update_config`
/// Tauri command and the tray toggle handler.
pub fn apply_config_update<R: tauri::Runtime>(
    new_config: config::ConfigDocument,
    app: &tauri::AppHandle<R>,
    shared_config: &SharedConfig,
    runtime: &ThreadRuntime,
    config_dir: &ConfigDir,
) -> Result<(), String> {
    let candidate =
        config::ActiveConfig::from_document(new_config).map_err(|error| error.to_string())?;
    apply_compiled_config_update(candidate, app, shared_config, runtime, config_dir)
}

fn apply_compiled_config_update<R: tauri::Runtime>(
    candidate: config::ActiveConfig,
    app: &tauri::AppHandle<R>,
    shared_config: &SharedConfig,
    runtime: &ThreadRuntime,
    config_dir: &ConfigDir,
) -> Result<(), String> {
    let _update_guard = runtime
        .config_update_lock
        .lock()
        .map_err(|_| "config update lock poisoned".to_string())?;

    if runtime.should_allow_exit() {
        return Err("thread runtime is already shut down".to_string());
    }

    config::save_atomic(&candidate, config_dir.as_path()).map_err(|error| error.to_string())?;

    let enabled = candidate.enabled();
    let document = candidate.document().clone();
    let (restart_required, previous_config) =
        crate::replace_live_config(shared_config, candidate.clone())?;
    if restart_required {
        if let Err(err) = runtime.apply_worker_state(candidate, enabled) {
            if let Some(previous) = &previous_config {
                let _ = config::save_atomic(previous, config_dir.as_path());
            }
            crate::rollback_config_update(
                shared_config,
                runtime,
                previous_config,
                format!("failed to apply worker state: {err}"),
                restart_required,
            );
            return Err(format!("failed to apply worker state: {err}"));
        }
    }

    tray::sync_toggle_menu_label(app, enabled);

    // Notify frontend that config has been updated
    let _ = app.emit("config-updated", document);

    if restart_required {
        info!("config updated and worker state applied");
    } else {
        info!("config update requested but no effective change detected");
    }

    Ok(())
}

/// Tauri command that reads a JSON file and applies it as the new configuration.
#[tauri::command]
pub fn import_config(
    file_path: String,
    app: tauri::AppHandle,
    shared_config: tauri::State<'_, SharedConfig>,
    runtime: tauri::State<'_, ThreadRuntime>,
    config_dir: tauri::State<'_, ConfigDir>,
) -> Result<(), String> {
    let raw = fs::read(&file_path).map_err(|e| format!("failed to read file: {e}"))?;
    let candidate = config::decode_and_compile(&raw).map_err(|error| error.to_string())?;
    apply_compiled_config_update(
        candidate,
        &app,
        shared_config.inner(),
        runtime.inner(),
        config_dir.inner(),
    )
}

/// Tauri command that writes the current configuration as JSON to the given path.
#[tauri::command]
pub fn export_config(
    file_path: String,
    shared_config: tauri::State<'_, SharedConfig>,
) -> Result<(), String> {
    let document = shared_config.document()?;
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

/// Tauri command that persists and applies a new configuration.
#[tauri::command]
pub fn update_config(
    new_config: config::ConfigDocument,
    app: tauri::AppHandle,
    shared_config: tauri::State<'_, SharedConfig>,
    runtime: tauri::State<'_, ThreadRuntime>,
    config_dir: tauri::State<'_, ConfigDir>,
) -> Result<(), String> {
    apply_config_update(
        new_config,
        &app,
        shared_config.inner(),
        runtime.inner(),
        config_dir.inner(),
    )
}
