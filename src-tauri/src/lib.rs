mod config;
mod hook;
mod overlay;
mod tray;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, RwLock,
};
use std::thread::JoinHandle;

use crossbeam_channel::Sender;
use tauri::Manager;

#[derive(Clone)]
pub struct SharedConfig(pub Arc<RwLock<config::AppConfig>>);

impl SharedConfig {
    fn new(config: config::AppConfig) -> Self {
        Self(Arc::new(RwLock::new(config)))
    }
}

pub struct ThreadRuntime {
    hook_control_tx: Sender<hook::HookControl>,
    overlay_tx: Sender<overlay::OverlayCommand>,
    hook_handle: Mutex<Option<JoinHandle<()>>>,
    overlay_handle: Mutex<Option<JoinHandle<()>>>,
    is_shutdown: AtomicBool,
}

impl ThreadRuntime {
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

pub struct ExitState {
    allow_exit: AtomicBool,
}

impl ExitState {
    fn new() -> Self {
        Self {
            allow_exit: AtomicBool::new(false),
        }
    }

    pub fn request_exit(&self) {
        self.allow_exit.store(true, Ordering::SeqCst);
    }

    fn should_allow_exit(&self) -> bool {
        self.allow_exit.load(Ordering::SeqCst)
    }
}

#[tauri::command]
fn show_settings_window(app: tauri::AppHandle) -> Result<(), String> {
    tray::show_settings_window(&app).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shared_config = SharedConfig::new(config::load_or_default());

    let app = tauri::Builder::default()
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
