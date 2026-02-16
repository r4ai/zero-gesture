use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use log::warn;
use serde::{Deserialize, Serialize};

use crate::executor::Action;

/// What property of the foreground window to inspect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchTarget {
    /// Executable file name (e.g., `"chrome.exe"`).
    ProcessName,
    /// Win32 window class name (e.g., `"CabinetWClass"`).
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

/// Mouse button that starts a gesture session.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TriggerButton {
    LeftClick,
    RightClick,
    MiddleClick,
}

/// One element inside a gesture sequence.
///
/// A gesture sequence can combine directional movement and mouse inputs.
/// Maximum length is enforced by [`AppConfig::validate`] (`8` elements).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum GestureStep {
    Up,
    Down,
    Left,
    Right,
    WheelUp,
    WheelDown,
    LeftClick,
    RightClick,
    MiddleClick,
}

/// Timing mode for a gesture binding.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum GestureMode {
    /// Execute action when trigger button is released and sequence matches.
    #[default]
    Release,
    /// Execute action immediately while trigger button is held.
    Hold,
}

/// Gesture pattern definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GesturePattern {
    /// Button that starts this gesture.
    pub trigger: TriggerButton,
    /// Whether this gesture runs on trigger release or while holding trigger.
    #[serde(default)]
    pub mode: GestureMode,
    /// Ordered sequence of movement/input steps.
    ///
    /// - `release` mode: the full sequence to match on trigger release.
    /// - `hold` mode: current recognized sequence required before `step` fires.
    #[serde(default)]
    pub sequence: Vec<GestureStep>,
    /// Single non-movement input step for `hold` mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<GestureStep>,
}

/// A single gesture binding.
///
/// # Examples
///
/// ```json
/// {
///   "label": "Reload",
///   "gesture": {
///     "trigger": "right_click",
///     "mode": "release",
///     "sequence": ["right", "down"]
///   },
///   "action": {
///     "type": "keyboard",
///     "keys": ["ctrl", "r"]
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GestureBinding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Gesture pattern to match.
    pub gesture: GesturePattern,
    /// Action to execute when the gesture matches.
    pub action: Action,
}

fn has_consecutive_same_move_steps(sequence: &[GestureStep]) -> bool {
    sequence.windows(2).any(|pair| {
        pair[0] == pair[1]
            && matches!(
                pair[0],
                GestureStep::Up | GestureStep::Down | GestureStep::Left | GestureStep::Right
            )
    })
}

fn trigger_to_step(trigger: TriggerButton) -> GestureStep {
    match trigger {
        TriggerButton::LeftClick => GestureStep::LeftClick,
        TriggerButton::RightClick => GestureStep::RightClick,
        TriggerButton::MiddleClick => GestureStep::MiddleClick,
    }
}

fn is_supported_hold_step(step: GestureStep) -> bool {
    matches!(step, GestureStep::WheelUp | GestureStep::WheelDown)
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
/// assert_eq!(config.trail_color, "#00BFFF");
/// assert_eq!(config.min_segment_px, 12);
/// assert!(config.bindings.contains_key("default"));
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    /// Whether gesture recognition is enabled.
    ///
    /// When `false`, worker threads (hook/overlay) are not started.
    pub enabled: bool,

    /// CSS colour string used to draw the gesture trail (e.g. `"#00BFFF"`).
    pub trail_color: String,

    /// Thickness in logical pixels for the gesture trail line.
    pub trail_thickness: f32,

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

    /// Maximum cursor travel distance (in pixels) to replay the original
    /// trigger-button click when no gesture binding matches.
    ///
    /// If movement exceeds this threshold, replay is skipped.
    pub replay_distance_threshold_px: i32,

    /// Font family name for the gesture label overlay.
    pub label_font_family: String,

    /// Font size in pixels for the gesture label overlay.
    pub label_font_size: f32,

    /// Font weight for the gesture label overlay (Win32 range: 0..=1000).
    pub label_font_weight: i32,

    /// Padding in pixels around the gesture label text.
    pub label_padding: f32,

    /// Named app definitions for per-app gesture bindings.
    #[serde(default)]
    pub apps: HashMap<String, AppDefinition>,

    /// Gesture bindings grouped by app ID.
    ///
    /// - `"default"` is the global fallback set.
    /// - other keys reference entries in [`Self::apps`].
    pub bindings: HashMap<String, Vec<GestureBinding>>,
}

