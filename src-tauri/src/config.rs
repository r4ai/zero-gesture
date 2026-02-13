use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Configuration file name placed in the working directory.
const CONFIG_FILE_NAME: &str = "mouse-gesture.config.json";

/// Application-wide configuration persisted as JSON.
///
/// # Examples
///
/// ```
/// use mouse_gesture_lib::config::AppConfig;
///
/// let config = AppConfig::default();
/// assert_eq!(config.gesture_trigger_button, "right");
/// assert_eq!(config.trail_color, "#00BFFF");
/// assert_eq!(config.trail_thickness, 3.0);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Mouse button that triggers a gesture (e.g. `"right"`).
    pub gesture_trigger_button: String,

    /// CSS colour string used to draw the gesture trail (e.g. `"#00BFFF"`).
    pub trail_color: String,

    /// Thickness in logical pixels for the gesture trail line.
    pub trail_thickness: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gesture_trigger_button: "right".to_string(),
            trail_color: "#00BFFF".to_string(),
            trail_thickness: 3.0,
        }
    }
}

/// Loads [`AppConfig`] from the configuration file, falling back to
/// [`AppConfig::default`] if the file is missing or contains invalid JSON.
///
/// # Examples
///
/// ```no_run
/// use mouse_gesture_lib::config::load_or_default;
///
/// let config = load_or_default();
/// println!("trigger button: {}", config.gesture_trigger_button);
/// ```
pub fn load_or_default() -> AppConfig {
    let path = config_path();
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => return AppConfig::default(),
    };

    serde_json::from_str(&raw).unwrap_or_default()
}

/// Serializes `config` as pretty-printed JSON and writes it to the
/// configuration file.
///
/// # Errors
///
/// Returns [`io::Error`] if serialization or file I/O fails.
///
/// # Examples
///
/// ```no_run
/// use mouse_gesture_lib::config::{save, AppConfig};
///
/// let config = AppConfig::default();
/// save(&config).expect("failed to save config");
/// ```
#[allow(dead_code)]
pub fn save(config: &AppConfig) -> io::Result<()> {
    let body = serde_json::to_string_pretty(config)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(config_path(), body)
}

/// Returns the path to the configuration file.
fn config_path() -> PathBuf {
    PathBuf::from(CONFIG_FILE_NAME)
}
