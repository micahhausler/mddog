use crate::render::page::Bitmap;
use crate::render::theme::rgb;
use tiny_skia::{Color, FillRule, FilterQuality, LineCap, LineJoin, Mask, Paint, PathBuilder, PixmapMut, PixmapPaint, PixmapRef, Rect, Stroke, Transform};

#[derive(Clone)]
struct State {
    fill: (u32, f64),
    stroke: (u32, f64),
    width: f64,
    round: bool,
    alpha: f64,
    clip: Option<[f32; 4]>,
}

pub enum Pixels {
    Coverage,
    Straight,
    Premultiplied,
}

pub struct Canvas<'a> {
    px: PixmapMut<'a>,
    z: f32,
    top: f32,
    st: State,
    saved: Vec<State>,
    path: PathBuilder,
    mask: Option<([f32; 4], Mask)>,
}

fn mask(cache: &mut Option<([f32; 4], Mask)>, clip: Option<[f32; 4]>, w: u32, h: u32) -> Option<&Mask> {
    let clip = clip?;
    if cache.as_ref().is_none_or(|(c, _)| *c != clip) {
        let mut m = Mask::new(w, h)?;
        if let Some(r) = Rect::from_ltrb(clip[0], clip[1], clip[2], clip[3]) {
            m.fill_path(&PathBuilder::from_rect(r), FillRule::Winding, true, Transform::identity());
        }
        *cache = Some((clip, m));
    }
    cache.as_ref().map(|(_, m)| m)
}

fn color((v, a): (u32, f64), alpha: f64) -> Color {
    let (r, g, b) = rgb(v);
    Color::from_rgba(r as f32, g as f32, b as f32, (a * alpha).clamp(0.0, 1.0) as f32).unwrap_or(Color::BLACK)
}

