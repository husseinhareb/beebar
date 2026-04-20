use std::collections::HashMap;

use super::event::ClickEvent;
use super::layout::{BarLayout, ModuleRegion};
use super::module::{Module, ModuleId, ModuleView};
use crate::renderer::color::Color;
use crate::renderer::primitives::TextStyle;

/// Central bar state – owns all modules and the layout.
pub struct Bar {
    /// Human-readable name of this bar (from config key).
    pub name: String,
    pub modules: HashMap<ModuleId, Box<dyn Module>>,
    pub layout: BarLayout,
    pub height: u32,
    pub width: u32,
    pub background: Color,
    pub text_style: TextStyle,
    pub text_y_offset: f64,
    /// If true, the bar is anchored to the bottom of the screen.
    pub bottom: bool,
}

impl Bar {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            name: String::new(),
            modules: HashMap::new(),
            layout: BarLayout::default(),
            height,
            width,
            background: Color::BLACK,
            text_style: TextStyle::default(),
            text_y_offset: 0.0,
            bottom: false,
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

    pub fn module_view(&self, id: &ModuleId) -> Option<ModuleView> {
        self.modules
            .get(id)
            .map(|module| self.apply_text_style(module.view()))
    }

    pub fn apply_text_style(&self, mut view: ModuleView) -> ModuleView {
        let font_family = self.text_style.font_family.clone();
        let font_size = self.text_style.font_size;
        let default_color = if view.style.color == Color::WHITE {
            self.text_style.color
        } else {
            view.style.color
        };

        view.style.font_family = font_family.clone();
        view.style.font_size = font_size;
        view.style.color = default_color;

        for segment in &mut view.text_segments {
            segment.style.font_family = font_family.clone();
            segment.style.font_size = font_size;
            if segment.style.color == Color::WHITE {
                segment.style.color = default_color;
            }
        }

        view
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
                        screen_x: event.screen_x,
                        module_width: region.width,
                        y: event.y,
                        screen_y: event.screen_y,
                        button: event.button,
                    };
                    module.click(rel);
                }
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Bar;
    use crate::core::module::{ModuleView, TextSegment};
    use crate::renderer::color::Color;
    use crate::renderer::primitives::TextStyle;

    #[test]
    fn apply_text_style_preserves_colors_and_updates_fonts() {
        let mut bar = Bar::new(1920, 30);
        bar.text_style = TextStyle {
            font_family: "JetBrains Mono".to_string(),
            font_size: 16.0,
            color: Color::rgb(0.7, 0.7, 0.7),
        };

        let view = ModuleView {
            text: "cpu".to_string(),
            text_segments: vec![
                TextSegment {
                    text: "42%".to_string(),
                    style: TextStyle {
                        font_family: "serif".to_string(),
                        font_size: 10.0,
                        color: Color::rgb(0.8, 0.4, 0.2),
                    },
                },
                TextSegment {
                    text: " ok".to_string(),
                    style: TextStyle::default(),
                },
            ],
            style: TextStyle {
                font_family: "sans".to_string(),
                font_size: 11.0,
                color: Color::rgb(0.2, 0.4, 0.8),
            },
            ..Default::default()
        };

        let styled = bar.apply_text_style(view);

        assert_eq!(styled.style.font_family, "JetBrains Mono");
        assert_eq!(styled.style.font_size, 16.0);
        assert_eq!(styled.style.color.r, 0.2);
        assert_eq!(styled.style.color.g, 0.4);
        assert_eq!(styled.style.color.b, 0.8);
        assert_eq!(styled.text_segments[0].style.font_family, "JetBrains Mono");
        assert_eq!(styled.text_segments[0].style.font_size, 16.0);
        assert_eq!(styled.text_segments[0].style.color.r, 0.8);
        assert_eq!(styled.text_segments[0].style.color.g, 0.4);
        assert_eq!(styled.text_segments[0].style.color.b, 0.2);
        assert_eq!(
            styled.text_segments[1].style.color,
            Color::rgb(0.2, 0.4, 0.8)
        );
    }
}