impl AppConfig {
    /// Hard maximum number of steps inside one gesture sequence.
    pub const MAX_GESTURE_STEPS: usize = 8;

    /// Default timeout used by the safety timer.
    pub const DEFAULT_SAFETY_TIMEOUT_MS: u32 = 2000;

    /// Default minimum segment distance for gesture direction confirmation.
    pub const DEFAULT_MIN_SEGMENT_PX: i32 = 12;

    /// Default hysteresis distance for direction switching.
    pub const DEFAULT_DIRECTION_SWITCH_CONFIRM_PX: i32 = 8;

    /// Default deadzone for tiny ambiguous diagonal movement.
    pub const DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX: i32 = 2;

    /// Default cursor travel threshold for replaying unmatched trigger clicks.
    pub const DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX: i32 = 12;

    /// Default font family for the gesture label overlay.
    pub const DEFAULT_LABEL_FONT_FAMILY: &str = "Yu Gothic UI Semibold";

    /// Default font size (in pixels) for the gesture label overlay.
    pub const DEFAULT_LABEL_FONT_SIZE: f32 = 36.0;

    /// Default font weight for the gesture label overlay.
    pub const DEFAULT_LABEL_FONT_WEIGHT: i32 = 400;

    /// Default padding (in pixels) around the gesture label overlay text.
    pub const DEFAULT_LABEL_PADDING: f32 = 24.0;

    /// Reserved app ID for global fallback bindings.
    pub const DEFAULT_APP_ID: &str = "default";

    /// Validates and normalizes configuration values in-place.
    ///
    /// Invalid values are replaced with safe defaults and unsupported gesture
    /// bindings are dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use zero_gesture_lib::config::AppConfig;
    ///
    /// let mut cfg = AppConfig::default();
    /// cfg.min_segment_px = 0;
    /// cfg.validate();
    /// assert_eq!(cfg.min_segment_px, AppConfig::DEFAULT_MIN_SEGMENT_PX);
    /// ```
    pub fn validate(&mut self) {
        if self.safety_timeout_ms == 0 {
            warn!(
                "Invalid safety_timeout_ms={} in config, falling back to {}",
                self.safety_timeout_ms,
                Self::DEFAULT_SAFETY_TIMEOUT_MS
            );
            self.safety_timeout_ms = Self::DEFAULT_SAFETY_TIMEOUT_MS;
        }
        if self.min_segment_px <= 0 {
            warn!(
                "Invalid min_segment_px={} in config, falling back to {}",
                self.min_segment_px,
                Self::DEFAULT_MIN_SEGMENT_PX
            );
            self.min_segment_px = Self::DEFAULT_MIN_SEGMENT_PX;
        }
        if self.direction_switch_confirm_px <= 0 {
            warn!(
                "Invalid direction_switch_confirm_px={} in config, falling back to {}",
                self.direction_switch_confirm_px,
                Self::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX
            );
            self.direction_switch_confirm_px = Self::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX;
        }
        if self.axis_ambiguity_deadzone_px < 0 {
            warn!(
                "Invalid axis_ambiguity_deadzone_px={} in config, falling back to {}",
                self.axis_ambiguity_deadzone_px,
                Self::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX
            );
            self.axis_ambiguity_deadzone_px = Self::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX;
        }
        if self.replay_distance_threshold_px <= 0 {
            warn!(
                "Invalid replay_distance_threshold_px={} in config, falling back to {}",
                self.replay_distance_threshold_px,
                Self::DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX
            );
            self.replay_distance_threshold_px = Self::DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX;
        }

        let mut validated_bindings: HashMap<String, Vec<GestureBinding>> =
            HashMap::with_capacity(self.bindings.len().max(1));
        for (app_id, app_bindings) in &self.bindings {
            if app_id != Self::DEFAULT_APP_ID && !self.apps.contains_key(app_id) {
                warn!(
                    "Bindings reference app {:?} which is not defined in apps, skipping",
                    app_id
                );
                continue;
            }
            let normalized = Self::validate_bindings_for_app(app_id, app_bindings);
            validated_bindings.insert(app_id.clone(), normalized);
        }

        if !validated_bindings.contains_key(Self::DEFAULT_APP_ID) {
            warn!(
                "No \"{}\" bindings defined in configuration; inserting empty default set",
                Self::DEFAULT_APP_ID
            );
            validated_bindings.insert(Self::DEFAULT_APP_ID.to_string(), Vec::new());
        }
        self.bindings = validated_bindings;
    }