impl<'a> Canvas<'a> {
    pub fn new(out: &'a mut [u8], w: usize, h: usize, z: f64, top: f32, _smooth: bool, _display_p3: bool) -> Canvas<'a> {
        let px = PixmapMut::from_bytes(out, w as u32, h as u32).expect("a tile is never empty");
        let st = State { fill: (0, 1.0), stroke: (0, 1.0), width: 1.0, round: false, alpha: 1.0, clip: None };
        Canvas { px, z: z as f32, top, st, saved: vec![], path: PathBuilder::new(), mask: None }
    }

    fn transform(&self) -> Transform {
        Transform::from_row(self.z, 0.0, 0.0, self.z, 0.0, -self.top * self.z)
    }

    fn device(&self, x: f64, y: f64, w: f64, h: f64) -> [f32; 4] {
        let (x0, y0) = (x as f32 * self.z, (y as f32 - self.top) * self.z);
        [x0, y0, x0 + w as f32 * self.z, y0 + h as f32 * self.z]
    }

    pub fn save(&mut self) {
        self.saved.push(self.st.clone());
    }
    pub fn restore(&mut self) {
        if let Some(s) = self.saved.pop() {
            self.st = s;
        }
    }
    pub fn clip(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let r = self.device(x, y, w, h);
        self.st.clip = Some(match self.st.clip {
            Some(c) => [c[0].max(r[0]), c[1].max(r[1]), c[2].min(r[2]).max(c[0].max(r[0])), c[3].min(r[3]).max(c[1].max(r[1]))],
            None => r,
        });
    }
    pub fn fill_color(&mut self, rgbv: u32, a: f64) {
        self.st.fill = (rgbv, a);
    }
    pub fn stroke_color(&mut self, rgbv: u32, a: f64) {
        self.st.stroke = (rgbv, a);
    }
    pub fn line_width(&mut self, w: f64) {
        self.st.width = w;
    }
    pub fn round_lines(&mut self) {
        self.st.round = true;
    }
    pub fn alpha(&mut self, a: f64) {
        self.st.alpha = a;
    }
    pub fn fill_rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        let Some(c) = self.st.clip else {
            if let Some(r) = Rect::from_xywh(x as f32, y as f32, w as f32, h as f32) {
                self.fill(&PathBuilder::from_rect(r));
            }
            return;
        };
        let d = self.device(x, y, w, h);
        let Some(r) = Rect::from_ltrb(d[0].max(c[0]), d[1].max(c[1]), d[2].min(c[2]), d[3].min(c[3])) else { return };
        let paint = self.paint(self.st.fill);
        self.px.fill_rect(r, &paint, Transform::identity(), None);
    }
    pub fn begin_path(&mut self) {
        self.path = PathBuilder::new();
    }
    pub fn move_to(&mut self, x: f64, y: f64) {
        self.path.move_to(x as f32, y as f32);
    }
    pub fn line_to(&mut self, x: f64, y: f64) {
        self.path.line_to(x as f32, y as f32);
    }
    pub fn curve_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) {
        self.path.cubic_to(x1 as f32, y1 as f32, x2 as f32, y2 as f32, x as f32, y as f32);
    }
    pub fn close_path(&mut self) {
        self.path.close();
    }
    pub fn fill_path(&mut self) {
        if let Some(p) = std::mem::take(&mut self.path).finish() {
            self.fill(&p);
        }
    }
    pub fn stroke_path(&mut self) {
        if let Some(p) = std::mem::take(&mut self.path).finish() {
            self.stroke(&p);
        }
    }
    pub fn fill_stroke_path(&mut self) {
        if let Some(p) = std::mem::take(&mut self.path).finish() {
            self.fill(&p);
            self.stroke(&p);
        }
    }

    fn paint(&self, c: (u32, f64)) -> Paint<'static> {
        let mut p = Paint::default();
        p.set_color(color(c, self.st.alpha));
        p.anti_alias = true;
        p
    }

    fn fill(&mut self, p: &tiny_skia::Path) {
        let (paint, t) = (self.paint(self.st.fill), self.transform());
        let m = mask(&mut self.mask, self.st.clip, self.px.width(), self.px.height());
        self.px.fill_path(p, &paint, FillRule::Winding, t, m);
    }

    fn stroke(&mut self, p: &tiny_skia::Path) {
        let (paint, t) = (self.paint(self.st.stroke), self.transform());
        let (cap, join) = if self.st.round { (LineCap::Round, LineJoin::Round) } else { (LineCap::Butt, LineJoin::Miter) };
        let s = Stroke { width: self.st.width as f32, line_cap: cap, line_join: join, ..Stroke::default() };
        let m = mask(&mut self.mask, self.st.clip, self.px.width(), self.px.height());
        self.px.stroke_path(p, &paint, &s, t, m);
    }

    pub fn image(&mut self, x: f64, y: f64, w: f64, h: f64, b: &Bitmap) {
        let Some(src) = PixmapRef::from_bytes(&b.px, b.w, b.h) else { return };
        let t = self.transform().pre_translate(x as f32, y as f32).pre_scale(w as f32 / b.w as f32, h as f32 / b.h as f32);
        let native = t.kx == 0.0 && t.ky == 0.0 && ((t.sx - 1.0) * b.w as f32).abs() < 0.5 && ((t.sy - 1.0) * b.h as f32).abs() < 0.5;
        let (t, quality) = if native { (Transform::from_translate(t.tx.round(), t.ty.round()), FilterQuality::Nearest) } else { (t, FilterQuality::Bicubic) };
        let paint = PixmapPaint { opacity: self.st.alpha as f32, quality, ..PixmapPaint::default() };
        let m = mask(&mut self.mask, self.st.clip, self.px.width(), self.px.height());
        self.px.draw_pixmap(0, 0, src, &paint, t, m);
    }

    pub fn place(&self, x: f32, y: f32) -> (f32, f32, f32) {
        (self.z, x * self.z, (y - self.top) * self.z)
    }

    pub fn glyph(&mut self, x: i32, y: i32, w: u32, h: u32, data: &[u8], pixels: Pixels) {
        let (pw, ph) = (self.px.width() as i32, self.px.height() as i32);
        let clip = self.st.clip.unwrap_or([0.0, 0.0, pw as f32, ph as f32]);
        let (cx0, cy0) = ((clip[0].round() as i32).max(0), (clip[1].round() as i32).max(0));
        let (cx1, cy1) = ((clip[2].round() as i32).min(pw), (clip[3].round() as i32).min(ph));
        let (x0, x1) = (x.max(cx0), x.saturating_add_unsigned(w).min(cx1));
        let (y0, y1) = (y.max(cy0), y.saturating_add_unsigned(h).min(cy1));
        let (r, g, b) = rgb(self.st.fill.0);
        let fa = (self.st.fill.1 * self.st.alpha) as f32;
        let fc = [r as f32, g as f32, b as f32];
        let alpha = self.st.alpha as f32;
        let table = coverage();
        let out = self.px.data_mut();
        for py in y0..y1 {
            let row = (py - y) as usize * w as usize;
            for px in x0..x1 {
                let i = row + (px - x) as usize;
                let (src, sa) = match pixels {
                    Pixels::Coverage => {
                        let a = table[usize::from(data[i])] * fa;
                        ([fc[0] * a, fc[1] * a, fc[2] * a], a)
                    }
                    Pixels::Straight | Pixels::Premultiplied => {
                        let s = &data[i * 4..i * 4 + 4];
                        let a = f32::from(s[3]) / 255.0 * alpha;
                        let m = if matches!(pixels, Pixels::Straight) { a } else { alpha };
                        ([f32::from(s[0]) / 255.0 * m, f32::from(s[1]) / 255.0 * m, f32::from(s[2]) / 255.0 * m], a)
                    }
                };
                if sa <= 0.0 {
                    continue;
                }
                let o = ((py * pw + px) * 4) as usize;
                let d = &mut out[o..o + 4];
                for k in 0..3 {
                    d[k] = (src[k] * 255.0 + f32::from(d[k]) * (1.0 - sa) + 0.5) as u8;
                }
                d[3] = (sa * 255.0 + f32::from(d[3]) * (1.0 - sa) + 0.5) as u8;
            }
        }
    }
}

fn coverage() -> &'static [f32; 256] {
    static TABLE: std::sync::OnceLock<[f32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| std::array::from_fn(|i| (i as f32 / 255.0).powf(1.0 / 1.25)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_glyphs_blend_by_their_alpha_kind() {
        for (pixels, src) in [(Pixels::Straight, [255, 0, 0, 128]), (Pixels::Premultiplied, [128, 0, 0, 128])] {
            let mut out = vec![0u8; 4];
            let mut c = Canvas::new(&mut out, 1, 1, 1.0, 0.0, false, false);
            c.glyph(0, 0, 1, 1, &src, pixels);
            drop(c);
            assert_eq!(out, [128, 0, 0, 128]);
        }
    }
}
