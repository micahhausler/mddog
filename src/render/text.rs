//! Fonts, text measurement and drawing through CoreText.
use core_foundation::attributed_string::CFMutableAttributedString;
use core_foundation::base::{CFRange, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::context::CGContext;
use core_graphics::geometry::{CGAffineTransform, CGPoint};
use core_text::font::{CTFont, new_from_name, new_ui_font_for_language};
use core_text::font_descriptor::{kCTFontBoldTrait, kCTFontItalicTrait};
use core_text::line::CTLine;
use core_text::string_attributes::{kCTFontAttributeName, kCTForegroundColorFromContextAttributeName, kCTKernAttributeName};
use std::collections::HashMap;

pub const BOLD: u16 = 1;
pub const ITALIC: u16 = 2;
pub const MONO: u16 = 4;
pub const MATH: u16 = 8;
/// Digits all one width, as CSS's `font-variant-numeric: tabular-nums`.
pub const TABULAR: u16 = 16;
/// The style bits above the flags hold a CSS font weight, 0 for the font's own.
const WEIGHT_SHIFT: u16 = 5;
pub const WEIGHT_MASK: u16 = !0 << WEIGHT_SHIFT;
/// Style bits for CSS font weight `w`, 1 to 1000.
pub const fn weight(w: u16) -> u16 {
    w << WEIGHT_SHIFT
}
/// Weight 600, as GitHub draws strong text and table headers.
pub const SEMIBOLD: u16 = weight(600);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FontKey {
    /// Size in 1/16 px.
    pub size: u32,
    pub style: u16,
    /// Letter spacing in 1/1000 em, as CSS's `letter-spacing`.
    pub track: i16,
}

impl FontKey {
    pub fn new(size: f32, style: u16) -> FontKey {
        FontKey { size: (size * 16.0).round() as u32, style, track: 0 }
    }
    /// The same font with `em` of letter spacing.
    pub fn tracked(self, em: f32) -> FontKey {
        FontKey { track: (em * 1000.0).round() as i16, ..self }
    }
    /// The underline's and the strike line's offsets from the baseline, and their thickness, in px.
    pub fn lines(&self) -> (f32, f32, f32) {
        let px = self.px();
        (0.12 * px, -0.3 * px, (px / 16.0).max(1.0))
    }

    pub fn px(&self) -> f32 {
        self.size as f32 / 16.0
    }
}

pub struct Fonts {
    fonts: HashMap<FontKey, CTFont>,
    lines: HashMap<(FontKey, String), Line>,
    /// The theme's body and code font lists, first choice first.
    body: Vec<String>,
    code: Vec<String>,
    /// Output pixels per CSS pixel.
    scale: f32,
}

impl Fonts {
    pub fn new() -> Fonts {
        Fonts { fonts: HashMap::new(), lines: HashMap::new(), body: vec![], code: vec![], scale: 1.0 }
    }

    /// Draws with the theme's fonts from now on.
    pub fn set_families(&mut self, body: Vec<String>, code: Vec<String>) {
        if (&body, &code) != (&self.body, &self.code) {
            (self.body, self.code) = (body, code);
            self.fonts.clear();
            self.lines.clear();
        }
    }

    /// Lays out at `scale` output pixels per CSS pixel from now on.
    pub fn set_scale(&mut self, scale: f32) {
        if scale != self.scale {
            self.scale = scale;
            self.fonts.clear();
            self.lines.clear();
        }
    }

    pub fn font(&mut self, k: FontKey) -> &CTFont {
        let k = FontKey { track: 0, ..k };
        let (body, code, scale) = (&self.body, &self.code, self.scale);
        self.fonts.entry(k).or_insert_with(|| at_scale(make_font(k.px() / scale, k.style, if k.style & MONO != 0 { code } else { body }), scale))
    }

    pub fn ascent_descent(&mut self, k: FontKey) -> (f32, f32) {
        let f = self.font(k);
        (f.ascent() as f32, f.descent() as f32)
    }

    fn line(&mut self, k: FontKey, s: &str) -> &Line {
        if self.lines.len() > 20000 {
            self.lines.clear();
        }
        if !self.lines.contains_key(&(k, s.to_string())) {
            let scale = f64::from(self.scale);
            let kern = f64::from(k.track) / 1000.0 * f64::from(k.px());
            let mut line = CTLine::new_with_attributed_string(attributed(self.font(k), s, kern).as_concrete_TypeRef());
            let mut emoji = vec![];
            if has_bitmap_run(&line) {
                let families = if k.style & MONO != 0 { self.code.clone() } else { self.body.clone() };
                let css_font = if self.scale == 1.0 { self.font(k).clone() } else { make_font(k.px() / self.scale, k.style, &families) };
                let css = CTLine::new_with_attributed_string(attributed(&css_font, s, kern / scale).as_concrete_TypeRef());
                let mut fit = clusters(&css, scale);
                fit.sort_by_key(|f| f.0.location);
                let track = kern / scale;
                let mut d = attributed(self.font(k), s, kern);
                let hidden = copy_at(&css_font, 0.001);
                for &(at, _, want) in &fit {
                    let (head, rest) = (CFRange::init(at.location, 1), CFRange::init(at.location + 1, at.length - 1));
                    // SAFETY: CoreText's own attribute names, each with the type its key expects.
                    unsafe {
                        d.set_attribute(at, kCTFontAttributeName, &hidden);
                        d.set_attribute(at, kCTKernAttributeName, &CFNumber::from(0.0));
                        d.set_attribute(head, kCTRunDelegateAttributeName, &gap((want + track) * scale));
                        if rest.length > 0 {
                            d.set_attribute(rest, kCTRunDelegateAttributeName, &gap(0.0));
                        }
                    }
                }
                line = CTLine::new_with_attributed_string(d.as_concrete_TypeRef());
                emoji = placed(&css, &line, &fit, scale);
            }
            let width = line.get_typographic_bounds().width as f32;
            self.lines.insert((k, s.to_string()), Line { line, width, emoji });
        }
        &self.lines[&(k, s.to_string())]
    }

    pub fn measure(&mut self, k: FontKey, s: &str) -> f32 {
        self.line(k, s).width
    }

    /// x offset of the char at `idx` (in chars) from the start of the string.
    pub fn char_x(&mut self, k: FontKey, s: &str, idx: usize) -> f32 {
        let total = s.chars().count();
        if idx == 0 {
            return 0.0;
        }
        if idx >= total {
            return self.measure(k, s);
        }
        let u16idx: usize = s.chars().take(idx).map(char::len_utf16).sum();
        self.line(k, s).line.get_string_offset_for_string_index(u16idx as isize) as f32
    }

    pub fn draw(&mut self, ctx: &CGContext, k: FontKey, s: &str, x: f32, y: f32) {
        let l = self.line(k, s);
        let (x, y) = (f64::from(x), f64::from(y));
        ctx.set_text_matrix(&CGAffineTransform::new(1.0, 0.0, 0.0, -1.0, x, y));
        if l.emoji.is_empty() {
            l.line.draw(ctx);
            return;
        }
        ctx.save();
        l.line.draw(ctx);
        ctx.restore();
        let m = ctx.get_ctm();
        let snap = |v: f64, k: f64, t: f64, quarter: bool| {
            let d = k * v + t;
            let d = if quarter { ((d * 4.0).round() / 4.0 + 0.5).floor() } else { d.round() };
            (d - t) / k
        };
        for (g, px, py, full) in &l.emoji {
            let (gx, gy) = (snap(x + px, m.a, m.tx, true), snap(y - py, m.d, m.ty, false));
            ctx.set_text_matrix(&CGAffineTransform::new(1.0, 0.0, 0.0, -1.0, gx, gy));
            full.draw_glyphs(&[*g], &[CGPoint::new(0.0, 0.0)], ctx.clone());
        }
    }
}

/// A laid-out string. CoreText draws bitmap emoji such as Apple Color Emoji without the font's scale matrix, so `line` holds each emoji as an empty gap as wide as Chrome makes it (its picture at device size, divided by the scale), and `emoji` holds each one's glyph, place and font at device size to draw into that gap.
struct Line {
    line: CTLine,
    width: f32,
    emoji: Vec<(core_graphics::base::CGGlyph, f64, f64, CTFont)>,
}

fn attributed(font: &CTFont, s: &str, kern: f64) -> CFMutableAttributedString {
    let cf = CFString::new(s);
    let mut a = CFMutableAttributedString::new();
    a.replace_str(&cf, CFRange::init(0, 0));
    let len = a.char_len();
    // SAFETY: the keys are CoreText's own attribute names, and each value has the type its key expects.
    unsafe {
        a.set_attribute(CFRange::init(0, len), kCTFontAttributeName, font);
        a.set_attribute(CFRange::init(0, len), kCTForegroundColorFromContextAttributeName, &CFBoolean::true_value());
        if kern != 0.0 {
            a.set_attribute(CFRange::init(0, len), kCTKernAttributeName, &CFNumber::from(kern));
        }
    }
    a
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

/// CSS's names for the system font, in lower case.
pub fn system_ui(name: &str) -> bool {
    matches!(name, "system-ui" | "-apple-system" | "blinkmacsystemfont" | "ui-sans-serif" | "sans-serif")
}

/// The first font of `names` that is installed, as CSS's `font-family` picks. `system-ui` and its kin are the system font, and `ui-monospace` and `monospace` its monospaced cut.
fn family(names: &[String], size: f64, mono: bool) -> CTFont {
    for n in names {
        match n.to_ascii_lowercase().as_str() {
            n if system_ui(n) => return new_ui_font_for_language(2, size, None),
            "ui-monospace" | "monospace" => {
                if let Some(f) = system_mono(size) {
                    return f;
                }
            }
            _ => {
                // CoreText answers an unknown name with a stand-in, so check the name it gave.
                if let Ok(f) = new_from_name(n, size)
                    && [f.family_name(), f.postscript_name(), f.display_name()].iter().any(|got| got.eq_ignore_ascii_case(n))
                {
                    return f;
                }
            }
        }
    }
    if mono { new_from_name("Menlo-Regular", size).unwrap_or_else(|_| new_ui_font_for_language(2, size, None)) } else { new_ui_font_for_language(2, size, None) }
}

// `NSFont` lives in AppKit, which nothing else links.
#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

/// SF Mono, which only AppKit names.
fn system_mono(size: f64) -> Option<CTFont> {
    objc2::rc::autoreleasepool(|_| {
        let cls = objc2::runtime::AnyClass::get(c"NSFont")?;
        // SAFETY: `monospacedSystemFontOfSize:weight:` takes two CGFloats and returns an autoreleased NSFont, which is toll-free bridged to CTFontRef. The get rule retains it before the pool drains.
        unsafe {
            let f: *const objc2::runtime::AnyObject = objc2::msg_send![cls, monospacedSystemFontOfSize: size, weight: 0.0f64];
            (!f.is_null()).then(|| CTFont::wrap_under_get_rule(f as core_text::font::CTFontRef))
        }
    })
}

unsafe extern "C" {
    fn CTFontCreateWithFontDescriptor(descriptor: core_text::font_descriptor::CTFontDescriptorRef, size: f64, matrix: *const CGAffineTransform) -> core_text::font::CTFontRef;
}

/// `f` drawn `scale` times larger. CoreText picks the system font's optical size and tracking from the point
/// size, so a 16 px body font made at 32 pt for a Retina screen gets the display cut, about 6% narrower than
/// a browser's text. Scaling a 16 pt font keeps the text cut, as a browser does.
fn at_scale(f: CTFont, scale: f32) -> CTFont {
    if scale == 1.0 {
        return f;
    }
    let m = CGAffineTransform::new(scale as f64, 0.0, 0.0, scale as f64, 0.0, 0.0);
    // SAFETY: the descriptor and matrix outlive the call, and the create rule takes the returned font's reference.
    unsafe { CTFont::wrap_under_create_rule(CTFontCreateWithFontDescriptor(f.copy_descriptor().as_concrete_TypeRef(), f.pt_size(), &m)) }
}

const SBIX: u32 = u32::from_be_bytes(*b"sbix");

fn has_sbix(f: &CTFont) -> bool {
    f.get_available_font_tables().is_some_and(|t| t.iter().any(|tag| *tag == SBIX))
}

unsafe extern "C" {
    static kCTRunDelegateAttributeName: core_foundation::string::CFStringRef;
    fn CTRunDelegateCreate(callbacks: *const DelegateCallbacks, refcon: *mut std::ffi::c_void) -> core_foundation::base::CFTypeRef;
    fn CTRunGetStringRange(run: core_text::run::CTRunRef) -> CFRange;
    fn CTFontCreateCopyWithAttributes(
        font: core_text::font::CTFontRef,
        size: f64,
        matrix: *const CGAffineTransform,
        attributes: core_text::font_descriptor::CTFontDescriptorRef,
    ) -> core_text::font::CTFontRef;
}

fn copy_at(font: &CTFont, size: f64) -> CTFont {
    // SAFETY: the font and matrix outlive the call, and the create rule takes the returned font's reference.
    unsafe { CTFont::wrap_under_create_rule(CTFontCreateCopyWithAttributes(font.as_concrete_TypeRef(), size, &CGAffineTransform::new(1.0, 0.0, 0.0, 1.0, 0.0, 0.0), std::ptr::null())) }
}

fn run_font(run: &core_text::run::CTRun) -> Option<CTFont> {
    // SAFETY: a CoreText constant, set once when the framework loads.
    let key = unsafe { kCTFontAttributeName };
    run.attributes().and_then(|a| a.find(key).and_then(|f| f.downcast::<CTFont>()))
}

fn bitmap_fonts(line: &CTLine) -> impl Iterator<Item = (core_text::run::CTRun, CTFont)> {
    line.glyph_runs().iter().filter_map(|r| run_font(&r).filter(has_sbix).map(|f| (r.clone(), f))).collect::<Vec<_>>().into_iter()
}

fn has_bitmap_run(line: &CTLine) -> bool {
    bitmap_fonts(line).next().is_some()
}

fn clusters(line: &CTLine, scale: f64) -> Vec<(CFRange, f64, f64)> {
    let mut out: Vec<(CFRange, f64, f64)> = vec![];
    for (run, font) in bitmap_fonts(line) {
        let device = copy_at(&font, font.pt_size() * scale);
        // SAFETY: the run belongs to `line`, which outlives the call.
        let range = unsafe { CTRunGetStringRange(run.as_concrete_TypeRef()) };
        let (glyphs, pos, idx) = (run.glyphs(), run.positions(), run.string_indices());
        let end = pos.first().map_or(0.0, |p| p.x) + run.get_typographic_bounds().width;
        let mut held: Option<(isize, isize, f64)> = None;
        let first = out.len();
        for j in 0..glyphs.len() {
            let now = pos.get(j + 1).map_or(end, |p| p.x) - pos[j].x;
            let want = device.get_bounding_rects_for_glyphs(0, &glyphs[j..=j]).size.width.round() / scale;
            let next = idx.iter().copied().filter(|&i| i > idx[j]).min().unwrap_or(range.location + range.length);
            let (lo, hi, want) = held.take().map_or((idx[j], next, want), |(l, h, w)| (l.min(idx[j]), h.max(next), w.max(want)));
            if now.abs() < 1e-6 {
                held = Some((lo, hi, want));
            } else {
                out.push((CFRange::init(lo, hi - lo), now, want));
            }
        }
        if let Some((lo, hi, want)) = held {
            match out[first..].last_mut() {
                Some(c) => {
                    let (l, h) = (c.0.location.min(lo), (c.0.location + c.0.length).max(hi));
                    *c = (CFRange::init(l, h - l), c.1, c.2.max(want));
                }
                None => out.push((CFRange::init(lo, hi - lo), 0.0, want)),
            }
        }
    }
    out
}

fn placed(css: &CTLine, line: &CTLine, fit: &[(CFRange, f64, f64)], scale: f64) -> Vec<(core_graphics::base::CGGlyph, f64, f64, CTFont)> {
    let mut at = std::collections::HashMap::new();
    for run in line.glyph_runs().iter() {
        for (i, p) in run.string_indices().iter().zip(run.positions().iter()) {
            at.entry(*i).or_insert(p.x);
        }
    }
    let mut out = vec![];
    for (run, font) in bitmap_fonts(css) {
        let full = copy_at(&font, font.pt_size() * scale);
        for (g, (p, i)) in run.glyphs().iter().zip(run.positions().iter().zip(run.string_indices().iter())) {
            let Some(x) = fit.iter().find(|f| (f.0.location..f.0.location + f.0.length).contains(i)).and_then(|f| at.get(&f.0.location)) else { continue };
            out.push((*g, *x, p.y * scale, full.clone()));
        }
    }
    out
}

#[repr(C)]
struct DelegateCallbacks {
    version: isize,
    dealloc: extern "C" fn(*mut std::ffi::c_void),
    ascent: extern "C" fn(*mut std::ffi::c_void) -> f64,
    descent: extern "C" fn(*mut std::ffi::c_void) -> f64,
    width: extern "C" fn(*mut std::ffi::c_void) -> f64,
}

extern "C" fn gap_free(r: *mut std::ffi::c_void) {
    // SAFETY: `gap` made `r` with `Box::into_raw`, and CoreText frees each delegate once.
    drop(unsafe { Box::from_raw(r.cast::<f64>()) });
}

extern "C" fn gap_zero(_: *mut std::ffi::c_void) -> f64 {
    0.0
}

extern "C" fn gap_width(r: *mut std::ffi::c_void) -> f64 {
    // SAFETY: `gap` made `r` from a live `Box<f64>`, which `gap_free` alone frees.
    unsafe { *r.cast::<f64>() }
}

static GAP: DelegateCallbacks = DelegateCallbacks { version: 1, dealloc: gap_free, ascent: gap_zero, descent: gap_zero, width: gap_width };

fn gap(width: f64) -> core_foundation::base::CFType {
    let r = Box::into_raw(Box::new(width)).cast();
    // SAFETY: the callbacks are static, `r` passes to the delegate, which frees it through `gap_free`, and the create rule takes the delegate's reference.
    unsafe { core_foundation::base::CFType::wrap_under_create_rule(CTRunDelegateCreate(&raw const GAP, r)) }
}

/// CSS font weights and the CoreText weight traits macOS gives them, as `NSFontWeight` names them.
const WEIGHTS: [(f64, f64); 9] = [(100.0, -0.8), (200.0, -0.6), (300.0, -0.4), (400.0, 0.0), (500.0, 0.23), (600.0, 0.3), (700.0, 0.4), (800.0, 0.56), (900.0, 0.62)];

/// `x` mapped through the table's straight segments: from the pairs' first value to their second, or back when `back`.
fn through_weights(x: f64, back: bool) -> f64 {
    let pairs = WEIGHTS.map(|(c, t)| if back { (t, c) } else { (c, t) });
    let x = x.clamp(pairs[0].0, pairs[8].0);
    let i = pairs.windows(2).position(|w| x <= w[1].0).unwrap_or(7);
    let ((x0, y0), (x1, y1)) = (pairs[i], pairs[i + 1]);
    y0 + (x - x0) / (x1 - x0) * (y1 - y0)
}

unsafe extern "C" {
    static kCTFontVariationAxisIdentifierKey: core_foundation::string::CFStringRef;
    static kCTFontVariationAxisMinimumValueKey: core_foundation::string::CFStringRef;
    static kCTFontVariationAxisMaximumValueKey: core_foundation::string::CFStringRef;
}

fn on_weight_axis(f: &CTFont, css: f64) -> Option<CTFont> {
    use core_foundation::dictionary::CFDictionary;
    let wght = i64::from(u32::from_be_bytes(*b"wght"));
    // SAFETY: CoreText's own constant strings, set once when the framework loads.
    let (id, lo, hi, var) = unsafe {
        (
            CFString::wrap_under_get_rule(kCTFontVariationAxisIdentifierKey),
            CFString::wrap_under_get_rule(kCTFontVariationAxisMinimumValueKey),
            CFString::wrap_under_get_rule(kCTFontVariationAxisMaximumValueKey),
            CFString::wrap_under_get_rule(core_text::font_descriptor::kCTFontVariationAttribute),
        )
    };
    let num = |a: &CFDictionary<CFString, core_foundation::base::CFType>, k: &CFString| a.find(k).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_f64());
    let axis = f.get_variation_axes()?.iter().find(|a| a.find(&id).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_i64()) == Some(wght))?.clone();
    let value = css.clamp(num(&axis, &lo)?, num(&axis, &hi)?);
    let v = CFDictionary::from_CFType_pairs(&[(CFNumber::from(wght), CFNumber::from(value))]);
    let attrs = CFDictionary::from_CFType_pairs(&[(var, v.as_CFType())]);
    f.copy_descriptor().create_copy_with_attributes(attrs.to_untyped()).ok().map(|d| core_text::font::new_from_descriptor(&d, f.pt_size()))
}

