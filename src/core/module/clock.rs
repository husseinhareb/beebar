use chrono::Local;

use super::{Module, ModuleView};
use crate::renderer::primitives::TextStyle;

pub struct ClockModule {
    format: String,
    current: String,
}

impl ClockModule {
    pub fn new(format: impl Into<String>) -> Self {
        Self {
            format: format.into(),
            current: String::new(),
        }
    }
}

impl Module for ClockModule {
    fn update(&mut self) {
        self.current = Local::now().format(&self.format).to_string();
    }

    fn view(&self) -> ModuleView {
        ModuleView {
            text: self.current.clone(),
            style: TextStyle::default(),
            ..Default::default()
        }
    }

    fn interval_ms(&self) -> Option<u64> {
        Some(1000)
    }
}
