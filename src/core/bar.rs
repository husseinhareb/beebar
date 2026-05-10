use std::collections::HashMap;

use super::event::ClickEvent;
use super::layout::{BarLayout, GroupRegion, LayoutSpacing, ModuleRegion};
use super::module::{Module, ModuleId, ModuleView};
use super::popup::PopupMenu;
use crate::renderer::color::Color;
use crate::renderer::primitives::{Point, Rect, Renderer, TextStyle};

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
    /// Vertical inset of pill groups (px).
    pub margin_top: f64,
    pub margin_bottom: f64,
    /// Horizontal inset of the outermost pill on each side (px).
    pub margin_left: f64,
    pub margin_right: f64,
    /// Horizontal gap between adjacent pills (px).
    pub group_spacing: f64,
    /// Corner radius for pill backgrounds (px).
    pub corner_radius: f64,
    /// Default fill color for pill groups.
    pub group_background: Option<Color>,
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
            margin_top: 0.0,
            margin_bottom: 0.0,
            margin_left: 0.0,
            margin_right: 0.0,
            group_spacing: 0.0,
            corner_radius: 0.0,
            group_background: None,
        }
    }

    /// Register a module instance under `id`. Layout placement is done
    /// separately via [`Bar::add_group`].
    pub fn add_module(&mut self, id: impl Into<ModuleId>, module: Box<dyn Module>) {
        self.modules.insert(id.into(), module);
    }

    /// Append a pill group (list of module ids) to the given side.
    pub fn add_group(
        &mut self,
        ids: Vec<ModuleId>,
        section: super::layout::Alignment,
    ) {
        if ids.is_empty() {
            return;
        }
        match section {
            super::layout::Alignment::Left => self.layout.left.push(ids),
            super::layout::Alignment::Center => self.layout.center.push(ids),
            super::layout::Alignment::Right => self.layout.right.push(ids),
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
                        bar_x: event.bar_x,
                        screen_x: event.screen_x,
                        module_width: region.width,
                        y: event.y,
                        bar_y: event.bar_y,
                        screen_y: event.screen_y,
                        button: event.button,
                    };
                    module.click(rel);
                }
                break;
            }
        }
    }

    pub fn active_popup(&self) -> Option<(ModuleId, PopupMenu)> {
        for id in self
            .layout
            .left
            .iter()
            .chain(self.layout.center.iter())
            .chain(self.layout.right.iter())
            .flatten()
        {
            let Some(module) = self.modules.get(id) else {
                continue;
            };
            if let Some(popup) = module.popup() {
                return Some((id.clone(), popup));
            }
        }

        None
    }

    pub fn handle_popup_click(
        &mut self,
        id: &ModuleId,
        item_index: usize,
        button: crate::core::event::MouseButton,
    ) {
        if let Some(module) = self.modules.get_mut(id) {
            module.popup_click(item_index, button);
        }
    }

    pub fn dismiss_all_popups(&mut self) {
        for module in self.modules.values_mut() {
            module.dismiss_popup();
        }
    }

    /// Horizontal layout spacing derived from bar config.
    pub fn spacing(&self) -> LayoutSpacing {
        LayoutSpacing {
            margin_left: self.margin_left,
            margin_right: self.margin_right,
            group_spacing: self.group_spacing,
        }
    }

    /// Vertical inset that pill groups are drawn within. Returns `(top, height)`
    /// where height is clamped to at least 1px when the margins make the bar
    /// content area smaller than the configured paddings.
    pub fn pill_band(&self) -> (f64, f64) {
        let total_h = self.height as f64;
        let inner = (total_h - self.margin_top - self.margin_bottom).max(1.0);
        (self.margin_top, inner)
    }

    /// Measure the rendered width of a module, including its padding.
    pub fn measure_module<R: Renderer>(&self, id: &ModuleId, renderer: &R, pill_height: f64) -> f64 {
        let Some(view) = self.module_view(id) else {
            return 0.0;
        };
        if !view.icons.is_empty() {
            let icon_size = view
                .icon_size
                .map(|s| s as f64)
                .unwrap_or((pill_height - 4.0).max(8.0));
            let n = view.icons.len() as f64;
            view.padding.0
                + view.padding.1
                + n * icon_size
                + (n - 1.0).max(0.0) * view.icon_spacing
        } else {
            view.text_width(renderer) + view.padding.0 + view.padding.1
        }
    }

    /// Compute the full group layout against a measurement renderer.
    pub fn compute_groups<R: Renderer>(&self, width: f64, renderer: &R) -> Vec<GroupRegion> {
        let (_, pill_h) = self.pill_band();
        let measure = |id: &ModuleId| -> f64 { self.measure_module(id, renderer, pill_h) };
        self.layout.compute(width, self.spacing(), &measure)
    }
}

/// Render the bar to a renderer. Handles pill backgrounds, modules, and
/// vertical centering within the pill band.
pub fn render_bar<R: Renderer>(bar: &Bar, renderer: &mut R, width: u32, height: u32) {
    renderer.begin(width, height);

    // Bar surface background (may be fully transparent).
    renderer.draw_rect(
        Rect {
            x: 0.0,
            y: 0.0,
            width: width as f64,
            height: height as f64,
        },
        bar.background,
    );

    let (pill_top, pill_h) = bar.pill_band();
    let groups = bar.compute_groups(width as f64, renderer);

    // Pass 1: pill group backgrounds.
    if let Some(group_bg) = bar.group_background {
        for group in &groups {
            if group.width <= 0.0 {
                continue;
            }
            renderer.draw_rounded_rect(
                Rect {
                    x: group.x,
                    y: pill_top,
                    width: group.width,
                    height: pill_h,
                },
                bar.corner_radius,
                group_bg,
            );
        }
    }

    // Pass 2: modules. Per-module backgrounds (if set) draw inside the pill.
    for group in &groups {
        for region in &group.modules {
            let Some(view) = bar.module_view(&region.id) else {
                continue;
            };

            if let Some(bg) = view.background {
                renderer.draw_rect(
                    Rect {
                        x: region.x,
                        y: pill_top,
                        width: region.width,
                        height: pill_h,
                    },
                    bg,
                );
            }

            if !view.icons.is_empty() {
                let icon_size = view
                    .icon_size
                    .map(|s| s as f64)
                    .unwrap_or((pill_h - 4.0).max(8.0));
                let mut ix = region.x + view.padding.0;
                let iy = pill_top + ((pill_h - icon_size) / 2.0).max(0.0);
                for icon_data in &view.icons {
                    renderer.draw_icon(
                        Point { x: ix, y: iy },
                        &icon_data.pixels,
                        icon_data.width,
                        icon_data.height,
                        icon_size as u32,
                    );
                    ix += icon_size + view.icon_spacing;
                }
            } else {
                let text_h = view.text_height(renderer);
                let y = pill_top + (pill_h - text_h) / 2.0 + bar.text_y_offset;
                let mut x = region.x + view.padding.0;
                if view.text_segments.is_empty() {
                    renderer.draw_text(Point { x, y }, &view.text, &view.style);
                } else {
                    for segment in &view.text_segments {
                        x += renderer.draw_text(Point { x, y }, &segment.text, &segment.style);
                    }
                }
            }
        }
    }

    renderer.end();
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
