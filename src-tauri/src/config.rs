use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::executor::Action;

/// Configuration file name placed in the working directory.
const CONFIG_FILE_NAME: &str = "zero-gesture.config.json";

/// Application-wide configuration persisted as JSON.
///
/// # Examples
///
/// ```
/// use zero_gesture_lib::config::AppConfig;
///
/// let config = AppConfig::default();
/// assert_eq!(config.gesture_trigger_button, "right");
/// assert_eq!(config.trail_color, "#00BFFF");
/// assert_eq!(config.trail_thickness, 3.0);
/// assert_eq!(config.gesture_threshold, 10);
/// assert_eq!(config.safety_timeout_ms, 2000);
/// assert_eq!(config.min_segment_px, 12);
/// assert_eq!(config.direction_switch_confirm_px, 8);
/// assert_eq!(config.axis_ambiguity_deadzone_px, 2);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Mouse button that triggers a gesture (e.g. `"right"`).
    pub gesture_trigger_button: String,

    /// CSS colour string used to draw the gesture trail (e.g. `"#00BFFF"`).
    pub trail_color: String,

    /// Thickness in logical pixels for the gesture trail line.
    pub trail_thickness: f32,

    /// Pixel distance threshold before a held button becomes a gesture.
    pub gesture_threshold: i32,

    /// Timeout in milliseconds used for stuck-state recovery.
    pub safety_timeout_ms: u32,

    /// Minimum movement distance (in pixels) required to confirm a gesture
    /// direction segment.
    pub min_segment_px: i32,

    /// Minimum movement distance (in pixels) required to switch to a new
    /// direction candidate.
    pub direction_switch_confirm_px: i32,

    /// Deadzone (in pixels) used to ignore tiny ambiguous diagonal movement.
    pub axis_ambiguity_deadzone_px: i32,

    /// Gesture-to-action bindings.
    ///
    /// Keys are `GestureKind` variant names (e.g. `"Left"`, `"DownRight"`),
    /// values are the action to execute when that gesture is recognised.
    pub bindings: HashMap<String, Action>,
}

impl AppConfig {
    /// Default pixel distance threshold for gesture activation.
    pub const DEFAULT_GESTURE_THRESHOLD: i32 = 10;

    /// Default timeout used by the safety timer.
    pub const DEFAULT_SAFETY_TIMEOUT_MS: u32 = 2000;

    /// Default minimum segment distance for gesture direction confirmation.
    pub const DEFAULT_MIN_SEGMENT_PX: i32 = 12;

    /// Default hysteresis distance for direction switching.
    pub const DEFAULT_DIRECTION_SWITCH_CONFIRM_PX: i32 = 8;

    /// Default deadzone for tiny ambiguous diagonal movement.
    pub const DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX: i32 = 2;

    /// Default gesture-to-action bindings.
    fn default_bindings() -> HashMap<String, Action> {
        HashMap::from([
            (
                "Left".to_string(),
                Action::Keyboard {
                    keys: vec!["alt".to_string(), "left".to_string()],
                },
            ),
            (
                "Right".to_string(),
                Action::Keyboard {
                    keys: vec!["alt".to_string(), "right".to_string()],
                },
            ),
            (
                "Up".to_string(),
                Action::Keyboard {
                    keys: vec!["pageup".to_string()],
                },
            ),
            (
                "Down".to_string(),
                Action::Keyboard {
                    keys: vec!["pagedown".to_string()],
                },
            ),
            (
                "DownUp".to_string(),
                Action::Keyboard {
                    keys: vec!["ctrl".to_string(), "home".to_string()],
                },
            ),
            (
                "UpDown".to_string(),
                Action::Keyboard {
                    keys: vec!["ctrl".to_string(), "end".to_string()],
                },
            ),
        ])
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            gesture_trigger_button: "right".to_string(),
            trail_color: "#00BFFF".to_string(),
            trail_thickness: 3.0,
            gesture_threshold: Self::DEFAULT_GESTURE_THRESHOLD,
            safety_timeout_ms: Self::DEFAULT_SAFETY_TIMEOUT_MS,
            min_segment_px: Self::DEFAULT_MIN_SEGMENT_PX,
            direction_switch_confirm_px: Self::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX,
            axis_ambiguity_deadzone_px: Self::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX,
            bindings: Self::default_bindings(),
        }
    }
}

