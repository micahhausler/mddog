use crate::render::page::Bitmap;
use crate::render::theme::rgb;
use core_graphics::base::kCGImageAlphaPremultipliedLast;
use core_graphics::color_space::{CGColorSpace, kCGColorSpaceDisplayP3};
use core_graphics::context::{CGContext, CGLineCap, CGLineJoin, CGPathDrawingMode};
use core_graphics::data_provider::CGDataProvider;
use core_graphics::geometry::{CGPoint, CGRect, CGSize};
use core_graphics::image::CGImage;
use std::marker::PhantomData;
use std::sync::Arc;

pub struct Canvas<'a> {
    pub ctx: CGContext,
    out: PhantomData<&'a mut [u8]>,
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect {
    CGRect::new(&CGPoint::new(x, y), &CGSize::new(w, h))
}

fn cg_image(b: &Bitmap) -> CGImage {
    let cs = CGColorSpace::create_device_rgb();
    let prov = CGDataProvider::from_buffer(Arc::new(b.px.clone()));
    CGImage::new(b.w as usize, b.h as usize, 8, 32, b.w as usize * 4, &cs, kCGImageAlphaPremultipliedLast, &prov, true, 0)
}

impl<'a> Canvas<'a> {
    pub fn new(out: &'a mut [u8], w: usize, h: usize, z: f64, top: f32, smooth: bool, display_p3: bool) -> Canvas<'a> {
        // CoreGraphics takes DeviceRGB colors as sRGB, so a Display P3 context converts every fill, glyph and image.
        let cs = if display_p3 {
            // SAFETY: a constant CoreGraphics exports, read once and never written.
            let p3 = unsafe { kCGColorSpaceDisplayP3 };
            CGColorSpace::create_with_name(p3).unwrap_or_else(CGColorSpace::create_device_rgb)
        } else {
            CGColorSpace::create_device_rgb()
        };
        // CoreGraphics draws into `out`, which holds w * h * 4 bytes and outlives the context: the canvas borrows it.
        let ctx = CGContext::create_bitmap_context(Some(out.as_mut_ptr().cast()), w, h, 8, w * 4, &cs, kCGImageAlphaPremultipliedLast);
        ctx.set_allows_font_smoothing(smooth);
        ctx.set_should_smooth_fonts(smooth);
        ctx.set_allows_antialiasing(true);
        ctx.set_should_antialias(true);
        ctx.set_allows_font_subpixel_positioning(true);
        ctx.set_should_subpixel_position_fonts(true);
        ctx.set_allows_font_subpixel_quantization(false);
        ctx.translate(0.0, h as f64);
        ctx.scale(1.0, -1.0);
        ctx.scale(z, z);
        ctx.translate(0.0, -(top as f64));
        Canvas { ctx, out: PhantomData }
    }

    pub fn save(&mut self) {
        self.ctx.save();
    }
    pub fn restore(&mut self) {
        self.ctx.restore();
    }
    pub fn clip(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.ctx.clip_to_rect(rect(x, y, w, h));
    }
    pub fn fill_color(&mut self, rgbv: u32, a: f64) {
        let (r, g, b) = rgb(rgbv);
        self.ctx.set_rgb_fill_color(r, g, b, a);
    }
    pub fn stroke_color(&mut self, rgbv: u32, a: f64) {
        let (r, g, b) = rgb(rgbv);
        self.ctx.set_rgb_stroke_color(r, g, b, a);
    }
    pub fn line_width(&mut self, w: f64) {
        self.ctx.set_line_width(w);
    }
    pub fn round_lines(&mut self) {
        self.ctx.set_line_cap(CGLineCap::CGLineCapRound);
        self.ctx.set_line_join(CGLineJoin::CGLineJoinRound);
    }
    pub fn alpha(&mut self, a: f64) {
        self.ctx.set_alpha(a);
    }
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.ctx.fill_rect(rect(x, y, w, h));
    }
    pub fn begin_path(&mut self) {
        self.ctx.begin_path();
    }
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.ctx.move_to_point(x, y);
    }
    pub fn line_to(&mut self, x: f64, y: f64) {
        self.ctx.add_line_to_point(x, y);
    }
    pub fn curve_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) {
        self.ctx.add_curve_to_point(x1, y1, x2, y2, x, y);
    }
    pub fn close_path(&mut self) {
        self.ctx.close_path();
    }
    pub fn fill_path(&mut self) {
        self.ctx.fill_path();
    }
    pub fn stroke_path(&mut self) {
        self.ctx.stroke_path();
    }
    pub fn fill_stroke_path(&mut self) {
        self.ctx.draw_path(CGPathDrawingMode::CGPathFillStroke);
    }
    pub fn image(&mut self, x: f64, y: f64, w: f64, h: f64, b: &Bitmap) {
        let img = cg_image(b);
        self.ctx.save();
        self.ctx.translate(x, y + h);
        self.ctx.scale(1.0, -1.0);
        self.ctx.draw_image(rect(0.0, 0.0, w, h), &img);
        self.ctx.restore();
    }
}