/// `f` at CSS weight `css`. CoreText draws a variable family such as SF at any weight, and a family of fixed
/// cuts at its nearest one, the heavier on a tie from 400 up and the lighter below, as CSS leans.
fn weighted(f: &CTFont, css: u16) -> CTFont {
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_text::font_descriptor::{TraitAccessors, kCTFontTraitsAttribute, kCTFontWeightTrait};
    let css = f64::from(css);
    if let Some(v) = on_weight_axis(f, css) {
        return v;
    }
    let want = through_weights(css, false);
    let with_trait = |f: &CTFont, t: f64| {
        // SAFETY: both keys are CoreText's own constant strings.
        let (traits, weight) = unsafe { (CFString::wrap_under_get_rule(kCTFontTraitsAttribute), CFString::wrap_under_get_rule(kCTFontWeightTrait)) };
        let w = CFDictionary::from_CFType_pairs(&[(weight, CFNumber::from(t))]);
        let attrs = CFDictionary::from_CFType_pairs(&[(traits, w.as_CFType())]);
        f.copy_descriptor().create_copy_with_attributes(attrs.to_untyped()).ok().map(|d| core_text::font::new_from_descriptor(&d, f.pt_size()))
    };
    let got = |f: &CTFont| f.all_traits().normalized_weight();
    if let Some(n) = with_trait(f, want)
        && (got(&n) - want).abs() < 0.02
    {
        return n;
    }
    // CoreText hands back the regular cut when a fixed family lacks the weight, so pick from the family's cuts.
    // Upright cuts of the same width only, so a condensed or italic cut never stands in.
    let width = f.all_traits().normalized_width();
    let cuts: Vec<(f64, core_text::font_descriptor::CTFontDescriptor)> = core_text::font_collection::create_for_family(&f.family_name())
        .and_then(|c| c.get_descriptors())
        .map(|ds| {
            ds.iter()
                .filter(|d| d.traits().symbolic_traits() & kCTFontItalicTrait == 0 && (d.traits().normalized_width() - width).abs() < 0.05)
                .map(|d| (d.traits().normalized_weight(), d.clone()))
                .collect()
        })
        .unwrap_or_default();
    let off = |t: f64| {
        let c = through_weights(t, true);
        // CSS looks heavier first from 400 up and lighter first below, so a tie goes that way.
        ((c - css).abs(), if css >= 400.0 { -c } else { c })
    };
    match cuts.into_iter().min_by(|a, b| off(a.0).partial_cmp(&off(b.0)).unwrap_or(std::cmp::Ordering::Equal)) {
        Some((_, d)) => core_text::font::new_from_descriptor(&d, f.pt_size()),
        // No family to choose from: CoreText's bold stands in for any heavy weight.
        None if css >= 600.0 => f.clone_with_symbolic_traits(kCTFontBoldTrait, kCTFontBoldTrait).unwrap_or_else(|| f.clone()),
        None => f.clone(),
    }
}

