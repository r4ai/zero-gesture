use tauri_plugin_log::log::LevelFilter;

/// Environment variable name used to control the application log level.
const LOG_LEVEL_ENV_VAR: &str = "ZG_LOG_LEVEL";
const DEFAULT_LOG_LEVEL: LevelFilter = LevelFilter::Debug;

/// Parses a log level string into [`LevelFilter`].
///
/// Accepted values are case-insensitive: `off`, `error`, `warn`, `info`,
/// `debug`, and `trace`.
fn parse_log_level(value: &str) -> Option<LevelFilter> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("off") {
        Some(LevelFilter::Off)
    } else if value.eq_ignore_ascii_case("error") {
        Some(LevelFilter::Error)
    } else if value.eq_ignore_ascii_case("warn") {
        Some(LevelFilter::Warn)
    } else if value.eq_ignore_ascii_case("info") {
        Some(LevelFilter::Info)
    } else if value.eq_ignore_ascii_case("debug") {
        Some(LevelFilter::Debug)
    } else if value.eq_ignore_ascii_case("trace") {
        Some(LevelFilter::Trace)
    } else {
        None
    }
}

/// Resolves the runtime log level from the environment variable.
///
/// Falls back to [`DEFAULT_LOG_LEVEL`] when the variable is not set or
/// contains an unsupported value.
pub fn resolve_log_level() -> LevelFilter {
    std::env::var(LOG_LEVEL_ENV_VAR)
        .ok()
        .and_then(|value| parse_log_level(&value))
        .unwrap_or(DEFAULT_LOG_LEVEL)
}

#[cfg(test)]
mod tests {
    use super::{parse_log_level, LevelFilter};

    #[test]
    fn parse_log_level_supports_case_insensitive_values() {
        assert_eq!(parse_log_level("TRACE"), Some(LevelFilter::Trace));
        assert_eq!(parse_log_level("debug"), Some(LevelFilter::Debug));
        assert_eq!(parse_log_level("Info"), Some(LevelFilter::Info));
        assert_eq!(parse_log_level("warn"), Some(LevelFilter::Warn));
        assert_eq!(parse_log_level("error"), Some(LevelFilter::Error));
        assert_eq!(parse_log_level("off"), Some(LevelFilter::Off));
    }

    #[test]
    fn parse_log_level_trims_whitespace_and_rejects_unknown_values() {
        assert_eq!(parse_log_level("  debug  "), Some(LevelFilter::Debug));
        assert_eq!(parse_log_level("verbose"), None);
        assert_eq!(parse_log_level(""), None);
    }
}
