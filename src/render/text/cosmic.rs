use super::*;
use crate::render::canvas::{Canvas, Pixels};
use cosmic_text::{Attrs, Buffer, CacheKeyFlags, Family, FeatureTag, FontFeatures, FontSystem, LayoutGlyph, Metrics, Shaping, Style, SwashCache, SwashContent, Weight, Wrap, fontdb};
use std::collections::HashMap;
use std::sync::OnceLock;

const SANS_FONTS: &[&str] = &["Adwaita Sans", "Inter", "Cantarell", "Noto Sans", "DejaVu Sans", "Liberation Sans", "Ubuntu"];
const MONO_FONTS: &[&str] = &["Adwaita Mono", "Noto Sans Mono", "DejaVu Sans Mono", "Liberation Mono", "Ubuntu Mono"];
const SERIF_FONTS: &[&str] = &["Noto Serif", "DejaVu Serif", "Liberation Serif"];
const MATH_FONTS: &[&str] = &["STIX Two Text", "STIXGeneral", "STIX Two Math", "Latin Modern Roman", "DejaVu Serif"];
const EMOJI_FONTS: &[&str] = &["Noto Color Emoji", "Twemoji", "JoyPixels"];

struct System {
    locale: String,
    db: fontdb::Database,
    sans: String,
    mono: String,
    serif: String,
    math: Option<String>,
    emoji: Option<String>,
}

fn installed(db: &fontdb::Database, name: &str) -> Option<String> {
    db.faces().flat_map(|f| f.families.iter()).find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(n, _)| n.clone())
}

fn fc_match(alias: &str) -> Option<std::process::Child> {
    use std::process::{Command, Stdio};
    Command::new("fc-match").args(["-f", "%{family[0]}", alias]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()
}

fn matched(child: Option<std::process::Child>, until: std::time::Instant) -> Option<String> {
    let mut child = child?;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < until => std::thread::sleep(std::time::Duration::from_millis(1)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let out = child.wait_with_output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn generic(db: &fontdb::Database, matched: Option<String>, fallbacks: &[&str]) -> String {
    matched
        .and_then(|n| installed(db, &n))
        .or_else(|| fallbacks.iter().find_map(|n| installed(db, n)))
        .or_else(|| db.faces().filter_map(|f| f.families.first()).map(|(n, _)| n).find(|n| !["emoji", "symbol", "math"].iter().any(|w| n.to_ascii_lowercase().contains(w))).cloned())
        .unwrap_or_default()
}

fn color(db: &fontdb::Database, name: &str) -> bool {
    use cosmic_text::skrifa::{FontRef, Tag};
    let q = fontdb::Query { families: &[Family::Name(name)], ..fontdb::Query::default() };
    db.query(&q)
        .and_then(|id| db.with_face_data(id, |data, i| FontRef::from_index(data, i).ok().is_some_and(|f| [b"CBDT", b"COLR", b"sbix"].iter().any(|t| f.table_data(Tag::new(t)).is_some()))))
        .unwrap_or(false)
}

fn emoji_family(db: &fontdb::Database, matched: Option<String>) -> Option<String> {
    matched.and_then(|n| installed(db, &n)).filter(|n| color(db, n)).or_else(|| EMOJI_FONTS.iter().find_map(|n| installed(db, n)))
}

fn system() -> &'static System {
    static SYSTEM: OnceLock<System> = OnceLock::new();
    SYSTEM.get_or_init(|| {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let asked = ["sans-serif", "monospace", "serif", "emoji"].map(fc_match);
        let (locale, mut db) = FontSystem::new().into_locale_and_db();
        let [sans, mono, serif, emoji] = asked.map(|c| matched(c, until));
        let (sans, mono, serif) = (generic(&db, sans, SANS_FONTS), generic(&db, mono, MONO_FONTS), generic(&db, serif, SERIF_FONTS));
        db.set_sans_serif_family(sans.clone());
        db.set_monospace_family(mono.clone());
        db.set_serif_family(serif.clone());
        let math = MATH_FONTS.iter().find_map(|n| installed(&db, n));
        let emoji = emoji_family(&db, emoji);
        System { locale, db, sans, mono, serif, math, emoji }
    })
}

pub fn warm() {
    system();
}

pub fn sans_family() -> &'static str {
    &system().sans
}

pub fn mono_family() -> &'static str {
    &system().mono
}

