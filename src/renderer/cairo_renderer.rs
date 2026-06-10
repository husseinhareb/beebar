use cairo::{Context, Format, ImageSurface};
use pango::{EllipsizeMode, FontDescription, Weight};
use pangocairo::functions as pangocairo;

use super::color::Color;
use super::primitives::{Point, Rect, Renderer, TextStyle};

fn build_font_desc(style: &TextStyle) -> FontDescription {
    let mut font_desc = FontDescription::from_string(&style.font_family);
    font_desc.set_absolute_size(style.font_size * pango::SCALE as f64);
    if style.bold {
        font_desc.set_weight(Weight::Bold);
    }
    font_desc
}

/// Cairo + Pango based renderer. Draws to an in-memory image surface.
pub struct CairoRenderer {
    surface: ImageSurface,
    cr: Option<Context>,
    pixel_data: Vec<u8>,
}

impl CairoRenderer {
    pub fn new(width: u32, height: u32) -> Self {
        let surface = ImageSurface::create(Format::ARgb32, width as i32, height as i32)
            .expect("failed to create cairo surface");
        Self {
            surface,
            cr: None,
            pixel_data: Vec::new(),
        }
    }
}

impl Renderer for CairoRenderer {
    fn begin(&mut self, width: u32, height: u32) {
        self.surface = ImageSurface::create(Format::ARgb32, width as i32, height as i32)
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

    fn draw_rounded_rect(&mut self, rect: Rect, radius: f64, color: Color) {
        let cr = self.cr.as_ref().expect("call begin() first");
        let r = radius
            .max(0.0)
            .min(rect.width.max(0.0) / 2.0)
            .min(rect.height.max(0.0) / 2.0);
        if r <= 0.0 || rect.width <= 0.0 || rect.height <= 0.0 {
            cr.set_source_rgba(color.r, color.g, color.b, color.a);
            cr.rectangle(rect.x, rect.y, rect.width, rect.height);
            let _ = cr.fill();
            return;
        }

        use std::f64::consts::PI;
        let x = rect.x;
        let y = rect.y;
        let w = rect.width;
        let h = rect.height;
        cr.new_sub_path();
        // Top-right corner.
        cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
        // Bottom-right corner.
        cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
        // Bottom-left corner.
        cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
        // Top-left corner.
        cr.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
        cr.close_path();

        cr.set_source_rgba(color.r, color.g, color.b, color.a);
        let _ = cr.fill();
    }

    fn draw_text(&mut self, pos: Point, text: &str, style: &TextStyle) -> f64 {
        let cr = self.cr.as_ref().expect("call begin() first");
        let layout = pangocairo::create_layout(cr);
        let font_desc = build_font_desc(style);
        layout.set_font_description(Some(&font_desc));
        layout.set_text(text);

        cr.set_source_rgba(style.color.r, style.color.g, style.color.b, style.color.a);
        cr.move_to(pos.x, pos.y);
        pangocairo::show_layout(cr, &layout);

        let (w, _) = layout.pixel_size();
        w as f64
    }

    fn draw_text_ellipsized(&mut self, rect: Rect, text: &str, style: &TextStyle) -> f64 {
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return 0.0;
        }

        let cr = self.cr.as_ref().expect("call begin() first");
        let layout = pangocairo::create_layout(cr);
        let font_desc = build_font_desc(style);
        layout.set_font_description(Some(&font_desc));
        layout.set_text(text);
        layout.set_width((rect.width * pango::SCALE as f64).round() as i32);
        layout.set_ellipsize(EllipsizeMode::End);
        layout.set_single_paragraph_mode(true);

        cr.set_source_rgba(style.color.r, style.color.g, style.color.b, style.color.a);
        cr.move_to(rect.x, rect.y);
        pangocairo::show_layout(cr, &layout);

        let (w, _) = layout.pixel_size();
        (w as f64).min(rect.width)
    }

    fn push_clip(&mut self, rect: Rect) {
        let cr = self.cr.as_ref().expect("call begin() first");
        cr.save().ok();
        if rect.width <= 0.0 || rect.height <= 0.0 {
            cr.rectangle(rect.x, rect.y, 0.0, 0.0);
        } else {
            cr.rectangle(rect.x, rect.y, rect.width, rect.height);
        }
        cr.clip();
    }

