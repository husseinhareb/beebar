pub mod battery;
pub mod clock;
pub mod cpu;
pub mod custom;
pub mod tray;
pub mod workspaces;

use crate::core::event::ClickEvent;
use crate::renderer::color::Color;
use crate::renderer::primitives::TextStyle;

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
pub struct ModuleView {
    pub text: String,
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
            style: TextStyle::default(),
            background: None,
            padding: (8.0, 8.0),
            icons: Vec::new(),
            icon_spacing: 4.0,
        }
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