    /// Returns a validated copy of this configuration.
    pub fn validated(mut self) -> Self {
        self.validate();
        self
    }

    fn validate_bindings_for_app(
        app_id: &str,
        app_bindings: &[GestureBinding],
    ) -> Vec<GestureBinding> {
        let mut validated: Vec<GestureBinding> = Vec::new();
        let mut seen_release: HashSet<(TriggerButton, Vec<GestureStep>)> = HashSet::new();
        let mut seen_hold: HashSet<(TriggerButton, Vec<GestureStep>, GestureStep)> = HashSet::new();

        for binding in app_bindings {
            match binding.gesture.mode {
                GestureMode::Release => {
                    if binding.gesture.sequence.is_empty() {
                        warn!(
                            "Empty release gesture sequence in bindings for app {:?}, skipping",
                            app_id
                        );
                        continue;
                    }
                    if binding.gesture.sequence.len() > Self::MAX_GESTURE_STEPS {
                        warn!(
                            "Release gesture sequence too long ({} > {}) in app {:?}, skipping",
                            binding.gesture.sequence.len(),
                            Self::MAX_GESTURE_STEPS,
                            app_id
                        );
                        continue;
                    }
                    if has_consecutive_same_move_steps(&binding.gesture.sequence) {
                        warn!(
                            "Release gesture sequence contains consecutive identical directional moves {:?} in app {:?}, skipping",
                            binding.gesture.sequence, app_id
                        );
                        continue;
                    }

                    let trigger_step = trigger_to_step(binding.gesture.trigger);
                    if binding.gesture.sequence.contains(&trigger_step) {
                        warn!(
                            "Release gesture sequence contains its own trigger step {:?} in app {:?}, skipping",
                            trigger_step, app_id
                        );
                        continue;
                    }

                    let sequence = binding.gesture.sequence.clone();
                    if !seen_release.insert((binding.gesture.trigger, sequence)) {
                        warn!(
                            "Duplicate release gesture binding for trigger={:?}, sequence={:?} in app {:?}, skipping",
                            binding.gesture.trigger, binding.gesture.sequence, app_id
                        );
                        continue;
                    }

                    let mut normalized = binding.clone();
                    if normalized.gesture.step.is_some() {
                        warn!(
                            "Release gesture includes hold-only `step` in app {:?}, dropping step",
                            app_id
                        );
                        normalized.gesture.step = None;
                    }
                    validated.push(normalized);
                }
                GestureMode::Hold => {
                    if binding.gesture.sequence.len() > Self::MAX_GESTURE_STEPS {
                        warn!(
                            "Hold gesture sequence too long ({} > {}) in app {:?}, skipping",
                            binding.gesture.sequence.len(),
                            Self::MAX_GESTURE_STEPS,
                            app_id
                        );
                        continue;
                    }
                    if has_consecutive_same_move_steps(&binding.gesture.sequence) {
                        warn!(
                            "Hold gesture sequence contains consecutive identical directional moves {:?} in app {:?}, skipping",
                            binding.gesture.sequence, app_id
                        );
                        continue;
                    }

                    let trigger_step = trigger_to_step(binding.gesture.trigger);
                    if binding.gesture.sequence.contains(&trigger_step) {
                        warn!(
                            "Hold gesture sequence contains its own trigger step {:?} in app {:?}, skipping",
                            trigger_step, app_id
                        );
                        continue;
                    }

                    let Some(step) = binding.gesture.step else {
                        warn!("Hold gesture is missing step in app {:?}, skipping", app_id);
                        continue;
                    };
                    if !is_supported_hold_step(step) {
                        warn!(
                            "Unsupported hold step {:?} in app {:?}, skipping",
                            step, app_id
                        );
                        continue;
                    }

                    let sequence = binding.gesture.sequence.clone();
                    if !seen_hold.insert((binding.gesture.trigger, sequence, step)) {
                        warn!(
                            "Duplicate hold gesture binding for trigger={:?}, sequence={:?}, step={:?} in app {:?}, skipping",
                            binding.gesture.trigger, binding.gesture.sequence, step, app_id
                        );
                        continue;
                    }

                    validated.push(binding.clone());
                }
            }
        }

        validated
    }