/// The font for CSS size `px` and `style`.
fn make_font(px: f32, style: u16, names: &[String]) -> CTFont {
    let size = px as f64;
    if style & MATH != 0 {
        let n = match (style & BOLD != 0, style & ITALIC != 0) {
            (false, false) => "STIXGeneral-Regular",
            (false, true) => "STIXGeneral-Italic",
            (true, false) => "STIXGeneral-Bold",
            (true, true) => "STIXGeneral-BoldItalic",
        };
        if let Ok(f) = new_from_name(n, size) {
            return f;
        }
    }
    let mut f = family(names, size, style & MONO != 0);
    let mut tr = 0;
    let w = style >> WEIGHT_SHIFT;
    if w != 0 {
        f = weighted(&f, w);
    } else if style & BOLD != 0 {
        tr |= kCTFontBoldTrait;
    }
    if style & ITALIC != 0 {
        tr |= kCTFontItalicTrait;
    }
    if tr != 0
        && let Some(n) = f.clone_with_symbolic_traits(tr, tr)
    {
        f = n;
    }
    if style & TABULAR != 0 {
        f = tabular(&f);
    }
    f
}

unsafe extern "C" {
    fn CTFontDescriptorCreateCopyWithFeature(
        original: core_text::font_descriptor::CTFontDescriptorRef,
        feature_type: core_foundation::number::CFNumberRef,
        feature_selector: core_foundation::number::CFNumberRef,
    ) -> core_text::font_descriptor::CTFontDescriptorRef;
}

