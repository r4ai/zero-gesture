use log::warn;

use crate::config::{MatchMethod, MatchTarget};
use crate::window_info::ForegroundWindowInfo;

/// Compiled matching logic for a single [`AppMatcher`](crate::config::AppMatcher).
///
/// Pre-processes the match value at startup so the hot path only does
/// simple string operations or regex matching.
#[derive(Debug, Clone)]
pub(super) enum CompiledMatchLogic {
    /// Exact match (case-insensitive): value is pre-lowercased.
    ExactCaseInsensitive(String),
    /// Exact match (case-sensitive): value as-is.
    ExactCaseSensitive(String),
    /// Substring match (case-insensitive): value is pre-lowercased.
    Contains(String),
    /// Regex pattern match.
    Regex(regex::Regex),
}

/// A compiled matcher combining a target and pre-processed matching logic.
#[derive(Debug, Clone)]
pub(super) struct CompiledMatcher {
    pub(super) target: MatchTarget,
    pub(super) logic: CompiledMatchLogic,
}

impl CompiledMatcher {
    /// Test whether this matcher matches the given window info.
    fn matches(&self, info: &ForegroundWindowInfo) -> bool {
        let target_value = match &self.target {
            MatchTarget::ProcessName => info.process_name.as_deref(),
            MatchTarget::WindowClass => info.window_class.as_deref(),
            MatchTarget::Title => info.title.as_deref(),
        };
        let target_value = match target_value {
            Some(v) => v,
            None => return false,
        };
        match &self.logic {
            CompiledMatchLogic::ExactCaseInsensitive(pattern) => {
                target_value.to_lowercase() == *pattern
            }
            CompiledMatchLogic::ExactCaseSensitive(pattern) => target_value == pattern,
            CompiledMatchLogic::Contains(pattern) => {
                target_value.to_lowercase().contains(pattern.as_str())
            }
            CompiledMatchLogic::Regex(re) => re.is_match(target_value),
        }
    }
}

/// A compiled app definition with its ID and matchers.
#[derive(Debug, Clone)]
pub(super) struct CompiledApp {
    pub(super) id: String,
    pub(super) matchers: Vec<CompiledMatcher>,
}

/// Linear scan to find the first app whose matchers match the given window info.
///
/// Returns the app ID if a match is found, `None` otherwise.
/// Each app's matchers use OR logic — any single matcher matching is sufficient.
pub(super) fn match_app<'a>(
    apps: &'a [CompiledApp],
    info: &ForegroundWindowInfo,
) -> Option<&'a str> {
    for app in apps {
        if app.matchers.iter().any(|m| m.matches(info)) {
            return Some(&app.id);
        }
    }
    None
}

/// Compile an [`AppMatcher`](crate::config::AppMatcher) into a [`CompiledMatcher`].
///
/// Returns `None` if the matcher is invalid (e.g. invalid regex pattern).
pub(super) fn compile_matcher(m: &crate::config::AppMatcher) -> Option<CompiledMatcher> {
    let logic = match (&m.target, &m.method) {
        // Exact on process_name/title → case-insensitive
        (MatchTarget::ProcessName | MatchTarget::Title, MatchMethod::Exact) => {
            CompiledMatchLogic::ExactCaseInsensitive(m.value.to_lowercase())
        }
        // Exact on window_class → case-sensitive
        (MatchTarget::WindowClass, MatchMethod::Exact) => {
            CompiledMatchLogic::ExactCaseSensitive(m.value.clone())
        }
        // Contains → always case-insensitive
        (_, MatchMethod::Contains) => CompiledMatchLogic::Contains(m.value.to_lowercase()),
        // Regex
        (_, MatchMethod::Regex) => match regex::Regex::new(&m.value) {
            Ok(re) => CompiledMatchLogic::Regex(re),
            Err(err) => {
                warn!(
                    "Invalid regex pattern {:?} for {:?} matcher: {}",
                    m.value, m.target, err
                );
                return None;
            }
        },
    };
    Some(CompiledMatcher {
        target: m.target.clone(),
        logic,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_app_process_name_exact() {
        let apps = vec![CompiledApp {
            id: "browser".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::ProcessName,
                logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: Some("chrome.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));

        // Case insensitive
        let info = ForegroundWindowInfo {
            process_name: Some("Chrome.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));

        // No match
        let info = ForegroundWindowInfo {
            process_name: Some("firefox.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), None);
    }

    #[test]
    fn match_app_window_class_exact_case_sensitive() {
        let apps = vec![CompiledApp {
            id: "explorer".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::WindowClass,
                logic: CompiledMatchLogic::ExactCaseSensitive("CabinetWClass".to_string()),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: Some("CabinetWClass".to_string()),
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("explorer"));

        // Case mismatch → no match (case-sensitive)
        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: Some("cabinetwclass".to_string()),
            title: None,
        };
        assert_eq!(match_app(&apps, &info), None);
    }

    #[test]
    fn match_app_title_contains() {
        let apps = vec![CompiledApp {
            id: "browser".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::Title,
                logic: CompiledMatchLogic::Contains("google chrome".to_string()),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: None,
            title: Some("My Page - Google Chrome".to_string()),
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));

        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: None,
            title: Some("Firefox".to_string()),
        };
        assert_eq!(match_app(&apps, &info), None);
    }

    #[test]
    fn match_app_title_contains_non_ascii_case_insensitive() {
        let apps = vec![CompiledApp {
            id: "browser".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::Title,
                logic: CompiledMatchLogic::Contains("i\u{307}stanbul".to_string()),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: None,
            title: Some("İSTANBUL".to_string()),
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));
    }

    #[test]
    fn match_app_regex() {
        let apps = vec![CompiledApp {
            id: "terminals".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::ProcessName,
                logic: CompiledMatchLogic::Regex(
                    regex::Regex::new(r"^(windowsterminal|cmd|powershell)\.exe$").unwrap(),
                ),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: Some("windowsterminal.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("terminals"));

        let info = ForegroundWindowInfo {
            process_name: Some("notepad.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), None);
    }

    #[test]
    fn match_app_or_logic() {
        let apps = vec![CompiledApp {
            id: "browser".to_string(),
            matchers: vec![
                CompiledMatcher {
                    target: MatchTarget::ProcessName,
                    logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
                },
                CompiledMatcher {
                    target: MatchTarget::ProcessName,
                    logic: CompiledMatchLogic::ExactCaseInsensitive("firefox.exe".to_string()),
                },
            ],
        }];

        // Matches second matcher
        let info = ForegroundWindowInfo {
            process_name: Some("firefox.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));
    }

    #[test]
    fn match_app_first_match_wins() {
        let apps = vec![
            CompiledApp {
                id: "browser".to_string(),
                matchers: vec![CompiledMatcher {
                    target: MatchTarget::ProcessName,
                    logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
                }],
            },
            CompiledApp {
                id: "google".to_string(),
                matchers: vec![CompiledMatcher {
                    target: MatchTarget::ProcessName,
                    logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
                }],
            },
        ];

        let info = ForegroundWindowInfo {
            process_name: Some("chrome.exe".to_string()),
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), Some("browser"));
    }

    #[test]
    fn match_app_none_field_no_panic() {
        let apps = vec![CompiledApp {
            id: "browser".to_string(),
            matchers: vec![CompiledMatcher {
                target: MatchTarget::ProcessName,
                logic: CompiledMatchLogic::ExactCaseInsensitive("chrome.exe".to_string()),
            }],
        }];

        let info = ForegroundWindowInfo {
            process_name: None,
            window_class: None,
            title: None,
        };
        assert_eq!(match_app(&apps, &info), None);
    }
}
