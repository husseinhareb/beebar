use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

use crate::renderer::color::Color;

const BUNDLED_DEFAULT_CONFIG: &str = include_str!("../../config/config.toml");
const DEFAULT_BACKGROUND: &str = "#1e1e2e";
const DEFAULT_FOREGROUND: &str = "#cdd6f4";
const DEFAULT_FONT_FAMILY: &str = "monospace";
const DEFAULT_FONT_SIZE: f64 = 14.0;
const DEFAULT_PADDING: f64 = 8.0;

#[derive(Debug, Deserialize, Clone)]
pub struct BarConfig {
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
    pub text_y_offset: f64,

    #[serde(default = "default_padding")]
    pub padding_left: f64,

    #[serde(default = "default_padding")]
    pub padding_right: f64,

    /// Position of the bar: "top" (default) or "bottom".
    #[serde(default = "default_position")]
    pub position: String,

    /// Monitor name to display on (e.g. "HDMI-1"). Empty = primary/first.
    #[serde(default)]
    pub monitor: Option<String>,

    #[serde(default)]
    pub modules_left: Vec<String>,

    #[serde(default)]
    pub modules_center: Vec<String>,

    #[serde(default)]
    pub modules_right: Vec<String>,
}

fn default_position() -> String {
    "top".to_string()
}

impl BarConfig {
    /// Returns true if the bar is positioned at the bottom.
    pub fn is_bottom(&self) -> bool {
        self.position.eq_ignore_ascii_case("bottom")
    }
}

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub bar: std::collections::HashMap<String, BarConfig>,

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
    pub foreground: Option<String>,

    #[serde(default)]
    pub background: Option<String>,

    #[serde(default)]
    pub padding_left: Option<f64>,

    #[serde(default)]
    pub padding_right: Option<f64>,

    #[serde(default)]
    pub icon: Option<String>,

    #[serde(default)]
    pub icon_unknown: Option<String>,

    #[serde(default)]
    pub icon_unavailable: Option<String>,

    #[serde(default)]
    pub icon_muted: Option<String>,

    #[serde(default)]
    pub icon_low: Option<String>,

    #[serde(default)]
    pub icon_medium: Option<String>,

    #[serde(default)]
    pub icon_high: Option<String>,

    #[serde(default)]
    pub icon_full: Option<String>,

    #[serde(default)]
    pub labels: Option<Vec<String>>,

    #[serde(default)]
    pub count: Option<u32>,

    #[serde(default)]
    pub icon_size: Option<u32>,

    #[serde(default)]
    pub slider_width: Option<u32>,

    #[serde(default)]
    pub icon_gap: Option<u32>,

    #[serde(default)]
    pub icon_spacing: Option<f64>,

    #[serde(default)]
    pub max_volume: Option<u16>,

    #[serde(default)]
    pub max_brightness: Option<u16>,

    #[serde(default)]
    pub backend: Option<String>,

    #[serde(default)]
    pub device: Option<String>,

    #[serde(default)]
    pub interface: Option<String>,

    #[serde(default)]
    pub sensor: Option<String>,

    #[serde(default)]
    pub warn_threshold: Option<f32>,

    #[serde(default)]
    pub critical_threshold: Option<f32>,

    #[serde(default)]
    pub active_color: Option<String>,

    #[serde(default)]
    pub occupied_color: Option<String>,

    #[serde(default)]
    pub empty_color: Option<String>,

    #[serde(default)]
    pub filled_color: Option<String>,

    #[serde(default)]
    pub muted_color: Option<String>,

    #[serde(default)]
    pub unavailable_color: Option<String>,

    #[serde(default)]
    pub warn_color: Option<String>,

    #[serde(default)]
    pub critical_color: Option<String>,

    #[serde(default)]
    pub glyph_left: Option<String>,

    #[serde(default)]
    pub glyph_right: Option<String>,

    #[serde(default)]
    pub glyph_filled: Option<String>,

    #[serde(default)]
    pub glyph_empty: Option<String>,
}

fn default_height() -> u32 {
    30
}

fn default_background() -> String {
    DEFAULT_BACKGROUND.to_string()
}

fn default_foreground() -> String {
    DEFAULT_FOREGROUND.to_string()
}

fn default_font() -> String {
    DEFAULT_FONT_FAMILY.to_string()
}

fn default_font_size() -> f64 {
    DEFAULT_FONT_SIZE
}

fn default_padding() -> f64 {
    DEFAULT_PADDING
}