/// `f` with monospaced digits: AAT feature type 6 (number spacing), selector 0.
fn tabular(f: &CTFont) -> CTFont {
    let (kind, sel) = (CFNumber::from(6i32), CFNumber::from(0i32));
    // SAFETY: every argument outlives its call, and the create rule takes each returned reference.
    unsafe {
        let d = CTFontDescriptorCreateCopyWithFeature(f.copy_descriptor().as_concrete_TypeRef(), kind.as_concrete_TypeRef(), sel.as_concrete_TypeRef());
        if d.is_null() {
            return f.clone();
        }
        let d = core_text::font_descriptor::CTFontDescriptor::wrap_under_create_rule(d);
        CTFont::wrap_under_create_rule(CTFontCreateWithFontDescriptor(d.as_concrete_TypeRef(), f.pt_size(), std::ptr::null()))
    }
}

#[cfg(test)]
mod weight_tests {
    use super::*;
    use core_text::font_descriptor::TraitAccessors;

    fn font(style: u16, names: &[&str]) -> CTFont {
        let names: Vec<String> = names.iter().map(std::string::ToString::to_string).collect();
        make_font(24.0, style, &names)
    }

    fn w(style: u16, names: &[&str]) -> f64 {
        font(style, names).all_traits().normalized_weight()
    }

