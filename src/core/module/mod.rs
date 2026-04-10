pub mod battery;
pub mod clock;
pub mod cpu;
pub mod custom;

use crate::core::event::ClickEvent;
use crate::renderer::color::Color;
use crate::renderer::primitives::TextStyle;

/// Unique identifier for a module instance.
pub type ModuleId = String;

/// The visual output of a module – what the renderer should draw.
#[derive(Debug, Clone)]
pub struct ModuleView {
    pub text: String,
    pub style: TextStyle,
    pub background: Option<Color>,
    pub padding: (f64, f64), // (left, right)
}

impl Default for ModuleView {
    fn default() -> Self {
        Self {
            text: String::new(),
            style: TextStyle::default(),
            background: None,
            padding: (8.0, 8.0),
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

    /// Desired update interval in milliseconds. `None` means event-driven only.
    fn interval_ms(&self) -> Option<u64> {
        Some(1000)
    }
}
