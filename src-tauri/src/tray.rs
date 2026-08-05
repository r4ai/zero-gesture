//! System tray integration.
//!
//! Creates the tray icon with a context menu ("Toggle Gestures" / "Open
//! Settings" / "Quit") and handles tray events such as left-click (open
//! settings) and menu actions.

use log::warn;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Runtime, WebviewUrl};

/// Menu item ID for the gesture enable/disable toggle.
const MENU_TOGGLE_ENABLED: &str = "toggle-enabled";

/// Menu item ID for the "Open Settings" action.
const MENU_OPEN_SETTINGS: &str = "open-settings";

/// Menu item ID for the "Quit" action.
const MENU_QUIT: &str = "quit";

/// Managed state containing the tray toggle menu item handle.
pub struct TrayToggleMenuItem<R: Runtime>(pub MenuItem<R>);

/// Returns the label for the toggle menu item based on the current `enabled`
/// state.
fn toggle_label(enabled: bool) -> &'static str {
    if enabled {
        "Disable Gestures"
    } else {
        "Enable Gestures"
    }
}

/// Builds and registers the system tray icon and its context menu.
///
/// The tray provides three menu items:
///
/// * **Toggle Gestures** — enables or disables gesture recognition.
/// * **Open Settings** — opens (or focuses) the settings webview window.
/// * **Quit** — performs a graceful shutdown of background threads and exits
///   the application.
///
/// A left-click on the tray icon also opens the settings window.
///
/// # Errors
///
/// Returns [`tauri::Error`] if menu or tray construction fails.
pub fn setup<R: Runtime>(app: &mut App<R>, enabled: bool) -> tauri::Result<()> {
    let toggle_item = MenuItem::with_id(
        app,
        MENU_TOGGLE_ENABLED,
        toggle_label(enabled),
        true,
        None::<&str>,
    )?;
    let open_settings_item =
        MenuItem::with_id(app, MENU_OPEN_SETTINGS, "Open Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle_item, &open_settings_item, &quit_item])?;
    app.manage(TrayToggleMenuItem(toggle_item.clone()));

    let mut tray = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            MENU_TOGGLE_ENABLED => {
                handle_toggle(app);
            }
            MENU_OPEN_SETTINGS => {
                if let Err(error) = launch_settings_process() {
                    warn!("failed to launch Settings: {error}");
                }
            }
            MENU_QUIT => {
                let runtime = app.state::<crate::ThreadRuntime>();
                if let Err(error) = quit_engine_with(|| runtime.shutdown(), || app.exit(0)) {
                    warn!("failed to stop Engine workers: {error}");
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|_tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Err(error) = launch_settings_process() {
                    warn!("failed to launch Settings: {error}");
                }
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    tray.build(app)?;
    Ok(())
}

/// Creates the macOS packaging-spike status item without starting a WebView.
///
/// P04a proves only the same-bundle process topology. Gesture control remains
/// unavailable until the macOS IPC and native adapters are implemented.
#[cfg(target_os = "macos")]
pub fn setup_macos_packaging_spike<R: Runtime>(app: &mut App<R>) -> tauri::Result<()> {
    let open_settings_item =
        MenuItem::with_id(app, MENU_OPEN_SETTINGS, "Open Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_settings_item, &quit_item])?;

    let mut tray = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            MENU_OPEN_SETTINGS => {
                if let Err(error) = launch_settings_process() {
                    warn!("failed to launch Settings: {error}");
                }
            }
            MENU_QUIT => {
                let runtime = app.state::<crate::ThreadRuntime>();
                if let Err(error) = quit_engine_with(|| runtime.shutdown(), || app.exit(0)) {
                    warn!("failed to stop macOS packaging-spike Engine: {error}");
                }
            }
            _ => {}
        })
        .on_tray_icon_event(|_tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Err(error) = launch_settings_process() {
                    warn!("failed to launch Settings: {error}");
                }
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

fn launch_settings_process() -> std::io::Result<()> {
    let executable = std::env::current_exe()?;
    spawn_settings_process_with(&executable, |path, argument| {
        std::process::Command::new(path)
            .arg(argument)
            .spawn()
            .map(|_| ())
    })
}

fn spawn_settings_process_with<E>(
    executable: &std::path::Path,
    spawn: impl FnOnce(&std::path::Path, &str) -> Result<(), E>,
) -> Result<(), E> {
    spawn(executable, "--settings")
}

fn quit_engine_with<E>(
    shutdown: impl FnOnce() -> Result<(), E>,
    exit: impl FnOnce(),
) -> Result<(), E> {
    let result = shutdown();
    exit();
    result
}

fn exit_settings_if_requested(close_requested: bool, exit: impl FnOnce()) {
    if close_requested {
        exit();
    }
}

/// Handles the "Toggle Gestures" menu action.
fn handle_toggle<R: Runtime>(app: &AppHandle<R>) {
    let control = app.state::<crate::ipc::EngineControl>();

    let current = match control.current_config() {
        Ok(current) => current,
        Err(error) => {
            warn!("configuration unavailable in toggle handler: {error}");
            return;
        }
    };
    let Some(config) = current.config else {
        warn!("configuration unavailable in toggle handler");
        return;
    };

    match control.set_enabled(!config.shared.enabled, current.revision) {
        Ok(applied) => {
            if let Some(config) = applied.current.config {
                sync_toggle_menu_label(app, config.shared.enabled);
            }
        }
        Err(error) => {
            warn!("failed to toggle gestures: {error}");
        }
    }
}

/// Synchronizes tray toggle menu text with the current enabled state.
fn sync_toggle_menu_label<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    if let Some(toggle_item) = app.try_state::<TrayToggleMenuItem<R>>() {
        let _ = toggle_item.0.set_text(toggle_label(enabled));
    } else {
        warn!("tray toggle menu item is not available to sync label");
    }
}

type MainThreadTask = Box<dyn FnOnce() + Send + 'static>;

fn schedule_toggle_menu_label_with<E>(
    enabled: bool,
    reconcile: impl FnOnce(bool) + Send + 'static,
    schedule: impl FnOnce(MainThreadTask) -> Result<(), E>,
) -> Result<(), E> {
    schedule(Box::new(move || {
        reconcile(enabled);
    }))
}

/// Queues tray reconciliation without waiting for Tauri's main thread.
///
/// The Engine IPC owner uses this after worker projection and before returning
/// Applied. The queued task may call the synchronous menu API only after the
/// owner callback has returned.
pub(crate) fn schedule_toggle_menu_label<R: Runtime>(
    app: &AppHandle<R>,
    enabled: bool,
) -> tauri::Result<()> {
    let app = app.clone();
    let scheduler = app.clone();
    schedule_toggle_menu_label_with(
        enabled,
        move |enabled| sync_toggle_menu_label(&app, enabled),
        move |task| scheduler.run_on_main_thread(task),
    )
}

/// Opens the settings webview window, or brings it to the foreground if it
/// already exists.
///
/// If no window with the label `"main"` exists, a new 800 × 600 webview
/// window is created at the default URL.
///
/// # Errors
///
/// Returns [`tauri::Error`] if the window cannot be created or focused.
pub fn show_settings_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
        return Ok(());
    }

    let builder = tauri::WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Zero Gesture")
        .inner_size(800.0, 600.0);
    #[cfg(all(windows, debug_assertions))]
    let builder = if let Some(data_dir) = std::env::var_os("ZG_P05A_TEST_WEBVIEW_DATA_DIR") {
        builder.data_directory(std::path::PathBuf::from(data_dir))
    } else {
        builder
    };
    let window = builder.build()?;

    #[cfg(windows)]
    {
        let settings = app.clone();
        window.on_window_event(move |event| {
            exit_settings_if_requested(
                matches!(event, tauri::WindowEvent::CloseRequested { .. }),
                || settings.exit(0),
            );
        });
    }

    window.show()?;
    window.set_focus()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{self, ConfigOwner};
    use std::path::Path;
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn tray_originated_commit_returns_applied_before_label_reconciliation() {
        let directory = tempfile::tempdir().unwrap();
        let (mut owner, _) = ConfigOwner::startup(directory.path());
        let (revision, _, _) = owner.current_bytes(Instant::now()).unwrap();
        let mut document = config::ConfigDocument::default();
        document.shared.enabled = false;
        let bytes = serde_json::to_vec(&document).unwrap();
        let label = Arc::new(Mutex::new(toggle_label(true)));
        let reconcile_label = Arc::clone(&label);
        let (task_tx, task_rx) = mpsc::sync_channel(1);
        let (applied_tx, applied_rx) = mpsc::sync_channel(1);

        let worker = thread::spawn(move || {
            let prepared = owner.prepare(1, revision, &bytes, Instant::now()).unwrap();
            let applied = owner
                .commit(
                    1,
                    prepared.token,
                    prepared.base_revision,
                    prepared.base_generation,
                    Instant::now(),
                )
                .unwrap();
            schedule_toggle_menu_label_with(
                false,
                move |enabled| {
                    *reconcile_label.lock().unwrap() = toggle_label(enabled);
                },
                move |task| {
                    task_tx.send(task).unwrap();
                    Ok::<(), ()>(())
                },
            )
            .unwrap();
            applied_tx.send(applied).unwrap();
        });

        let applied = applied_rx.recv_timeout(Duration::from_millis(500)).unwrap();
        assert_eq!((applied.revision, applied.generation), (2, 2));
        assert_eq!(*label.lock().unwrap(), toggle_label(true));
        task_rx.recv_timeout(Duration::from_millis(500)).unwrap()();
        assert_eq!(*label.lock().unwrap(), toggle_label(false));
        worker.join().unwrap();
    }

    #[test]
    fn settings_originated_applied_eventually_reconciles_tray_label() {
        let label = Arc::new(Mutex::new(toggle_label(false)));
        let reconcile_label = Arc::clone(&label);
        let (task_tx, task_rx) = mpsc::sync_channel(1);

        schedule_toggle_menu_label_with(
            true,
            move |enabled| {
                *reconcile_label.lock().unwrap() = toggle_label(enabled);
            },
            move |task| {
                task_tx.send(task).unwrap();
                Ok::<(), ()>(())
            },
        )
        .unwrap();

        assert_eq!(*label.lock().unwrap(), toggle_label(false));
        task_rx.recv_timeout(Duration::from_millis(500)).unwrap()();
        assert_eq!(*label.lock().unwrap(), toggle_label(true));
    }

    #[test]
    fn repeated_tray_open_requests_use_settings_mode() {
        let executable = Path::new(r"C:\Program Files\Zero Gesture\zero-gesture.exe");
        let requests = Arc::new(Mutex::new(Vec::new()));

        for _ in 0..2 {
            let requests = Arc::clone(&requests);
            spawn_settings_process_with(executable, move |path, argument| {
                requests
                    .lock()
                    .unwrap()
                    .push((path.to_path_buf(), argument.to_string()));
                Ok::<(), ()>(())
            })
            .unwrap();
        }

        assert_eq!(
            *requests.lock().unwrap(),
            vec![
                (executable.to_path_buf(), "--settings".to_string()),
                (executable.to_path_buf(), "--settings".to_string()),
            ]
        );
    }

    #[test]
    fn engine_quit_stops_workers_before_process_exit() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let shutdown_events = Arc::clone(&events);
        let exit_events = Arc::clone(&events);

        quit_engine_with(
            move || {
                shutdown_events.lock().unwrap().push("shutdown");
                Ok::<(), ()>(())
            },
            move || exit_events.lock().unwrap().push("exit"),
        )
        .unwrap();

        assert_eq!(*events.lock().unwrap(), vec!["shutdown", "exit"]);
    }

    #[test]
    fn engine_quit_has_no_autostart_mutation_capability() {
        let autostart_enabled = std::cell::Cell::new(true);

        quit_engine_with(|| Ok::<(), ()>(()), || {}).unwrap();

        assert!(autostart_enabled.get());
    }

    #[test]
    fn settings_close_request_exits_the_process() {
        let exited = std::cell::Cell::new(false);

        exit_settings_if_requested(true, || exited.set(true));

        assert!(exited.get());
    }
}