    #[test]
    fn sf_draws_every_weight() {
        let ws: Vec<f64> = (1..=9).map(|n| w(weight(n * 100), &["system-ui"])).collect();
        assert!(ws.windows(2).all(|p| p[0] < p[1]), "SF gets heavier at each step: {ws:?}");
        // The in-between weights Obsidian's headings use land between the named cuts.
        let (six, sixty_eight, seven) = (w(SEMIBOLD, &["system-ui"]), w(weight(680), &["system-ui"]), w(weight(700), &["system-ui"]));
        assert!(six < sixty_eight && sixty_eight < seven, "{six} {sixty_eight} {seven}");
        // A weight goes on SF's weight axis exactly.
        assert!(font(SEMIBOLD, &["system-ui"]).postscript_name().ends_with(&format!("wght{:x}", 600 << 16)), "SF takes the exact weight on its axis, as Chrome does");
        assert!(font(SEMIBOLD | ITALIC, &["system-ui"]).all_traits().normalized_weight() > 0.25, "italic keeps the weight");
    }

    #[test]
    fn fixed_families_take_the_nearest_cut() {
        // Menlo has only regular and bold.
        assert_eq!(w(SEMIBOLD | MONO, &["Menlo"]), w(BOLD | MONO, &["Menlo"]));
        assert_eq!(w(weight(500) | MONO, &["Menlo"]), w(MONO, &["Menlo"]));
        assert_eq!(w(weight(100) | MONO, &["Menlo"]), w(MONO, &["Menlo"]));
        // Helvetica Neue has a medium (500) and a bold (700): 600 ties and goes heavier, 550 goes to medium.
        let name = |style| font(style, &["Helvetica Neue"]).postscript_name();
        assert_eq!(name(SEMIBOLD), "HelveticaNeue-Bold");
        assert_eq!(name(weight(550)), "HelveticaNeue-Medium");
        assert_eq!(name(weight(300)), "HelveticaNeue-Light");
        // 450 ties regular and medium, and CSS looks up first.
        assert_eq!(name(weight(450)), "HelveticaNeue-Medium");
    }

