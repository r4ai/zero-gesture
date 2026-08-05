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
mod runtime_shell;
mod tray;
pub mod window_info;

use std::ffi::OsString;
use std::fmt;
use std::io;
#[cfg(any(windows, target_os = "macos"))]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};

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
    hook_thread_tid: Arc<AtomicU32>,
    hook_stop: Arc<AtomicBool>,
    hook_handle: Option<JoinHandle<()>>,
    hook_events: Option<crossbeam_channel::Receiver<hook::HookEvent>>,
}

#[derive(Debug)]
enum RuntimeProjectionError {
    StatePoisoned,
    RuntimeShutDown,
    WorkerSpawn {
        worker: &'static str,
        source: io::Error,
    },
    WorkerPanicked(&'static str),
    TestMarker(io::Error),
}

impl fmt::Display for RuntimeProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StatePoisoned => formatter.write_str("thread runtime state is poisoned"),
            Self::RuntimeShutDown => formatter.write_str("thread runtime is shut down"),
            Self::WorkerSpawn { worker, source } => {
                write!(formatter, "failed to spawn {worker} worker: {source}")
            }
            Self::WorkerPanicked(worker) => write!(formatter, "{worker} worker panicked"),
            Self::TestMarker(source) => {
                write!(formatter, "failed to write worker start marker: {source}")
            }
        }
    }
}

impl std::error::Error for RuntimeProjectionError {}

impl WorkerThreads {
    /// Spawns the single Windows input owner. Renderer generations are owned
    /// inside it and are created only outside the native callback.
    fn spawn(reader: config::ConfigSnapshotReader) -> Result<Self, RuntimeProjectionError> {
        #[cfg(debug_assertions)]
        if let Some(path) = std::env::var_os("ZG_P03_TEST_WORKER_START_MARKER") {
            std::fs::write(path, b"worker-started").map_err(RuntimeProjectionError::TestMarker)?;
        }
        info!("starting native input owner");
        let (hook_thread_tid, hook_stop, hook_handle, hook_events) =
            hook::spawn(reader).map_err(|source| RuntimeProjectionError::WorkerSpawn {
                worker: "hook",
                source,
            })?;
        info!("native input owner started");

        Ok(Self {
            hook_thread_tid,
            hook_stop,
            hook_handle: Some(hook_handle),
            hook_events: Some(hook_events),
        })
    }

    /// Sends shutdown signals to both background threads and waits for them.
    fn shutdown(&mut self) -> Result<(), RuntimeProjectionError> {
        info!("stopping worker threads");
        self.hook_stop.store(true, Ordering::Release);
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

        let hook_panicked = self
            .hook_handle
            .take()
            .is_some_and(|handle| handle.join().is_err());
        info!("worker threads stopped");
        if hook_panicked {
            Err(RuntimeProjectionError::WorkerPanicked("hook"))
        } else {
            Ok(())
        }
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

pub(crate) struct ThreadRuntime {
    state: Mutex<RuntimeState>,
}

impl ThreadRuntime {
    /// Starts the native owner after IPC readiness and configuration
    /// publication have been established. Disabled or unavailable snapshots
    /// remain fail-open inside the owner without restarting it.
    fn start(reader: config::ConfigSnapshotReader) -> Result<Self, RuntimeProjectionError> {
        let initial_state = RuntimeState::Running(WorkerThreads::spawn(reader)?);
        Ok(Self {
            state: Mutex::new(initial_state),
        })
    }

    /// Creates the temporary P03a Settings-side config runtime.
    ///
    /// Settings keeps the existing in-process config commands until P03b but
    /// never starts Engine input or rendering workers.
    fn settings() -> Self {
        Self {
            state: Mutex::new(RuntimeState::Disabled),
        }
    }

    fn monitor_owner(&self, app_handle: tauri::AppHandle) -> Result<(), RuntimeProjectionError> {
        let events = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| RuntimeProjectionError::StatePoisoned)?;
            let RuntimeState::Running(workers) = &mut *state else {
                return Err(RuntimeProjectionError::RuntimeShutDown);
            };
            workers
                .hook_events
                .take()
                .ok_or(RuntimeProjectionError::RuntimeShutDown)?
        };
        thread::Builder::new()
            .name("input-owner-monitor".to_string())
            .spawn(move || {
                if let Ok(hook::HookEvent::Fatal(failure)) = events.recv() {
                    error!("native input owner stopped: {failure}");
                    if let Some(runtime) = app_handle.try_state::<ThreadRuntime>() {
                        if let Err(error) = runtime.shutdown() {
                            error!("failed to stop Engine after native owner failure: {error}");
                        }
                    }
                    std::process::exit(1);
                }
            })
            .map(|_| ())
            .map_err(|source| RuntimeProjectionError::WorkerSpawn {
                worker: "input-owner-monitor",
                source,
            })
    }

