//! Draws display-list items into RGBA tiles.
use crate::render::canvas::Canvas;
use crate::render::icons;
use crate::render::page::*;
use crate::render::text::Fonts;
use crate::render::theme::{Rgb, Theme};
use std::rc::Rc;

/// Things drawn over the page that are not part of the display list.
#[derive(Default, Clone)]
pub struct Overlay {
    /// Highlight rects (page coordinates): (x, y, w, h, current)
    pub marks: Vec<(f32, f32, f32, f32, bool)>,
    pub sel: Vec<(f32, f32, f32, f32)>,
    /// Horizontal scroll offsets per hscroll region.
    pub hoff: Vec<f32>,
    /// Hovered button index and the one that shows "Copied!".
    pub hover_btn: Option<usize>,
    pub copied_btn: Option<usize>,
    pub expanded: Option<Expanded>,
}

/// An expanded diagram over the view: the box it covers and the diagram, drawn to fit it (device px).
#[derive(Clone)]
pub struct Expanded {
    pub y: f32,
    pub h: f32,
    pub bmp: Rc<Bitmap>,
    pub x: f32,
    pub top: f32,
}

pub struct Ctx<'a> {
    pub page: &'a Page,
    pub th: &'a Theme,
    pub scale: f32,
    pub ov: &'a Overlay,
    /// Font smoothing, which CoreGraphics bitmap contexts leave off unless asked.
    pub smooth: bool,
    /// Convert the page's sRGB colors to Display P3 pixels.
    pub display_p3: bool,
}

fn argb(c: &mut Canvas, v: u32, stroke: bool) {
    let a = ((v >> 24) & 255) as f64 / 255.0;
    if stroke { c.stroke_color(v & 0xffffff, a) } else { c.fill_color(v & 0xffffff, a) }
}

fn rrect(c: &mut Canvas, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    c.begin_path();
    let (x2, y2) = (x + w, y + h);
    if r <= 0.0 {
        c.move_to(x, y);
        c.line_to(x2, y);
        c.line_to(x2, y2);
        c.line_to(x, y2);
        c.close_path();
        return;
    }
    let k = r * 0.5522847498;
    c.move_to(x + r, y);
    c.line_to(x2 - r, y);
    c.curve_to(x2 - r + k, y, x2, y + r - k, x2, y + r);
    c.line_to(x2, y2 - r);
    c.curve_to(x2, y2 - r + k, x2 - r + k, y2, x2 - r, y2);
    c.line_to(x + r, y2);
    c.curve_to(x + r - k, y2, x, y2 - r + k, x, y2 - r);
    c.line_to(x, y + r);
    c.curve_to(x, y + r - k, x + r - k, y, x + r, y);
    c.close_path();
}