    #[test]
    fn a_weight_maps_there_and_back() {
        for w in [100.0, 450.0, 680.0, 900.0] {
            assert!((through_weights(through_weights(w, false), true) - w).abs() < 1e-9, "{w}");
        }
    }
}

#[cfg(test)]
mod emoji_tests {
    use super::*;
    use core_graphics::base::kCGImageAlphaPremultipliedLast;
    use core_graphics::color_space::CGColorSpace;

    fn ink(f: &mut Fonts, k: FontKey, s: &str) -> Vec<Vec<u32>> {
        let (w, h) = ((f.measure(k, s) + 2.0 * k.px()) as usize, (3.0 * k.px()) as usize);
        let mut px = vec![0u32; w * h];
        let ctx = CGContext::create_bitmap_context(Some(px.as_mut_ptr().cast()), w, h, 8, w * 4, &CGColorSpace::create_device_rgb(), kCGImageAlphaPremultipliedLast);
        ctx.translate(0.0, h as f64);
        ctx.scale(1.0, -1.0);
        f.draw(&ctx, k, s, k.px().round(), (2.0 * k.px()).round());
        drop(ctx);
        let rows: Vec<&[u32]> = px.chunks(w).filter(|r| r.iter().any(|&p| p != 0)).collect();
        let left = rows.iter().filter_map(|r| r.iter().position(|&p| p != 0)).min().unwrap_or(0);
        let right = rows.iter().filter_map(|r| r.iter().rposition(|&p| p != 0)).max().unwrap_or(0);
        rows.iter().map(|r| r[left..=right].to_vec()).collect()
    }

    fn left_of(f: &mut Fonts, k: FontKey, s: &str, keep: &str) -> Vec<u32> {
        let (w, h) = ((f.measure(k, s) + 2.0 * k.px()) as usize, (3.0 * k.px()) as usize);
        let mut px = vec![0u32; w * h];
        let ctx = CGContext::create_bitmap_context(Some(px.as_mut_ptr().cast()), w, h, 8, w * 4, &CGColorSpace::create_device_rgb(), kCGImageAlphaPremultipliedLast);
        ctx.translate(0.0, h as f64);
        ctx.scale(1.0, -1.0);
        ctx.set_rgb_fill_color(1.0, 1.0, 1.0, 1.0);
        f.draw(&ctx, k, s, k.px() + 0.3, 2.0 * k.px() + 0.6);
        drop(ctx);
        let cut = (k.px() + f.measure(k, keep)) as usize;
        px.chunks(w).flat_map(|r| r[..cut].to_vec()).collect()
    }

