use std::collections::HashMap;

use regex::Regex;

use crate::domain::{AppBindingSet, GestureConfig, HoldBinding, ReleaseBinding};
use crate::executor::generate_label;
use crate::window_info::ForegroundWindowInfo;

use super::document::{
    self, ApplicationRecord, BindingRecord, ConfigDocument, ConfigError, DocumentAction,
    GestureMode, MatchMethod, MatchTarget, TriggerButton, DEFAULT_APP_ID,
};
use super::Action;

#[derive(Debug, Clone)]
enum CompiledMatchLogic {
    ExactCaseInsensitive(String),
    ExactCaseSensitive(String),
    Contains(String),
    Regex(Regex),
}

#[derive(Debug, Clone)]
struct CompiledMatcher {
    target: MatchTarget,
    logic: CompiledMatchLogic,
}

impl CompiledMatcher {
    fn matches(&self, info: &ForegroundWindowInfo) -> bool {
        let value = match self.target {
            MatchTarget::ProcessName => info.process_name.as_deref(),
            MatchTarget::WindowClass => info.window_class.as_deref(),
            MatchTarget::Title => info.title.as_deref(),
            MatchTarget::BundleIdentifier => None,
        };
        let Some(value) = value else {
            return false;
        };
        match &self.logic {
            CompiledMatchLogic::ExactCaseInsensitive(pattern) => value.to_lowercase() == *pattern,
            CompiledMatchLogic::ExactCaseSensitive(pattern) => value == pattern,
            CompiledMatchLogic::Contains(pattern) => {
                value.to_lowercase().contains(pattern.as_str())
            }
            CompiledMatchLogic::Regex(regex) => regex.is_match(value),
        }
    }
}

#[derive(Debug, Clone)]
struct CompiledApplication {
    id: String,
    matchers: Vec<CompiledMatcher>,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeAppearance {
    pub(crate) trail_color: String,
    pub(crate) trail_thickness: f32,
    pub(crate) label_font_family: String,
    pub(crate) label_font_size: f32,
    pub(crate) label_font_weight: i32,
    pub(crate) label_padding: f32,
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeConfig {
    pub(crate) enabled: bool,
    applications: Vec<CompiledApplication>,
    pub(crate) gesture: GestureConfig,
    pub(crate) appearance: RuntimeAppearance,
}

impl RuntimeConfig {
    pub(crate) fn match_windows_app<'a>(&'a self, info: &ForegroundWindowInfo) -> Option<&'a str> {
        self.applications
            .iter()
            .find(|application| {
                application
                    .matchers
                    .iter()
                    .any(|matcher| matcher.matches(info))
            })
            .map(|application| application.id.as_str())
    }
}

pub(crate) fn compile(document: &ConfigDocument) -> Result<RuntimeConfig, ConfigError> {
    document::validate(document)?;

    let mut applications = Vec::new();
    for (index, record) in document.applications.iter().enumerate() {
        let application = match record {
            ApplicationRecord::Shared(application) | ApplicationRecord::Windows(application) => {
                application
            }
            ApplicationRecord::Macos(_) => continue,
        };
        let mut matchers = Vec::with_capacity(application.matchers.len());
        for (matcher_index, matcher) in application.matchers.iter().enumerate() {
            let path = format!("applications[{index}].application.matchers[{matcher_index}].value");
            let logic = match (matcher.target, matcher.method) {
                (MatchTarget::ProcessName | MatchTarget::Title, MatchMethod::Exact) => {
                    CompiledMatchLogic::ExactCaseInsensitive(matcher.value.to_lowercase())
                }
                (MatchTarget::WindowClass, MatchMethod::Exact) => {
                    CompiledMatchLogic::ExactCaseSensitive(matcher.value.clone())
                }
                (_, MatchMethod::Contains) => {
                    CompiledMatchLogic::Contains(matcher.value.to_lowercase())
                }
                (_, MatchMethod::Regex) => {
                    CompiledMatchLogic::Regex(Regex::new(&matcher.value).map_err(|error| {
                        ConfigError::at(path, format!("invalid regex: {error}"))
                    })?)
                }
                (MatchTarget::BundleIdentifier, MatchMethod::Exact) => {
                    unreachable!("macOS records are filtered before Windows matcher compilation")
                }
            };
            matchers.push(CompiledMatcher {
                target: matcher.target,
                logic,
            });
        }
        applications.push(CompiledApplication {
            id: application.id.clone(),
            matchers,
        });
    }

    let mut binding_sets: HashMap<String, AppBindingSet> = HashMap::new();
    for record in &document.bindings {
        let binding = match record {
            BindingRecord::Shared(binding) | BindingRecord::Windows(binding) => binding,
            BindingRecord::Macos(_) => continue,
        };
        let application_id = binding
            .application_id
            .clone()
            .unwrap_or_else(|| DEFAULT_APP_ID.to_string());
        let set = binding_sets
            .entry(application_id)
            .or_insert_with(|| AppBindingSet {
                release_bindings: Vec::new(),
                hold_bindings: Vec::new(),
            });
        let action = compile_action(&binding.action);
        let label = binding
            .label
            .clone()
            .unwrap_or_else(|| generate_label(&action));
        let trigger = match binding.gesture.trigger {
            TriggerButton::LeftClick => crate::domain::TriggerButton::Left,
            TriggerButton::RightClick => crate::domain::TriggerButton::Right,
            TriggerButton::MiddleClick => crate::domain::TriggerButton::Middle,
        };
        match binding.gesture.mode {
            GestureMode::Release => set.release_bindings.push(ReleaseBinding {
                trigger,
                sequence: binding.gesture.sequence.clone(),
                action,
                label,
            }),
            GestureMode::Hold => set.hold_bindings.push(HoldBinding {
                trigger,
                sequence: binding.gesture.sequence.clone(),
                step: binding
                    .gesture
                    .step
                    .expect("validated hold binding must define a step"),
                action,
                label,
            }),
        }
    }
    binding_sets
        .entry(DEFAULT_APP_ID.to_string())
        .or_insert_with(|| AppBindingSet {
            release_bindings: Vec::new(),
            hold_bindings: Vec::new(),
        });

    let recognition = &document.shared.recognition;
    let appearance = document
        .platforms
        .windows
        .appearance
        .as_ref()
        .unwrap_or(&document.shared.appearance);
    Ok(RuntimeConfig {
        enabled: document.shared.enabled,
        applications,
        gesture: GestureConfig {
            safety_timeout_ms: recognition.safety_timeout_ms,
            min_segment_px: recognition.min_segment_px,
            direction_switch_confirm_px: recognition.direction_switch_confirm_px,
            axis_ambiguity_deadzone_px: recognition.axis_ambiguity_deadzone_px,
            replay_distance_threshold_px: recognition.replay_distance_threshold_px,
            max_gesture_steps: usize::from(recognition.max_gesture_steps),
            binding_sets,
        },
        appearance: RuntimeAppearance {
            trail_color: appearance.trail_color.clone(),
            trail_thickness: appearance.trail_thickness,
            label_font_family: appearance.label_font_family.clone(),
            label_font_size: appearance.label_font_size,
            label_font_weight: appearance.label_font_weight,
            label_padding: appearance.label_padding,
        },
    })
}

