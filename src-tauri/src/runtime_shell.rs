use std::fmt;
use std::path::Path;
use std::time::Duration;

const ENGINE_AUTOSTART_ARGUMENTS: [&str; 1] = ["--engine"];
const SETTINGS_LAUNCH_TIMEOUT: Duration = Duration::from_secs(10);
const SETTINGS_FORWARD_TIMEOUT_MILLIS: u32 = 2_000;
const SYNCHRONIZE_ACCESS: u32 = 0x0010_0000;
const WINDOWS_RUN_KEY: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
const WINDOWS_STARTUP_APPROVED_KEY: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const WMCOPYDATA_SINGLE_INSTANCE_DATA: usize = 1542;

#[derive(Debug, PartialEq, Eq)]
enum EngineAutostartError<E> {
    Snapshot(E),
    Registration(E),
    Rewrite(E),
    Verification(E),
    IncorrectCommand {
        expected: String,
        actual: Option<String>,
    },
    Rollback(E),
}

impl<E: fmt::Display> fmt::Display for EngineAutostartError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(error) => {
                write!(formatter, "autostart state snapshot failed: {error}")
            }
            Self::Registration(error) => {
                write!(formatter, "autostart registration failed: {error}")
            }
            Self::Rewrite(error) => {
                write!(formatter, "autostart command rewrite failed: {error}")
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
            Self::Rollback(error) => {
                write!(formatter, "autostart rollback failed: {error}")
            }
        }
    }
}

#[derive(Clone, Copy)]
struct AutostartRegistration<'a> {
    name: &'a str,
    command: &'a str,
}

fn engine_autostart_arguments() -> [&'static str; 1] {
    ENGINE_AUTOSTART_ARGUMENTS
}

pub(crate) fn engine_autostart_registration_name(package_name: &str) -> &str {
    package_name
}

fn ensure_engine_autostart_with<S, E>(
    registration: AutostartRegistration<'_>,
    snapshot: impl FnOnce(&str) -> Result<S, E>,
    enable: impl FnOnce() -> Result<(), E>,
    rewrite: impl FnOnce(&str, &str) -> Result<(), E>,
    verify: impl FnOnce(&str) -> Result<Option<String>, E>,
    rollback: impl FnOnce(&str, S) -> Result<(), E>,
) -> Result<(), EngineAutostartError<E>> {
    let prior = snapshot(registration.name).map_err(EngineAutostartError::Snapshot)?;
    let result = enable()
        .map_err(EngineAutostartError::Registration)
        .and_then(|()| {
            rewrite(registration.name, registration.command).map_err(EngineAutostartError::Rewrite)
        })
        .and_then(|()| {
            let actual = verify(registration.name).map_err(EngineAutostartError::Verification)?;
            if actual.as_deref() == Some(registration.command) {
                Ok(())
            } else {
                Err(EngineAutostartError::IncorrectCommand {
                    expected: registration.command.to_string(),
                    actual,
                })
            }
        });
    if result.is_err() {
        rollback(registration.name, prior).map_err(EngineAutostartError::Rollback)?;
    }
    result
}

#[cfg(windows)]
struct WindowsAutostartState {
    run: Option<winreg::RegValue>,
    startup_approved: Option<winreg::RegValue>,
}

#[cfg(windows)]
fn open_windows_autostart_key(path: &str) -> std::io::Result<winreg::RegKey> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(path, KEY_QUERY_VALUE | KEY_SET_VALUE)
}

