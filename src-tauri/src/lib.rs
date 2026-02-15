pub mod config;
pub mod executor;
pub mod gesture;
mod hook;
#[path = "log.rs"]
mod log_config;
pub mod overlay;
mod tray;
pub mod window_info;

use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, Mutex, RwLock, RwLockWriteGuard,
};
use std::thread::JoinHandle;

use crossbeam_channel::Sender;
use log::{debug, info, warn};
use tauri::{Emitter, Manager};

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

/// Absolute directory where the app stores configuration files.
#[derive(Clone)]
pub struct ConfigDir(pub PathBuf);

impl ConfigDir {
    fn as_path(&self) -> &Path {
        self.0.as_path()
    }
}

/// Owns the background threads (hook and overlay) and provides a way to shut
/// them down cleanly.
///
/// This is an internal helper owned by [`ThreadRuntime`].
/// [`ThreadRuntime`] itself is stored as Tauri managed state so that, for
/// example, the tray "Quit" handler can trigger a graceful shutdown.
struct WorkerThreads {
    hook_control_tx: Sender<hook::HookControl>,
    hook_thread_tid: Arc<AtomicU32>,
    overlay_tx: Sender<overlay::OverlayCommand>,
    hook_handle: Option<JoinHandle<()>>,
    overlay_handle: Option<JoinHandle<()>>,
}

impl WorkerThreads {
    /// Spawns the hook and overlay threads from the current shared config.
    fn spawn(shared_config: SharedConfig) -> Self {
        info!("starting worker threads");
        let (overlay_tx, overlay_handle) = overlay::spawn(shared_config.clone());
        let (hook_control_tx, hook_thread_tid, hook_handle) =
            hook::spawn(shared_config, overlay_tx.clone());
        info!("worker threads started");

        Self {
            hook_control_tx,
            hook_thread_tid,
            overlay_tx,
            hook_handle: Some(hook_handle),
            overlay_handle: Some(overlay_handle),
        }
    }

    /// Sends shutdown signals to both background threads and waits for them.
    fn shutdown(&mut self) {
        info!("stopping worker threads");
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

        // Also send through channels as a fallback.
        let _ = self.hook_control_tx.send(hook::HookControl::Shutdown);
        let _ = self.overlay_tx.send(overlay::OverlayCommand::Shutdown);

        if let Some(handle) = self.hook_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.overlay_handle.take() {
            let _ = handle.join();
        }
        info!("worker threads stopped");
    }
}

/// Internal state of the [`ThreadRuntime`].
enum RuntimeState {
    /// Worker threads are running.
    Running(WorkerThreads),
    /// Gesture recognition is disabled; workers are not running.
    Disabled,
    /// The runtime has been shut down for application exit.
    ShutDown,
}

pub struct ThreadRuntime {
    state: Mutex<RuntimeState>,
    config_update_lock: Mutex<()>,
}

impl ThreadRuntime {
    /// Spawns the hook and overlay threads and returns a runtime that manages
    /// their lifetimes.
    ///
    /// If `config.enabled` is `false`, the runtime starts in the Disabled
    /// state and no worker threads are created.
    pub fn start(shared_config: SharedConfig) -> Self {
        let enabled = shared_config.0.read().map(|c| c.enabled).unwrap_or(true);
        let initial_state = if enabled {
            info!("thread runtime starting in enabled mode");
            RuntimeState::Running(WorkerThreads::spawn(shared_config))
        } else {
            info!("thread runtime starting in disabled mode");
            RuntimeState::Disabled
        };
        Self {
            state: Mutex::new(initial_state),
            config_update_lock: Mutex::new(()),
        }
    }

    /// Returns `true` if [`ThreadRuntime::shutdown`] has been called,
    /// meaning the application is ready to exit.
    pub fn should_allow_exit(&self) -> bool {
        match self.state.lock() {
            Ok(state) => matches!(*state, RuntimeState::ShutDown),
            Err(_) => {
                warn!("thread runtime lock poisoned while checking exit state");
                true
            }
        }
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
        info!("thread runtime shutdown requested");
        let _update_guard = match self.config_update_lock.lock() {
            Ok(guard) => guard,
            Err(_) => {
                warn!("config update lock poisoned during shutdown");
                return;
            }
        };

        match self.state.lock() {
            Ok(mut state) => {
                let prev = std::mem::replace(&mut *state, RuntimeState::ShutDown);
                match prev {
                    RuntimeState::Running(mut workers) => workers.shutdown(),
                    RuntimeState::Disabled => info!("workers already stopped"),
                    RuntimeState::ShutDown => {
                        debug!("thread runtime already shut down");
                    }
                }
                info!("thread runtime shut down");
            }
            Err(_) => {
                warn!("thread runtime lock poisoned during shutdown");
            }
        }
    }

