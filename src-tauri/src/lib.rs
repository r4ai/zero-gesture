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
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};

use crossbeam_channel::Sender;
use log::{debug, error, info, warn};
use tauri::Manager;

/// Absolute directory where the app stores configuration files.
#[derive(Clone)]
pub struct ConfigDir(pub PathBuf);

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
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("ZG_P03_TEST_WORKER_START_MARKER") {
            std::fs::write(path, b"worker-started")
                .expect("write P03 worker-start fault-injection marker");
        }
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
}

impl ThreadRuntime {
    /// Spawns the hook and overlay threads and returns a runtime that manages
    /// their lifetimes.
    ///
    /// If `config.enabled` is `false`, the runtime starts in the Disabled
    /// state and no worker threads are created.
    pub fn start(active: Option<config::ActiveConfig>) -> Self {
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
        }
    }

    /// Creates the temporary P03a Settings-side config runtime.
    ///
    /// Settings keeps the existing in-process config commands until P03b but
    /// never starts Engine input or rendering workers.
    pub fn settings() -> Self {
        Self {
            state: Mutex::new(RuntimeState::Disabled),
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

    fn apply_config(&self, active: config::ActiveConfig) {
        let Ok(mut state) = self.state.lock() else {
            warn!("thread runtime lock poisoned while applying config");
            return;
        };
        if matches!(*state, RuntimeState::ShutDown) {
            return;
        }

        let previous = std::mem::replace(&mut *state, RuntimeState::Disabled);
        if let RuntimeState::Running(mut workers) = previous {
            workers.shutdown();
        }
        if active.enabled() {
            *state = RuntimeState::Running(WorkerThreads::spawn(active));
        }
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

impl SettingsEngineState {
    fn control(&self) -> Result<&ipc::EngineControl, String> {
        match self {
            Self::Connected(control) => Ok(control),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }
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
    #[cfg(debug_assertions)]
    let log_builder =
        if let Some(config_dir) = std::env::var_os("ZG_P03_TEST_CONFIG_DIR") {
            tauri_plugin_log::Builder::new().level(log_level).targets([
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder {
                    path: PathBuf::from(config_dir).join("logs"),
                    file_name: Some("engine-test".to_string()),
                }),
            ])
        } else {
            tauri_plugin_log::Builder::new().level(log_level)
        };
    #[cfg(not(debug_assertions))]
    let log_builder = tauri_plugin_log::Builder::new().level(log_level);

    let app = tauri::Builder::default()
        .plugin(log_builder.build())
        .setup(move |app| {
            let config_dir_path = engine_config_dir(app.path().app_config_dir()?);
            let Some(server) = prepare_engine_server(&config_dir_path).map_err(|error| {
                tauri::Error::Setup((Box::new(error) as Box<dyn std::error::Error>).into())
            })?
            else {
                info!("Engine already running for this user");
                app.handle().exit(0);
                return Ok(());
            };
            let engine_control = ipc::EngineControl::for_prepared_server(&server);
            let (config_owner, initial_config) = config::ConfigOwner::startup(&config_dir_path);
            let enabled = initial_config
                .as_ref()
                .is_some_and(config::ActiveConfig::enabled);
            app.manage(engine_control);
            app.manage(ConfigDir(config_dir_path));
            app.manage(ThreadRuntime::start(initial_config));
            app.manage(commands::CaptureState(std::sync::Mutex::new(None)));
            tray::setup(app, enabled)?;

            let stop = Arc::new(AtomicBool::new(false));
            let server_stop = Arc::clone(&stop);
            let app_handle = app.handle().clone();
            let handle = thread::Builder::new()
                .name("engine-ipc".to_string())
                .spawn(move || {
                    let result = server.run(server_stop, config_owner, |active, _generation| {
                        if let Some(runtime) = app_handle.try_state::<ThreadRuntime>() {
                            runtime.apply_config(active.clone());
                        }
                        tray::sync_toggle_menu_label(&app_handle, active.enabled());
                    });
                    match result {
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
                    }
                })
                .map_err(|error| {
                    tauri::Error::Setup((Box::new(error) as Box<dyn std::error::Error>).into())
                })?;
            app.manage(EngineIpcRuntime {
                stop,
                handle: Mutex::new(Some(handle)),
            });
            #[cfg(debug_assertions)]
            if let Some(path) = std::env::var_os("ZG_P03_TEST_READY_MARKER") {
                std::fs::write(path, b"engine-ready")?;
            }
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

fn engine_config_dir(default: PathBuf) -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("ZG_P03_TEST_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    default
}

fn prepare_engine_server(
    config_dir: &Path,
) -> Result<Option<ipc::EngineServer>, ipc::ControlError> {
    #[cfg(all(debug_assertions, windows))]
    if let Some(namespace) = std::env::var_os("ZG_P03_TEST_NAMESPACE") {
        let namespace = namespace.to_str().ok_or_else(|| {
            ipc::ControlError::Security("P03 test namespace must be valid Unicode".to_string())
        })?;
        return ipc::EngineServer::for_debug_namespace(config_dir, namespace);
    }
    ipc::EngineServer::new(config_dir)
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
            app.manage(engine);
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