    /// Default gesture bindings (under `"default"`).
    fn default_bindings() -> HashMap<String, Vec<GestureBinding>> {
        let defaults = vec![
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Left],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["alt".to_string(), "left".to_string()],
                },
                label: Some("Back".to_string()),
            },
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Right],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["alt".to_string(), "right".to_string()],
                },
                label: Some("Forward".to_string()),
            },
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Up],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["pageup".to_string()],
                },
                label: Some("Scroll Up".to_string()),
            },
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Down],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["pagedown".to_string()],
                },
                label: Some("Scroll Down".to_string()),
            },
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Down, GestureStep::Up],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["ctrl".to_string(), "home".to_string()],
                },
                label: Some("Top of Page".to_string()),
            },
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Up, GestureStep::Down],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["ctrl".to_string(), "end".to_string()],
                },
                label: Some("Bottom of Page".to_string()),
            },
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Up, GestureStep::Right],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["ctrl".to_string(), "tab".to_string()],
                },
                label: Some("Next Tab".to_string()),
            },
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Up, GestureStep::Left],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["ctrl".to_string(), "shift".to_string(), "tab".to_string()],
                },
                label: Some("Previous Tab".to_string()),
            },
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Right, GestureStep::Down],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["ctrl".to_string(), "r".to_string()],
                },
                label: Some("Reload".to_string()),
            },
            GestureBinding {
                gesture: GesturePattern {
                    trigger: TriggerButton::RightClick,
                    mode: GestureMode::Release,
                    sequence: vec![GestureStep::Down, GestureStep::Right],
                    step: None,
                },
                action: Action::Keyboard {
                    keys: vec!["ctrl".to_string(), "w".to_string()],
                },
                label: Some("Close Tab".to_string()),
            },
        ];
        HashMap::from([(Self::DEFAULT_APP_ID.to_string(), defaults)])
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trail_color: "#00BFFF".to_string(),
            trail_thickness: 3.0,
            safety_timeout_ms: Self::DEFAULT_SAFETY_TIMEOUT_MS,
            min_segment_px: Self::DEFAULT_MIN_SEGMENT_PX,
            direction_switch_confirm_px: Self::DEFAULT_DIRECTION_SWITCH_CONFIRM_PX,
            axis_ambiguity_deadzone_px: Self::DEFAULT_AXIS_AMBIGUITY_DEADZONE_PX,
            replay_distance_threshold_px: Self::DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX,
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
/// use std::path::Path;
/// use zero_gesture_lib::config::load_or_default;
///
/// let config = load_or_default(Path::new("./config"));
/// assert!(config.bindings.contains_key("default"));
/// ```
pub fn load_or_default(config_dir: &Path) -> AppConfig {
    let raw = match fs::read_to_string(config_path(config_dir)) {
        Ok(raw) => raw,
        Err(_) => return AppConfig::default(),
    };

    let cfg: AppConfig = serde_json::from_str(&raw).unwrap_or_default();
    cfg.validated()
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
/// use std::path::Path;
/// use zero_gesture_lib::config::{save, AppConfig};
///
/// let config = AppConfig::default();
/// save(&config, Path::new("./config")).expect("failed to save config");
/// ```
pub fn save(config: &AppConfig, config_dir: &Path) -> io::Result<()> {
    let normalized = config.clone().validated();
    let body = serde_json::to_string_pretty(&normalized)
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

    /// Helper to get the `"default"` bindings map from config.
    fn get_default_bindings(cfg: &AppConfig) -> &Vec<GestureBinding> {
        cfg.bindings
            .get("default")
            .expect("default bindings must exist")
    }

    fn keyboard_action(key: &str) -> Action {
        Action::Keyboard {
            keys: vec![key.to_string()],
        }
    }

    fn release_binding(
        trigger: TriggerButton,
        sequence: Vec<GestureStep>,
        key: &str,
    ) -> GestureBinding {
        GestureBinding {
            label: None,
            gesture: GesturePattern {
                trigger,
                mode: GestureMode::Release,
                sequence,
                step: None,
            },
            action: keyboard_action(key),
        }
    }

    fn hold_binding(
        trigger: TriggerButton,
        sequence: Vec<GestureStep>,
        step: Option<GestureStep>,
        key: &str,
    ) -> GestureBinding {
        GestureBinding {
            label: None,
            gesture: GesturePattern {
                trigger,
                mode: GestureMode::Hold,
                sequence,
                step,
            },
            action: keyboard_action(key),
        }
    }

    #[test]
    fn default_contains_expected_values() {
        let cfg = AppConfig::default();
        assert!(cfg.enabled);
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
        assert_eq!(
            cfg.replay_distance_threshold_px,
            AppConfig::DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX
        );
        assert_eq!(cfg.label_font_family, AppConfig::DEFAULT_LABEL_FONT_FAMILY);
        assert_eq!(cfg.label_font_size, AppConfig::DEFAULT_LABEL_FONT_SIZE);
        assert_eq!(cfg.label_font_weight, AppConfig::DEFAULT_LABEL_FONT_WEIGHT);
        assert_eq!(cfg.label_padding, AppConfig::DEFAULT_LABEL_PADDING);

        let defaults = get_default_bindings(&cfg);
        assert!(!defaults.is_empty());
        assert!(defaults.iter().all(|binding| {
            !binding.gesture.sequence.is_empty()
                && binding.gesture.sequence.len() <= AppConfig::MAX_GESTURE_STEPS
        }));
    }

    #[test]
    fn default_bindings_match_legacy_defaults() {
        let cfg = AppConfig::default();
        let defaults = get_default_bindings(&cfg);
        assert_eq!(defaults.len(), 10);

        let expected: Vec<(Vec<GestureStep>, Vec<&str>, &str)> = vec![
            (vec![GestureStep::Left], vec!["alt", "left"], "Back"),
            (vec![GestureStep::Right], vec!["alt", "right"], "Forward"),
            (vec![GestureStep::Up], vec!["pageup"], "Scroll Up"),
            (vec![GestureStep::Down], vec!["pagedown"], "Scroll Down"),
            (
                vec![GestureStep::Down, GestureStep::Up],
                vec!["ctrl", "home"],
                "Top of Page",
            ),
            (
                vec![GestureStep::Up, GestureStep::Down],
                vec!["ctrl", "end"],
                "Bottom of Page",
            ),
            (
                vec![GestureStep::Up, GestureStep::Right],
                vec!["ctrl", "tab"],
                "Next Tab",
            ),
            (
                vec![GestureStep::Up, GestureStep::Left],
                vec!["ctrl", "shift", "tab"],
                "Previous Tab",
            ),
            (
                vec![GestureStep::Right, GestureStep::Down],
                vec!["ctrl", "r"],
                "Reload",
            ),
            (
                vec![GestureStep::Down, GestureStep::Right],
                vec!["ctrl", "w"],
                "Close Tab",
            ),
        ];

        for (sequence, keys, label) in expected {
            let binding = defaults
                .iter()
                .find(|binding| binding.gesture.sequence == sequence)
                .expect("expected sequence must exist in default bindings");
            assert_eq!(binding.gesture.trigger, TriggerButton::RightClick);
            assert_eq!(binding.label.as_deref(), Some(label));
            let expected_keys = keys
                .into_iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            let Action::Keyboard { keys: actual } = &binding.action;
            assert_eq!(actual, &expected_keys);
        }
    }

    #[test]
    fn deserialize_config_with_sequence_bindings() {
        let raw = r##"{
            "bindings": {
                "default": [
                    {
                        "label": "Reload",
                        "gesture": {
                            "trigger": "right_click",
                            "sequence": ["right", "down"]
                        },
                        "action": {
                            "type": "keyboard",
                            "keys": ["ctrl", "r"]
                        }
                    },
                    {
                        "gesture": {
                            "trigger": "middle_click",
                            "sequence": ["wheel_up"]
                        },
                        "action": {
                            "type": "keyboard",
                            "keys": ["pageup"]
                        }
                    }
                ]
            }
        }"##;

        let cfg: AppConfig = serde_json::from_str(raw).expect("config with bindings must parse");
        let defaults = get_default_bindings(&cfg);
        assert_eq!(defaults.len(), 2);
        assert_eq!(defaults[0].gesture.trigger, TriggerButton::RightClick);
        assert_eq!(
            defaults[0].gesture.sequence,
            vec![GestureStep::Right, GestureStep::Down]
        );
        assert_eq!(defaults[0].label, Some("Reload".to_string()));
        assert_eq!(defaults[1].label, None);
    }

    #[test]
    fn deserialize_config_with_hold_binding() {
        let raw = r##"{
            "bindings": {
                "default": [
                    {
                        "label": "Scroll Up While Hold",
                        "gesture": {
                            "trigger": "right_click",
                            "mode": "hold",
                            "step": "wheel_up"
                        },
                        "action": {
                            "type": "keyboard",
                            "keys": ["pageup"]
                        }
                    }
                ]
            }
        }"##;

        let cfg: AppConfig = serde_json::from_str(raw).expect("hold binding config must parse");
        let defaults = get_default_bindings(&cfg);
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].gesture.mode, GestureMode::Hold);
        assert_eq!(defaults[0].gesture.step, Some(GestureStep::WheelUp));
        assert!(defaults[0].gesture.sequence.is_empty());
    }

    #[test]
    fn deserialize_config_with_sequence_scoped_hold_binding() {
        let raw = r##"{
            "bindings": {
                "default": [
                    {
                        "label": "Right then WheelDown",
                        "gesture": {
                            "trigger": "right_click",
                            "mode": "hold",
                            "sequence": ["right"],
                            "step": "wheel_down"
                        },
                        "action": {
                            "type": "keyboard",
                            "keys": ["pagedown"]
                        }
                    }
                ]
            }
        }"##;

        let cfg: AppConfig =
            serde_json::from_str(raw).expect("sequence-scoped hold binding must parse");
        let defaults = get_default_bindings(&cfg);
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].gesture.mode, GestureMode::Hold);
        assert_eq!(defaults[0].gesture.sequence, vec![GestureStep::Right]);
        assert_eq!(defaults[0].gesture.step, Some(GestureStep::WheelDown));
    }

    #[test]
    fn deserialize_config_with_apps_and_per_app_bindings() {
        let raw = r##"{
            "apps": {
                "browser": {
                    "matchers": [
                        { "target": "process_name", "method": "exact", "value": "chrome.exe" }
                    ]
                }
            },
            "bindings": {
                "default": [
                    {
                        "gesture": {
                            "trigger": "right_click",
                            "sequence": ["left"]
                        },
                        "action": {
                            "type": "keyboard",
                            "keys": ["alt", "left"]
                        }
                    }
                ],
                "browser": [
                    {
                        "label": "Reload",
                        "gesture": {
                            "trigger": "right_click",
                            "sequence": ["right", "down"]
                        },
                        "action": {
                            "type": "keyboard",
                            "keys": ["ctrl", "r"]
                        }
                    }
                ]
            }
        }"##;

        let cfg: AppConfig = serde_json::from_str(raw).expect("per-app config must parse");
        assert_eq!(cfg.apps.len(), 1);
        assert_eq!(cfg.bindings.len(), 2);
        assert_eq!(cfg.bindings["browser"].len(), 1);
    }

    #[test]
    fn deserialize_json_with_enabled_false() {
        let raw = r##"{ "enabled": false }"##;
        let cfg: AppConfig = serde_json::from_str(raw).expect("JSON with enabled=false must parse");
        assert!(!cfg.enabled);
        assert!(cfg.bindings.contains_key("default"));
    }

    #[test]
    fn validate_normalizes_numeric_thresholds() {
        let mut cfg = AppConfig {
            safety_timeout_ms: 0,
            min_segment_px: 0,
            direction_switch_confirm_px: -1,
            axis_ambiguity_deadzone_px: -1,
            replay_distance_threshold_px: 0,
            ..AppConfig::default()
        };

        cfg.validate();

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
        assert_eq!(
            cfg.replay_distance_threshold_px,
            AppConfig::DEFAULT_REPLAY_DISTANCE_THRESHOLD_PX
        );
    }

    #[test]
    fn validate_inserts_empty_default_bindings_when_missing() {
        let mut cfg = AppConfig {
            bindings: HashMap::new(),
            ..AppConfig::default()
        };

        cfg.validate();

        assert!(cfg.bindings.contains_key(AppConfig::DEFAULT_APP_ID));
        assert_eq!(cfg.bindings[AppConfig::DEFAULT_APP_ID].len(), 0);
    }

    #[test]
    fn validate_removes_bindings_for_unknown_apps() {
        let mut cfg = AppConfig {
            bindings: HashMap::from([
                (
                    AppConfig::DEFAULT_APP_ID.to_string(),
                    vec![release_binding(
                        TriggerButton::RightClick,
                        vec![GestureStep::Left],
                        "a",
                    )],
                ),
                (
                    "missing-app".to_string(),
                    vec![release_binding(
                        TriggerButton::RightClick,
                        vec![GestureStep::Right],
                        "b",
                    )],
                ),
            ]),
            ..AppConfig::default()
        };

        cfg.validate();

        assert!(cfg.bindings.contains_key(AppConfig::DEFAULT_APP_ID));
        assert!(!cfg.bindings.contains_key("missing-app"));
    }

    #[test]
    fn validate_filters_invalid_release_bindings_and_deduplicates() {
        let mut cfg = AppConfig {
            bindings: HashMap::from([(
                AppConfig::DEFAULT_APP_ID.to_string(),
                vec![
                    release_binding(TriggerButton::RightClick, vec![], "empty"),
                    release_binding(
                        TriggerButton::RightClick,
                        vec![GestureStep::RightClick],
                        "contains-trigger",
                    ),
                    release_binding(
                        TriggerButton::RightClick,
                        vec![GestureStep::Left, GestureStep::Left],
                        "same-move",
                    ),
                    release_binding(TriggerButton::RightClick, vec![GestureStep::Up], "first"),
                    release_binding(TriggerButton::RightClick, vec![GestureStep::Up], "second"),
                ],
            )]),
            ..AppConfig::default()
        };

        cfg.validate();

        let defaults = get_default_bindings(&cfg);
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].action, keyboard_action("first"));
    }

    #[test]
    fn validate_filters_invalid_hold_bindings_and_deduplicates() {
        let mut cfg = AppConfig {
            bindings: HashMap::from([(
                AppConfig::DEFAULT_APP_ID.to_string(),
                vec![
                    hold_binding(TriggerButton::RightClick, Vec::new(), None, "missing-step"),
                    hold_binding(
                        TriggerButton::RightClick,
                        Vec::new(),
                        Some(GestureStep::LeftClick),
                        "unsupported-step",
                    ),
                    hold_binding(
                        TriggerButton::RightClick,
                        vec![GestureStep::RightClick],
                        Some(GestureStep::WheelUp),
                        "contains-trigger",
                    ),
                    hold_binding(
                        TriggerButton::RightClick,
                        vec![GestureStep::Left, GestureStep::Left],
                        Some(GestureStep::WheelUp),
                        "same-move",
                    ),
                    hold_binding(
                        TriggerButton::RightClick,
                        Vec::new(),
                        Some(GestureStep::WheelUp),
                        "first",
                    ),
                    hold_binding(
                        TriggerButton::RightClick,
                        Vec::new(),
                        Some(GestureStep::WheelUp),
                        "second",
                    ),
                ],
            )]),
            ..AppConfig::default()
        };

        cfg.validate();

        let defaults = get_default_bindings(&cfg);
        assert_eq!(defaults.len(), 1);
        assert_eq!(defaults[0].gesture.mode, GestureMode::Hold);
        assert_eq!(defaults[0].gesture.step, Some(GestureStep::WheelUp));
        assert_eq!(defaults[0].action, keyboard_action("first"));
    }

    #[test]
    fn save_creates_directory_and_roundtrips_from_config_dir() {
        let temp_dir =
            tempfile::tempdir().expect("must be able to create temp dir for config test");
        let temp_path = temp_dir.path();

        let expected = AppConfig {
            trail_color: "#ffffff".to_string(),
            ..AppConfig::default()
        };

        save(&expected, temp_path).expect("save must succeed");
        let loaded = load_or_default(temp_path);
        assert_eq!(loaded, expected);
    }
}
