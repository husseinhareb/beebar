use std::process::Command;

use super::{Module, ModuleView};
use crate::renderer::primitives::TextStyle;

/// A module that runs an arbitrary shell command and displays its stdout.
pub struct CustomModule {
    command: String,
    output: String,
}

impl CustomModule {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            output: String::new(),
        }
    }
}

impl Module for CustomModule {
    fn update(&mut self) {
        let result = Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .output();

        if let Ok(out) = result {
            self.output = String::from_utf8_lossy(&out.stdout).trim().to_string();
        }
    }

    fn view(&self) -> ModuleView {
        ModuleView {
            text: self.output.clone(),
            style: TextStyle::default(),
            ..Default::default()
        }
    }
}