    /// Returns `true` if [`ThreadRuntime::shutdown`] has been called,
    /// meaning the application is ready to exit.
    fn should_allow_exit(&self) -> bool {
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
    pub(crate) fn shutdown(&self) -> Result<(), RuntimeProjectionError> {
        info!("thread runtime shutdown requested");
        let mut state = self
            .state
            .lock()
            .map_err(|_| RuntimeProjectionError::StatePoisoned)?;
        let previous = std::mem::replace(&mut *state, RuntimeState::ShutDown);
        let result = match previous {
            RuntimeState::Running(mut workers) => workers.shutdown(),
            RuntimeState::Disabled => {
                info!("workers already stopped");
                Ok(())
            }
            RuntimeState::ShutDown => {
                debug!("thread runtime already shut down");
                Ok(())
            }
        };
        info!("thread runtime shut down");
        result
    }

    fn observe_applied(&self, active: &config::ActiveConfig) -> Result<(), RuntimeProjectionError> {
        let state = self
            .state
            .lock()
            .map_err(|_| RuntimeProjectionError::StatePoisoned)?;
        match &*state {
            RuntimeState::Running(workers) => {
                #[cfg(debug_assertions)]
                if std::env::var_os("ZG_P03_TEST_FAIL_WORKER_SPAWN").is_some() && active.enabled() {
                    return Err(RuntimeProjectionError::WorkerSpawn {
                        worker: "replacement",
                        source: io::Error::other("injected owner notification failure"),
                    });
                }
                if workers
                    .hook_handle
                    .as_ref()
                    .is_none_or(JoinHandle::is_finished)
                {
                    return Err(RuntimeProjectionError::WorkerPanicked("hook"));
                }
                Ok(())
            }
            RuntimeState::Disabled => Ok(()),
            RuntimeState::ShutDown => Err(RuntimeProjectionError::RuntimeShutDown),
        }
    }

    #[cfg(test)]
    fn poison_for_test(&self) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = self.state.lock().unwrap();
            panic!("poison runtime state for projection test");
        }));
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

fn tauri_context() -> tauri::Context {
    tauri::generate_context!()
}

#[cfg(windows)]
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
    let log_builder = tauri_plugin_log::Builder::new().level(log_level).targets([
        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
            file_name: Some("engine".to_string()),
        }),
    ]);

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
            let snapshot_reader = config_owner.reader();
            app.manage(engine_control);
            app.manage(ConfigDir(config_dir_path));
            let thread_runtime = ThreadRuntime::start(snapshot_reader).map_err(|error| {
                tauri::Error::Setup((Box::new(error) as Box<dyn std::error::Error>).into())
            })?;
            app.manage(thread_runtime);
            app.state::<ThreadRuntime>()
                .monitor_owner(app.handle().clone())
                .map_err(|error| {
                    tauri::Error::Setup((Box::new(error) as Box<dyn std::error::Error>).into())
                })?;
            app.manage(commands::CaptureState(std::sync::Mutex::new(None)));
            tray::setup(app, enabled)?;

            start_engine_ipc(app, server, config_owner)?;
            #[cfg(debug_assertions)]
            if let Some(path) = std::env::var_os("ZG_P03_TEST_READY_MARKER") {
                std::fs::write(path, b"engine-ready")?;
            }
            Ok(())
        })
        .build(tauri_context())
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