    fn inked(f: &mut Fonts, k: FontKey, s: &str, from: f32) -> usize {
        let (w, h) = ((f.measure(k, s) + 2.0 * k.px()) as usize, (3.0 * k.px()) as usize);
        let mut px = vec![0u32; w * h];
        let ctx = CGContext::create_bitmap_context(Some(px.as_mut_ptr().cast()), w, h, 8, w * 4, &CGColorSpace::create_device_rgb(), kCGImageAlphaPremultipliedLast);
        ctx.translate(0.0, h as f64);
        ctx.scale(1.0, -1.0);
        ctx.set_rgb_fill_color(1.0, 1.0, 1.0, 1.0);
        f.draw(&ctx, k, s, k.px(), 2.0 * k.px());
        drop(ctx);
        let from = (k.px() + from) as usize;
        px.chunks(w).map(|r| r[from..].iter().filter(|&&p| p != 0).count()).sum()
    }

    #[test]
    fn text_after_an_emoji_draws_in_place() {
        for scale in [1.0f32, 2.0] {
            let mut f = Fonts::new();
            f.set_families(vec!["system-ui".into()], vec!["Menlo".into()]);
            f.set_scale(scale);
            let k = FontKey::new(16.0 * scale, 0);
            let plain = inked(&mut f, k, "bbbbbbbb", 0.0);
            for e in ["✅", "👨‍👩‍👧", "🇦🇺", "❤️", "1️⃣", "👩‍❤️‍👨", "👩‍❤️‍💋‍👨"] {
                let s = format!("a{e}bbbbbbbb");
                let parts = f.measure(k, "a") + f.measure(k, e) + f.measure(k, "bbbbbbbb");
                assert!((f.measure(k, &s) - parts).abs() < 0.05, "{e} at {scale}x: the line is as wide as its parts");
                let after = f.measure(k, &format!("a{e}"));
                assert!(inked(&mut f, k, &s, after) * 10 >= plain * 9, "{e} at {scale}x: the text after it draws");
            }
            let t = k.tracked(0.1);
            assert!((f.measure(t, "a✅b") - f.measure(k, "a✅b") - 3.0 * (f.measure(t, "ab") - f.measure(k, "ab")) / 2.0).abs() < 0.01, "letter-spacing follows the emoji too at {scale}x");
        }
    }

    #[test]
    fn mixed_direction_text_keeps_its_order_around_emoji() {
        for scale in [1.0f32, 2.0] {
            let mut f = Fonts::new();
            f.set_families(vec!["system-ui".into()], vec!["Menlo".into()]);
            f.set_scale(scale);
            let k = FontKey::new(16.0 * scale, 0);
            for s in ["שלום 🇦🇺 עולם", "שלום 1️⃣ abc עולם", "abc ✅ שלום 🇦🇺 def", "مرحبا 🇯🇵 123 عالم", "אב🇦🇺גד"] {
                let plain = CTLine::new_with_attributed_string(attributed(f.font(k), s, 0.0).as_concrete_TypeRef());
                let ours = f.line(k, s).line.clone();
                // SAFETY: each run belongs to `plain`, which outlives the call.
                let emoji: Vec<CFRange> = bitmap_fonts(&plain).map(|(r, _)| unsafe { CTRunGetStringRange(r.as_concrete_TypeRef()) }).collect();
                let order = |line: &CTLine| -> Vec<isize> {
                    let mut v: Vec<(isize, f64)> = line
                        .glyph_runs()
                        .iter()
                        .flat_map(|r| r.string_indices().iter().copied().zip(r.positions().iter().map(|p| p.x)).collect::<Vec<_>>())
                        .filter(|g| !emoji.iter().any(|e| (e.location..e.location + e.length).contains(&g.0)))
                        .collect();
                    v.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
                    v.into_iter().map(|g| g.0).collect()
                };
                assert_eq!(order(&ours), order(&plain), "{s} at {scale}x: the text keeps its visual order");
            }
        }
    }

    #[test]
    fn text_beside_an_emoji_draws_as_anywhere_else() {
        for scale in [1.0f32, 2.0] {
            let mut f = Fonts::new();
            f.set_families(vec!["system-ui".into()], vec!["Menlo".into()]);
            f.set_scale(scale);
            let k = FontKey::new(16.0 * scale, 0);
            for text in ["abc def", "ạ́ é̃ q̣̃"] {
                assert_eq!(left_of(&mut f, k, &format!("{text} ✅yz"), text), left_of(&mut f, k, &format!("{text} xyz"), text), "{text} at {scale}x");
            }
            for emoji in ["✅", "🚫", "👨‍👩‍👧", "✅🚫"] {
                let w = f.measure(k, emoji);
                assert!((f.measure(k, &format!("{emoji} after")) - w - f.measure(k, " after")).abs() < 0.01, "{emoji}");
            }
        }
    }