    /// Ensures the worker state matches the current `enabled` setting.
    ///
    /// * `enabled=true` → (re)starts workers regardless of current state.
    /// * `enabled=false` → stops workers if running and transitions to
    ///   `Disabled`.
    ///
    /// Returns an error if the runtime has already been shut down.
    fn apply_worker_state(&self, shared_config: SharedConfig, enabled: bool) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "thread runtime lock poisoned".to_string())?;

        if matches!(*state, RuntimeState::ShutDown) {
            return Err("thread runtime is already shut down".to_string());
        }

        if enabled {
            let was_running = matches!(*state, RuntimeState::Running(_));
            if was_running {
                info!("enabled=true: restarting worker threads");
            } else {
                info!("enabled=true: starting worker threads");
            }
            // Tear down existing workers first so replacement overlay startup
            // cannot race with the old overlay's Win32 class registration.
            let prev = std::mem::replace(&mut *state, RuntimeState::Disabled);
            if let RuntimeState::Running(mut workers) = prev {
                workers.shutdown();
            }
            *state = RuntimeState::Running(WorkerThreads::spawn(shared_config));
        } else {
            match &mut *state {
                RuntimeState::Running(workers) => {
                    info!("enabled=false: stopping worker threads");
                    workers.shutdown();
                    *state = RuntimeState::Disabled;
                }
                _ => {
                    info!("enabled=false: workers already stopped");
                }
            }
        }

        Ok(())
    }
}

/// Replaces the in-memory config and returns whether workers should restart.
fn replace_live_config(
    shared_config: &SharedConfig,
    next: config::AppConfig,
) -> Result<(bool, config::AppConfig), String> {
    let mut current: RwLockWriteGuard<'_, config::AppConfig> = shared_config
        .0
        .write()
        .map_err(|_| "shared config lock poisoned".to_string())?;

    let previous = current.clone();
    let restart_required = previous != next;
    *current = next;
    Ok((restart_required, previous))
}

/// Restores a previous config after an update failure.
fn rollback_config_update(
    shared_config: &SharedConfig,
    runtime: &ThreadRuntime,
    previous_config: config::AppConfig,
    restart_required: bool,
) {
    let enabled = previous_config.enabled;
    if let Err(err) = replace_live_config(shared_config, previous_config) {
        warn!("failed to roll back in-memory config after update failure: {err}");
        return;
    }

    if restart_required && !runtime.should_allow_exit() {
        if let Err(err) = runtime.apply_worker_state(shared_config.clone(), enabled) {
            warn!("failed to roll back worker threads after update failure: {err}");
        }
    }
}

/// Tauri command that opens (or focuses) the settings window.
#[tauri::command]
fn show_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    tray::show_settings_window(&app).map_err(|e| e.to_string())
}

/// Persists and applies a new configuration.
///
/// Any effective config change restarts or stops worker threads depending on
/// the `enabled` field. This function is called by both the `update_config`
/// Tauri command and the tray toggle handler.
pub fn apply_config_update<R: tauri::Runtime>(
    new_config: config::AppConfig,
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

    let (restart_required, previous_config) =
        replace_live_config(shared_config, new_config.clone())?;
    if restart_required {
        if let Err(err) = runtime.apply_worker_state(shared_config.clone(), new_config.enabled) {
            rollback_config_update(shared_config, runtime, previous_config, restart_required);
            return Err(format!("failed to apply worker state: {err}"));
        }
    }

    if let Err(err) = config::save(&new_config, config_dir.as_path()) {
        rollback_config_update(shared_config, runtime, previous_config, restart_required);
        return Err(format!("failed to save config: {err}"));
    }

    // Notify frontend that config has been updated
    let _ = app.emit("config-updated", new_config.clone());

    if restart_required {
        info!("config updated and worker state applied");
    } else {
        info!("config update requested but no effective change detected");
    }

    Ok(())
}

/// Tauri command that retrieves the current configuration.
#[tauri::command]
fn get_config(shared_config: tauri::State<'_, SharedConfig>) -> Result<config::AppConfig, String> {
    shared_config
        .0
        .read()
        .map(|c| c.clone())
        .map_err(|_| "failed to read shared config".to_string())
}

/// Tauri command that persists and applies a new configuration.
#[tauri::command]
fn update_config(
    new_config: config::AppConfig,
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

/// Application entry point — builds and runs the Tauri application.
///
/// Sets up logging, loads configuration, spawns background threads,
/// creates the system tray, and enters the event loop.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let log_level = log_config::resolve_log_level();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().level(log_level).build())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let config_dir_path = app.path().app_config_dir()?;
            let shared_config =
                SharedConfig::new(config::load_or_default(config_dir_path.as_path()));

            app.manage(shared_config.clone());
            app.manage(ConfigDir(config_dir_path));
            app.manage(ThreadRuntime::start(shared_config));
            tray::setup(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            show_settings_window,
            get_config,
            update_config
        ])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_live_config_updates_shared_state() {
        let shared = SharedConfig::new(config::AppConfig::default());

        let next = config::AppConfig {
            gesture_trigger_button: "middle".to_string(),
            ..config::AppConfig::default()
        };

        let (restart_required, previous_config) =
            replace_live_config(&shared, next.clone()).unwrap();
        assert!(restart_required);
        assert_eq!(previous_config, config::AppConfig::default());

        let current = shared.0.read().unwrap().clone();
        assert_eq!(current, next);
    }
}