#[cfg(windows)]
fn read_optional_raw_value(
    path: &str,
    name: &str,
    missing_key_is_empty: bool,
) -> std::io::Result<Option<winreg::RegValue>> {
    let key = match open_windows_autostart_key(path) {
        Ok(key) => key,
        Err(error) if missing_key_is_empty && error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    match key.get_raw_value(name) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn snapshot_windows_autostart(name: &str) -> std::io::Result<WindowsAutostartState> {
    Ok(WindowsAutostartState {
        run: read_optional_raw_value(WINDOWS_RUN_KEY, name, false)?,
        startup_approved: read_optional_raw_value(WINDOWS_STARTUP_APPROVED_KEY, name, true)?,
    })
}

#[cfg(windows)]
fn restore_windows_autostart_value(
    path: &str,
    name: &str,
    value: Option<winreg::RegValue>,
    missing_key_is_empty: bool,
) -> std::io::Result<()> {
    let key = match open_windows_autostart_key(path) {
        Ok(key) => key,
        Err(error)
            if missing_key_is_empty
                && value.is_none()
                && error.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    match value {
        Some(value) => key.set_raw_value(name, &value),
        None => match key.delete_value(name) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
    }
}

#[cfg(windows)]
fn restore_windows_autostart(name: &str, state: WindowsAutostartState) -> std::io::Result<()> {
    let run = restore_windows_autostart_value(WINDOWS_RUN_KEY, name, state.run, false);
    let startup = restore_windows_autostart_value(
        WINDOWS_STARTUP_APPROVED_KEY,
        name,
        state.startup_approved,
        true,
    );
    if let Err(error) = run {
        Err(error)
    } else {
        startup
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
    let run = open_windows_autostart_key(WINDOWS_RUN_KEY)?;
    run.set_value(app_name, &command)
}

#[cfg(windows)]
fn read_windows_autostart_command(app_name: &str) -> std::io::Result<Option<String>> {
    let run = open_windows_autostart_key(WINDOWS_RUN_KEY)?;
    match run.get_value(app_name) {
        Ok(command) => Ok(Some(command)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
pub(crate) fn autostart_plugin<R: tauri::Runtime>(
    registration_name: &str,
) -> tauri::plugin::TauriPlugin<R> {
    tauri_plugin_autostart::Builder::new()
        .app_name(registration_name)
        .args(engine_autostart_arguments())
        .build()
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
pub(crate) fn acquire_settings_launch_gate() -> Result<crate::ipc::SettingsLaunchGate, String> {
    crate::ipc::acquire_settings_launch_gate(SETTINGS_LAUNCH_TIMEOUT)
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
    registration_name: &str,
) -> Result<(), String> {
    #[cfg(debug_assertions)]
    if std::env::var_os("ZG_P05A_TEST_SKIP_AUTOSTART").is_some() {
        return Ok(());
    }

    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let command =
        windows_autostart_command(&std::env::current_exe().map_err(|error| error.to_string())?)
            .map_err(str::to_string)?;
    let registration = AutostartRegistration {
        name: registration_name,
        command: &command,
    };
    ensure_engine_autostart_with(
        registration,
        |name| snapshot_windows_autostart(name).map_err(|error| error.to_string()),
        || manager.enable().map_err(|error| error.to_string()),
        |name, command| {
            write_windows_autostart_command(name, command).map_err(|error| error.to_string())
        },
        |name| read_windows_autostart_command(name).map_err(|error| error.to_string()),
        |name, prior| restore_windows_autostart(name, prior).map_err(|error| error.to_string()),
    )
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    struct FakeRegistry {
        run: BTreeMap<String, String>,
        startup_approved: BTreeMap<String, Vec<u8>>,
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum FakeFailure {
        Enable,
        Rewrite,
        Verify,
        Mismatch,
    }

    fn apply_fake_autostart(
        registry: &RefCell<FakeRegistry>,
        registration: AutostartRegistration<'_>,
        failure: Option<FakeFailure>,
    ) -> Result<(), EngineAutostartError<&'static str>> {
        let plugin_registration_name = registration.name;
        ensure_engine_autostart_with(
            registration,
            |_| Ok(registry.borrow().clone()),
            || {
                let mut registry = registry.borrow_mut();
                registry.run.insert(
                    plugin_registration_name.to_string(),
                    r"C:\Program Files\Zero Gesture\zero-gesture.exe --engine".to_string(),
                );
                registry
                    .startup_approved
                    .insert(plugin_registration_name.to_string(), vec![2; 12]);
                if failure == Some(FakeFailure::Enable) {
                    Err("enable failed")
                } else {
                    Ok(())
                }
            },
            |name, command| {
                if failure == Some(FakeFailure::Rewrite) {
                    return Err("rewrite failed");
                }
                registry
                    .borrow_mut()
                    .run
                    .insert(name.to_string(), command.to_string());
                Ok(())
            },
            |name| {
                if failure == Some(FakeFailure::Verify) {
                    return Err("verification failed");
                }
                if failure == Some(FakeFailure::Mismatch) {
                    return Ok(Some("wrong command".to_string()));
                }
                Ok(registry.borrow().run.get(name).cloned())
            },
            |_, prior| {
                *registry.borrow_mut() = prior;
                Ok(())
            },
        )
    }

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
        let registry = RefCell::new(FakeRegistry::default());
        let name = engine_autostart_registration_name("Package Product Name");
        let command = r#""C:\Program Files\Zero Gesture\zero-gesture.exe" --engine"#;
        let registration = AutostartRegistration { name, command };

        apply_fake_autostart(&registry, registration, None).unwrap();
        apply_fake_autostart(&registry, registration, None).unwrap();

        assert_eq!(registry.borrow().run.len(), 1);
        assert_eq!(registry.borrow().run.get(name), Some(&command.to_string()));
        assert_eq!(registry.borrow().startup_approved.len(), 1);
        assert!(registry.borrow().startup_approved.contains_key(name));
    }

    #[test]
    fn autostart_registration_failure_restores_prior_values() {
        let prior = FakeRegistry {
            run: BTreeMap::from([("Package Product Name".to_string(), "prior".to_string())]),
            startup_approved: BTreeMap::from([("Package Product Name".to_string(), vec![3; 12])]),
        };
        let registry = RefCell::new(prior.clone());
        let registration = AutostartRegistration {
            name: engine_autostart_registration_name("Package Product Name"),
            command: r#""C:\Zero Gesture\zero-gesture.exe" --engine"#,
        };

        let error =
            apply_fake_autostart(&registry, registration, Some(FakeFailure::Enable)).unwrap_err();

        assert_eq!(error, EngineAutostartError::Registration("enable failed"));
        assert_eq!(*registry.borrow(), prior);
    }

    #[test]
    fn autostart_rewrite_failure_removes_plugin_intermediate_values() {
        let registry = RefCell::new(FakeRegistry::default());
        let registration = AutostartRegistration {
            name: engine_autostart_registration_name("Package Product Name"),
            command: r#""C:\Zero Gesture\zero-gesture.exe" --engine"#,
        };

        let error =
            apply_fake_autostart(&registry, registration, Some(FakeFailure::Rewrite)).unwrap_err();

        assert_eq!(error, EngineAutostartError::Rewrite("rewrite failed"));
        assert_eq!(*registry.borrow(), FakeRegistry::default());
    }

    #[test]
    fn autostart_command_mismatch_restores_prior_values() {
        let prior = FakeRegistry {
            run: BTreeMap::from([("Package Product Name".to_string(), "prior".to_string())]),
            startup_approved: BTreeMap::new(),
        };
        let registry = RefCell::new(prior.clone());
        let registration = AutostartRegistration {
            name: engine_autostart_registration_name("Package Product Name"),
            command: r#""C:\Zero Gesture\zero-gesture.exe" --engine"#,
        };
        let error =
            apply_fake_autostart(&registry, registration, Some(FakeFailure::Mismatch)).unwrap_err();

        assert_eq!(
            error,
            EngineAutostartError::IncorrectCommand {
                expected: registration.command.to_string(),
                actual: Some("wrong command".to_string())
            }
        );
        assert_eq!(*registry.borrow(), prior);
    }

    #[test]
    fn autostart_verification_error_restores_prior_values() {
        let prior = FakeRegistry {
            run: BTreeMap::new(),
            startup_approved: BTreeMap::from([("Package Product Name".to_string(), vec![3; 12])]),
        };
        let registry = RefCell::new(prior.clone());
        let registration = AutostartRegistration {
            name: engine_autostart_registration_name("Package Product Name"),
            command: r#""C:\Zero Gesture\zero-gesture.exe" --engine"#,
        };
        let error =
            apply_fake_autostart(&registry, registration, Some(FakeFailure::Verify)).unwrap_err();

        assert_eq!(
            error,
            EngineAutostartError::Verification("verification failed")
        );
        assert_eq!(*registry.borrow(), prior);
    }
}