    fn green(f: &mut Fonts, k: FontKey, s: &str, native: bool) -> (usize, usize) {
        let (w, h) = ((f.measure(k, s) + 2.0 * k.px()) as usize, (3.0 * k.px()) as usize);
        let mut px = vec![0u8; w * h * 4];
        let ctx = CGContext::create_bitmap_context(Some(px.as_mut_ptr().cast()), w, h, 8, w * 4, &CGColorSpace::create_device_rgb(), kCGImageAlphaPremultipliedLast);
        ctx.translate(0.0, h as f64);
        ctx.scale(1.0, -1.0);
        if native {
            ctx.set_text_matrix(&CGAffineTransform::new(1.0, 0.0, 0.0, -1.0, f64::from(k.px()), f64::from(2.0 * k.px())));
            CTLine::new_with_attributed_string(attributed(f.font(k), s, 0.0).as_concrete_TypeRef()).draw(&ctx);
        } else {
            f.draw(&ctx, k, s, k.px(), 2.0 * k.px());
        }
        drop(ctx);
        let xs: Vec<usize> = px.chunks(4).enumerate().filter(|(_, p)| p[1] > 150 && p[0] < 80 && p[3] > 200).map(|(i, _)| i % w).collect();
        (*xs.iter().min().unwrap_or(&0), *xs.iter().max().unwrap_or(&0))
    }

    #[test]
    fn every_emoji_draws_where_coretext_places_it() {
        for s in ["a ✅ b", "✅❌", "✅🚀🎉a", "שלום ✅ עולם", "שלום ✅✅ עולם"] {
            let mut f = Fonts::new();
            f.set_families(vec!["system-ui".into()], vec!["Menlo".into()]);
            let one = FontKey::new(16.0, 0);
            let plain = CTLine::new_with_attributed_string(attributed(f.font(one), s, 0.0).as_concrete_TypeRef());
            let sbix: usize = plain.glyph_runs().iter().filter(|r| run_font(r).is_some_and(|f| has_sbix(&f))).map(|r| r.glyph_count() as usize).sum();
            assert_eq!(f.line(one, s).emoji.len(), sbix, "{s}: every bitmap glyph is drawn");
            let (ours, theirs) = (green(&mut f, one, s, false), green(&mut f, one, s, true));
            assert!(ours.0.abs_diff(theirs.0) <= 1, "{s} at 1x starts where CoreText puts it: {ours:?} vs CoreText {theirs:?}");
            f.set_scale(2.0);
            let two = green(&mut f, FontKey::new(32.0, 0), s, false);
            assert!(two.0.abs_diff(2 * ours.0) <= 2, "{s} at 2x starts at twice its 1x place: {two:?} vs 1x {ours:?}");
        }
    }

    fn native(px: f32) -> Vec<Vec<u32>> {
        let font = new_from_name("AppleColorEmoji", f64::from(px)).expect("macOS ships Apple Color Emoji");
        let glyph = font.get_glyph_with_name("u2705");
        let (w, h) = ((3.0 * px) as usize, (3.0 * px) as usize);
        let mut buf = vec![0u32; w * h];
        let ctx = CGContext::create_bitmap_context(Some(buf.as_mut_ptr().cast()), w, h, 8, w * 4, &CGColorSpace::create_device_rgb(), kCGImageAlphaPremultipliedLast);
        ctx.translate(0.0, h as f64);
        ctx.scale(1.0, -1.0);
        ctx.set_text_matrix(&CGAffineTransform::new(1.0, 0.0, 0.0, -1.0, f64::from(px.round()), f64::from((2.0 * px).round())));
        font.draw_glyphs(&[glyph], &[CGPoint::new(0.0, 0.0)], ctx.clone());
        drop(ctx);
        crop(&buf, w)
    }

    fn crop(px: &[u32], w: usize) -> Vec<Vec<u32>> {
        let rows: Vec<&[u32]> = px.chunks(w).filter(|r| r.iter().any(|&p| p != 0)).collect();
        let left = rows.iter().filter_map(|r| r.iter().position(|&p| p != 0)).min().unwrap_or(0);
        let right = rows.iter().filter_map(|r| r.iter().rposition(|&p| p != 0)).max().unwrap_or(0);
        rows.iter().map(|r| r[left..=right].to_vec()).collect()
    }

    #[test]
    fn emoji_draw_like_a_native_font_of_their_size() {
        let mut f = Fonts::new();
        f.set_families(vec!["system-ui".into()], vec!["Menlo".into()]);
        let text = f.measure(FontKey::new(16.0, 0), "a  b");
        for scale in [1.0f32, 1.5, 2.0] {
            let k = FontKey::new(16.0 * scale, 0);
            f.set_scale(scale);
            assert!((f.measure(k, "a  b") - text * scale).abs() < 1.0, "text lays out the same at {scale}x");
            assert_eq!(ink(&mut f, k, "✅"), native(k.px()), "{scale}x draws the emoji as a {} px font does, and nothing else", k.px());
        }
    }

    #[test]
    fn emoji_are_as_wide_as_chrome_makes_them() {
        let mut f = Fonts::new();
        f.set_families(vec!["system-ui".into()], vec!["Menlo".into()]);
        for (px, chrome) in [(16.0f32, [20.0f32, 17.6, 16.0, 16.0]), (24.0, [24.0, 24.0, 24.0, 24.0])] {
            for (scale, want) in [1.0f32, 1.25, 1.5, 2.0].into_iter().zip(chrome) {
                f.set_scale(scale);
                let got = f.measure(FontKey::new(px * scale, 0), "✅") / scale;
                assert!((got - want).abs() < 0.01, "a {px} px ✅ at {scale}x is {got} CSS px, Chrome makes it {want}");
            }
        }
    }
}
