use std::fmt;
use std::path::Path;
use std::time::Duration;

const ENGINE_AUTOSTART_ARGUMENTS: [&str; 1] = ["--engine"];
const SETTINGS_LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);
const SETTINGS_FORWARD_TIMEOUT_MILLIS: u32 = 2_000;
const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
const WINDOWS_RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
const WMCOPYDATA_SINGLE_INSTANCE_DATA: usize = 1542;

#[derive(Debug, PartialEq, Eq)]
enum EngineAutostartError<E> {
    Registration(E),
    Verification(E),
    IncorrectCommand {
        expected: String,
        actual: Option<String>,
    },
}

impl<E: fmt::Display> fmt::Display for EngineAutostartError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Registration(error) => {
                write!(formatter, "autostart registration failed: {error}")
            }
            Self::Verification(error) => {
                write!(
                    formatter,
                    "autostart registration verification failed: {error}"
                )
            }
            Self::IncorrectCommand { expected, actual } => write!(
                formatter,
                "autostart command mismatch: expected {expected:?}, found {actual:?}"
            ),
        }
    }
}

fn engine_autostart_arguments() -> [&'static str; 1] {
    ENGINE_AUTOSTART_ARGUMENTS
}

fn ensure_engine_autostart_with<E>(
    expected_command: &str,
    mut register: impl FnMut(&str) -> Result<(), E>,
    mut registered_command: impl FnMut() -> Result<Option<String>, E>,
) -> Result<(), EngineAutostartError<E>> {
    register(expected_command).map_err(EngineAutostartError::Registration)?;
    let actual = registered_command().map_err(EngineAutostartError::Verification)?;
    if actual.as_deref() == Some(expected_command) {
        Ok(())
    } else {
        Err(EngineAutostartError::IncorrectCommand {
            expected: expected_command.to_string(),
            actual,
        })
    }
}

fn windows_autostart_command(executable: &Path) -> Result<String, &'static str> {
    let executable = executable
        .to_str()
        .ok_or("Windows executable path is not valid UTF-8")?;
    if executable.contains('"') {
        return Err("Windows executable path contains a quote");
    }
    Ok(format!("\"{executable}\" --engine"))
}

#[cfg(windows)]
fn write_windows_autostart_command(app_name: &str, command: &str) -> std::io::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let (run, _) = current_user.create_subkey(WINDOWS_RUN_KEY)?;
    run.set_value(app_name, &command)
}

#[cfg(windows)]
fn read_windows_autostart_command(app_name: &str) -> std::io::Result<Option<String>> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    let run = match current_user.open_subkey_with_flags(WINDOWS_RUN_KEY, KEY_READ) {
        Ok(run) => run,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    match run.get_value(app_name) {
        Ok(command) => Ok(Some(command)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
pub(crate) fn autostart_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(engine_autostart_arguments().to_vec()),
    )
}

#[cfg(windows)]
pub(crate) fn single_instance_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_single_instance::init(|app, _arguments, _working_directory| {
        let existing = app.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("settings-activation".to_string())
            .spawn(move || {
                let activation_app = existing.clone();
                if let Err(error) = existing.run_on_main_thread(move || {
                    if let Err(error) = crate::tray::show_settings_window(&activation_app) {
                        log::warn!("failed to activate existing Settings instance: {error}");
                    }
                }) {
                    log::warn!("failed to schedule existing Settings activation: {error}");
                }
            })
        {
            log::warn!("failed to start existing Settings activation: {error}");
        }
    })
}

#[cfg(windows)]
pub(crate) fn acquire_settings_launch_lock() -> Result<crate::ipc::SettingsLaunchLock, String> {
    crate::ipc::acquire_settings_launch_lock(SETTINGS_LAUNCH_TIMEOUT)
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
pub(crate) fn forward_to_existing_settings(identifier: &str) -> Result<bool, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows_sys::Win32::System::Threading::OpenMutexW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SendMessageTimeoutW, SMTO_ABORTIFHUNG, SMTO_BLOCK, WM_COPYDATA,
    };

    let wide = |value: &str| {
        std::ffi::OsStr::new(value)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>()
    };
    let class_name = wide(&format!("{identifier}-sic"));
    let window_name = wide(&format!("{identifier}-siw"));
    let receiver = unsafe { FindWindowW(class_name.as_ptr(), window_name.as_ptr()) };
    if receiver.is_null() {
        let mutex_name = wide(&format!("{identifier}-sim"));
        let existing_mutex = unsafe { OpenMutexW(SYNCHRONIZE_ACCESS, 0, mutex_name.as_ptr()) };
        if !existing_mutex.is_null() {
            unsafe {
                CloseHandle(existing_mutex);
            }
            return Err("existing Settings instance has no activation receiver".to_string());
        }
        return Ok(false);
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let cwd = cwd.to_str().unwrap_or_default();
    let arguments = std::env::args().collect::<Vec<_>>().join("|");
    let message = format!("{cwd}|{arguments}\0");
    let copy_data = COPYDATASTRUCT {
        dwData: WMCOPYDATA_SINGLE_INSTANCE_DATA,
        cbData: message.len() as u32,
        lpData: message.as_ptr().cast_mut().cast(),
    };
    let mut response = 0;
    let delivered = unsafe {
        SendMessageTimeoutW(
            receiver,
            WM_COPYDATA,
            0,
            (&copy_data as *const COPYDATASTRUCT) as isize,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            SETTINGS_FORWARD_TIMEOUT_MILLIS,
            &mut response,
        )
    };
    if delivered == 0 || response == 0 {
        Err("existing Settings receiver did not accept activation".to_string())
    } else {
        Ok(true)
    }
}

