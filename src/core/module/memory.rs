use sysinfo::System;

use super::{Module, ModuleChrome, ModuleView};
use crate::renderer::primitives::TextStyle;

const DEFAULT_MEMORY_ICON: &str = "󰍛";
const DEFAULT_FORMAT: &str = "{icon} {used}/{total}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct MemoryState {
    used_bytes: u64,
    total_bytes: u64,
    available_bytes: u64,
}

pub struct MemoryModule {
    sys: System,
    state: MemoryState,
    chrome: ModuleChrome,
    format: String,
    icon: String,
}

impl MemoryModule {
    pub fn new(format: Option<String>, icon: Option<String>, chrome: ModuleChrome) -> Self {
        Self {
            sys: System::new(),
            state: MemoryState::default(),
            chrome,
            format: resolve_format(format),
            icon: normalize_optional_string(icon)
                .unwrap_or_else(|| DEFAULT_MEMORY_ICON.to_string()),
        }
    }

    fn used_percent(&self) -> u64 {
        if self.state.total_bytes == 0 {
            0
        } else {
            ((self.state.used_bytes as f64 / self.state.total_bytes as f64) * 100.0).round() as u64
        }
    }

    fn render_text(&self) -> String {
        let used = format_bytes(self.state.used_bytes);
        let total = format_bytes(self.state.total_bytes);
        let available = format_bytes(self.state.available_bytes);
        let percent = self.used_percent().to_string();

        self.format
            .replace("{icon}", &self.icon)
            .replace("{used}", &used)
            .replace("{total}", &total)
            .replace("{available}", &available)
            .replace("{percent}", &percent)
    }
}

impl Module for MemoryModule {
    fn update(&mut self) {
        self.sys.refresh_memory();
        self.state = MemoryState {
            used_bytes: self.sys.used_memory(),
            total_bytes: self.sys.total_memory(),
            available_bytes: self.sys.available_memory(),
        };
    }

    fn view(&self) -> ModuleView {
        self.chrome.apply(ModuleView {
            text: self.render_text(),
            style: TextStyle::default(),
            ..Default::default()
        })
    }
}

fn normalize_optional_string(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

fn resolve_format(format: Option<String>) -> String {
    match format {
        Some(format) if !format.trim().is_empty() => format,
        _ => DEFAULT_FORMAT.to_string(),
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];

    let mut value = bytes as f64;
    let mut unit_index = 0usize;

    while value >= 1024.0 && unit_index + 1 < UNITS.len() {
        value /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{bytes} {}", UNITS[unit_index])
    } else if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit_index])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit_index])
    } else {
        format!("{value:.2} {}", UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::module::{Module, ModuleChrome};

    const GIB: u64 = 1024 * 1024 * 1024;

    fn default_chrome() -> ModuleChrome {
        ModuleChrome {
            foreground: None,
            background: None,
            padding: (8.0, 8.0),
            icon_spacing: None,
        }
    }

    #[test]
    fn uses_default_memory_icon() {
        let module = MemoryModule::new(None, None, default_chrome());

        assert!(module.view().text.starts_with("󰍛 "));
    }

    #[test]
    fn uses_configured_memory_icon() {
        let module = MemoryModule::new(None, Some("MEM".to_string()), default_chrome());

        assert!(module.view().text.starts_with("MEM "));
    }

    #[test]
    fn formats_memory_sizes_with_binary_units() {
        assert_eq!(format_bytes(999), "999 B");
        assert_eq!(format_bytes(1_536), "1.50 KiB");
        assert_eq!(format_bytes(5 * GIB), "5.00 GiB");
    }

    #[test]
    fn renders_configured_format_tokens() {
        let mut module = MemoryModule::new(
            Some("{icon} {used}/{total} free:{available} {percent}%".to_string()),
            Some("RAM".to_string()),
            default_chrome(),
        );
        module.state = MemoryState {
            used_bytes: 3 * GIB,
            total_bytes: 8 * GIB,
            available_bytes: 5 * GIB,
        };

        assert_eq!(
            module.view().text,
            "RAM 3.00 GiB/8.00 GiB free:5.00 GiB 38%"
        );
    }

    #[test]
    fn keeps_user_format_whitespace() {
        let module = MemoryModule::new(
            Some("  {icon} {percent}%  ".to_string()),
            None,
            default_chrome(),
        );

        assert_eq!(module.view().text, "  󰍛 0%  ");
    }
}
