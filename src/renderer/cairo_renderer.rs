use cairo::{Context, Format, ImageSurface};
use pango::FontDescription;
use pangocairo::functions as pangocairo;

use super::color::Color;
use super::primitives::{Point, Rect, Renderer, TextStyle};

/// Cairo + Pango based renderer. Draws to an in-memory image surface.
pub struct CairoRenderer {
    surface: ImageSurface,
    cr: Option<Context>,
    pixel_data: Vec<u8>,
}

impl CairoRenderer {
    pub fn new(width: u32, height: u32) -> Self {
        let surface =
            ImageSurface::create(Format::ARgb32, width as i32, height as i32)
                .expect("failed to create cairo surface");
        Self { surface, cr: None, pixel_data: Vec::new() }
    }
}

impl Renderer for CairoRenderer {
    fn begin(&mut self, width: u32, height: u32) {
        self.surface =
            ImageSurface::create(Format::ARgb32, width as i32, height as i32)
                .expect("failed to create cairo surface");
        let cr = Context::new(&self.surface).expect("failed to create cairo context");
        // Clear
        cr.set_operator(cairo::Operator::Clear);
        cr.paint().unwrap();
        cr.set_operator(cairo::Operator::Over);
        self.cr = Some(cr);
    }

    fn draw_rect(&mut self, rect: Rect, color: Color) {
        let cr = self.cr.as_ref().expect("call begin() first");
        cr.set_source_rgba(color.r, color.g, color.b, color.a);
        cr.rectangle(rect.x, rect.y, rect.width, rect.height);
        let _ = cr.fill();
    }

    fn draw_text(&mut self, pos: Point, text: &str, style: &TextStyle) -> f64 {
        let cr = self.cr.as_ref().expect("call begin() first");
        let layout = pangocairo::create_layout(cr);
        let mut font_desc = FontDescription::new();
        font_desc.set_family(&style.font_family);
        font_desc.set_absolute_size(style.font_size * pango::SCALE as f64);
        layout.set_font_description(Some(&font_desc));
        layout.set_text(text);

        cr.set_source_rgba(style.color.r, style.color.g, style.color.b, style.color.a);
        cr.move_to(pos.x, pos.y);
        pangocairo::show_layout(cr, &layout);

        let (w, _) = layout.pixel_size();
        w as f64
    }

    fn measure_text(&self, text: &str, style: &TextStyle) -> f64 {
        // Use the active context if available, otherwise create a temporary one
        let tmp_cr;
        let cr = match self.cr.as_ref() {
            Some(cr) => cr,
            None => {
                let tmp_surface =
                    ImageSurface::create(Format::ARgb32, 1, 1).expect("tmp surface");
                tmp_cr = Context::new(&tmp_surface).expect("tmp context");
                &tmp_cr
            }
        };
        let layout = pangocairo::create_layout(cr);
        let mut font_desc = FontDescription::new();
        font_desc.set_family(&style.font_family);
        font_desc.set_absolute_size(style.font_size * pango::SCALE as f64);
        layout.set_font_description(Some(&font_desc));
        layout.set_text(text);
        let (w, _) = layout.pixel_size();
        w as f64
    }

    fn end(&mut self) {
        self.cr = None;
        self.surface.flush();
        let data = self.surface.data().expect("failed to get surface data");
        self.pixel_data = data.to_vec();
    }

    fn data(&self) -> &[u8] {
        &self.pixel_data
    }
}
