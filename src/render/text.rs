//! Fonts, text measurement and drawing. CoreText does the work on macOS, and cosmic-text elsewhere.
#[cfg(target_os = "macos")]
mod coretext;
#[cfg(target_os = "macos")]
pub use coretext::Fonts;
#[cfg(target_os = "macos")]
pub fn warm() {}
#[cfg(target_os = "macos")]
pub fn missing_fonts() -> Option<&'static str> {
    None
}
#[cfg(not(target_os = "macos"))]
mod cosmic;
#[cfg(not(target_os = "macos"))]
pub use cosmic::{Fonts, missing_fonts, mono_family, sans_family, serif_family, warm};

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
pub const fn weight_of(style: u16) -> u16 {
    style >> WEIGHT_SHIFT
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

/// CSS's names for the system font, in lower case.
pub fn system_ui(name: &str) -> bool {
    matches!(name, "system-ui" | "-apple-system" | "blinkmacsystemfont" | "ui-sans-serif" | "sans-serif")
}
