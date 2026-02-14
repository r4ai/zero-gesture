use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::executor::Action;

/// What property of the foreground window to inspect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchTarget {
    /// Executable file name (e.g., "chrome.exe").
    ProcessName,
    /// Win32 window class name (e.g., "CabinetWClass").
    WindowClass,
    /// Window title text.
    Title,
}

/// How to compare the target value against the pattern.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchMethod {
    /// Exact match (case-insensitive for process_name/title, case-sensitive for window_class).
    Exact,
    /// Substring match (case-insensitive).
    Contains,
    /// Regex pattern match.
    Regex,
}

/// A single matching rule for identifying an application.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppMatcher {
    /// What property of the foreground window to inspect.
    pub target: MatchTarget,
    /// How to compare the target value against the pattern.
    pub method: MatchMethod,
    /// The pattern to match against.
    pub value: String,
}

/// Definition of an application for per-app gesture bindings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppDefinition {
    /// Matching rules (OR logic — any match counts).
    pub matchers: Vec<AppMatcher>,
}

/// A gesture-to-action binding with an optional human-readable label.
///
/// When serialized, the `action` fields are flattened into the same JSON object
/// so that existing configs (`{ "type": "keyboard", "keys": [...] }`) remain
/// compatible. An optional `label` field can be added for display purposes.
///
/// # Examples
///
/// ```
/// use zero_gesture_lib::config::GestureBinding;
/// use zero_gesture_lib::executor::Action;
///
/// let binding = GestureBinding {
///     action: Action::Keyboard { keys: vec!["alt".into(), "left".into()] },
///     label: Some("Back".into()),
/// };
/// let json = serde_json::to_string(&binding).unwrap();
/// assert!(json.contains("\"label\":\"Back\""));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GestureBinding {
    #[serde(flatten)]
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Configuration file name.
const CONFIG_FILE_NAME: &str = "zero-gesture.config.json";

