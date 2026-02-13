pub mod config;
mod hook;
pub mod overlay;
mod tray;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use std::thread::JoinHandle;

use crossbeam_channel::Sender;
use tauri::Manager;

/// Thread-safe, clonable handle to the application configuration.
///
/// Wraps [`config::AppConfig`] in `Arc<RwLock<…>>` so that it can be shared
/// across threads (e.g. the hook thread and the UI thread).
///
/// # Examples
///
/// ```
/// use mouse_gesture_lib::SharedConfig;
/// use mouse_gesture_lib::config::AppConfig;
///
/// let shared = SharedConfig::new(AppConfig::default());
/// let cloned = shared.clone();
///
/// // Read the config from another handle.
/// let config = cloned.0.read().unwrap();
/// assert_eq!(config.gesture_trigger_button, "right");
/// ```
#[derive(Clone)]
pub struct SharedConfig(pub Arc<RwLock<config::AppConfig>>);

impl SharedConfig {
    /// Creates a new [`SharedConfig`] from the given [`config::AppConfig`].
    pub fn new(config: config::AppConfig) -> Self {
        Self(Arc::new(RwLock::new(config)))
    }
}

/// Owns the background threads (hook and overlay) and provides a way to shut
/// them down cleanly.
///
/// Created via [`ThreadRuntime::start`] during application setup and stored as
/// Tauri managed state so that the tray "Quit" handler can trigger a graceful
/// shutdown.
pub struct ThreadRuntime {
    hook_control_tx: Sender<hook::HookControl>,
    overlay_tx: Sender<overlay::OverlayCommand>,
    hook_handle: Mutex<Option<JoinHandle<()>>>,
    overlay_handle: Mutex<Option<JoinHandle<()>>>,
    is_shutdown: AtomicBool,
}

impl ThreadRuntime {
    /// Spawns the hook and overlay threads and returns a runtime that manages
    /// their lifetimes.
    fn start(shared_config: SharedConfig) -> Self {
        let (overlay_tx, overlay_handle) = overlay::spawn();
        let (hook_control_tx, hook_handle) = hook::spawn(shared_config, overlay_tx.clone());

        Self {
            hook_control_tx,
            overlay_tx,
            hook_handle: Mutex::new(Some(hook_handle)),
            overlay_handle: Mutex::new(Some(overlay_handle)),
            is_shutdown: AtomicBool::new(false),
        }
    }

    /// Sends shutdown signals to both background threads and waits for them
    /// to terminate.
    ///
    /// This method is idempotent — calling it more than once is safe and has
    /// no effect after the first call.
    pub fn shutdown(&self) {
        if self.is_shutdown.swap(true, Ordering::SeqCst) {
            return;
        }

        let _ = self.hook_control_tx.send(hook::HookControl::Shutdown);
        let _ = self.overlay_tx.send(overlay::OverlayCommand::Shutdown);

        if let Some(handle) = self.hook_handle.lock().ok().and_then(|mut h| h.take()) {
            let _ = handle.join();
        }
        if let Some(handle) = self.overlay_handle.lock().ok().and_then(|mut h| h.take()) {
            let _ = handle.join();
        }
    }
}

/// Controls whether the Tauri application is allowed to exit.
///
/// By default the app prevents exit (to keep running in the system tray).
/// Call [`ExitState::request_exit`] to allow the process to terminate.
pub struct ExitState {
    allow_exit: AtomicBool,
}

impl ExitState {
    /// Creates a new [`ExitState`] that blocks exit by default.
    fn new() -> Self {
        Self {
            allow_exit: AtomicBool::new(false),
        }
    }

    /// Marks the application as ready to exit. Subsequent
    /// `RunEvent::ExitRequested` events will no longer be prevented.
    pub fn request_exit(&self) {
        self.allow_exit.store(true, Ordering::SeqCst);
    }

    /// Returns `true` if [`ExitState::request_exit`] has been called.
    fn should_allow_exit(&self) -> bool {
        self.allow_exit.load(Ordering::SeqCst)
    }
}

/// Tauri command that opens (or focuses) the settings window.
#[tauri::command]
fn show_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    tray::show_settings_window(&app).map_err(|e| e.to_string())
}

/// Application entry point — builds and runs the Tauri application.
///
/// Sets up logging, loads configuration, spawns background threads,
/// creates the system tray, and enters the event loop.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shared_config = SharedConfig::new(config::load_or_default());

    let app = tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .manage(shared_config.clone())
        .manage(ExitState::new())
        .setup(move |app| {
            app.manage(ThreadRuntime::start(shared_config.clone()));
            tray::setup(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![show_settings_window])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            let exit_state = app.state::<ExitState>();
            if !exit_state.should_allow_exit() {
                api.prevent_exit();
            }
        }
    });
}
