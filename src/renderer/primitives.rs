use super::color::Color;

/// Basic rectangle.
#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A 2D point.
#[derive(Debug, Clone, Copy)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Style for rendered text.
#[derive(Debug, Clone)]
pub struct TextStyle {
    pub font_family: String,
    pub font_size: f64,
    pub color: Color,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: "monospace".to_string(),
            font_size: 14.0,
            color: Color::WHITE,
        }
    }
}

/// The renderer trait – anything that can draw primitives onto a surface.
pub trait Renderer {
    /// Begin a new frame with the given dimensions.
    fn begin(&mut self, width: u32, height: u32);

    /// Fill a rectangle.
    fn draw_rect(&mut self, rect: Rect, color: Color);

    /// Fill a rectangle with rounded corners. Default falls back to
    /// `draw_rect` so renderers without arc support degrade gracefully.
    fn draw_rounded_rect(&mut self, rect: Rect, radius: f64, color: Color) {
        let _ = radius;
        self.draw_rect(rect, color);
    }

    /// Draw text and return the width it occupied.
    fn draw_text(&mut self, pos: Point, text: &str, style: &TextStyle) -> f64;

    /// Measure the width of text without drawing.
    fn measure_text(&self, text: &str, style: &TextStyle) -> f64;

    /// Measure the height of text without drawing.
    fn measure_text_height(&self, text: &str, style: &TextStyle) -> f64;

    /// Finish the frame.
    fn end(&mut self);

    /// Get the underlying pixel data (ARGB32, stride = width * 4).
    fn data(&self) -> &[u8];

    /// Draw an ARGB32 icon at the given position, scaled to `size`×`size` pixels.
    /// Default implementation is a no-op so existing renderers compile unchanged.
    fn draw_icon(&mut self, pos: Point, pixels: &[u8], src_width: u32, src_height: u32, size: u32) {
        let _ = (pos, pixels, src_width, src_height, size);
    }
}