fn compile_action(action: &DocumentAction) -> Action {
    let DocumentAction::Keyboard { keys } = action;
    Action::Keyboard {
        keys: keys
            .iter()
            .map(|key| key.windows_name().to_string())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        AppMatcher, AppearanceSettings, Application, GestureBinding, GesturePattern, Key,
        PlatformOverride,
    };

    fn app(id: &str, target: MatchTarget) -> Application {
        Application {
            id: id.to_string(),
            label: None,
            matchers: vec![AppMatcher {
                target,
                method: MatchMethod::Exact,
                value: id.to_string(),
            }],
        }
    }

    fn binding(id: &str, application_id: Option<&str>, key: Key) -> GestureBinding {
        GestureBinding {
            id: id.to_string(),
            label: None,
            application_id: application_id.map(ToString::to_string),
            gesture: GesturePattern {
                trigger: TriggerButton::RightClick,
                mode: GestureMode::Release,
                sequence: vec![crate::config::GestureStep::Right],
                step: None,
            },
            action: DocumentAction::Keyboard { keys: vec![key] },
        }
    }

    #[test]
    fn compile_filters_variants_without_reordering_selected_records() {
        let document = ConfigDocument {
            applications: vec![
                ApplicationRecord::Shared(app("shared", MatchTarget::Title)),
                ApplicationRecord::Macos(app("mac", MatchTarget::BundleIdentifier)),
                ApplicationRecord::Windows(app("windows", MatchTarget::WindowClass)),
            ],
            bindings: vec![
                BindingRecord::Shared(binding("shared-binding", Some("shared"), Key::Primary)),
                BindingRecord::Macos(binding("mac-binding", Some("mac"), Key::Command)),
                BindingRecord::Windows(binding("windows-binding", Some("windows"), Key::Ctrl)),
            ],
            ..ConfigDocument::default()
        };
        let compiled = compile(&document).unwrap();
        assert_eq!(
            compiled.match_windows_app(&ForegroundWindowInfo {
                process_name: None,
                window_class: None,
                title: Some("shared".to_string()),
            }),
            Some("shared")
        );
        assert_eq!(
            compiled.match_windows_app(&ForegroundWindowInfo {
                process_name: None,
                window_class: Some("windows".to_string()),
                title: None,
            }),
            Some("windows")
        );
        assert_eq!(compiled.gesture.binding_sets.len(), 3);
        assert!(!compiled.gesture.binding_sets.contains_key("mac"));
    }

    #[test]
    fn platform_appearance_override_replaces_the_whole_field() {
        let mut document = ConfigDocument::default();
        let shared = document.shared.appearance.clone();
        document.platforms.windows = PlatformOverride {
            appearance: Some(AppearanceSettings {
                trail_color: "#fff".to_string(),
                trail_thickness: 9.0,
                label_font_family: "Override".to_string(),
                label_font_size: 11.0,
                label_font_weight: 500,
                label_padding: 7.0,
            }),
        };
        let overridden = compile(&document).unwrap();
        assert_eq!(overridden.appearance.trail_thickness, 9.0);
        assert_eq!(overridden.appearance.label_font_family, "Override");

        document.platforms.windows.appearance = None;
        let inherited = compile(&document).unwrap();
        assert_eq!(inherited.appearance.trail_thickness, shared.trail_thickness);
        assert_eq!(
            inherited.appearance.label_font_family,
            shared.label_font_family
        );
    }
}