    fn pop_clip(&mut self) {
        let cr = self.cr.as_ref().expect("call begin() first");
        cr.restore().ok();
    }

    fn measure_text(&self, text: &str, style: &TextStyle) -> f64 {
        // Use the active context if available, otherwise create a temporary one
        let tmp_cr;
        let cr = match self.cr.as_ref() {
            Some(cr) => cr,
            None => {
                let tmp_surface = ImageSurface::create(Format::ARgb32, 1, 1).expect("tmp surface");
                tmp_cr = Context::new(&tmp_surface).expect("tmp context");
                &tmp_cr
            }
        };
        let layout = pangocairo::create_layout(cr);
        let font_desc = build_font_desc(style);
        layout.set_font_description(Some(&font_desc));
        layout.set_text(text);
        let (w, _) = layout.pixel_size();
        w as f64
    }

    fn measure_text_height(&self, text: &str, style: &TextStyle) -> f64 {
        let tmp_cr;
        let cr = match self.cr.as_ref() {
            Some(cr) => cr,
            None => {
                let tmp_surface = ImageSurface::create(Format::ARgb32, 1, 1).expect("tmp surface");
                tmp_cr = Context::new(&tmp_surface).expect("tmp context");
                &tmp_cr
            }
        };
        let layout = pangocairo::create_layout(cr);
        let font_desc = build_font_desc(style);
        layout.set_font_description(Some(&font_desc));
        layout.set_text(text);
        let (_, h) = layout.pixel_size();
        h as f64
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

    fn draw_icon(&mut self, pos: Point, pixels: &[u8], src_width: u32, src_height: u32, size: u32) {
        let cr = match self.cr.as_ref() {
            Some(c) => c,
            None => return,
        };

        // Build a Cairo ImageSurface by copying the raw ARGB32 pixels.
        // We use a temporary surface + context to blit the data in, so
        // `create_for_data` borrow issues are avoided.
        let expected = (src_width * src_height * 4) as usize;
        if pixels.len() < expected {
            return;
        }
        let stride = cairo::Format::ARgb32
            .stride_for_width(src_width)
            .unwrap_or(src_width as i32 * 4) as usize;
        // Create a blank surface and write pixels row by row.
        let mut icon_surface = match cairo::ImageSurface::create(
            cairo::Format::ARgb32,
            src_width as i32,
            src_height as i32,
        ) {
            Ok(s) => s,
            Err(_) => return,
        };
        {
            let mut surf_data = match icon_surface.data() {
                Ok(d) => d,
                Err(_) => return,
            };
            let src_stride = src_width as usize * 4;
            for row in 0..src_height as usize {
                let src_off = row * src_stride;
                let dst_off = row * stride;
                let copy_len = src_stride
                    .min(stride)
                    .min(surf_data.len().saturating_sub(dst_off));
                if src_off + copy_len <= pixels.len() {
                    surf_data[dst_off..dst_off + copy_len]
                        .copy_from_slice(&pixels[src_off..src_off + copy_len]);
                }
            }
        } // surf_data borrow dropped here, surface is now usable

        cr.save().ok();

        // Preserve aspect ratio: scale uniformly by the larger dimension so the
        // icon fits inside the size×size slot without stretching, then center
        // the (possibly non-square) result within the slot. Scaling X and Y
        // independently would distort non-square pixmaps (e.g. some Steam /
        // Telegram tray icons).
        let src_w = src_width as f64;
        let src_h = src_height as f64;
        let scale = (size as f64 / src_w).min(size as f64 / src_h);
        let scaled_w = src_w * scale;
        let scaled_h = src_h * scale;
        let offset_x = (size as f64 - scaled_w) / 2.0;
        let offset_y = (size as f64 - scaled_h) / 2.0;

        cr.translate(pos.x + offset_x, pos.y + offset_y);
        cr.scale(scale, scale);

        cr.set_source_surface(&icon_surface, 0.0, 0.0).ok();
        // Set bilinear filter on the source pattern for smooth scaling.
        cr.source().set_filter(cairo::Filter::Bilinear);
        cr.rectangle(0.0, 0.0, src_w, src_h);
        cr.fill().ok();
        cr.restore().ok();
    }
}