impl Config {
    pub fn load() -> Self {
        let paths = config_search_paths();

        for path in &paths {
            if path.exists() {
                log::info!("Loading config from {}", path.display());
                let content = fs::read_to_string(path).expect("Failed to read config");
                return toml::from_str(&content).expect("Failed to parse config");
            }
        }

        log::warn!("No config file found in any searched location, using bundled defaults");
        toml::from_str(BUNDLED_DEFAULT_CONFIG).expect("Failed to parse bundled default config")
    }
}

impl BarConfig {
    pub fn resolved_font_family(&self) -> String {
        let family = self.font.trim();
        if family.is_empty() {
            default_font()
        } else {
            family.to_string()
        }
    }

    pub fn resolved_font_size(&self) -> f64 {
        if self.font_size.is_finite() && self.font_size > 0.0 {
            self.font_size
        } else {
            default_font_size()
        }
    }

    pub fn resolved_background_color(&self) -> Color {
        resolve_color(
            Some(self.background.as_str()),
            Color::from_hex(DEFAULT_BACKGROUND).unwrap_or(Color::BLACK),
            "bar.background",
        )
    }

    pub fn resolved_foreground_color(&self) -> Color {
        resolve_color(
            Some(self.foreground.as_str()),
            Color::from_hex(DEFAULT_FOREGROUND).unwrap_or(Color::WHITE),
            "bar.foreground",
        )
    }

    pub fn resolved_padding(&self) -> (f64, f64) {
        (
            resolve_length(
                Some(self.padding_left),
                default_padding(),
                "bar.padding_left",
            ),
            resolve_length(
                Some(self.padding_right),
                default_padding(),
                "bar.padding_right",
            ),
        )
    }
}

pub fn resolve_color(raw: Option<&str>, fallback: Color, context: &str) -> Color {
    resolve_optional_color(raw, context).unwrap_or(fallback)
}

pub fn resolve_optional_color(raw: Option<&str>, context: &str) -> Option<Color> {
    let value = raw?.trim();
    if value.is_empty() {
        return None;
    }

    match Color::from_hex(value) {
        Some(color) => Some(color),
        None => {
            log::warn!("Invalid color '{}' for {}, ignoring it", value, context);
            None
        }
    }
}

pub fn resolve_length(raw: Option<f64>, fallback: f64, context: &str) -> f64 {
    match raw {
        Some(value) if value.is_finite() && value >= 0.0 => value,
        Some(value) => {
            log::warn!(
                "Invalid size '{}' for {}, using {}",
                value,
                context,
                fallback
            );
            fallback
        }
        None => fallback,
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

fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(path) = std::env::var("BEEBAR_CONFIG") {
        push_unique(&mut paths, PathBuf::from(path));
    }

    if let Some(path) = dirs_maybe() {
        push_unique(&mut paths, path);
    }

    if let Ok(cwd) = std::env::current_dir() {
        for ancestor in cwd.ancestors() {
            push_unique(&mut paths, ancestor.join("config/config.toml"));
        }
    } else {
        push_unique(&mut paths, PathBuf::from("config/config.toml"));
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            for ancestor in parent.ancestors() {
                push_unique(&mut paths, ancestor.join("config/config.toml"));
            }
        }
    }

    paths
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

#[cfg(test)]
mod tests {
    use super::Config;
    use crate::renderer::color::Color;

    #[test]
    fn parses_custom_font_settings() {
        let config: Config = toml::from_str(
            r#"
                height = 30
                font = "JetBrains Mono"
                font_size = 16.5
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.resolved_font_family(), "JetBrains Mono");
        assert_eq!(config.resolved_font_size(), 16.5);
    }

    #[test]
    fn falls_back_for_blank_or_invalid_font_settings() {
        let config: Config = toml::from_str(
            r#"
                height = 30
                font = "   "
                font_size = 0.0
            "#,
        )
        .expect("config should parse");

        assert_eq!(config.resolved_font_family(), "monospace");
        assert_eq!(config.resolved_font_size(), 14.0);
    }

    #[test]
    fn parses_bar_colors_and_padding() {
        let config: Config = toml::from_str(
            r##"
                height = 30
                background = "#112233"
                foreground = "#ddeeff"
                padding_left = 12.0
                padding_right = 6.0
            "##,
        )
        .expect("config should parse");

        assert_eq!(
            config.resolved_background_color(),
            Color::from_hex("#112233").unwrap()
        );
        assert_eq!(
            config.resolved_foreground_color(),
            Color::from_hex("#ddeeff").unwrap()
        );
        assert_eq!(config.resolved_padding(), (12.0, 6.0));
    }
}
