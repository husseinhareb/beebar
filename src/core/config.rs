use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

const BUNDLED_DEFAULT_CONFIG: &str = include_str!("../../config/config.toml");

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_height")]
    pub height: u32,

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
    pub count: Option<u32>,

    #[serde(default)]
    pub icon_size: Option<u32>,
}

fn default_height() -> u32 {
    30
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