/// Application-wide configuration persisted as JSON.
///
/// # Examples
///
/// ```
/// use zero_gesture_lib::config::AppConfig;
///
/// let config = AppConfig::default();
/// assert!(config.enabled);
/// assert_eq!(config.gesture_trigger_button, "right");
/// assert_eq!(config.trail_color, "#00BFFF");
/// assert_eq!(config.trail_thickness, 3.0);
/// assert_eq!(config.gesture_threshold, 10);
/// assert_eq!(config.safety_timeout_ms, 2000);
/// assert_eq!(config.min_segment_px, 12);
/// assert_eq!(config.direction_switch_confirm_px, 8);
/// assert_eq!(config.axis_ambiguity_deadzone_px, 2);
/// assert_eq!(config.label_font_family, "Yu Gothic UI Semibold");
/// assert_eq!(config.label_font_size, 36.0);
/// assert_eq!(config.label_font_weight, 400);
/// assert_eq!(config.label_padding, 24.0);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    /// Whether gesture recognition is enabled.
    ///
    /// When `false`, worker threads (hook/overlay) are not started.
    pub enabled: bool,

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

    /// Font family name for the gesture label overlay (e.g. `"Yu Gothic UI Semibold"`).
    pub label_font_family: String,

    /// Font size in pixels for the gesture label overlay.
    pub label_font_size: f32,

    /// Font weight for the gesture label overlay (Win32 range: 0..=1000).
    pub label_font_weight: i32,

    /// Padding in pixels around the gesture label text.
    pub label_padding: f32,

    /// Named app definitions for per-app gesture bindings.
    ///
    /// Each app has a list of matchers (OR logic — any match counts).
    #[serde(default)]
    pub apps: HashMap<String, AppDefinition>,

    /// Gesture-to-action bindings, keyed by app ID.
    ///
    /// The `"default"` key contains the global fallback bindings. Other keys
    /// reference app IDs defined in `apps`. When a matched app has no binding
    /// for a gesture, the `"default"` bindings are used as fallback.
    pub bindings: HashMap<String, HashMap<String, GestureBinding>>,
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

    /// Default font family for the gesture label overlay.
    pub const DEFAULT_LABEL_FONT_FAMILY: &str = "Yu Gothic UI Semibold";

    /// Default font size (in pixels) for the gesture label overlay.
    pub const DEFAULT_LABEL_FONT_SIZE: f32 = 36.0;

    /// Default font weight for the gesture label overlay.
    pub const DEFAULT_LABEL_FONT_WEIGHT: i32 = 400;

    /// Default padding (in pixels) around the gesture label text.
    pub const DEFAULT_LABEL_PADDING: f32 = 24.0;

    /// Default gesture-to-action bindings (placed under `"default"` key).
    fn default_bindings() -> HashMap<String, HashMap<String, GestureBinding>> {
        let default_map = HashMap::from([
            (
                "Left".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["alt".to_string(), "left".to_string()],
                    },
                    label: Some("Back".to_string()),
                },
            ),
            (
                "Right".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["alt".to_string(), "right".to_string()],
                    },
                    label: Some("Forward".to_string()),
                },
            ),
            (
                "Up".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["pageup".to_string()],
                    },
                    label: Some("Scroll Up".to_string()),
                },
            ),
            (
                "Down".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["pagedown".to_string()],
                    },
                    label: Some("Scroll Down".to_string()),
                },
            ),
            (
                "DownUp".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["ctrl".to_string(), "home".to_string()],
                    },
                    label: Some("Top of Page".to_string()),
                },
            ),
            (
                "UpDown".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["ctrl".to_string(), "end".to_string()],
                    },
                    label: Some("Bottom of Page".to_string()),
                },
            ),
            (
                "UpRight".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["ctrl".to_string(), "tab".to_string()],
                    },
                    label: Some("Next Tab".to_string()),
                },
            ),
            (
                "UpLeft".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["ctrl".to_string(), "shift".to_string(), "tab".to_string()],
                    },
                    label: Some("Previous Tab".to_string()),
                },
            ),
            (
                "RightDown".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["ctrl".to_string(), "r".to_string()],
                    },
                    label: Some("Reload".to_string()),
                },
            ),
            (
                "DownRight".to_string(),
                GestureBinding {
                    action: Action::Keyboard {
                        keys: vec!["ctrl".to_string(), "w".to_string()],
                    },
                    label: Some("Close Tab".to_string()),
                },
            ),
        ]);
        HashMap::from([("default".to_string(), default_map)])
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gesture_trigger_button: "right".to_string(),
            trail_color: "#00BFFF".to_string(),
            trail_thickness: 3.0,
            gesture_threshold: Self::DEFAULT_GESTURE_THRESHOLD,
            safety_timeout_ms: Self::DEFAULT_SAFETY_TIMEOUT_MS,
            min_segment_px: Self::DEFAULT_MIN_SEGMENT_PX,
            direction_switch_confirm_px: Self::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX,
            axis_ambiguity_deadzone_px: Self::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX,
            label_font_family: Self::DEFAULT_LABEL_FONT_FAMILY.to_string(),
            label_font_size: Self::DEFAULT_LABEL_FONT_SIZE,
            label_font_weight: Self::DEFAULT_LABEL_FONT_WEIGHT,
            label_padding: Self::DEFAULT_LABEL_PADDING,
            apps: HashMap::new(),
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
/// use std::path::Path;
///
/// let config = load_or_default(Path::new("./config"));
/// println!("trigger button: {}", config.gesture_trigger_button);
/// ```
pub fn load_or_default(config_dir: &Path) -> AppConfig {
    let raw = match fs::read_to_string(config_path(config_dir)) {
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
/// use std::path::Path;
///
/// let config = AppConfig::default();
/// save(&config, Path::new("./config")).expect("failed to save config");
/// ```
pub fn save(config: &AppConfig, config_dir: &Path) -> io::Result<()> {
    let body = serde_json::to_string_pretty(config)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::create_dir_all(config_dir)?;
    fs::write(config_path(config_dir), body)
}

/// Returns the path to the configuration file.
fn config_path(config_dir: &Path) -> PathBuf {
    config_dir.join(CONFIG_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to get the "default" bindings map from config.
    fn get_default_bindings(cfg: &AppConfig) -> &HashMap<String, GestureBinding> {
        cfg.bindings
            .get("default")
            .expect("default bindings must exist")
    }

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
        assert_eq!(cfg.label_font_family, AppConfig::DEFAULT_LABEL_FONT_FAMILY);
        assert_eq!(cfg.label_font_size, AppConfig::DEFAULT_LABEL_FONT_SIZE);
        assert_eq!(cfg.label_font_weight, AppConfig::DEFAULT_LABEL_FONT_WEIGHT);
        assert_eq!(cfg.label_padding, AppConfig::DEFAULT_LABEL_PADDING);
        assert!(cfg.apps.is_empty());
        let bindings = get_default_bindings(&cfg);
        assert_eq!(bindings.len(), 10);
        assert!(bindings.contains_key("Left"));
        assert!(bindings.contains_key("Right"));
        assert!(bindings.contains_key("Up"));
        assert!(bindings.contains_key("Down"));
        assert!(bindings.contains_key("DownUp"));
        assert!(bindings.contains_key("UpDown"));
        assert!(bindings.contains_key("UpRight"));
        assert!(bindings.contains_key("UpLeft"));
        assert!(bindings.contains_key("RightDown"));
        assert!(bindings.contains_key("DownRight"));
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
        assert_eq!(cfg.label_font_family, AppConfig::DEFAULT_LABEL_FONT_FAMILY);
        assert_eq!(cfg.label_font_size, AppConfig::DEFAULT_LABEL_FONT_SIZE);
        assert_eq!(cfg.label_font_weight, AppConfig::DEFAULT_LABEL_FONT_WEIGHT);
        assert_eq!(cfg.label_padding, AppConfig::DEFAULT_LABEL_PADDING);
        let bindings = get_default_bindings(&cfg);
        assert_eq!(bindings.len(), 10);
        assert!(bindings.contains_key("Left"));
        assert!(bindings.contains_key("Right"));
        assert!(bindings.contains_key("Up"));
        assert!(bindings.contains_key("Down"));
        assert!(bindings.contains_key("DownUp"));
        assert!(bindings.contains_key("UpDown"));
        assert!(bindings.contains_key("UpRight"));
        assert!(bindings.contains_key("UpLeft"));
        assert!(bindings.contains_key("RightDown"));
        assert!(bindings.contains_key("DownRight"));
    }

    #[test]
    fn deserialize_config_with_bindings() {
        let raw = r##"{
            "gesture_trigger_button": "right",
            "trail_color": "#00BFFF",
            "trail_thickness": 3.0,
            "bindings": {
                "default": {
                    "Left": { "type": "keyboard", "keys": ["alt", "left"] },
                    "Right": { "type": "keyboard", "keys": ["alt", "right"], "label": "Forward" },
                    "Down": { "type": "keyboard", "keys": ["ctrl", "w"] }
                }
            }
        }"##;

        let cfg: AppConfig = serde_json::from_str(raw).expect("config with bindings must parse");
        let bindings = get_default_bindings(&cfg);
        assert_eq!(bindings.len(), 3);
        assert!(bindings.contains_key("Left"));
        assert!(bindings.contains_key("Right"));
        assert!(bindings.contains_key("Down"));
        // Without label
        assert_eq!(bindings["Left"].label, None);
        // With label
        assert_eq!(bindings["Right"].label, Some("Forward".to_string()));
    }

    #[test]
    fn deserialize_json_with_label_font_weight() {
        let raw = r##"{ "label_font_weight": 700 }"##;
        let cfg: AppConfig =
            serde_json::from_str(raw).expect("JSON with label_font_weight must parse");
        assert_eq!(cfg.label_font_weight, 700);
    }

    #[test]
    fn deserialize_json_with_enabled_false() {
        let raw = r##"{ "gesture_trigger_button": "right", "enabled": false }"##;
        let cfg: AppConfig = serde_json::from_str(raw).expect("JSON with enabled=false must parse");
        assert!(!cfg.enabled);
    }

    #[test]
    fn deserialize_legacy_json_defaults_enabled_to_true() {
        let raw = r##"{ "gesture_trigger_button": "right" }"##;
        let cfg: AppConfig = serde_json::from_str(raw).expect("legacy JSON must parse");
        assert!(cfg.enabled);
    }

    #[test]
    fn deserialize_legacy_json_gets_default_bindings() {
        let raw = r##"{ "gesture_trigger_button": "right" }"##;
        let cfg: AppConfig = serde_json::from_str(raw).expect("legacy JSON must parse");
        let bindings = get_default_bindings(&cfg);
        assert_eq!(bindings.len(), 10);
        assert!(bindings.contains_key("Left"));
        assert!(bindings.contains_key("Right"));
        assert!(bindings.contains_key("Up"));
        assert!(bindings.contains_key("Down"));
        assert!(bindings.contains_key("DownUp"));
        assert!(bindings.contains_key("UpDown"));
        assert!(bindings.contains_key("UpRight"));
        assert!(bindings.contains_key("UpLeft"));
        assert!(bindings.contains_key("RightDown"));
        assert!(bindings.contains_key("DownRight"));
    }

    #[test]
    fn deserialize_config_with_apps_and_per_app_bindings() {
        let raw = r##"{
            "apps": {
                "browser": {
                    "matchers": [
                        { "target": "process_name", "method": "exact", "value": "chrome.exe" },
                        { "target": "process_name", "method": "exact", "value": "firefox.exe" }
                    ]
                },
                "explorer": {
                    "matchers": [
                        { "target": "window_class", "method": "exact", "value": "CabinetWClass" }
                    ]
                }
            },
            "bindings": {
                "default": {
                    "Left": { "type": "keyboard", "keys": ["alt", "left"], "label": "Back" }
                },
                "browser": {
                    "Left": { "type": "keyboard", "keys": ["alt", "left"], "label": "Back" }
                },
                "explorer": {
                    "Left": { "type": "keyboard", "keys": ["alt", "up"], "label": "Up" }
                }
            }
        }"##;

        let cfg: AppConfig = serde_json::from_str(raw).expect("per-app config must parse");
        assert_eq!(cfg.apps.len(), 2);
        assert!(cfg.apps.contains_key("browser"));
        assert!(cfg.apps.contains_key("explorer"));
        assert_eq!(cfg.apps["browser"].matchers.len(), 2);
        assert_eq!(cfg.apps["explorer"].matchers.len(), 1);
        assert_eq!(cfg.bindings.len(), 3);
        assert!(cfg.bindings.contains_key("default"));
        assert!(cfg.bindings.contains_key("browser"));
        assert!(cfg.bindings.contains_key("explorer"));
    }

    #[test]
    fn deserialize_matcher_types() {
        let raw = r##"{
            "apps": {
                "terminals": {
                    "matchers": [
                        { "target": "process_name", "method": "regex", "value": "^(WindowsTerminal|cmd)\\.exe$" },
                        { "target": "title", "method": "contains", "value": "Terminal" },
                        { "target": "window_class", "method": "exact", "value": "CASCADIA_HOSTING_WINDOW_CLASS" }
                    ]
                }
            }
        }"##;

        let cfg: AppConfig = serde_json::from_str(raw).expect("matcher types must parse");
        let matchers = &cfg.apps["terminals"].matchers;
        assert_eq!(matchers[0].target, super::MatchTarget::ProcessName);
        assert_eq!(matchers[0].method, super::MatchMethod::Regex);
        assert_eq!(matchers[1].target, super::MatchTarget::Title);
        assert_eq!(matchers[1].method, super::MatchMethod::Contains);
        assert_eq!(matchers[2].target, super::MatchTarget::WindowClass);
        assert_eq!(matchers[2].method, super::MatchMethod::Exact);
    }

    #[test]
    fn save_creates_directory_and_roundtrips_from_config_dir() {
        let temp_dir =
            tempfile::tempdir().expect("must be able to create temp dir for config test");
        let temp_path = temp_dir.path();

        let expected = AppConfig {
            gesture_trigger_button: "middle".to_string(),
            ..AppConfig::default()
        };

        save(&expected, temp_path).expect("save must succeed");
        let loaded = load_or_default(temp_path);
        assert_eq!(loaded, expected);
    }
}
