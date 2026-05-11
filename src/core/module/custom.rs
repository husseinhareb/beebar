use std::process::Command;

use super::{Module, ModuleChrome, ModuleView};
use crate::renderer::primitives::TextStyle;

/// A module that runs an arbitrary shell command and displays its stdout.
pub struct CustomModule {
    command: String,
    output: String,
    chrome: ModuleChrome,
}

impl CustomModule {
    pub fn new(command: impl Into<String>, chrome: ModuleChrome) -> Self {
        Self {
            command: command.into(),
            output: String::new(),
            chrome,
        }
    }
}

impl Module for CustomModule {
    fn update(&mut self) {
        let result = Command::new("sh").arg("-c").arg(&self.command).output();

        if let Ok(out) = result {
            self.output = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
    }

    fn update_interval(&self) -> std::time::Duration {
        // Custom shell commands can be expensive; users can dial it back via
        // `refresh_interval_ms` on the module.
        self.chrome
            .update_interval
            .unwrap_or(std::time::Duration::from_secs(1))
    }

    fn view(&self) -> ModuleView {
        self.chrome.apply(ModuleView {
            text: self.output.clone(),
            style: TextStyle::default(),
            ..Default::default()
        })
    }
}
