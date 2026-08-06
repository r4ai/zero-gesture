use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProcessMode {
    Engine,
    Settings,
    InstalledAcceptance(InstalledAcceptance),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InstalledAcceptance {
    Status(PathBuf),
    Quit,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ModeError {
    UnknownArgument(OsString),
    ConflictingModes,
    MissingValue(&'static str),
}

pub fn select_mode(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<ProcessMode, ModeError> {
    let mut mode = None;
    let mut arguments = arguments.into_iter().skip(1);
    while let Some(argument) = arguments.next() {
        let selected = match argument.to_str() {
            Some("--engine") => ProcessMode::Engine,
            Some("--settings") => ProcessMode::Settings,
            Some("--installed-acceptance-status") => {
                let output = arguments
                    .next()
                    .ok_or(ModeError::MissingValue("--installed-acceptance-status"))?;
                if output.to_string_lossy().starts_with("--") {
                    return Err(ModeError::MissingValue("--installed-acceptance-status"));
                }
                ProcessMode::InstalledAcceptance(InstalledAcceptance::Status(output.into()))
            }
            Some("--installed-acceptance-quit") => {
                ProcessMode::InstalledAcceptance(InstalledAcceptance::Quit)
            }
            _ => return Err(ModeError::UnknownArgument(argument)),
        };
        if mode.replace(selected).is_some() {
            return Err(ModeError::ConflictingModes);
        }
    }
    Ok(mode.unwrap_or(ProcessMode::Settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn default_invocation_selects_settings_mode() {
        assert_eq!(
            select_mode(arguments(&["zero-gesture"])).unwrap(),
            ProcessMode::Settings
        );
    }

    #[test]
    fn explicit_settings_argument_selects_settings_mode() {
        assert_eq!(
            select_mode(arguments(&["zero-gesture", "--settings"])).unwrap(),
            ProcessMode::Settings
        );
    }

    #[test]
    fn engine_argument_selects_engine_mode() {
        assert_eq!(
            select_mode(arguments(&["zero-gesture", "--engine"])).unwrap(),
            ProcessMode::Engine
        );
    }

    #[test]
    fn installed_acceptance_status_requires_an_explicit_output_path() {
        assert_eq!(
            select_mode(arguments(&[
                "zero-gesture",
                "--installed-acceptance-status",
                r"C:\runner temp\engine-status.json"
            ]))
            .unwrap(),
            ProcessMode::InstalledAcceptance(InstalledAcceptance::Status(PathBuf::from(
                r"C:\runner temp\engine-status.json"
            )))
        );
        assert_eq!(
            select_mode(arguments(&[
                "zero-gesture",
                "--installed-acceptance-status"
            ])),
            Err(ModeError::MissingValue("--installed-acceptance-status"))
        );
        assert_eq!(
            select_mode(arguments(&[
                "zero-gesture",
                "--installed-acceptance-status",
                "--engine"
            ])),
            Err(ModeError::MissingValue("--installed-acceptance-status"))
        );
    }

    #[test]
    fn installed_acceptance_quit_rejects_trailing_arguments() {
        assert_eq!(
            select_mode(arguments(&["zero-gesture", "--installed-acceptance-quit"])).unwrap(),
            ProcessMode::InstalledAcceptance(InstalledAcceptance::Quit)
        );
        assert!(matches!(
            select_mode(arguments(&[
                "zero-gesture",
                "--installed-acceptance-quit",
                "unexpected"
            ])),
            Err(ModeError::UnknownArgument(_))
        ));
    }

    #[test]
    fn unknown_argument_is_rejected() {
        assert!(matches!(
            select_mode(arguments(&["zero-gesture", "--unknown"])),
            Err(ModeError::UnknownArgument(_))
        ));
    }

    #[test]
    fn conflicting_modes_are_rejected() {
        assert_eq!(
            select_mode(arguments(&["zero-gesture", "--engine", "--settings"])),
            Err(ModeError::ConflictingModes)
        );
    }
}
