use chrono::Local;

use super::{Module, ModuleChrome, ModuleView};
use crate::renderer::primitives::TextStyle;

pub struct ClockModule {
    format: String,
    current: String,
    chrome: ModuleChrome,
}

impl ClockModule {
    pub fn new(format: impl Into<String>, chrome: ModuleChrome) -> Self {
        Self {
            format: format.into(),
            current: String::new(),
            chrome,
        }
    }
}

impl Module for ClockModule {
    fn update(&mut self) {
        self.current = Local::now().format(&self.format).to_string();
    }

    fn view(&self) -> ModuleView {
        self.chrome.apply(ModuleView {
            text: self.current.clone(),
            style: TextStyle::default(),
            ..Default::default()
        })
    }
}
