use std::collections::HashMap;

use super::event::ClickEvent;
use super::layout::{BarLayout, ModuleRegion};
use super::module::{Module, ModuleId};

/// Central bar state – owns all modules and the layout.
pub struct Bar {
    pub modules: HashMap<ModuleId, Box<dyn Module>>,
    pub layout: BarLayout,
    pub height: u32,
    pub width: u32,
}

impl Bar {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            modules: HashMap::new(),
            layout: BarLayout::default(),
            height,
            width,
        }
    }

    /// Register a module and place it in the given alignment section.
    pub fn add_module(
        &mut self,
        id: impl Into<ModuleId>,
        module: Box<dyn Module>,
        section: super::layout::Alignment,
    ) {
        let id = id.into();
        self.modules.insert(id.clone(), module);
        match section {
            super::layout::Alignment::Left => self.layout.left.push(id),
            super::layout::Alignment::Center => self.layout.center.push(id),
            super::layout::Alignment::Right => self.layout.right.push(id),
        }
    }

    /// Update every module.
    pub fn update_all(&mut self) {
        for module in self.modules.values_mut() {
            module.update();
        }
    }

    /// Dispatch a click to the module whose region contains the click.
    pub fn handle_click(&mut self, regions: &[ModuleRegion], event: &ClickEvent) {
        for region in regions {
            if event.x >= region.x && event.x < region.x + region.width {
                if let Some(module) = self.modules.get_mut(&region.id) {
                    // Pass x relative to this module's own left edge so each
                    // module can determine which internal slot was clicked.
                    let rel = ClickEvent {
                        x: event.x - region.x,
                        y: event.y,
                        button: event.button,
                    };
                    module.click(rel);
                }
                break;
            }
        }
    }
}
