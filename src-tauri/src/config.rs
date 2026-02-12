use std::fs;
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const CONFIG_FILE_NAME: &str = "mouse-gesture.config.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub gesture_trigger_button: String,
    pub trail_color: String,
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

pub fn load_or_default() -> AppConfig {
    let path = config_path();
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => return AppConfig::default(),
    };

    serde_json::from_str(&raw).unwrap_or_default()
}

#[allow(dead_code)]
pub fn save(config: &AppConfig) -> io::Result<()> {
    let body = serde_json::to_string_pretty(config)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    fs::write(config_path(), body)
}

fn config_path() -> PathBuf {
    PathBuf::from(CONFIG_FILE_NAME)
}
