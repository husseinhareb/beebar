use sysinfo::System;

use super::{Module, ModuleChrome, ModuleView};
use crate::renderer::primitives::TextStyle;

const DEFAULT_CPU_ICON: &str = "";

pub struct CpuModule {
    sys: System,
    usage: f32,
    chrome: ModuleChrome,
    label: String,
}

impl CpuModule {
    pub fn new(chrome: ModuleChrome, label: Option<String>) -> Self {
        Self {
            sys: System::new(),
            usage: 0.0,
            chrome,
            label: label
                .and_then(|value| {
                    let trimmed = value.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                })
                .unwrap_or_else(|| DEFAULT_CPU_ICON.to_string()),
        }
    }
}

impl Module for CpuModule {
    fn update(&mut self) {
        self.sys.refresh_cpu_all();
        let cpus = self.sys.cpus();
        if !cpus.is_empty() {
            self.usage = cpus.iter().map(|c| c.cpu_usage()).sum::<f32>() / cpus.len() as f32;
        }
    }

    fn view(&self) -> ModuleView {
        self.chrome.apply(ModuleView {
            text: format!("{} {:.0}%", self.label, self.usage),
            style: TextStyle::default(),
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::CpuModule;
    use crate::core::module::{Module, ModuleChrome};

    fn default_chrome() -> ModuleChrome {
        ModuleChrome {
            foreground: None,
            background: None,
            padding: (8.0, 8.0),
            icon_spacing: None,
        }
    }

    #[test]
    fn uses_default_cpu_icon() {
        let module = CpuModule::new(default_chrome(), None);

        assert!(module.view().text.starts_with(" "));
    }

    #[test]
    fn uses_configured_cpu_icon() {
        let module = CpuModule::new(default_chrome(), Some("󰻠".to_string()));

        assert!(module.view().text.starts_with("󰻠 "));
    }
}