pub fn serif_family() -> &'static str {
    &system().serif
}

pub fn missing_fonts() -> Option<&'static str> {
    system()
        .sans
        .is_empty()
        .then_some("no fonts found, so mddog can't draw text. Install a font package, such as fonts-dejavu-core on Debian or Ubuntu, dejavu-sans-fonts on Fedora, or font-dejavu on Alpine.")
}

fn family(names: &[String], mono: bool) -> String {
    let sys = system();
    for n in names {
        match n.to_ascii_lowercase().as_str() {
            n if system_ui(n) => return sys.sans.clone(),
            "ui-monospace" | "monospace" => return sys.mono.clone(),
            "ui-serif" | "serif" => return sys.serif.clone(),
            n => {
                if let Some(f) = installed(&sys.db, n) {
                    return f;
                }
            }
        }
    }
    if mono { sys.mono.clone() } else { sys.sans.clone() }
}

fn covers(db: &fontdb::Database, id: fontdb::ID, w: u16) -> bool {
    use cosmic_text::skrifa::{FontRef, MetadataProvider, Tag};
    db.with_face_data(id, |data, i| {
        let axis = FontRef::from_index(data, i).ok()?.axes().get_by_tag(Tag::new(b"wght"))?;
        Some(axis.min_value() <= f32::from(w) && f32::from(w) <= axis.max_value())
    })
    .flatten()
    .unwrap_or(false)
}

fn nearest_weight(name: &str, w: Weight, style: Style) -> Weight {
    let db = &system().db;
    let q = fontdb::Query { families: &[Family::Name(name)], weight: w, stretch: fontdb::Stretch::Normal, style };
    db.query(&q).and_then(|id| db.face(id)).map_or(w, |f| if f.weight == w || covers(db, f.id, w.0) { w } else { f.weight })
}

const LIFT: f32 = 65536.0;

struct Line {
    glyphs: Vec<LayoutGlyph>,
    w: f32,
    rtl: bool,
}

pub struct Fonts {
    fs: FontSystem,
    cache: SwashCache,
    lines: HashMap<(FontKey, String), Line>,
    metrics: HashMap<FontKey, (f32, f32)>,
    weights: HashMap<u16, Weight>,
    body: String,
    code: String,
    scale: f32,
}

impl Fonts {
    pub fn new() -> Fonts {
        let sys = system();
        let fs = FontSystem::new_with_locale_and_db(sys.locale.clone(), sys.db.clone());
        Fonts { fs, cache: SwashCache::new(), lines: HashMap::new(), metrics: HashMap::new(), weights: HashMap::new(), body: sys.sans.clone(), code: sys.mono.clone(), scale: 1.0 }
    }

    pub fn set_families(&mut self, body: Vec<String>, code: Vec<String>) {
        let (body, code) = (family(&body, false), family(&code, true));
        if (&body, &code) != (&self.body, &self.code) {
            (self.body, self.code) = (body, code);
            self.lines.clear();
            self.metrics.clear();
            self.weights.clear();
        }
    }

    pub fn set_scale(&mut self, scale: f32) {
        if scale != self.scale {
            self.scale = scale;
            self.cache = SwashCache::new();
            self.lines.clear();
        }
    }

    fn name(&self, k: FontKey) -> String {
        match system().math.as_deref() {
            Some(m) if k.style & MATH != 0 => m.to_string(),
            _ if k.style & MONO != 0 => self.code.clone(),
            _ => self.body.clone(),
        }
    }
}

fn style(k: FontKey) -> Style {
    if k.style & ITALIC != 0 { Style::Italic } else { Style::Normal }
}

fn attrs(name: &str, k: FontKey, weight: Weight) -> Attrs<'_> {
    let mut a = Attrs::new().family(Family::Name(name)).weight(weight).style(style(k)).cache_key_flags(CacheKeyFlags::DISABLE_HINTING);
    if k.style & TABULAR != 0 {
        let mut f = FontFeatures::new();
        f.enable(FeatureTag::new(b"tnum"));
        a = a.font_features(f);
    }
    if k.track != 0 {
        a = a.letter_spacing(f32::from(k.track) / 1000.0);
    }
    a
}

