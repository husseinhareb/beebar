use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::renderer::color::Color;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_height")]
    pub height: u32,

    #[serde(default = "default_background")]
    pub background: String,

    #[serde(default = "default_foreground")]
    pub foreground: String,

    #[serde(default = "default_font")]
    pub font: String,

    #[serde(default = "default_font_size")]
    pub font_size: f64,

    #[serde(default)]
    pub modules_left: Vec<String>,

    #[serde(default)]
    pub modules_center: Vec<String>,

    #[serde(default)]
    pub modules_right: Vec<String>,

    #[serde(default)]
    pub module: std::collections::HashMap<String, ModuleConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModuleConfig {
    #[serde(rename = "type")]
    pub kind: String,

    #[serde(default)]
    pub format: Option<String>,

    #[serde(default)]
    pub command: Option<String>,

    #[serde(default)]
    pub interval: Option<u64>,

    #[serde(default)]
    pub background: Option<String>,

    #[serde(default)]
    pub foreground: Option<String>,
}

fn default_height() -> u32 {
    30
}
fn default_background() -> String {
    "#1e1e2e".into()
}
fn default_foreground() -> String {
    "#cdd6f4".into()
}
fn default_font() -> String {
    "monospace".into()
}
fn default_font_size() -> f64 {
    14.0
}

impl Config {
    pub fn load() -> Self {
        let paths = [
            // XDG config
            dirs_maybe(),
            // Fallback: ./config/config.toml
            Some(PathBuf::from("config/config.toml")),
        ];

        for path in paths.into_iter().flatten() {
            if path.exists() {
                log::info!("Loading config from {}", path.display());
                let content = fs::read_to_string(&path).expect("Failed to read config");
                return toml::from_str(&content).expect("Failed to parse config");
            }
        }

        log::warn!("No config file found, using defaults");
        toml::from_str("").unwrap()
    }

    pub fn background_color(&self) -> Color {
        Color::from_hex(&self.background).unwrap_or(Color::BLACK)
    }

    pub fn foreground_color(&self) -> Color {
        Color::from_hex(&self.foreground).unwrap_or(Color::WHITE)
    }
}

fn dirs_maybe() -> Option<PathBuf> {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        })
        .map(|p| p.join("beebar/config.toml"))
}
