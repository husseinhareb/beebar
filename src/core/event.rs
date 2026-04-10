/// Mouse button identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    ScrollUp,
    ScrollDown,
}

/// A click event on the bar surface.
#[derive(Debug, Clone)]
pub struct ClickEvent {
    pub x: f64,
    pub y: f64,
    pub button: MouseButton,
}

/// Events produced by the core and consumed by the main loop.
#[derive(Debug)]
pub enum BarEvent {
    /// A module needs to be redrawn.
    Redraw,
    /// The bar should quit.
    Quit,
}
