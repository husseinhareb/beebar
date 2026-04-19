pub mod battery;
pub mod brightness;
pub mod clock;
pub mod cpu;
pub mod custom;
pub mod tray;
pub mod volume;
pub mod workspaces;

use crate::core::config::{ModuleConfig, resolve_length, resolve_optional_color};
use crate::core::event::ClickEvent;
use crate::renderer::color::Color;
use crate::renderer::primitives::{Renderer, TextStyle};

/// Unique identifier for a module instance.
pub type ModuleId = String;

/// An icon to render: raw ARGB32 pixels, width and height in pixels.
#[derive(Debug, Clone)]
pub struct IconData {
    /// ARGB32 pixel data, row-major, length = width * height * 4.
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The visual output of a module – what the renderer should draw.
#[derive(Debug, Clone)]
pub struct TextSegment {
    pub text: String,
    pub style: TextStyle,
}

/// The visual output of a module – what the renderer should draw.
#[derive(Debug, Clone)]
pub struct ModuleView {
    pub text: String,
    pub text_segments: Vec<TextSegment>,
    pub style: TextStyle,
    pub background: Option<Color>,
    pub padding: (f64, f64), // (left, right)
    /// Icon-only slots used by modules like the system tray.
    /// When non-empty the renderer draws icons instead of (or alongside) text.
    pub icons: Vec<IconData>,
    /// Per-icon spacing (pixels between icons).
    pub icon_spacing: f64,
}

impl Default for ModuleView {
    fn default() -> Self {
        Self {
            text: String::new(),
            text_segments: Vec::new(),
            style: TextStyle::default(),
            background: None,
            padding: (8.0, 8.0),
            icons: Vec::new(),
            icon_spacing: 4.0,
        }
    }
}

impl ModuleView {
    pub fn text_width<R: Renderer>(&self, renderer: &R) -> f64 {
        if self.text_segments.is_empty() {
            renderer.measure_text(&self.text, &self.style)
        } else {
            self.text_segments
                .iter()
                .map(|segment| renderer.measure_text(&segment.text, &segment.style))
                .sum()
        }
    }

    pub fn text_height(&self) -> f64 {
        self.text_segments
            .iter()
            .map(|segment| segment.style.font_size)
            .fold(self.style.font_size, f64::max)
    }
}

pub fn char_len(text: &str) -> usize {
    text.chars().count()
}

pub fn pad_text_right(text: &str, width: usize) -> String {
    let padding = width.saturating_sub(char_len(text));
    if padding == 0 {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(padding))
    }
}

pub fn prefix_text(prefix: &str, text: &str) -> String {
    if prefix.trim().is_empty() {
        text.to_string()
    } else {
        format!("{prefix} {text}")
    }
}

#[derive(Debug, Clone)]
pub struct ModuleChrome {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub padding: (f64, f64),
    pub icon_spacing: Option<f64>,
}

impl ModuleChrome {
    pub fn from_config(config: &ModuleConfig, default_padding: (f64, f64)) -> Self {
        Self {
            foreground: resolve_optional_color(config.foreground.as_deref(), "module.foreground"),
            background: resolve_optional_color(config.background.as_deref(), "module.background"),
            padding: (
                resolve_length(
                    config.padding_left,
                    default_padding.0,
                    "module.padding_left",
                ),
                resolve_length(
                    config.padding_right,
                    default_padding.1,
                    "module.padding_right",
                ),
            ),
            icon_spacing: config.icon_spacing,
        }
    }

    pub fn apply(&self, mut view: ModuleView) -> ModuleView {
        if let Some(color) = self.foreground {
            view.style.color = color;
        }
        if let Some(color) = self.background {
            view.background = Some(color);
        }
        view.padding = self.padding;
        if let Some(spacing) = self.icon_spacing {
            view.icon_spacing = spacing;
        }
        view
    }
}

#[derive(Debug, Clone)]
pub struct SliderGlyphs {
    pub left: String,
    pub filled: String,
    pub empty: String,
    pub right: String,
}

impl SliderGlyphs {
    pub fn new(
        left: impl Into<String>,
        filled: impl Into<String>,
        empty: impl Into<String>,
        right: impl Into<String>,
    ) -> Self {
        Self {
            left: left.into(),
            filled: filled.into(),
            empty: empty.into(),
            right: right.into(),
        }
    }

    pub fn from_config(config: &ModuleConfig) -> Self {
        Self {
            left: config.glyph_left.clone().unwrap_or_else(|| "▐".to_string()),
            filled: config
                .glyph_filled
                .clone()
                .unwrap_or_else(|| "█".to_string()),
            empty: config
                .glyph_empty
                .clone()
                .unwrap_or_else(|| "░".to_string()),
            right: config
                .glyph_right
                .clone()
                .unwrap_or_else(|| "▌".to_string()),
        }
    }

    pub fn unit_chars(&self) -> usize {
        char_len(&self.filled).max(char_len(&self.empty)).max(1)
    }

    pub fn total_chars(&self, slots: usize) -> usize {
        char_len(&self.left) + char_len(&self.right) + slots * self.unit_chars()
    }
}

/// Trait that every bar module must implement.
pub trait Module: Send {
    /// Called periodically (or on event) to refresh internal state.
    fn update(&mut self);

    /// Return the current renderable view for this module.
    fn view(&self) -> ModuleView;

    /// Handle a click event on this module's area. Default: no-op.
    fn click(&mut self, _event: ClickEvent) {}
}
