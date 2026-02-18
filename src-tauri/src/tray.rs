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
pub fn setup<R: Runtime>(app: &mut App<R>) -> tauri::Result<()> {
    let enabled = {
        let shared = app.state::<crate::SharedConfig>();
        shared.0.read().map(|c| c.enabled).unwrap_or(true)
    };

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
                let _ = show_settings_window(app);
            }
            MENU_QUIT => {
                let runtime = app.state::<crate::ThreadRuntime>();
                runtime.shutdown();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = show_settings_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    tray.build(app)?;
    Ok(())
}

/// Handles the "Toggle Gestures" menu action.
fn handle_toggle<R: Runtime>(app: &AppHandle<R>) {
    let shared_config = app.state::<crate::SharedConfig>();
    let runtime = app.state::<crate::ThreadRuntime>();
    let config_dir = app.state::<crate::ConfigDir>();

    // Read the current enabled state and build a toggled config.
    let new_config = {
        let current = match shared_config.0.read() {
            Ok(c) => c,
            Err(_) => {
                warn!("shared config lock poisoned in toggle handler");
                return;
            }
        };
        let mut next = current.clone();
        next.enabled = !next.enabled;
        next
    };

    if let Err(err) = crate::commands::apply_config_update(
        new_config,
        app,
        shared_config.inner(),
        runtime.inner(),
        config_dir.inner(),
    ) {
        warn!("failed to toggle gestures: {err}");
        return;
    }
}

/// Synchronizes tray toggle menu text with the current enabled state.
pub fn sync_toggle_menu_label<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    if let Some(toggle_item) = app.try_state::<TrayToggleMenuItem<R>>() {
        let _ = toggle_item.0.set_text(toggle_label(enabled));
    } else {
        warn!("tray toggle menu item is not available to sync label");
    }
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

    let window = tauri::WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("Zero Gesture")
        .inner_size(800.0, 600.0)
        .build()?;

    window.show()?;
    window.set_focus()?;
    Ok(())
}
