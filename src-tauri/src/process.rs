use std::ffi::OsString;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessMode {
    Engine,
    Settings,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ModeError {
    UnknownArgument(OsString),
    ConflictingModes,
}

pub fn select_mode(
    arguments: impl IntoIterator<Item = OsString>,
) -> Result<ProcessMode, ModeError> {
    let mut mode = None;
    for argument in arguments.into_iter().skip(1) {
        let selected = match argument.to_str() {
            Some("--engine") => ProcessMode::Engine,
            Some("--settings") => ProcessMode::Settings,
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
