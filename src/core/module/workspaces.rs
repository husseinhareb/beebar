use std::fs;
use std::path::PathBuf;

use super::{Module, ModuleView};
use crate::renderer::primitives::TextStyle;

/// Reads the active workspace index from a file (`/tmp/beewm_workspace`)
/// and renders numbered workspace indicators.
pub struct WorkspacesModule {
    count: u32,
    active: u32,
    state_path: PathBuf,
}

impl WorkspacesModule {
    pub fn new(count: u32) -> Self {
        Self {
            count,
            active: 1,
            state_path: PathBuf::from("/tmp/beewm_workspace"),
        }
    }
}

impl Module for WorkspacesModule {
    fn update(&mut self) {
        if let Ok(content) = fs::read_to_string(&self.state_path) {
            if let Ok(n) = content.trim().parse::<u32>() {
                if n >= 1 && n <= self.count {
                    self.active = n;
                }
            }
        }
    }

    fn view(&self) -> ModuleView {
        let mut parts = Vec::new();
        for i in 1..=self.count {
            if i == self.active {
                parts.push(format!("[{}]", i));
            } else {
                parts.push(format!(" {} ", i));
            }
        }
        ModuleView {
            text: parts.join(""),
            style: TextStyle::default(),
            background: None,
            padding: (8.0, 8.0),
            ..Default::default()
        }
    }
}