/// Draws page rows from `top` (layout pixels) into `out`, a `w` by `h` RGBA buffer, at `z` output pixels per
/// layout pixel.
#[expect(clippy::too_many_arguments, reason = "the tile's place, size and scale, each a plain number")]
pub fn render_into(fonts: &mut Fonts, cx: &Ctx, top: f32, h: usize, w: usize, items: &[u32], z: f64, out: &mut [u8]) {
    if w == 0 || h == 0 {
        return;
    }
    let ctx = &mut Canvas::new(out, w, h, z, top, cx.smooth, cx.display_p3);
    ctx.fill_color(cx.th.bg, 1.0);
    ctx.fill_rect(0.0, top as f64, w as f64 / z, h as f64 / z);
    let page = cx.page;
    let s = cx.scale as f64;
    for &i in items {
        let it = &page.items[i as usize];
        let (dx, clip_rect) = if it.clip > 0 {
            let hs = &page.hscroll[it.clip as usize - 1];
            let off = cx.ov.hoff.get(it.clip as usize - 1).copied().unwrap_or(0.0);
            (-off, Some(hs))
        } else {
            (0.0, None)
        };
        ctx.save();
        if let Some(hs) = clip_rect {
            ctx.clip(hs.x as f64, hs.y as f64, hs.w as f64, hs.h as f64);
        }
        let (x, y, iw, ih) = ((it.x + dx) as f64, it.y as f64, it.w as f64, it.h as f64);
        match &it.k {
            Kind::Rect { fill, stroke, radius } => {
                let half = stroke.map(|(w, _)| w as f64 / 2.0).unwrap_or(0.0);
                if (fill >> 24) != 0 {
                    argb(ctx, *fill, false);
                    rrect(ctx, x, y, iw, ih, *radius as f64);
                    ctx.fill_path();
                }
                if let Some((sw, sc)) = stroke {
                    argb(ctx, *sc, true);
                    ctx.line_width(*sw as f64);
                    rrect(ctx, x + half, y + half, iw - 2.0 * half, ih - 2.0 * half, (*radius as f64 - half).max(0.0));
                    ctx.stroke_path();
                }
            }
            Kind::Text { color, .. } => draw_text(ctx, fonts, it, x, *color),
            Kind::Img(b) => ctx.image(x, y, iw, ih, b),
            Kind::Poly { pts, color, width, fill } => {
                argb(ctx, *color, *fill);
                argb(ctx, *color, !*fill);
                ctx.line_width(*width as f64);
                ctx.round_lines();
                ctx.begin_path();
                for (i, p) in pts.iter().enumerate() {
                    let (px, py) = (x + p.0 as f64, y + p.1 as f64);
                    if i == 0 {
                        ctx.move_to(px, py);
                    } else {
                        ctx.line_to(px, py);
                    }
                }
                if *fill {
                    ctx.close_path();
                    ctx.fill_path();
                } else {
                    ctx.stroke_path();
                }
            }
        }
        ctx.restore();
    }
    // Chrome's find-in-page look: a solid box with the text inside it drawn again in one dark color,
    // so the match reads the same over links, syntax colors and any theme. Before the scroll bars and
    // buttons, so a box never hides them.
    for &(x, y, w, hh, cur) in &cx.ov.marks {
        if y > top + h as f32 || y + hh < top {
            continue;
        }
        ctx.save();
        // A mark in a code block that scrolls sideways stays inside the block.
        let mid = y + hh / 2.0;
        if let Some(hs) = page.hscroll.iter().find(|hs| hs.y <= mid && mid < hs.y + hs.h) {
            ctx.clip(hs.x as f64, hs.y as f64, hs.w as f64, hs.h as f64);
        }
        ctx.clip(x as f64, y as f64, w as f64, hh as f64);
        ctx.fill_color(if cur { cx.th.hl_cur_bg } else { cx.th.hl_bg }, 1.0);
        ctx.fill_rect(x as f64, y as f64, w as f64, hh as f64);
        let fg = if cur { cx.th.hl_cur_fg } else { cx.th.hl_fg };
        for &i in items {
            let it = &page.items[i as usize];
            let dx = if it.clip > 0 { -cx.ov.hoff.get(it.clip as usize - 1).copied().unwrap_or(0.0) } else { 0.0 };
            // Only the runs under this mark: a one-letter search on a long code line has many marks.
            let ix = it.x + dx;
            if !matches!(it.k, Kind::Text { .. }) || it.y > y + hh || it.y + it.h < y || ix > x + w || ix + it.w < x {
                continue;
            }
            draw_text(ctx, fonts, it, f64::from(ix), fg);
        }
        ctx.restore();
    }
    for (i, hs) in page.hscroll.iter().enumerate() {
        if hs.max <= 0.0 || hs.y > top + h as f32 || hs.y + hs.h < top {
            continue;
        }
        let off = cx.ov.hoff.get(i).copied().unwrap_or(0.0);
        let (tx, tw) = hs.thumb(off, s as f32);
        ctx.fill_color(cx.th.fg_muted, 0.55);
        rrect(ctx, f64::from(tx), f64::from(hs.bar_y), f64::from(tw), f64::from(HScroll::BAR) * s, f64::from(HScroll::BAR) / 2.0 * s);
        ctx.fill_path();
    }
    for (i, b) in page.buttons.iter().enumerate() {
        if b.w <= 0.0 || b.y > top + h as f32 || b.y + b.h < top {
            continue;
        }
        draw_button(ctx, cx, b, cx.ov.hover_btn == Some(i), cx.ov.copied_btn == Some(i), z);
        let hover = cx.ov.hover_btn == Some(i);
        let copied = cx.ov.copied_btn == Some(i);
        let label = if copied {
            Some("Copied!")
        } else if hover && b.kind == BtnKind::CopyCode {
            Some("Copy code to clipboard")
        } else {
            None
        };
        if let Some(t) = label {
            let sc = cx.scale;
            let f = crate::render::text::FontKey::new(12.0 * sc, 0);
            let tw = fonts.measure(f, t);
            let (a, d) = fonts.ascent_descent(f);
            let (px, py) = (4.0 * sc, 3.0 * sc);
            let bx = b.x - 6.0 * sc - tw - 2.0 * px;
            let by = b.y + (b.h - (a + d + 2.0 * py)) / 2.0;
            ctx.fill_color(cx.th.tooltip_bg, 1.0);
            rrect(ctx, bx as f64, by as f64, (tw + 2.0 * px) as f64, (a + d + 2.0 * py) as f64, 6.0 * s);
            ctx.fill_path();
            ctx.fill_color(0xffffff, 1.0);
            fonts.draw(ctx, f, t, bx + px, by + py + a);
        }
    }
    for &(x, y, w, hh) in &cx.ov.sel {
        if y > top + h as f32 || y + hh < top {
            continue;
        }
        ctx.fill_color(cx.th.sel_bg, 0.6);
        ctx.fill_rect(x as f64, y as f64, w as f64, hh as f64);
    }
    if let Some(e) = &cx.ov.expanded
        && e.y <= top + h as f32
        && e.y + e.h >= top
    {
        ctx.fill_color(cx.th.bg, 1.0);
        ctx.fill_rect(0.0, e.y as f64, page.w as f64, e.h as f64);
        ctx.save();
        ctx.clip(0.0, e.y as f64, page.w as f64, e.h as f64);
        ctx.image(e.x as f64, e.top as f64, e.bmp.w as f64, e.bmp.h as f64, &e.bmp);
        ctx.restore();
    }
}

