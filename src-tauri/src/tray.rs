//! System tray integration.
//!
//! Creates the tray icon with a context menu ("Open Settings" / "Quit") and
//! handles tray events such as left-click (open settings) and menu actions.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Runtime, WebviewUrl};

/// Menu item ID for the "Open Settings" action.
const MENU_OPEN_SETTINGS: &str = "open-settings";

/// Menu item ID for the "Quit" action.
const MENU_QUIT: &str = "quit";

/// Builds and registers the system tray icon and its context menu.
///
/// The tray provides two menu items:
///
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
    let open_settings_item =
        MenuItem::with_id(app, MENU_OPEN_SETTINGS, "Open Settings", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_settings_item, &quit_item])?;

    let mut tray = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
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