#[cfg(target_os = "macos")]
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
            let (config_owner, _) = config::ConfigOwner::startup(&config_dir_path);
            let snapshot_reader = config_owner.reader();
            app.manage(engine_control);
            app.manage(ConfigDir(config_dir_path));
            let thread_runtime = ThreadRuntime::start(snapshot_reader).map_err(|error| {
                tauri::Error::Setup((Box::new(error) as Box<dyn std::error::Error>).into())
            })?;
            app.manage(thread_runtime);
            app.state::<ThreadRuntime>()
                .monitor_owner(app.handle().clone())
                .map_err(|error| {
                    tauri::Error::Setup((Box::new(error) as Box<dyn std::error::Error>).into())
                })?;
            tray::setup_macos_packaging_spike(app)?;
            if !app.webview_windows().is_empty() {
                return Err(
                    io::Error::other("macOS Engine must not own a managed WebView window").into(),
                );
            }
            start_engine_ipc(app, server, config_owner)?;
            #[cfg(debug_assertions)]
            if let Some(path) = std::env::var_os("ZG_P03_TEST_READY_MARKER") {
                std::fs::write(path, b"engine-ready")?;
            }
            Ok(())
        })
        .build(tauri_context())
        .map_err(|error| format!("failed to build macOS Engine: {error}"))?;

    app.run(|app, event| {
        if !app.webview_windows().is_empty() {
            app.exit(1);
            return;
        }
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

#[cfg(not(any(windows, target_os = "macos")))]
fn run_engine() -> Result<(), String> {
    Err("Engine mode is supported only on Windows and macOS".to_string())
}

#[cfg(any(windows, target_os = "macos"))]
fn engine_config_dir(default: PathBuf) -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("ZG_P03_TEST_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    default
}

#[cfg(any(windows, target_os = "macos"))]
fn prepare_engine_server(
    config_dir: &Path,
) -> Result<Option<ipc::EngineServer>, ipc::ControlError> {
    #[cfg(debug_assertions)]
    if let Some(namespace) = std::env::var_os("ZG_P03_TEST_NAMESPACE") {
        let namespace = namespace.to_str().ok_or_else(|| {
            ipc::ControlError::Security("P03 test namespace must be valid Unicode".to_string())
        })?;
        return ipc::EngineServer::for_debug_namespace(config_dir, namespace);
    }
    ipc::EngineServer::new(config_dir)
}

#[cfg(any(windows, target_os = "macos"))]
fn start_engine_ipc(
    app: &mut tauri::App,
    server: ipc::EngineServer,
    config_owner: config::ConfigOwner,
) -> tauri::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let server_stop = Arc::clone(&stop);
    let app_handle = app.handle().clone();
    let handle = thread::Builder::new()
        .name("engine-ipc".to_string())
        .spawn(move || {
            let result = server.run(server_stop, config_owner, |active, _generation| {
                let runtime = app_handle.try_state::<ThreadRuntime>().ok_or_else(|| {
                    ipc::ControlError::projection("thread runtime state is unavailable")
                })?;
                runtime
                    .observe_applied(active)
                    .map_err(ipc::ControlError::projection)?;
                #[cfg(windows)]
                if let Err(error) = tray::schedule_toggle_menu_label(&app_handle, active.enabled())
                {
                    warn!("failed to schedule tray label reconciliation: {error}");
                }
                Ok(())
            });
            match result {
                Ok(ipc::ServerExit::Shutdown) => {
                    info!("Engine shutdown requested over IPC");
                    if let Some(runtime) = app_handle.try_state::<ThreadRuntime>() {
                        if let Err(error) = runtime.shutdown() {
                            error!("failed to stop Engine workers: {error}");
                        }
                    }
                    app_handle.exit(0);
                }
                Ok(ipc::ServerExit::Stopped) => {}
                Err(error) => {
                    error!("Engine IPC owner stopped: {error}");
                    if let Some(runtime) = app_handle.try_state::<ThreadRuntime>() {
                        if let Err(error) = runtime.shutdown() {
                            error!("failed to stop Engine workers after fatal IPC error: {error}");
                        }
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
    Ok(())
}

fn run_settings() -> Result<(), String> {
    let context = tauri_context();
    #[cfg(windows)]
    let autostart_registration_name =
        runtime_shell::engine_autostart_registration_name(&context.package_info().name).to_string();
    let log_level = log_config::resolve_log_level();
    #[cfg(debug_assertions)]
    let log_builder =
        if let Some(config_dir) = std::env::var_os("ZG_P03_TEST_CONFIG_DIR") {
            tauri_plugin_log::Builder::new().level(log_level).targets([
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Folder {
                    path: PathBuf::from(config_dir).join("logs"),
                    file_name: Some("settings-test".to_string()),
                }),
            ])
        } else {
            tauri_plugin_log::Builder::new().level(log_level).targets([
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
                tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                    file_name: Some("settings".to_string()),
                }),
            ])
        };
    #[cfg(not(debug_assertions))]
    let log_builder = tauri_plugin_log::Builder::new().level(log_level).targets([
        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout),
        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
            file_name: Some("settings".to_string()),
        }),
    ]);
    #[cfg(windows)]
    let settings_launch_gate = runtime_shell::acquire_settings_launch_gate()?;
    #[cfg(windows)]
    if runtime_shell::forward_to_existing_settings(&context.config().identifier)? {
        return Ok(());
    }
    let builder = tauri::Builder::default();
    #[cfg(windows)]
    let builder = builder
        .plugin(runtime_shell::single_instance_plugin())
        .plugin(runtime_shell::autostart_plugin(
            &autostart_registration_name,
        ));
    let app = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(log_builder.build())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            #[cfg(windows)]
            runtime_shell::ensure_engine_autostart(app, &autostart_registration_name).map_err(
                |error| {
                    tauri::Error::Setup(
                        (Box::new(io::Error::other(error)) as Box<dyn std::error::Error>).into(),
                    )
                },
            )?;
            #[cfg(any(windows, target_os = "macos"))]
            let config_dir_path = engine_config_dir(app.path().app_config_dir()?);
            #[cfg(not(any(windows, target_os = "macos")))]
            let config_dir_path = app.path().app_config_dir()?;
            #[cfg(all(windows, debug_assertions))]
            let unavailable_delay = std::env::var("ZG_P05A_TEST_ENGINE_UNAVAILABLE_DELAY_MS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok());
            #[cfg(all(windows, debug_assertions))]
            if let Some(delay) = unavailable_delay {
                std::thread::sleep(std::time::Duration::from_millis(delay));
            }
            let engine_result = std::env::current_exe()
                .map_err(|error| error.to_string())
                .and_then(|executable| {
                    #[cfg(all(windows, debug_assertions))]
                    if unavailable_delay.is_some() {
                        return Err("Engine unavailable in process test".to_string());
                    }
                    ipc::EngineControl::connect_or_start(&executable, &config_dir_path)
                        .map_err(|error| error.to_string())
                });
            let engine = match engine_result {
                Ok(control) => {
                    #[cfg(debug_assertions)]
                    if let Some(path) = std::env::var_os("ZG_P04B1_TEST_SETTINGS_CONNECTED_MARKER")
                    {
                        std::fs::write(path, b"settings-connected")?;
                    }
                    SettingsEngineState::Connected(control)
                }
                Err(message) => {
                    warn!("Engine unavailable to Settings: {message}");
                    SettingsEngineState::Unavailable(message)
                }
            };
            app.manage(engine);
            app.manage(ConfigDir(config_dir_path));
            app.manage(ThreadRuntime::settings());
            app.manage(commands::CaptureState(std::sync::Mutex::new(None)));
            #[cfg(all(windows, debug_assertions))]
            let skip_window = std::env::var_os("ZG_P05A_TEST_SKIP_SETTINGS_WINDOW").is_some();
            #[cfg(not(all(windows, debug_assertions)))]
            let skip_window = false;
            if !skip_window {
                tray::show_settings_window(app.handle())?;
            }
            #[cfg(all(windows, debug_assertions))]
            schedule_settings_test_exit()?;
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
        .build(context)
        .map_err(|error| format!("failed to build Settings: {error}"))?;

    #[cfg(windows)]
    let mut settings_launch_gate = Some(settings_launch_gate);
    app.run(move |app, event| {
        #[cfg(windows)]
        if matches!(event, tauri::RunEvent::Ready)
            && settings_launch_gate
                .take()
                .is_some_and(|gate| gate.signal_release().is_err())
        {
            app.exit(1);
        }
        if let tauri::RunEvent::ExitRequested { .. } = event {
            if let Some(runtime) = app.try_state::<ThreadRuntime>() {
                let _ = runtime.shutdown();
            }
        }
    });
    Ok(())
}

#[cfg(all(windows, debug_assertions))]
fn schedule_settings_test_exit() -> std::io::Result<()> {
    let Some(trigger) = std::env::var_os("ZG_P05A_TEST_EXIT_SETTINGS_TRIGGER") else {
        return Ok(());
    };
    std::thread::Builder::new()
        .name("settings-test-exit".to_string())
        .spawn(move || {
            let trigger = PathBuf::from(trigger);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            while !trigger.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if trigger.exists() {
                std::process::exit(0);
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_engine_keeps_the_tauri_application_config_directory() {
        assert!(std::env::var_os("ZG_P03_TEST_CONFIG_DIR").is_none());
        let tauri_config_dir = PathBuf::from("/tauri/stable/application-config");
        assert_eq!(
            engine_config_dir(tauri_config_dir.clone()),
            tauri_config_dir
        );
    }

    #[cfg(windows)]
    #[test]
    fn thread_runtime_observes_applied_config_without_restarting_native_owner() {
        let directory = tempfile::tempdir().unwrap();
        let (owner, _) = config::ConfigOwner::startup(directory.path());
        let runtime = ThreadRuntime::start(owner.reader()).unwrap();
        let owner_thread = {
            let state = runtime.state.lock().unwrap();
            let RuntimeState::Running(workers) = &*state else {
                panic!("Engine must start one native input owner");
            };
            workers.hook_handle.as_ref().unwrap().thread().id()
        };

        let mut changed = config::ConfigDocument::default();
        changed.shared.enabled = true;
        changed.shared.appearance.trail_thickness = 7.0;
        match &mut changed.bindings[0] {
            config::BindingRecord::Shared(binding)
            | config::BindingRecord::Windows(binding)
            | config::BindingRecord::Macos(binding) => {
                binding.label = Some("runtime-projection".to_string());
            }
        }
        runtime
            .observe_applied(&config::ActiveConfig::from_document(changed.clone()).unwrap())
            .unwrap();

        let mut disabled = changed;
        disabled.shared.enabled = false;
        runtime
            .observe_applied(&config::ActiveConfig::from_document(disabled).unwrap())
            .unwrap();
        let state = runtime.state.lock().unwrap();
        let RuntimeState::Running(workers) = &*state else {
            panic!("config projection must not restart the native input owner");
        };
        assert_eq!(
            workers.hook_handle.as_ref().unwrap().thread().id(),
            owner_thread
        );
        drop(state);
        runtime.shutdown().unwrap();
    }

    #[test]
    fn thread_runtime_poison_is_a_typed_projection_failure() {
        let runtime = ThreadRuntime::settings();
        runtime.poison_for_test();

        let error = runtime
            .observe_applied(
                &config::ActiveConfig::from_document(config::ConfigDocument::default()).unwrap(),
            )
            .unwrap_err();
        assert!(matches!(error, RuntimeProjectionError::StatePoisoned));
    }
}