impl Fonts {
    pub fn ascent_descent(&mut self, k: FontKey) -> (f32, f32) {
        let k = FontKey { track: 0, ..k };
        if let Some(m) = self.metrics.get(&k) {
            return *m;
        }
        let px = k.px();
        let first = self.line(k, "H").glyphs.first().map(|g| (g.font_id, g.font_weight));
        let m = first
            .and_then(|(id, w)| self.fs.get_font(id, w))
            .map(|f| {
                let m = f.as_swash().metrics(&[]);
                let em = f32::from(m.units_per_em.max(1));
                (m.ascent / em * px, m.descent.abs() / em * px)
            })
            .unwrap_or((px * 0.95, px * 0.25));
        self.metrics.insert(k, m);
        m
    }

    fn line(&mut self, k: FontKey, s: &str) -> &Line {
        if self.lines.len() > 20000 {
            self.lines.clear();
        }
        let key = (k, s.to_string());
        if !self.lines.contains_key(&key) {
            let name = self.name(k);
            let weight = *self.weights.entry(k.style).or_insert_with(|| {
                let w = weight_of(k.style);
                let wanted = if w != 0 {
                    Weight(w)
                } else if k.style & BOLD != 0 {
                    Weight::BOLD
                } else {
                    Weight::NORMAL
                };
                nearest_weight(&name, wanted, style(k))
            });
            let attrs = attrs(&name, k, weight);
            let px = k.px().max(0.1);
            let mut b = Buffer::new(&mut self.fs, Metrics::new(px, px * 1.25));
            b.set_wrap(Wrap::None);
            b.set_size(None, None);
            let emoji = system().emoji.as_deref();
            match emoji.filter(|_| s.contains(['\u{fe0f}', '\u{20e3}'])) {
                Some(e) => {
                    let pictured = Attrs { family: Family::Name(e), weight: Weight::NORMAL, style: Style::Normal, ..attrs.clone() };
                    let spans = unicode_segmentation::UnicodeSegmentation::graphemes(s, true).map(|g| (g, if g.contains(['\u{fe0f}', '\u{20e3}']) { pictured.clone() } else { attrs.clone() }));
                    b.set_rich_text(spans, &attrs, Shaping::Advanced, None);
                }
                None => b.set_text(s, &attrs, Shaping::Advanced, None),
            }
            b.shape_until_scroll(&mut self.fs, false);
            let mut glyphs = vec![];
            let (mut w, mut rtl): (f32, bool) = (0.0, false);
            for run in b.layout_runs() {
                glyphs.extend_from_slice(run.glyphs);
                w = w.max(run.line_w);
                rtl |= run.rtl;
            }
            if let Some(e) = emoji {
                let db = self.fs.db();
                let pictures: Vec<bool> = glyphs.iter().map(|g| db.face(g.font_id).is_some_and(|f| f.families.iter().any(|(n, _)| n == e))).collect();
                w += whole_css_px(&mut glyphs, &pictures, self.scale);
            }
            self.lines.insert(key.clone(), Line { glyphs, w, rtl });
        }
        &self.lines[&key]
    }

    pub fn measure(&mut self, k: FontKey, s: &str) -> f32 {
        self.line(k, s).w
    }

    pub fn char_x(&mut self, k: FontKey, s: &str, idx: usize) -> f32 {
        let b = s.char_indices().nth(idx).map_or(s.len(), |(b, _)| b);
        let line = self.line(k, s);
        let (start, end) = if line.rtl { (line.w, 0.0) } else { (0.0, line.w) };
        if b == 0 {
            return start;
        }
        if b == s.len() {
            return end;
        }
        line.glyphs.iter().find(|g| g.start <= b && b < g.end).map_or(end, |g| {
            let chars = s[g.start..g.end].chars().count().max(1) as f32;
            let part = s[g.start..b].chars().count() as f32 / chars;
            if g.level.is_rtl() { g.x + g.w * (1.0 - part) } else { g.x + g.w * part }
        })
    }

