use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 2;
pub const DEFAULT_APP_ID: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    path: String,
    message: String,
}

impl ConfigError {
    pub(crate) fn at(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConfigDocument {
    pub schema_version: u32,
    pub shared: SharedSettings,
    pub applications: Vec<ApplicationRecord>,
    pub bindings: Vec<BindingRecord>,
    pub platforms: PlatformSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SharedSettings {
    pub enabled: bool,
    pub recognition: RecognitionSettings,
    pub appearance: AppearanceSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecognitionSettings {
    pub safety_timeout_ms: u32,
    pub min_segment_px: i32,
    pub direction_switch_confirm_px: i32,
    pub axis_ambiguity_deadzone_px: i32,
    pub replay_distance_threshold_px: i32,
    pub max_gesture_steps: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppearanceSettings {
    pub trail_color: String,
    pub trail_thickness: f32,
    pub label_font_family: String,
    pub label_font_size: f32,
    pub label_font_weight: i32,
    pub label_padding: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct PlatformSettings {
    pub windows: PlatformOverride,
    pub macos: PlatformOverride,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct PlatformOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<AppearanceSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "platform",
    content = "application",
    rename_all = "lowercase",
    deny_unknown_fields
)]
pub enum ApplicationRecord {
    Shared(Application),
    Windows(Application),
    Macos(Application),
}

impl ApplicationRecord {
    pub fn application(&self) -> &Application {
        match self {
            Self::Shared(application) | Self::Windows(application) | Self::Macos(application) => {
                application
            }
        }
    }

    fn platform(&self) -> RecordPlatform {
        match self {
            Self::Shared(_) => RecordPlatform::Shared,
            Self::Windows(_) => RecordPlatform::Windows,
            Self::Macos(_) => RecordPlatform::Macos,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Application {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub matchers: Vec<AppMatcher>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AppMatcher {
    pub target: MatchTarget,
    pub method: MatchMethod,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchTarget {
    ProcessName,
    WindowClass,
    Title,
    BundleIdentifier,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MatchMethod {
    Exact,
    Contains,
    Regex,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(
    tag = "platform",
    content = "binding",
    rename_all = "lowercase",
    deny_unknown_fields
)]
pub enum BindingRecord {
    Shared(GestureBinding),
    Windows(GestureBinding),
    Macos(GestureBinding),
}

impl BindingRecord {
    pub fn binding(&self) -> &GestureBinding {
        match self {
            Self::Shared(binding) | Self::Windows(binding) | Self::Macos(binding) => binding,
        }
    }

    fn platform(&self) -> RecordPlatform {
        match self {
            Self::Shared(_) => RecordPlatform::Shared,
            Self::Windows(_) => RecordPlatform::Windows,
            Self::Macos(_) => RecordPlatform::Macos,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GestureBinding {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_id: Option<String>,
    pub gesture: GesturePattern,
    pub action: DocumentAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GesturePattern {
    pub trigger: TriggerButton,
    pub mode: GestureMode,
    pub sequence: Vec<GestureStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<GestureStep>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TriggerButton {
    LeftClick,
    RightClick,
    MiddleClick,
}

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum GestureMode {
    #[default]
    Release,
    Hold,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum DocumentAction {
    Keyboard { keys: Vec<Key> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Key {
    Primary,
    Secondary,
    Shift,
    Ctrl,
    Alt,
    Win,
    Command,
    Option,
    Left,
    Right,
    Up,
    Down,
    Tab,
    Enter,
    Escape,
    Backspace,
    Delete,
    Home,
    End,
    #[serde(rename = "pageup")]
    PageUp,
    #[serde(rename = "pagedown")]
    PageDown,
    Space,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    #[serde(rename = "0")]
    Digit0,
    #[serde(rename = "1")]
    Digit1,
    #[serde(rename = "2")]
    Digit2,
    #[serde(rename = "3")]
    Digit3,
    #[serde(rename = "4")]
    Digit4,
    #[serde(rename = "5")]
    Digit5,
    #[serde(rename = "6")]
    Digit6,
    #[serde(rename = "7")]
    Digit7,
    #[serde(rename = "8")]
    Digit8,
    #[serde(rename = "9")]
    Digit9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
}

impl Key {
    fn from_legacy(value: &str) -> Option<Self> {
        let lower = value.to_ascii_lowercase();
        Some(match lower.as_str() {
            "primary" => Self::Primary,
            "secondary" => Self::Secondary,
            "shift" => Self::Shift,
            "ctrl" | "control" => Self::Ctrl,
            "alt" | "menu" => Self::Alt,
            "win" | "lwin" | "super" => Self::Win,
            "left" => Self::Left,
            "right" => Self::Right,
            "up" => Self::Up,
            "down" => Self::Down,
            "tab" => Self::Tab,
            "enter" | "return" => Self::Enter,
            "escape" | "esc" => Self::Escape,
            "backspace" => Self::Backspace,
            "delete" | "del" => Self::Delete,
            "home" => Self::Home,
            "end" => Self::End,
            "pageup" | "pgup" => Self::PageUp,
            "pagedown" | "pgdn" => Self::PageDown,
            "space" => Self::Space,
            "a" => Self::A,
            "b" => Self::B,
            "c" => Self::C,
            "d" => Self::D,
            "e" => Self::E,
            "f" => Self::F,
            "g" => Self::G,
            "h" => Self::H,
            "i" => Self::I,
            "j" => Self::J,
            "k" => Self::K,
            "l" => Self::L,
            "m" => Self::M,
            "n" => Self::N,
            "o" => Self::O,
            "p" => Self::P,
            "q" => Self::Q,
            "r" => Self::R,
            "s" => Self::S,
            "t" => Self::T,
            "u" => Self::U,
            "v" => Self::V,
            "w" => Self::W,
            "x" => Self::X,
            "y" => Self::Y,
            "z" => Self::Z,
            "0" => Self::Digit0,
            "1" => Self::Digit1,
            "2" => Self::Digit2,
            "3" => Self::Digit3,
            "4" => Self::Digit4,
            "5" => Self::Digit5,
            "6" => Self::Digit6,
            "7" => Self::Digit7,
            "8" => Self::Digit8,
            "9" => Self::Digit9,
            "f1" => Self::F1,
            "f2" => Self::F2,
            "f3" => Self::F3,
            "f4" => Self::F4,
            "f5" => Self::F5,
            "f6" => Self::F6,
            "f7" => Self::F7,
            "f8" => Self::F8,
            "f9" => Self::F9,
            "f10" => Self::F10,
            "f11" => Self::F11,
            "f12" => Self::F12,
            "f13" => Self::F13,
            "f14" => Self::F14,
            "f15" => Self::F15,
            "f16" => Self::F16,
            "f17" => Self::F17,
            "f18" => Self::F18,
            "f19" => Self::F19,
            "f20" => Self::F20,
            "f21" => Self::F21,
            "f22" => Self::F22,
            "f23" => Self::F23,
            "f24" => Self::F24,
            _ => return None,
        })
    }

    pub(crate) fn is_portable(self) -> bool {
        !matches!(
            self,
            Self::Ctrl | Self::Alt | Self::Win | Self::Command | Self::Option
        )
    }

    fn is_windows(self) -> bool {
        !matches!(self, Self::Command | Self::Option)
    }

    fn is_macos(self) -> bool {
        !matches!(self, Self::Alt | Self::Win)
    }

    pub(crate) fn windows_name(self) -> &'static str {
        match self {
            Self::Primary | Self::Ctrl => "ctrl",
            Self::Secondary | Self::Alt => "alt",
            Self::Shift => "shift",
            Self::Win => "win",
            Self::Left => "left",
            Self::Right => "right",
            Self::Up => "up",
            Self::Down => "down",
            Self::Tab => "tab",
            Self::Enter => "enter",
            Self::Escape => "escape",
            Self::Backspace => "backspace",
            Self::Delete => "delete",
            Self::Home => "home",
            Self::End => "end",
            Self::PageUp => "pageup",
            Self::PageDown => "pagedown",
            Self::Space => "space",
            Self::A => "a",
            Self::B => "b",
            Self::C => "c",
            Self::D => "d",
            Self::E => "e",
            Self::F => "f",
            Self::G => "g",
            Self::H => "h",
            Self::I => "i",
            Self::J => "j",
            Self::K => "k",
            Self::L => "l",
            Self::M => "m",
            Self::N => "n",
            Self::O => "o",
            Self::P => "p",
            Self::Q => "q",
            Self::R => "r",
            Self::S => "s",
            Self::T => "t",
            Self::U => "u",
            Self::V => "v",
            Self::W => "w",
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
            Self::Digit0 => "0",
            Self::Digit1 => "1",
            Self::Digit2 => "2",
            Self::Digit3 => "3",
            Self::Digit4 => "4",
            Self::Digit5 => "5",
            Self::Digit6 => "6",
            Self::Digit7 => "7",
            Self::Digit8 => "8",
            Self::Digit9 => "9",
            Self::F1 => "f1",
            Self::F2 => "f2",
            Self::F3 => "f3",
            Self::F4 => "f4",
            Self::F5 => "f5",
            Self::F6 => "f6",
            Self::F7 => "f7",
            Self::F8 => "f8",
            Self::F9 => "f9",
            Self::F10 => "f10",
            Self::F11 => "f11",
            Self::F12 => "f12",
            Self::F13 => "f13",
            Self::F14 => "f14",
            Self::F15 => "f15",
            Self::F16 => "f16",
            Self::F17 => "f17",
            Self::F18 => "f18",
            Self::F19 => "f19",
            Self::F20 => "f20",
            Self::F21 => "f21",
            Self::F22 => "f22",
            Self::F23 => "f23",
            Self::F24 => "f24",
            Self::Command | Self::Option => {
                unreachable!("macOS-only keys are rejected before Windows compilation")
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordPlatform {
    Shared,
    Windows,
    Macos,
}

#[derive(Deserialize)]
struct SchemaProbe {
    #[serde(default)]
    schema_version: Option<u32>,
}

#[derive(Debug)]
pub(crate) enum DecodedDocument {
    Current(ConfigDocument),
    Migrated(ConfigDocument),
}

pub(crate) fn decode(bytes: &[u8]) -> Result<DecodedDocument, ConfigError> {
    let probe: SchemaProbe = serde_json::from_slice(bytes)
        .map_err(|error| ConfigError::at("$", format!("invalid JSON: {error}")))?;
    match probe.schema_version {
        Some(SCHEMA_VERSION) => {
            let document = serde_json::from_slice(bytes)
                .map_err(|error| ConfigError::at("$", format!("invalid v2 document: {error}")))?;
            Ok(DecodedDocument::Current(document))
        }
        None | Some(1) => {
            let legacy: LegacyConfig = serde_json::from_slice(bytes).map_err(|error| {
                ConfigError::at("$", format!("invalid legacy document: {error}"))
            })?;
            migrate_legacy(legacy).map(DecodedDocument::Migrated)
        }
        Some(version) => Err(ConfigError::at(
            "schema_version",
            format!("unsupported schema version {version}"),
        )),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct LegacyConfig {
    schema_version: Option<u32>,
    enabled: bool,
    trail_color: String,
    trail_thickness: f32,
    safety_timeout_ms: u32,
    min_segment_px: i32,
    direction_switch_confirm_px: i32,
    axis_ambiguity_deadzone_px: i32,
    replay_distance_threshold_px: i32,
    label_font_family: String,
    label_font_size: f32,
    label_font_weight: i32,
    label_padding: f32,
    apps: BTreeMap<String, LegacyApplication>,
    bindings: BTreeMap<String, Vec<LegacyBinding>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyApplication {
    #[serde(default)]
    label: Option<String>,
    matchers: Vec<AppMatcher>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyBinding {
    id: String,
    #[serde(default)]
    label: Option<String>,
    gesture: LegacyGesturePattern,
    action: LegacyAction,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyGesturePattern {
    trigger: TriggerButton,
    #[serde(default)]
    mode: GestureMode,
    #[serde(default)]
    sequence: Vec<GestureStep>,
    #[serde(default)]
    step: Option<GestureStep>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
enum LegacyAction {
    Keyboard { keys: Vec<String> },
}

impl Default for LegacyConfig {
    fn default() -> Self {
        Self {
            schema_version: None,
            enabled: true,
            trail_color: "#00BFFF".to_string(),
            trail_thickness: 3.0,
            safety_timeout_ms: 2_000,
            min_segment_px: 12,
            direction_switch_confirm_px: 8,
            axis_ambiguity_deadzone_px: 2,
            replay_distance_threshold_px: 12,
            label_font_family: "Yu Gothic UI Semibold".to_string(),
            label_font_size: 36.0,
            label_font_weight: 400,
            label_padding: 24.0,
            apps: BTreeMap::new(),
            bindings: BTreeMap::from([(DEFAULT_APP_ID.to_string(), default_legacy_bindings())]),
        }
    }
}

fn default_legacy_bindings() -> Vec<LegacyBinding> {
    [
        ("back", "Back", vec![GestureStep::Left], vec!["alt", "left"]),
        (
            "forward",
            "Forward",
            vec![GestureStep::Right],
            vec!["alt", "right"],
        ),
        (
            "scroll-up",
            "Scroll Up",
            vec![GestureStep::Up],
            vec!["pageup"],
        ),
        (
            "scroll-down",
            "Scroll Down",
            vec![GestureStep::Down],
            vec!["pagedown"],
        ),
        (
            "top-of-page",
            "Top of Page",
            vec![GestureStep::Down, GestureStep::Up],
            vec!["ctrl", "home"],
        ),
        (
            "bottom-of-page",
            "Bottom of Page",
            vec![GestureStep::Up, GestureStep::Down],
            vec!["ctrl", "end"],
        ),
        (
            "next-tab",
            "Next Tab",
            vec![GestureStep::Up, GestureStep::Right],
            vec!["ctrl", "tab"],
        ),
        (
            "previous-tab",
            "Previous Tab",
            vec![GestureStep::Up, GestureStep::Left],
            vec!["ctrl", "shift", "tab"],
        ),
        (
            "reload",
            "Reload",
            vec![GestureStep::Right, GestureStep::Down],
            vec!["ctrl", "r"],
        ),
        (
            "close-tab",
            "Close Tab",
            vec![GestureStep::Down, GestureStep::Right],
            vec!["ctrl", "w"],
        ),
    ]
    .into_iter()
    .map(|(id, label, sequence, keys)| LegacyBinding {
        id: id.to_string(),
        label: Some(label.to_string()),
        gesture: LegacyGesturePattern {
            trigger: TriggerButton::RightClick,
            mode: GestureMode::Release,
            sequence,
            step: None,
        },
        action: LegacyAction::Keyboard {
            keys: keys.into_iter().map(ToString::to_string).collect(),
        },
    })
    .collect()
}

fn migrate_legacy(legacy: LegacyConfig) -> Result<ConfigDocument, ConfigError> {
    if legacy.schema_version.is_some_and(|version| version != 1) {
        return Err(ConfigError::at(
            "schema_version",
            "legacy document must omit schema_version or use 1",
        ));
    }

    let mut windows_app_ids = HashSet::new();
    let mut applications = Vec::with_capacity(legacy.apps.len());
    for (id, application) in legacy.apps {
        let path = format!("apps.{id}");
        if id == DEFAULT_APP_ID || id.trim().is_empty() {
            return Err(ConfigError::at(
                format!("{path}.id"),
                "invalid application ID",
            ));
        }
        let platform = classify_legacy_application(&application, &path)?;
        let application = Application {
            id: id.clone(),
            label: application.label,
            matchers: application.matchers,
        };
        applications.push(match platform {
            RecordPlatform::Shared => ApplicationRecord::Shared(application),
            RecordPlatform::Windows => {
                windows_app_ids.insert(id);
                ApplicationRecord::Windows(application)
            }
            RecordPlatform::Macos => unreachable!(),
        });
    }

    let mut bindings = Vec::new();
    let mut legacy_bindings = legacy.bindings;
    if let Some(defaults) = legacy_bindings.remove(DEFAULT_APP_ID) {
        migrate_binding_group(None, defaults, &windows_app_ids, &mut bindings)?;
    }
    for (application_id, group) in legacy_bindings {
        migrate_binding_group(Some(application_id), group, &windows_app_ids, &mut bindings)?;
    }

    Ok(ConfigDocument {
        schema_version: SCHEMA_VERSION,
        shared: SharedSettings {
            enabled: legacy.enabled,
            recognition: RecognitionSettings {
                safety_timeout_ms: legacy.safety_timeout_ms,
                min_segment_px: legacy.min_segment_px,
                direction_switch_confirm_px: legacy.direction_switch_confirm_px,
                axis_ambiguity_deadzone_px: legacy.axis_ambiguity_deadzone_px,
                replay_distance_threshold_px: legacy.replay_distance_threshold_px,
                max_gesture_steps: 8,
            },
            appearance: AppearanceSettings {
                trail_color: legacy.trail_color,
                trail_thickness: legacy.trail_thickness,
                label_font_family: legacy.label_font_family,
                label_font_size: legacy.label_font_size,
                label_font_weight: legacy.label_font_weight,
                label_padding: legacy.label_padding,
            },
        },
        applications,
        bindings,
        platforms: PlatformSettings::default(),
    })
}

fn classify_legacy_application(
    application: &LegacyApplication,
    path: &str,
) -> Result<RecordPlatform, ConfigError> {
    let mut platform = RecordPlatform::Shared;
    for (index, matcher) in application.matchers.iter().enumerate() {
        match matcher.target {
            MatchTarget::ProcessName | MatchTarget::Title => {}
            MatchTarget::WindowClass => platform = RecordPlatform::Windows,
            MatchTarget::BundleIdentifier => {
                return Err(ConfigError::at(
                    format!("{path}.matchers[{index}].target"),
                    "bundle_identifier is not valid in a Windows legacy document",
                ));
            }
        }
    }
    Ok(platform)
}

fn migrate_binding_group(
    application_id: Option<String>,
    group: Vec<LegacyBinding>,
    windows_app_ids: &HashSet<String>,
    output: &mut Vec<BindingRecord>,
) -> Result<(), ConfigError> {
    let group_name = application_id.as_deref().unwrap_or(DEFAULT_APP_ID);
    for (index, binding) in group.into_iter().enumerate() {
        let path = format!("bindings.{group_name}[{index}]");
        let LegacyAction::Keyboard { keys: legacy_keys } = binding.action;
        let mut keys = Vec::with_capacity(legacy_keys.len());
        for (key_index, key) in legacy_keys.iter().enumerate() {
            let parsed = Key::from_legacy(key).ok_or_else(|| {
                ConfigError::at(
                    format!("{path}.action.keys[{key_index}]"),
                    format!("unsupported legacy key {key:?}"),
                )
            })?;
            if !parsed.is_windows() {
                return Err(ConfigError::at(
                    format!("{path}.action.keys[{key_index}]"),
                    format!("key {key:?} is not valid in a Windows legacy document"),
                ));
            }
            keys.push(parsed);
        }

        let is_windows = application_id
            .as_ref()
            .is_some_and(|id| windows_app_ids.contains(id))
            || keys.iter().any(|key| !key.is_portable());
        let migrated = GestureBinding {
            id: binding.id,
            label: binding.label,
            application_id: application_id.clone(),
            gesture: GesturePattern {
                trigger: binding.gesture.trigger,
                mode: binding.gesture.mode,
                sequence: binding.gesture.sequence,
                step: binding.gesture.step,
            },
            action: DocumentAction::Keyboard { keys },
        };
        output.push(if is_windows {
            BindingRecord::Windows(migrated)
        } else {
            BindingRecord::Shared(migrated)
        });
    }
    Ok(())
}

pub(crate) fn validate(document: &ConfigDocument) -> Result<(), ConfigError> {
    if document.schema_version != SCHEMA_VERSION {
        return Err(ConfigError::at(
            "schema_version",
            format!("unsupported schema version {}", document.schema_version),
        ));
    }
    validate_recognition(&document.shared.recognition)?;
    validate_appearance(&document.shared.appearance, "shared.appearance")?;
    if let Some(appearance) = &document.platforms.windows.appearance {
        validate_appearance(appearance, "platforms.windows.appearance")?;
    }
    if let Some(appearance) = &document.platforms.macos.appearance {
        validate_appearance(appearance, "platforms.macos.appearance")?;
    }

    let mut applications = HashMap::new();
    for (index, record) in document.applications.iter().enumerate() {
        let path = format!("applications[{index}].application");
        let application = record.application();
        if application.id.trim().is_empty() || application.id == DEFAULT_APP_ID {
            return Err(ConfigError::at(
                format!("{path}.id"),
                "invalid application ID",
            ));
        }
        if applications
            .insert(application.id.as_str(), record.platform())
            .is_some()
        {
            return Err(ConfigError::at(
                format!("{path}.id"),
                format!("duplicate application ID {:?}", application.id),
            ));
        }
        if application.matchers.is_empty() {
            return Err(ConfigError::at(
                format!("{path}.matchers"),
                "at least one matcher is required",
            ));
        }
        for (matcher_index, matcher) in application.matchers.iter().enumerate() {
            let matcher_path = format!("{path}.matchers[{matcher_index}]");
            let supported = match record.platform() {
                RecordPlatform::Shared => {
                    matches!(
                        matcher.target,
                        MatchTarget::ProcessName | MatchTarget::Title
                    )
                }
                RecordPlatform::Windows => matches!(
                    matcher.target,
                    MatchTarget::ProcessName | MatchTarget::WindowClass | MatchTarget::Title
                ),
                RecordPlatform::Macos => matches!(
                    matcher.target,
                    MatchTarget::ProcessName | MatchTarget::Title | MatchTarget::BundleIdentifier
                ),
            };
            if !supported {
                return Err(ConfigError::at(
                    format!("{matcher_path}.target"),
                    format!(
                        "{:?} is not supported by {:?} application records",
                        matcher.target,
                        record.platform()
                    ),
                ));
            }
        }
    }

    let mut binding_ids = HashSet::new();
    let mut signatures = HashSet::new();
    for (index, record) in document.bindings.iter().enumerate() {
        let path = format!("bindings[{index}].binding");
        let binding = record.binding();
        if binding.id.trim().is_empty() {
            return Err(ConfigError::at(
                format!("{path}.id"),
                "binding ID must not be empty",
            ));
        }
        let identity = (binding.application_id.as_deref(), binding.id.as_str());
        if !binding_ids.insert(identity) {
            return Err(ConfigError::at(
                format!("{path}.id"),
                format!("duplicate binding ID {:?}", binding.id),
            ));
        }

        let referenced_platform = match binding.application_id.as_deref() {
            None => None,
            Some(id) => Some(*applications.get(id).ok_or_else(|| {
                ConfigError::at(
                    format!("{path}.application_id"),
                    format!("unknown application reference {id:?}"),
                )
            })?),
        };
        validate_reference(record.platform(), referenced_platform, &path)?;
        validate_gesture(
            &binding.gesture,
            &path,
            usize::from(document.shared.recognition.max_gesture_steps),
        )?;
        validate_action(&binding.action, record.platform(), &path)?;

        let signature = (
            binding.application_id.as_deref(),
            binding.gesture.trigger,
            binding.gesture.mode,
            binding.gesture.sequence.as_slice(),
            binding.gesture.step,
        );
        if !signatures.insert(signature) {
            return Err(ConfigError::at(
                format!("{path}.gesture"),
                "duplicate gesture for the same application",
            ));
        }
    }
    Ok(())
}

fn validate_recognition(recognition: &RecognitionSettings) -> Result<(), ConfigError> {
    let positive = [
        (
            "safety_timeout_ms",
            i64::from(recognition.safety_timeout_ms),
        ),
        ("min_segment_px", i64::from(recognition.min_segment_px)),
        (
            "direction_switch_confirm_px",
            i64::from(recognition.direction_switch_confirm_px),
        ),
        (
            "replay_distance_threshold_px",
            i64::from(recognition.replay_distance_threshold_px),
        ),
        (
            "max_gesture_steps",
            i64::from(recognition.max_gesture_steps),
        ),
    ];
    for (field, value) in positive {
        if value <= 0 {
            return Err(ConfigError::at(
                format!("shared.recognition.{field}"),
                "must be greater than zero",
            ));
        }
    }
    if recognition.max_gesture_steps > 8 {
        return Err(ConfigError::at(
            "shared.recognition.max_gesture_steps",
            "must be 8 or less",
        ));
    }
    if recognition.axis_ambiguity_deadzone_px < 0 {
        return Err(ConfigError::at(
            "shared.recognition.axis_ambiguity_deadzone_px",
            "must be zero or greater",
        ));
    }
    Ok(())
}

fn validate_appearance(appearance: &AppearanceSettings, path: &str) -> Result<(), ConfigError> {
    let color = appearance
        .trail_color
        .strip_prefix('#')
        .unwrap_or(&appearance.trail_color);
    if !matches!(color.len(), 3 | 6) || !color.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ConfigError::at(
            format!("{path}.trail_color"),
            "must be a 3- or 6-digit hexadecimal color",
        ));
    }
    if !appearance.trail_thickness.is_finite() || appearance.trail_thickness <= 0.0 {
        return Err(ConfigError::at(
            format!("{path}.trail_thickness"),
            "must be finite and greater than zero",
        ));
    }
    if appearance.label_font_family.trim().is_empty() {
        return Err(ConfigError::at(
            format!("{path}.label_font_family"),
            "must not be empty",
        ));
    }
    if !appearance.label_font_size.is_finite() || appearance.label_font_size <= 0.0 {
        return Err(ConfigError::at(
            format!("{path}.label_font_size"),
            "must be finite and greater than zero",
        ));
    }
    if !(0..=1_000).contains(&appearance.label_font_weight) {
        return Err(ConfigError::at(
            format!("{path}.label_font_weight"),
            "must be between 0 and 1000",
        ));
    }
    if !appearance.label_padding.is_finite() || appearance.label_padding < 0.0 {
        return Err(ConfigError::at(
            format!("{path}.label_padding"),
            "must be finite and zero or greater",
        ));
    }
    Ok(())
}

fn validate_reference(
    binding: RecordPlatform,
    application: Option<RecordPlatform>,
    path: &str,
) -> Result<(), ConfigError> {
    let supported = matches!(
        (binding, application),
        (_, None)
            | (
                RecordPlatform::Windows,
                Some(RecordPlatform::Shared | RecordPlatform::Windows)
            )
            | (
                RecordPlatform::Macos,
                Some(RecordPlatform::Shared | RecordPlatform::Macos)
            )
            | (RecordPlatform::Shared, Some(RecordPlatform::Shared))
    );
    if supported {
        Ok(())
    } else {
        Err(ConfigError::at(
            format!("{path}.application_id"),
            "binding platform cannot reference this application platform",
        ))
    }
}

fn validate_gesture(
    gesture: &GesturePattern,
    path: &str,
    max_gesture_steps: usize,
) -> Result<(), ConfigError> {
    if gesture.sequence.len() > max_gesture_steps {
        return Err(ConfigError::at(
            format!("{path}.gesture.sequence"),
            format!("must contain at most {max_gesture_steps} steps"),
        ));
    }
    if gesture.sequence.windows(2).any(|pair| {
        pair[0] == pair[1]
            && matches!(
                pair[0],
                GestureStep::Up | GestureStep::Down | GestureStep::Left | GestureStep::Right
            )
    }) {
        return Err(ConfigError::at(
            format!("{path}.gesture.sequence"),
            "must not contain consecutive identical movement steps",
        ));
    }
    let trigger_step = match gesture.trigger {
        TriggerButton::LeftClick => GestureStep::LeftClick,
        TriggerButton::RightClick => GestureStep::RightClick,
        TriggerButton::MiddleClick => GestureStep::MiddleClick,
    };
    if gesture.sequence.contains(&trigger_step) {
        return Err(ConfigError::at(
            format!("{path}.gesture.sequence"),
            "must not contain its trigger click",
        ));
    }
    match gesture.mode {
        GestureMode::Release if gesture.sequence.is_empty() => Err(ConfigError::at(
            format!("{path}.gesture.sequence"),
            "release gesture sequence must not be empty",
        )),
        GestureMode::Release if gesture.step.is_some() => Err(ConfigError::at(
            format!("{path}.gesture.step"),
            "release gesture must not define step",
        )),
        GestureMode::Hold
            if !matches!(
                gesture.step,
                Some(GestureStep::WheelUp | GestureStep::WheelDown)
            ) =>
        {
            Err(ConfigError::at(
                format!("{path}.gesture.step"),
                "hold gesture step must be wheel_up or wheel_down",
            ))
        }
        _ => Ok(()),
    }
}

fn validate_action(
    action: &DocumentAction,
    platform: RecordPlatform,
    path: &str,
) -> Result<(), ConfigError> {
    let DocumentAction::Keyboard { keys } = action;
    if keys.is_empty() {
        return Err(ConfigError::at(
            format!("{path}.action.keys"),
            "must contain at least one key",
        ));
    }
    for (index, key) in keys.iter().copied().enumerate() {
        let supported = match platform {
            RecordPlatform::Shared => key.is_portable(),
            RecordPlatform::Windows => key.is_windows(),
            RecordPlatform::Macos => key.is_macos(),
        };
        if !supported {
            return Err(ConfigError::at(
                format!("{path}.action.keys[{index}]"),
                format!("{key:?} is not supported by {platform:?} binding records"),
            ));
        }
    }
    Ok(())
}

impl Default for ConfigDocument {
    fn default() -> Self {
        migrate_legacy(LegacyConfig::default()).expect("built-in v1 defaults must migrate")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_migrated(raw: &str) -> ConfigDocument {
        match decode(raw.as_bytes()).expect("legacy config must migrate") {
            DecodedDocument::Migrated(document) => document,
            DecodedDocument::Current(_) => panic!("expected migrated document"),
        }
    }

    #[test]
    fn deserialize_config_with_sequence_bindings() {
        let document = decode_migrated(
            r#"{"bindings":{"default":[{"id":"reload","label":"Reload","gesture":{"trigger":"right_click","sequence":["right","down"]},"action":{"type":"keyboard","keys":["ctrl","r"]}},{"id":"wheel-up","gesture":{"trigger":"middle_click","sequence":["wheel_up"]},"action":{"type":"keyboard","keys":["pageup"]}}]}}"#,
        );
        assert_eq!(document.bindings.len(), 2);
        assert_eq!(
            document.bindings[0].binding().gesture.sequence,
            [GestureStep::Right, GestureStep::Down]
        );
        assert_eq!(
            document.bindings[0].binding().label.as_deref(),
            Some("Reload")
        );
        assert!(matches!(document.bindings[0], BindingRecord::Windows(_)));
        assert!(matches!(document.bindings[1], BindingRecord::Shared(_)));
    }

    #[test]
    fn deserialize_config_with_hold_binding() {
        let document = decode_migrated(
            r#"{"bindings":{"default":[{"id":"hold-scroll-up","gesture":{"trigger":"right_click","mode":"hold","step":"wheel_up"},"action":{"type":"keyboard","keys":["pageup"]}}]}}"#,
        );
        let binding = document.bindings[0].binding();
        assert_eq!(binding.gesture.mode, GestureMode::Hold);
        assert_eq!(binding.gesture.step, Some(GestureStep::WheelUp));
        assert!(binding.gesture.sequence.is_empty());
    }

    #[test]
    fn deserialize_json_with_enabled_false() {
        let document = decode_migrated(r#"{"enabled":false}"#);
        assert!(!document.shared.enabled);
        assert!(!document.bindings.is_empty());
    }

    #[test]
    fn migrates_portable_v1_records_to_shared() {
        let document = decode_migrated(
            r#"{"apps":{"browser":{"matchers":[{"target":"title","method":"contains","value":"Docs"}]}},"bindings":{"browser":[{"id":"open","gesture":{"trigger":"right_click","sequence":["up"]},"action":{"type":"keyboard","keys":["primary","o"]}}]}}"#,
        );
        assert!(matches!(
            document.applications[0],
            ApplicationRecord::Shared(_)
        ));
        assert!(matches!(document.bindings[0], BindingRecord::Shared(_)));
    }

    #[test]
    fn migrates_windows_matcher_key_and_reference_as_whole_records() {
        let matcher = decode_migrated(
            r#"{"apps":{"native":{"matchers":[{"target":"window_class","method":"exact","value":"Native"}]}},"bindings":{"native":[{"id":"ref","gesture":{"trigger":"right_click","sequence":["up"]},"action":{"type":"keyboard","keys":["pageup"]}}],"default":[{"id":"key","gesture":{"trigger":"right_click","sequence":["down"]},"action":{"type":"keyboard","keys":["alt","left"]}}]}}"#,
        );
        assert!(matches!(
            matcher.applications[0],
            ApplicationRecord::Windows(_)
        ));
        assert!(matcher
            .bindings
            .iter()
            .all(|record| matches!(record, BindingRecord::Windows(_))));
    }

    #[test]
    fn migration_preserves_physical_ctrl() {
        let document = decode_migrated(
            r#"{"bindings":{"default":[{"id":"reload","gesture":{"trigger":"right_click","sequence":["right"]},"action":{"type":"keyboard","keys":["ctrl","r"]}}]}}"#,
        );
        let DocumentAction::Keyboard { keys } = &document.bindings[0].binding().action;
        assert_eq!(keys, &[Key::Ctrl, Key::R]);
        assert!(!keys.contains(&Key::Primary));
    }

    #[test]
    fn migration_preserves_ids_order_and_references() {
        let document = decode_migrated(
            r#"{"apps":{"browser":{"label":"Browser","matchers":[{"target":"title","method":"exact","value":"Browser"}]}},"bindings":{"browser":[{"id":"first","gesture":{"trigger":"right_click","sequence":["left"]},"action":{"type":"keyboard","keys":["pageup"]}},{"id":"second","gesture":{"trigger":"right_click","sequence":["right"]},"action":{"type":"keyboard","keys":["pagedown"]}}]}}"#,
        );
        assert_eq!(document.applications[0].application().id, "browser");
        assert_eq!(
            document
                .bindings
                .iter()
                .map(|record| record.binding().id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert!(document
            .bindings
            .iter()
            .all(|record| { record.binding().application_id.as_deref() == Some("browser") }));
    }

    #[test]
    fn rejects_unknown_and_newer_schema_without_fallback() {
        let newer = decode(br#"{"schema_version":3}"#).unwrap_err();
        assert_eq!(newer.path(), "schema_version");
        let unknown = decode(
            br##"{"schema_version":2,"shared":{"enabled":true,"recognition":{"safety_timeout_ms":1,"min_segment_px":1,"direction_switch_confirm_px":1,"axis_ambiguity_deadzone_px":0,"replay_distance_threshold_px":1,"max_gesture_steps":8},"appearance":{"trail_color":"#fff","trail_thickness":1.0,"label_font_family":"x","label_font_size":1.0,"label_font_weight":400,"label_padding":0.0},"extra":true},"applications":[],"bindings":[],"platforms":{"windows":{},"macos":{}}}"##,
        )
        .unwrap_err();
        assert!(unknown.to_string().contains("unknown field"));
    }

    #[test]
    fn validation_errors_name_exact_selector_key_and_reference_paths() {
        let mut document = ConfigDocument {
            applications: vec![ApplicationRecord::Shared(Application {
                id: "app".to_string(),
                label: None,
                matchers: vec![AppMatcher {
                    target: MatchTarget::WindowClass,
                    method: MatchMethod::Exact,
                    value: "Class".to_string(),
                }],
            })],
            ..ConfigDocument::default()
        };
        assert_eq!(
            validate(&document).unwrap_err().path(),
            "applications[0].application.matchers[0].target"
        );

        document.applications[0] =
            ApplicationRecord::Windows(document.applications[0].application().clone());
        document.bindings[0] = BindingRecord::Shared(document.bindings[0].binding().clone());
        let DocumentAction::Keyboard { keys } = &mut match &mut document.bindings[0] {
            BindingRecord::Shared(binding) => &mut binding.action,
            _ => unreachable!(),
        };
        *keys = vec![Key::Ctrl, Key::R];
        assert_eq!(
            validate(&document).unwrap_err().path(),
            "bindings[0].binding.action.keys[0]"
        );

        if let BindingRecord::Shared(binding) = &mut document.bindings[0] {
            binding.action = DocumentAction::Keyboard {
                keys: vec![Key::Primary, Key::R],
            };
            binding.application_id = Some("missing".to_string());
        }
        assert_eq!(
            validate(&document).unwrap_err().path(),
            "bindings[0].binding.application_id"
        );
    }
}
