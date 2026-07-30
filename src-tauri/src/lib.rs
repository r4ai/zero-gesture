pub mod capture;
pub mod commands;
pub mod config;
mod domain;
pub mod executor;
mod hook;
mod ipc;
#[path = "log.rs"]
mod log_config;
pub mod overlay;
mod process;
mod tray;
pub mod window_info;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex, RwLock,
};
use std::thread::{self, JoinHandle};

use crossbeam_channel::Sender;
use log::{debug, error, info, warn};
use tauri::Manager;

enum ConfigState {
    Active(Box<config::ActiveConfig>),
    Unavailable(String),
}

/// Thread-safe handle to the canonical document and its compiled snapshot.
#[derive(Clone)]
pub struct SharedConfig(Arc<RwLock<ConfigState>>);

impl SharedConfig {
    pub fn new(config: config::ActiveConfig) -> Self {
        Self(Arc::new(RwLock::new(ConfigState::Active(Box::new(config)))))
    }

    pub fn unavailable(error: impl Into<String>) -> Self {
        Self(Arc::new(RwLock::new(ConfigState::Unavailable(
            error.into(),
        ))))
    }

    pub fn active(&self) -> Result<config::ActiveConfig, String> {
        match &*self
            .0
            .read()
            .map_err(|_| "shared config lock poisoned".to_string())?
        {
            ConfigState::Active(active) => Ok(active.as_ref().clone()),
            ConfigState::Unavailable(error) => Err(error.clone()),
        }
    }

    pub fn document(&self) -> Result<config::ConfigDocument, String> {
        self.active().map(|active| active.document().clone())
    }

    fn replace(
        &self,
        next: config::ActiveConfig,
    ) -> Result<(bool, Option<config::ActiveConfig>), String> {
        let mut state = self
            .0
            .write()
            .map_err(|_| "shared config lock poisoned".to_string())?;
        let previous = match &*state {
            ConfigState::Active(active) => Some(active.as_ref().clone()),
            ConfigState::Unavailable(_) => None,
        };
        let restart_required = previous
            .as_ref()
            .is_none_or(|active| active.document() != next.document());
        *state = ConfigState::Active(Box::new(next));
        Ok((restart_required, previous))
    }

    fn restore(&self, previous: Option<config::ActiveConfig>, error: String) -> Result<(), String> {
        let mut state = self
            .0
            .write()
            .map_err(|_| "shared config lock poisoned".to_string())?;
        *state = previous.map_or(ConfigState::Unavailable(error), |active| {
            ConfigState::Active(Box::new(active))
        });
        Ok(())
    }

    fn mark_unavailable(&self, error: String) -> Result<(), String> {
        let mut state = self
            .0
            .write()
            .map_err(|_| "shared config lock poisoned".to_string())?;
        *state = ConfigState::Unavailable(error);
        Ok(())
    }
}

/// Absolute directory where the app stores configuration files.
#[derive(Clone)]
pub struct ConfigDir(pub PathBuf);

