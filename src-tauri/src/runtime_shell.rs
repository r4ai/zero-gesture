use std::fmt;

const ENGINE_AUTOSTART_ARGUMENTS: [&str; 1] = ["--engine"];

#[derive(Debug, PartialEq, Eq)]
enum EngineAutostartError<E> {
    Registration(E),
    Verification(E),
    NotEnabled,
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
            Self::NotEnabled => {
                formatter.write_str("autostart registration remained disabled after enable")
            }
        }
    }
}

fn engine_autostart_arguments() -> [&'static str; 1] {
    ENGINE_AUTOSTART_ARGUMENTS
}

fn ensure_engine_autostart_with<E>(
    mut register: impl FnMut() -> Result<(), E>,
    mut is_enabled: impl FnMut() -> Result<bool, E>,
) -> Result<(), EngineAutostartError<E>> {
    register().map_err(EngineAutostartError::Registration)?;
    match is_enabled().map_err(EngineAutostartError::Verification)? {
        true => Ok(()),
        false => Err(EngineAutostartError::NotEnabled),
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
pub(crate) fn ensure_engine_autostart<R: tauri::Runtime>(
    app: &tauri::App<R>,
) -> Result<(), String> {
    #[cfg(debug_assertions)]
    if std::env::var_os("ZG_P05A_TEST_SKIP_AUTOSTART").is_some() {
        return Ok(());
    }

    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    ensure_engine_autostart_with(|| manager.enable(), || manager.is_enabled())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn autostart_registration_uses_only_the_engine_mode_argument() {
        assert_eq!(engine_autostart_arguments(), ["--engine"]);
    }

    #[test]
    fn autostart_registration_is_idempotent() {
        let enabled = Cell::new(false);
        let registrations = Cell::new(0_u8);
        let mut register = || {
            registrations.set(registrations.get() + 1);
            enabled.set(true);
            Ok::<(), &'static str>(())
        };
        let mut is_enabled = || Ok::<bool, &'static str>(enabled.get());

        ensure_engine_autostart_with(&mut register, &mut is_enabled).unwrap();
        ensure_engine_autostart_with(&mut register, &mut is_enabled).unwrap();

        assert!(enabled.get());
        assert_eq!(registrations.get(), 2);
    }

    #[test]
    fn autostart_registration_failure_is_not_reported_as_success() {
        let error = ensure_engine_autostart_with(
            || Err::<(), _>("registration denied"),
            || Ok::<bool, &'static str>(false),
        )
        .unwrap_err();

        assert_eq!(
            error,
            EngineAutostartError::Registration("registration denied")
        );
    }

    #[test]
    fn autostart_verification_failure_is_not_reported_as_success() {
        let error = ensure_engine_autostart_with(
            || Ok::<(), &'static str>(()),
            || Ok::<bool, &'static str>(false),
        )
        .unwrap_err();

        assert_eq!(error, EngineAutostartError::NotEnabled);
    }

    #[test]
    fn autostart_verification_error_is_not_reported_as_success() {
        let error = ensure_engine_autostart_with(
            || Ok::<(), &'static str>(()),
            || Err::<bool, _>("registry unavailable"),
        )
        .unwrap_err();

        assert_eq!(
            error,
            EngineAutostartError::Verification("registry unavailable")
        );
    }
}
