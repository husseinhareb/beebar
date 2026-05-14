use std::fs;

use super::{Module, ModuleChrome, ModuleView, prefix_text};
use crate::core::config::ModuleConfig;
use crate::renderer::primitives::TextStyle;

#[derive(Debug, Clone)]
pub struct BatteryIcons {
    pub low: String,
    pub medium: String,
    pub high: String,
    pub full: String,
}

impl Default for BatteryIcons {
    fn default() -> Self {
        Self {
            low: "󰁻".to_string(),
            medium: "󰁾".to_string(),
            high: "󰂁".to_string(),
            full: "󰁹".to_string(),
        }
    }
}

impl BatteryIcons {
    pub fn from_config(config: &ModuleConfig) -> Self {
        let mut icons = Self::default();

        if let Some(value) = &config.icon_low {
            icons.low = value.clone();
        }
        if let Some(value) = &config.icon_medium {
            icons.medium = value.clone();
        }
        if let Some(value) = &config.icon_high {
            icons.high = value.clone();
        }
        if let Some(value) = &config.icon_full {
            icons.full = value.clone();
        }

        icons
    }
}

pub struct BatteryModule {
    capacity: u8,
    status: String,
    icons: BatteryIcons,
    chrome: ModuleChrome,
    device: String,
}

impl BatteryModule {
    pub fn new(icons: BatteryIcons, chrome: ModuleChrome, device: Option<String>) -> Self {
        let detected = device.unwrap_or_else(|| Self::detect_battery().unwrap_or_else(|| "BAT0".to_string()));
        Self {
            capacity: 0,
            status: String::from("Unknown"),
            icons,
            chrome,
            device: detected,
        }
    }

    fn read_sysfs(path: &str) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    fn detect_battery() -> Option<String> {
        let entries = fs::read_dir("/sys/class/power_supply").ok()?;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let type_path = format!("/sys/class/power_supply/{}/type", name);
            if Self::read_sysfs(&type_path).as_deref() == Some("Battery") {
                return Some(name);
            }
        }
        None
    }
}

impl Module for BatteryModule {
    fn update(&mut self) {
        let base = format!("/sys/class/power_supply/{}", self.device);
        if let Some(cap) = Self::read_sysfs(&format!("{}/capacity", base)) {
            self.capacity = cap.parse().unwrap_or(0);
        }
        if let Some(status) = Self::read_sysfs(&format!("{}/status", base)) {
            self.status = status;
        }
    }

    fn update_interval(&self) -> std::time::Duration {
        self.chrome
            .update_interval
            .unwrap_or(std::time::Duration::from_secs(1))
    }

    fn view(&self) -> ModuleView {
        let icon = match self.capacity {
            0..=20 => &self.icons.low,
            21..=50 => &self.icons.medium,
            51..=80 => &self.icons.high,
            _ => &self.icons.full,
        };
        self.chrome.apply(ModuleView {
            text: prefix_text(icon, &format!("{}%", self.capacity)),
            style: TextStyle::default(),
            ..Default::default()
        })
    }
}