    pub fn draw(&mut self, c: &mut Canvas, k: FontKey, s: &str, x: f32, y: f32) {
        self.line(k, s);
        let Fonts { fs, cache, lines, .. } = self;
        let Some(line) = lines.get(&(k, s.to_string())) else { return };
        let (z, ox, oy) = c.place(x, y);
        for g in &line.glyphs {
            let pg = g.physical((ox + LIFT, oy + LIFT), z);
            let Some(img) = cache.get_image(fs, pg.cache_key) else { continue };
            let (w, h) = (img.placement.width, img.placement.height);
            if w == 0 || h == 0 {
                continue;
            }
            let pixels = match (img.content, img.source) {
                (SwashContent::Mask, _) => Pixels::Coverage,
                (SwashContent::Color, swash::scale::Source::ColorOutline(_)) => Pixels::Premultiplied,
                (SwashContent::Color, _) => Pixels::Straight,
                (SwashContent::SubpixelMask, _) => continue,
            };
            c.glyph(pg.x - LIFT as i32 + img.placement.left, pg.y - LIFT as i32 - img.placement.top, w, h, &img.data, pixels);
        }
    }
}

fn whole_css_px(glyphs: &mut [LayoutGlyph], round: &[bool], scale: f32) -> f32 {
    if !round.contains(&true) {
        return 0.0;
    }
    let mut order: Vec<usize> = (0..glyphs.len()).collect();
    order.sort_by(|&a, &b| glyphs[a].x.total_cmp(&glyphs[b].x));
    let mut shift = 0.0;
    for i in order {
        let g = &mut glyphs[i];
        g.x += shift;
        if round[i] {
            let w = (g.w / scale).round() * scale;
            shift += w - g.w;
            g.w = w;
        }
    }
    shift
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caret_offsets_follow_the_text_direction() {
        let mut f = Fonts::new();
        let k = FontKey::new(16.0, 0);
        let ltr: Vec<f32> = (0..=3).map(|i| f.char_x(k, "abc", i)).collect();
        assert!(ltr[0].abs() < 0.01 && ltr.windows(2).all(|p| p[0] < p[1]), "{ltr:?}");
        assert!((ltr[3] - f.measure(k, "abc")).abs() < 0.01, "{ltr:?}");
        let s = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}";
        let w = f.measure(k, s);
        let rtl: Vec<f32> = (0..=4).map(|i| f.char_x(k, s, i)).collect();
        assert!((rtl[0] - w).abs() < 0.01 && rtl[4].abs() < 0.01, "a right-to-left run starts at its right edge: {rtl:?} in {w}");
        assert!(rtl.windows(2).all(|p| p[0] > p[1]), "{rtl:?}");
        let mixed = "abc\u{5e9}\u{5dc}\u{5d5}\u{5dd}";
        let w = f.measure(k, mixed);
        assert!((f.char_x(k, mixed, 7) - w).abs() < 0.01, "a left-to-right line ends at its right edge, whatever its last run");
    }

    #[test]
    fn a_weight_the_family_lacks_stays_in_the_family() {
        let mut f = Fonts::new();
        for style in [SEMIBOLD, weight(500), SEMIBOLD | ITALIC] {
            let id = f.line(FontKey::new(16.0, style), "H").glyphs[0].font_id;
            let face = f.fs.db().face(id).unwrap();
            assert!(face.families.iter().any(|(n, _)| n == sans_family()), "weight {} drew in {:?}, not {}", weight_of(style), face.families, sans_family());
        }
    }

    const EMOJI: [&str; 7] = ["✅", "👨‍👩‍👧", "🇦🇺", "❤️", "1️⃣", "🚀", "👩‍❤️‍👨"];

    fn families(f: &mut Fonts, k: FontKey, s: &str) -> Vec<String> {
        let ids: Vec<_> = f.line(k, s).glyphs.iter().map(|g| g.font_id).collect();
        ids.into_iter().map(|id| f.fs.db().face(id).and_then(|face| face.families.first()).map_or_else(String::new, |(n, _)| n.clone())).collect()
    }

    #[test]
    fn emoji_presentation_takes_the_emoji_font() {
        let mut f = Fonts::new();
        for style in [0, BOLD, SEMIBOLD, ITALIC] {
            let k = FontKey::new(32.0, style);
            for e in EMOJI {
                let got = families(&mut f, k, &format!("a{e}b"));
                let inner = &got[1..got.len() - 1];
                assert!(!inner.is_empty() && inner.iter().all(|n| n.to_ascii_lowercase().contains("emoji")), "{e} in style {style} drew in {got:?}");
            }
        }
        let k = FontKey::new(16.0, 0);
        assert!(families(&mut f, k, "\u{2764}").iter().all(|n| !n.to_ascii_lowercase().contains("emoji")), "a heart with no U+FE0F is text, as in Chrome");
    }

    #[test]
    fn emoji_are_as_wide_as_chrome_makes_them() {
        for scale in [1.0f32, 1.5, 2.0] {
            let mut f = Fonts::new();
            f.set_scale(scale);
            for (px, want) in [(16.0, 20.0), (15.0, 19.0), (13.0, 16.0), (32.0, 40.0)] {
                let k = FontKey::new(px * scale, 0);
                for e in EMOJI {
                    let got = f.measure(k, e) / scale;
                    assert!((got - want).abs() < 0.01, "{e} at {px} px and {scale}x is {got} CSS px, Chrome makes it {want}");
                }
            }
        }
    }

    #[test]
    fn the_emoji_font_is_known_by_its_color_glyphs() {
        let db = &system().db;
        assert!(color(db, "Noto Color Emoji"));
        assert!(!color(db, sans_family()) && !color(db, "DejaVu Sans"));
        assert_eq!(emoji_family(db, Some("DejaVu Sans".into())).as_deref(), Some("Noto Color Emoji"), "fc-match answers a text font when no emoji font is set up");
        assert_eq!(emoji_family(db, Some("Noto Color Emoji".into())).as_deref(), Some("Noto Color Emoji"));
        assert_eq!(emoji_family(db, None).as_deref(), Some("Noto Color Emoji"));
    }

    #[test]
    fn a_new_scale_rounds_emoji_again() {
        let mut f = Fonts::new();
        let k = FontKey::new(15.0, 0);
        assert!((f.measure(k, "✅") - 19.0).abs() < 0.01);
        f.set_scale(1.5);
        assert!((f.measure(k, "✅") - 18.0).abs() < 0.01, "10 CSS px text at 1.5x: {}", f.measure(k, "✅"));
    }

    #[test]
    fn text_beside_an_emoji_lays_out_as_anywhere_else() {
        let mut f = Fonts::new();
        let k = FontKey::new(16.0, 0);
        for e in EMOJI {
            let whole = f.measure(k, &format!("{e} after"));
            let parts = f.measure(k, e) + f.measure(k, " after");
            assert!((whole - parts).abs() < 0.01, "{e}: {whole} vs {parts}");
            let s = format!("a{e}b");
            let xs: Vec<f32> = (0..=s.chars().count()).map(|i| f.char_x(k, &s, i)).collect();
            assert!(xs.windows(2).all(|p| p[0] <= p[1]) && (xs[xs.len() - 1] - f.measure(k, &s)).abs() < 0.01, "{e}: {xs:?}");
        }
        let rtl = "\u{5e9}\u{5dc}\u{5d5}\u{5dd} ✅ \u{5e2}\u{5d5}\u{5dc}\u{5dd}";
        let parts = f.measure(k, "\u{5e9}\u{5dc}\u{5d5}\u{5dd} ") + f.measure(k, "✅") + f.measure(k, " \u{5e2}\u{5d5}\u{5dc}\u{5dd}");
        assert!((f.measure(k, rtl) - parts).abs() < 0.01, "an emoji in right-to-left text keeps its width");
    }

    #[test]
    fn letter_spacing_is_in_em() {
        let mut f = Fonts::new();
        for px in [16.0, 32.0] {
            let plain = f.measure(FontKey::new(px, 0), "abcd");
            let tracked = f.measure(FontKey::new(px, 0).tracked(-0.05), "abcd");
            assert!((plain - tracked - 4.0 * 0.05 * px).abs() < 0.01, "{px} px: {plain} untracked, {tracked} at -0.05 em");
        }
    }
}