/// Loads [`AppConfig`] from the configuration file, falling back to
/// [`AppConfig::default`] if the file is missing or contains invalid JSON.
///
/// # Examples
///
/// ```no_run
/// use zero_gesture_lib::config::load_or_default;
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
/// use zero_gesture_lib::config::{save, AppConfig};
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

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn default_contains_hook_related_values() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.gesture_threshold, AppConfig::DEFAULT_GESTURE_THRESHOLD);
        assert_eq!(cfg.safety_timeout_ms, AppConfig::DEFAULT_SAFETY_TIMEOUT_MS);
        assert_eq!(cfg.min_segment_px, AppConfig::DEFAULT_MIN_SEGMENT_PX);
        assert_eq!(
            cfg.direction_switch_confirm_px,
            AppConfig::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX
        );
        assert_eq!(
            cfg.axis_ambiguity_deadzone_px,
            AppConfig::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX
        );
        assert_eq!(cfg.bindings.len(), 6);
        assert!(cfg.bindings.contains_key("Left"));
        assert!(cfg.bindings.contains_key("Right"));
        assert!(cfg.bindings.contains_key("Up"));
        assert!(cfg.bindings.contains_key("Down"));
        assert!(cfg.bindings.contains_key("DownUp"));
        assert!(cfg.bindings.contains_key("UpDown"));
    }

    #[test]
    fn deserialize_legacy_json_fills_new_fields_from_defaults() {
        let raw = r##"{
            "gesture_trigger_button": "middle",
            "trail_color": "#ffffff",
            "trail_thickness": 5.0
        }"##;

        let cfg: AppConfig = serde_json::from_str(raw).expect("legacy JSON must deserialize");
        assert_eq!(cfg.gesture_trigger_button, "middle");
        assert_eq!(cfg.trail_color, "#ffffff");
        assert_eq!(cfg.trail_thickness, 5.0);
        assert_eq!(cfg.gesture_threshold, AppConfig::DEFAULT_GESTURE_THRESHOLD);
        assert_eq!(cfg.safety_timeout_ms, AppConfig::DEFAULT_SAFETY_TIMEOUT_MS);
        assert_eq!(cfg.min_segment_px, AppConfig::DEFAULT_MIN_SEGMENT_PX);
        assert_eq!(
            cfg.direction_switch_confirm_px,
            AppConfig::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX
        );
        assert_eq!(
            cfg.axis_ambiguity_deadzone_px,
            AppConfig::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX
        );
        assert_eq!(cfg.bindings.len(), 6);
        assert!(cfg.bindings.contains_key("Left"));
        assert!(cfg.bindings.contains_key("Right"));
        assert!(cfg.bindings.contains_key("Up"));
        assert!(cfg.bindings.contains_key("Down"));
        assert!(cfg.bindings.contains_key("DownUp"));
        assert!(cfg.bindings.contains_key("UpDown"));
    }

    #[test]
    fn deserialize_config_with_bindings() {
        let raw = r##"{
            "gesture_trigger_button": "right",
            "trail_color": "#00BFFF",
            "trail_thickness": 3.0,
            "bindings": {
                "Left": { "type": "keyboard", "keys": ["alt", "left"] },
                "Right": { "type": "keyboard", "keys": ["alt", "right"] },
                "Down": { "type": "keyboard", "keys": ["ctrl", "w"] }
            }
        }"##;

        let cfg: AppConfig = serde_json::from_str(raw).expect("config with bindings must parse");
        assert_eq!(cfg.bindings.len(), 3);
        assert!(cfg.bindings.contains_key("Left"));
        assert!(cfg.bindings.contains_key("Right"));
        assert!(cfg.bindings.contains_key("Down"));
    }

    #[test]
    fn deserialize_legacy_json_gets_default_bindings() {
        let raw = r##"{ "gesture_trigger_button": "right" }"##;
        let cfg: AppConfig = serde_json::from_str(raw).expect("legacy JSON must parse");
        assert_eq!(cfg.bindings.len(), 6);
        assert!(cfg.bindings.contains_key("Left"));
        assert!(cfg.bindings.contains_key("Right"));
        assert!(cfg.bindings.contains_key("Up"));
        assert!(cfg.bindings.contains_key("Down"));
        assert!(cfg.bindings.contains_key("DownUp"));
        assert!(cfg.bindings.contains_key("UpDown"));
    }
}
