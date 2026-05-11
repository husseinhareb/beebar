/// Which mouse button triggered a click. Wheel motion is folded into the
/// same enum so it flows through the existing click pipeline — modules opt
/// into wheel handling by matching `ScrollUp` / `ScrollDown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    ScrollUp,
    ScrollDown,
    Other(u32),
}

/// A click event on the bar surface.
#[derive(Debug, Clone)]
pub struct ClickEvent {
    /// X coordinate relative to the module's own left edge (0 = module left).
    pub x: f64,
    /// X coordinate relative to the bar surface.
    pub bar_x: f64,
    /// X coordinate in output/screen space, preserved across module routing.
    pub screen_x: f64,
    /// Width of the module region in pixels.
    pub module_width: f64,
    /// Y coordinate relative to the bar surface.
    pub y: f64,
    /// Y coordinate relative to the bar surface.
    pub bar_y: f64,
    /// Y coordinate in output/screen space, preserved across module routing.
    pub screen_y: f64,
    pub button: MouseButton,
}
