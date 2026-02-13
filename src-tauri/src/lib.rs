pub mod config;
pub mod executor;
pub mod gesture;
mod hook;
#[path = "log.rs"]
mod log_config;
pub mod overlay;
mod tray;

use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
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
/// use zero_gesture_lib::SharedConfig;
/// use zero_gesture_lib::config::AppConfig;
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
    hook_thread_tid: Arc<AtomicU32>,
    overlay_tx: Sender<overlay::OverlayCommand>,
    hook_handle: Mutex<Option<JoinHandle<()>>>,
    overlay_handle: Mutex<Option<JoinHandle<()>>>,
    is_shutdown: AtomicBool,
}

impl ThreadRuntime {
    /// Spawns the hook and overlay threads and returns a runtime that manages
    /// their lifetimes.
    pub fn start(shared_config: SharedConfig) -> Self {
        let (overlay_tx, overlay_handle) = overlay::spawn(shared_config.clone());
        let (hook_control_tx, hook_thread_tid, hook_handle) =
            hook::spawn(shared_config, overlay_tx.clone());

        Self {
            hook_control_tx,
            hook_thread_tid,
            overlay_tx,
            hook_handle: Mutex::new(Some(hook_handle)),
            overlay_handle: Mutex::new(Some(overlay_handle)),
            is_shutdown: AtomicBool::new(false),
        }
    }

    /// Returns `true` if [`ThreadRuntime::shutdown`] has been called,
    /// meaning the application is ready to exit.
    pub fn should_allow_exit(&self) -> bool {
        self.is_shutdown.load(Ordering::SeqCst)
    }

    /// Sends shutdown signals to both background threads and waits for them
    /// to terminate.
    ///
    /// After this call, [`ThreadRuntime::should_allow_exit`] returns `true`
    /// and subsequent `RunEvent::ExitRequested` events will no longer be
    /// prevented.
    ///
    /// This method is idempotent — calling it more than once is safe and has
    /// no effect after the first call.
    pub fn shutdown(&self) {
        if self.is_shutdown.swap(true, Ordering::SeqCst) {
            return;
        }

        // Post WM_QUIT to the hook thread's Win32 message loop.
        let tid = self.hook_thread_tid.load(Ordering::Acquire);
        if tid != 0 {
            #[cfg(windows)]
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    tid,
                    windows_sys::Win32::UI::WindowsAndMessaging::WM_QUIT,
                    0,
                    0,
                );
            }
        }
        // Also send through the channel as a fallback.
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
    let log_level = log_config::resolve_log_level();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().level(log_level).build())
        .plugin(tauri_plugin_opener::init())
        .manage(shared_config.clone())
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
            let runtime = app.state::<ThreadRuntime>();
            if !runtime.should_allow_exit() {
                api.prevent_exit();
            }
        }
    });
}