#[cfg(windows)]
pub(crate) fn ensure_engine_autostart<R: tauri::Runtime>(
    app: &tauri::App<R>,
) -> Result<(), String> {
    #[cfg(debug_assertions)]
    if std::env::var_os("ZG_P05A_TEST_SKIP_AUTOSTART").is_some() {
        return Ok(());
    }

    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let app_name = app.package_info().name.clone();
    let command =
        windows_autostart_command(&std::env::current_exe().map_err(|error| error.to_string())?)
            .map_err(str::to_string)?;
    ensure_engine_autostart_with(
        &command,
        |command| {
            manager.enable().map_err(|error| error.to_string())?;
            write_windows_autostart_command(&app_name, command).map_err(|error| error.to_string())
        },
        || read_windows_autostart_command(&app_name).map_err(|error| error.to_string()),
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[test]
    fn autostart_registration_uses_only_the_engine_mode_argument() {
        assert_eq!(engine_autostart_arguments(), ["--engine"]);
    }

    #[test]
    fn autostart_command_quotes_a_spaced_windows_executable_path() {
        let executable = Path::new(r"C:\Program Files\Zero Gesture\zero-gesture.exe");

        assert_eq!(
            windows_autostart_command(executable).unwrap(),
            r#""C:\Program Files\Zero Gesture\zero-gesture.exe" --engine"#
        );
    }

    #[test]
    fn autostart_registration_is_idempotent_for_one_named_command() {
        let registrations = RefCell::new(BTreeMap::new());
        let mut register = |command: &str| {
            registrations
                .borrow_mut()
                .insert("Zero Gesture", command.to_string());
            Ok::<(), &'static str>(())
        };
        let mut registered_command = || {
            Ok::<Option<String>, &'static str>(registrations.borrow().get("Zero Gesture").cloned())
        };
        let command = r#""C:\Program Files\Zero Gesture\zero-gesture.exe" --engine"#;

        ensure_engine_autostart_with(command, &mut register, &mut registered_command).unwrap();
        ensure_engine_autostart_with(command, &mut register, &mut registered_command).unwrap();

        assert_eq!(registrations.borrow().len(), 1);
        assert_eq!(
            registrations.borrow().get("Zero Gesture"),
            Some(&command.to_string())
        );
    }

    #[test]
    fn autostart_registration_failure_is_not_reported_as_success() {
        let error = ensure_engine_autostart_with(
            r#""C:\Zero Gesture\zero-gesture.exe" --engine"#,
            |_| Err::<(), _>("registration denied"),
            || Ok::<Option<String>, &'static str>(None),
        )
        .unwrap_err();

        assert_eq!(
            error,
            EngineAutostartError::Registration("registration denied")
        );
    }

    #[test]
    fn autostart_command_mismatch_is_not_reported_as_success() {
        let expected = r#""C:\Zero Gesture\zero-gesture.exe" --engine"#;
        let error = ensure_engine_autostart_with(
            expected,
            |_| Ok::<(), &'static str>(()),
            || Ok::<Option<String>, &'static str>(Some("wrong".to_string())),
        )
        .unwrap_err();

        assert_eq!(
            error,
            EngineAutostartError::IncorrectCommand {
                expected: expected.to_string(),
                actual: Some("wrong".to_string())
            }
        );
    }

    #[test]
    fn autostart_verification_error_is_not_reported_as_success() {
        let error = ensure_engine_autostart_with(
            r#""C:\Zero Gesture\zero-gesture.exe" --engine"#,
            |_| Ok::<(), &'static str>(()),
            || Err::<Option<String>, _>("registry unavailable"),
        )
        .unwrap_err();

        assert_eq!(
            error,
            EngineAutostartError::Verification("registry unavailable")
        );
    }
}