/// Draws a text item's glyphs, underline and strike in `color`, its left edge at `x`.
fn draw_text(ctx: &mut Canvas, fonts: &mut Fonts, it: &Item, x: f64, color: Rgb) {
    let Kind::Text { s: t, f, under, strike, asc, .. } = &it.k else { return };
    ctx.fill_color(color, 1.0);
    fonts.draw(ctx, *f, t, x as f32, it.y + asc);
    let (u, st, th) = f.lines();
    for (on, dy) in [(*under, u), (*strike, st)] {
        if on {
            ctx.fill_rect(x, f64::from(it.y + asc) + f64::from(dy), f64::from(it.w), f64::from(th));
        }
    }
}

/// Draws a diagram button: GitHub's bordered box for copy and expand, and a bare icon for the zoom bar,
/// faded when off. `z` is output pixels per layout pixel, so the icon is drawn sharp.
fn draw_button(ctx: &mut Canvas, cx: &Ctx, b: &Button, hover: bool, copied: bool, z: f64) {
    if b.kind == BtnKind::CopyCode {
        return draw_code_button(ctx, cx, b, hover, copied);
    }
    let s = cx.scale as f64;
    let (x, y, w, h) = (b.x as f64, b.y as f64, b.w as f64, b.h as f64);
    let color = match b.kind {
        BtnKind::Expand | BtnKind::CopyDiagram => {
            // The pointer changes only the cursor here.
            ctx.fill_color(cx.th.button_bg, 1.0);
            rrect(ctx, x, y, w, h, 6.0 * s);
            ctx.fill_path();
            ctx.stroke_color(cx.th.border, 1.0);
            ctx.line_width(s);
            rrect(ctx, x + s / 2.0, y + s / 2.0, w - s, h - s, 5.5 * s);
            ctx.stroke_path();
            if copied { cx.th.copied } else { cx.th.fg_muted }
        }
        _ => cx.th.text,
    };
    let size = 16.0 * s;
    let Some(img) = icons::bitmap(icons::Icon::of(b.kind, copied), color, (size * z).round() as u32) else { return };
    ctx.save();
    if b.off {
        ctx.alpha(0.35);
    }
    ctx.image(x + ((w - size) / 2.0).round(), y + ((h - size) / 2.0).round(), size, size, &img);
    ctx.restore();
}

/// A code block's copy button.
fn draw_code_button(ctx: &mut Canvas, cx: &Ctx, b: &Button, hover: bool, copied: bool) {
    let s = cx.scale as f64;
    let (x, y, w, h) = (b.x as f64, b.y as f64, b.w as f64, b.h as f64);
    ctx.fill_color(cx.th.pre_bg, 1.0);
    rrect(ctx, x, y, w, h, 6.0 * s);
    ctx.fill_path();
    if hover {
        ctx.fill_color(cx.th.rule, 0.6);
        rrect(ctx, x, y, w, h, 6.0 * s);
        ctx.fill_path();
    }
    ctx.stroke_color(cx.th.fg_muted, 1.0);
    ctx.line_width(1.4 * s);
    ctx.round_lines();
    let (mx, my) = (x + w / 2.0, y + h / 2.0);
    if copied {
        ctx.stroke_color(cx.th.copied, 1.0);
        ctx.line_width(2.0 * s);
        ctx.begin_path();
        ctx.move_to(mx - 5.0 * s, my);
        ctx.line_to(mx - 1.5 * s, my + 4.0 * s);
        ctx.line_to(mx + 5.5 * s, my - 4.0 * s);
        ctx.stroke_path();
    } else {
        rrect(ctx, mx - 3.0 * s, my - 6.0 * s, 9.0 * s, 10.0 * s, 1.5 * s);
        ctx.stroke_path();
        rrect(ctx, mx - 6.0 * s, my - 3.0 * s, 9.0 * s, 10.0 * s, 1.5 * s);
        ctx.fill_color(cx.th.pre_bg, 1.0);
        ctx.fill_stroke_path();
    }
}
