use std::fs;

use super::{Module, ModuleView};
use crate::renderer::primitives::TextStyle;

pub struct BatteryModule {
    capacity: u8,
    status: String,
}

impl BatteryModule {
    pub fn new() -> Self {
        Self {
            capacity: 0,
            status: String::from("Unknown"),
        }
    }

    fn read_sysfs(path: &str) -> Option<String> {
        fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }
}

impl Module for BatteryModule {
    fn update(&mut self) {
        if let Some(cap) = Self::read_sysfs("/sys/class/power_supply/BAT0/capacity") {
            self.capacity = cap.parse().unwrap_or(0);
        }
        if let Some(status) = Self::read_sysfs("/sys/class/power_supply/BAT0/status") {
            self.status = status;
        }
    }

    fn view(&self) -> ModuleView {
        let icon = match self.capacity {
            0..=20 => "",
            21..=50 => "",
            51..=80 => "",
            _ => "",
        };
        ModuleView {
            text: format!("{} {}%", icon, self.capacity),
            style: TextStyle::default(),
            ..Default::default()
        }
    }

    fn interval_ms(&self) -> Option<u64> {
        Some(10_000)
    }
}
