use crate::core::bar::Bar;

/// Trait that both Wayland and X11 backends implement.
pub trait Backend {
    /// Run the event loop. This blocks until the bar exits.
    fn run(&mut self, bar: &mut Bar);

    /// Request a redraw of the bar surface.
    fn request_redraw(&self);
}