impl ConfigDir {
    pub(crate) fn as_path(&self) -> &Path {
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
    fn spawn(active: config::ActiveConfig) -> Self {
        info!("starting worker threads");
        let runtime = active.runtime();
        let (overlay_tx, overlay_handle) = overlay::spawn(runtime.clone());
        let (hook_control_tx, hook_thread_tid, hook_handle) =
            hook::spawn(runtime, overlay_tx.clone());
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
    owns_workers: bool,
}

impl ThreadRuntime {
    /// Spawns the hook and overlay threads and returns a runtime that manages
    /// their lifetimes.
    ///
    /// If `config.enabled` is `false`, the runtime starts in the Disabled
    /// state and no worker threads are created.
    pub fn start(shared_config: SharedConfig) -> Self {
        let active = shared_config.active().ok();
        let initial_state = if active.as_ref().is_some_and(config::ActiveConfig::enabled) {
            info!("thread runtime starting in enabled mode");
            RuntimeState::Running(WorkerThreads::spawn(
                active.expect("enabled active config must exist"),
            ))
        } else {
            info!("thread runtime starting in disabled mode");
            RuntimeState::Disabled
        };
        Self {
            state: Mutex::new(initial_state),
            config_update_lock: Mutex::new(()),
            owns_workers: true,
        }
    }

    /// Creates the temporary P03a Settings-side config runtime.
    ///
    /// Settings keeps the existing in-process config commands until P03b but
    /// never starts Engine input or rendering workers.
    pub fn settings() -> Self {
        Self {
            state: Mutex::new(RuntimeState::Disabled),
            config_update_lock: Mutex::new(()),
            owns_workers: false,
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
    fn apply_worker_state(
        &self,
        active: config::ActiveConfig,
        enabled: bool,
    ) -> Result<(), String> {
        if !self.owns_workers {
            return Ok(());
        }
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
            *state = RuntimeState::Running(WorkerThreads::spawn(active));
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

    fn disable_for_config_error(&self) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "thread runtime lock poisoned".to_string())?;
        let previous = std::mem::replace(&mut *state, RuntimeState::Disabled);
        match previous {
            RuntimeState::Running(mut workers) => workers.shutdown(),
            RuntimeState::Disabled => {}
            RuntimeState::ShutDown => *state = RuntimeState::ShutDown,
        }
        Ok(())
    }

    #[cfg(test)]
    fn is_disabled(&self) -> bool {
        matches!(*self.state.lock().unwrap(), RuntimeState::Disabled)
    }
}

struct EngineIpcRuntime {
    stop: Arc<AtomicBool>,
    handle: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for EngineIpcRuntime {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Ok(handle) = self.handle.get_mut() {
            if let Some(handle) = handle.take() {
                let _ = handle.join();
            }
        }
    }
}

enum SettingsEngineState {
    Connected(ipc::EngineControl),
    Unavailable(String),
}

#[derive(serde::Serialize)]
#[serde(rename_all = "kebab-case")]
enum SettingsEngineErrorKind {
    Unavailable,
}

#[derive(serde::Serialize)]
struct SettingsEngineError {
    kind: SettingsEngineErrorKind,
    message: String,
}

#[tauri::command]
fn get_engine_status(
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<ipc::EngineStatus, SettingsEngineError> {
    match engine.inner() {
        SettingsEngineState::Connected(control) => {
            control.status().map_err(|error| SettingsEngineError {
                kind: SettingsEngineErrorKind::Unavailable,
                message: error.to_string(),
            })
        }
        SettingsEngineState::Unavailable(message) => Err(SettingsEngineError {
            kind: SettingsEngineErrorKind::Unavailable,
            message: message.clone(),
        }),
    }
}

#[tauri::command]
fn shutdown_engine(
    engine: tauri::State<'_, SettingsEngineState>,
) -> Result<bool, SettingsEngineError> {
    match engine.inner() {
        SettingsEngineState::Connected(control) => {
            control.shutdown().map_err(|error| SettingsEngineError {
                kind: SettingsEngineErrorKind::Unavailable,
                message: error.to_string(),
            })
        }
        SettingsEngineState::Unavailable(message) => Err(SettingsEngineError {
            kind: SettingsEngineErrorKind::Unavailable,
            message: message.clone(),
        }),
    }
}

/// Replaces the in-memory config and returns whether workers should restart.
pub fn replace_live_config(
    shared_config: &SharedConfig,
    next: config::ActiveConfig,
) -> Result<(bool, Option<config::ActiveConfig>), String> {
    shared_config.replace(next)
}

/// Restores a previous config after an update failure.
pub fn rollback_config_update(
    shared_config: &SharedConfig,
    runtime: &ThreadRuntime,
    previous_config: Option<config::ActiveConfig>,
    error: String,
    restart_required: bool,
) -> Result<(), String> {
    let enabled = previous_config
        .as_ref()
        .is_some_and(config::ActiveConfig::enabled);
    shared_config.restore(previous_config.clone(), error)?;

    if restart_required && enabled && !runtime.should_allow_exit() {
        if let Some(previous) = previous_config {
            runtime.apply_worker_state(previous, true)?;
        }
    }
    Ok(())
}

/// Selects Engine or Settings mode from the executable arguments.
pub fn run_from_args(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    match process::select_mode(arguments) {
        Ok(process::ProcessMode::Engine) => run_engine(),
        Ok(process::ProcessMode::Settings) => run_settings(),
        Err(error) => Err(format!("invalid process mode: {error:?}")),
    }
}

/// Compatibility entry point used by mobile tooling.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Err(error) = run_from_args(std::env::args_os()) {
        eprintln!("{error}");
    }
}

fn run_engine() -> Result<(), String> {
    let log_level = log_config::resolve_log_level();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().level(log_level).build())
        .setup(move |app| {
            let config_dir_path = app.path().app_config_dir()?;
            let Some(server) = ipc::EngineServer::new(&config_dir_path).map_err(|error| {
                tauri::Error::Setup((Box::new(error) as Box<dyn std::error::Error>).into())
            })?
            else {
                info!("Engine already running for this user");
                app.handle().exit(0);
                return Ok(());
            };
            let shared_config = match config::load(config_dir_path.as_path()) {
                Ok(config::LoadResult::Ready(active) | config::LoadResult::Missing(active)) => {
                    SharedConfig::new(active)
                }
                Err(load_error) => {
                    error!("configuration unavailable; gestures disabled: {load_error}");
                    SharedConfig::unavailable(load_error.to_string())
                }
            };

            app.manage(shared_config.clone());
            app.manage(ConfigDir(config_dir_path));
            app.manage(ThreadRuntime::start(shared_config));
            app.manage(commands::CaptureState(std::sync::Mutex::new(None)));
            tray::setup(app)?;

            let stop = Arc::new(AtomicBool::new(false));
            let server_stop = Arc::clone(&stop);
            let app_handle = app.handle().clone();
            let handle = thread::Builder::new()
                .name("engine-ipc".to_string())
                .spawn(move || match server.run(server_stop) {
                    Ok(ipc::ServerExit::Shutdown) => {
                        info!("Engine shutdown requested over IPC");
                        if let Some(runtime) = app_handle.try_state::<ThreadRuntime>() {
                            runtime.shutdown();
                        }
                        app_handle.exit(0);
                    }
                    Ok(ipc::ServerExit::Stopped) => {}
                    Err(error) => {
                        error!("Engine IPC owner stopped: {error}");
                        if let Some(runtime) = app_handle.try_state::<ThreadRuntime>() {
                            runtime.shutdown();
                        }
                        app_handle.exit(1);
                    }
                })
                .map_err(|error| {
                    tauri::Error::Setup((Box::new(error) as Box<dyn std::error::Error>).into())
                })?;
            app.manage(EngineIpcRuntime {
                stop,
                handle: Mutex::new(Some(handle)),
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .map_err(|error| format!("failed to build Engine: {error}"))?;

    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if let Some(runtime) = app.try_state::<ThreadRuntime>() {
                if !runtime.should_allow_exit() {
                    api.prevent_exit();
                }
            }
        }
    });
    Ok(())
}

fn run_settings() -> Result<(), String> {
    let log_level = log_config::resolve_log_level();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_log::Builder::new().level(log_level).build())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let config_dir_path = app.path().app_config_dir()?;
            let engine_result = std::env::current_exe()
                .map_err(|error| error.to_string())
                .and_then(|executable| {
                    ipc::EngineControl::connect_or_start(&executable, &config_dir_path)
                        .map_err(|error| error.to_string())
                });
            let engine = match engine_result {
                Ok(control) => SettingsEngineState::Connected(control),
                Err(message) => {
                    warn!("Engine unavailable to Settings: {message}");
                    SettingsEngineState::Unavailable(message)
                }
            };
            let shared_config = match config::load(config_dir_path.as_path()) {
                Ok(config::LoadResult::Ready(active) | config::LoadResult::Missing(active)) => {
                    SharedConfig::new(active)
                }
                Err(load_error) => {
                    error!("configuration unavailable in Settings: {load_error}");
                    SharedConfig::unavailable(load_error.to_string())
                }
            };
            app.manage(engine);
            app.manage(shared_config);
            app.manage(ConfigDir(config_dir_path));
            app.manage(ThreadRuntime::settings());
            app.manage(commands::CaptureState(std::sync::Mutex::new(None)));
            tray::show_settings_window(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_engine_status,
            shutdown_engine,
            commands::show_settings_window,
            commands::get_config,
            commands::update_config,
            commands::import_config,
            commands::export_config,
            commands::open_config_dir,
            commands::get_foreground_window_info,
            commands::start_window_capture,
            commands::stop_window_capture
        ])
        .build(tauri::generate_context!())
        .map_err(|error| format!("failed to build Settings: {error}"))?;

    app.run(|app, event| {
        if let tauri::RunEvent::ExitRequested { .. } = event {
            if let Some(runtime) = app.try_state::<ThreadRuntime>() {
                runtime.shutdown();
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_live_config_updates_shared_state() {
        let current =
            config::ActiveConfig::from_document(config::ConfigDocument::default()).unwrap();
        let shared = SharedConfig::new(current.clone());
        let mut document = config::ConfigDocument::default();
        document.shared.appearance.trail_thickness = 6.0;
        let next = config::ActiveConfig::from_document(document.clone()).unwrap();

        let (restart_required, previous_config) =
            replace_live_config(&shared, next.clone()).unwrap();
        assert!(restart_required);
        assert_eq!(previous_config.unwrap().document(), current.document());
        assert_eq!(shared.document().unwrap(), document);
    }
}
