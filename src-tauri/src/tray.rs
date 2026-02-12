use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, Runtime, WebviewUrl};

const MENU_OPEN_SETTINGS: &str = "open-settings";
const MENU_QUIT: &str = "quit";

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
                let exit_state = app.state::<crate::ExitState>();
                exit_state.request_exit();
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

pub fn show_settings_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
        return Ok(());
    }

    let window = tauri::WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
        .title("mouse-gesture")
        .inner_size(800.0, 600.0)
        .build()?;

    window.show()?;
    window.set_focus()?;
    Ok(())
}
