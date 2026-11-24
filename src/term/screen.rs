//! The screen the terminal is on.

/// The scale the first native page lays out at while `display_scale` reads the real one: every Mac
/// screen since 2016 but an external 1x display.
#[cfg(target_os = "macos")]
pub const SCALE_GUESS: f64 = 2.0;
#[cfg(not(target_os = "macos"))]
pub const SCALE_GUESS: f64 = 1.0;

static SCALE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The backing scale of the screen the terminal is on, from CoreGraphics. It needs a window-server
/// connection, about 25 ms, so the viewer starts it on a thread at launch and lays out at `SCALE_GUESS`
/// meanwhile. Unlike `NSScreen`, it runs off the main thread and starts no AppKit. `MDDOG_SCALE` overrides it, so
/// tests that click at terminal cells hit the same page points on a 1x CI runner as on a Retina Mac.
pub fn display_scale() -> f64 {
    use std::sync::atomic::Ordering::Relaxed;
    let cached = f64::from_bits(SCALE.load(Relaxed));
    if cached > 0.0 {
        return cached;
    }
    pinned().or_else(system_scale).map_or(SCALE_GUESS, |s| {
        let _ = SCALE.compare_exchange(0, s.to_bits(), Relaxed, Relaxed);
        f64::from_bits(SCALE.load(Relaxed))
    })
}

fn pinned() -> Option<f64> {
    std::env::var("MDDOG_SCALE").ok().and_then(|s| s.parse().ok()).filter(|&s: &f64| s > 0.0)
}

#[cfg(not(target_os = "macos"))]
pub fn follow_cell(cell_h: u16) {
    if pinned().is_none()
        && let Some(s) = scale_of_cell(cell_h)
    {
        SCALE.store(s.to_bits(), std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(target_os = "macos")]
pub fn follow_cell(_cell_h: u16) {}

#[cfg(not(target_os = "macos"))]
static PROBED_CELL_H: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

#[cfg(not(target_os = "macos"))]
pub fn set_probed_cell(cell_h: u16) {
    PROBED_CELL_H.store(cell_h, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(target_os = "macos")]
pub fn set_probed_cell(_cell_h: u16) {}

#[cfg(not(target_os = "macos"))]
fn system_scale() -> Option<f64> {
    let window = std::fs::File::open("/dev/tty").ok().map_or(0, |tty| {
        let size = crate::term::size_of(std::os::fd::AsRawFd::as_raw_fd(&tty));
        size.ypix.checked_div(size.rows).unwrap_or(0)
    });
    scale_of_cell(window).or_else(|| scale_of_cell(PROBED_CELL_H.load(std::sync::atomic::Ordering::Relaxed)))
}

#[cfg(not(target_os = "macos"))]
fn scale_of_cell(cell_h: u16) -> Option<f64> {
    (cell_h > 0).then(|| (f64::from(cell_h) / 19.0 * 4.0).round().clamp(4.0, 12.0) / 4.0)
}

#[cfg(target_os = "macos")]
fn system_scale() -> Option<f64> {
    use core_graphics::display::CGDisplay;
    let scale = |id| CGDisplay::new(id).display_mode().filter(|m| m.width() > 0).map(|m| m.pixel_width() as f64 / m.width() as f64);
    let scales: Vec<f64> = CGDisplay::active_displays().unwrap_or_default().into_iter().filter_map(scale).collect();
    if let Some(&first) = scales.first()
        && scales.iter().all(|&s| s == first)
    {
        return Some(first);
    }
    // Screens of mixed scales: `NSScreen.mainScreen` meant the one with the focused window, which is the
    // terminal mddog was just typed into.
    front_window_center()
        .and_then(|p| CGDisplay::displays_with_point(p, 1).ok())
        .and_then(|(ids, n)| if n > 0 { ids.first().copied() } else { None })
        .or(Some(CGDisplay::main().id))
        .and_then(scale)
        .or(Some(SCALE_GUESS))
}

/// The centre of the frontmost ordinary window, in global display coordinates. Bounds and layer need no
/// screen-recording permission.
#[cfg(target_os = "macos")]
fn front_window_center() -> Option<core_graphics::geometry::CGPoint> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly};
    type Dict = CFDictionary<CFString, CFType>;
    let dict = |v: &CFType| -> Option<Dict> {
        // SAFETY: the type ID check makes it a CFDictionary; the get rule retains it for the wrapper.
        (v.type_of() == Dict::type_id()).then(|| unsafe { Dict::wrap_under_get_rule(v.as_CFTypeRef().cast()) })
    };
    let num = |d: &Dict, k: &str| d.find(CFString::new(k)).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_f64());
    let list = copy_window_info(kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, kCGNullWindowID)?;
    list.iter().find_map(|item| {
        // SAFETY: the window list holds CFDictionaries, alive while `list` is.
        let w = unsafe { Dict::wrap_under_get_rule((*item).cast()) };
        if num(&w, "kCGWindowLayer") != Some(0.0) {
            return None;
        }
        let b = dict(&*w.find(CFString::new("kCGWindowBounds"))?)?;
        Some(core_graphics::geometry::CGPoint::new(num(&b, "X")? + num(&b, "Width")? / 2.0, num(&b, "Y")? + num(&b, "Height")? / 2.0))
    })
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn the_cell_height_sets_the_scale() {
        assert_eq!(scale_of_cell(0), None, "an unknown cell sets no scale, so a probed one can");
        assert_eq!(scale_of_cell(12), Some(1.0));
        assert_eq!(scale_of_cell(20), Some(1.0));
        assert_eq!(scale_of_cell(24), Some(1.25));
        assert_eq!(scale_of_cell(29), Some(1.5));
        assert_eq!(scale_of_cell(38), Some(2.0));
        assert_eq!(scale_of_cell(40), Some(2.0));
        assert_eq!(scale_of_cell(200), Some(3.0));
    }

    #[test]
    fn a_settled_cell_replaces_the_first_scale() {
        follow_cell(16);
        assert_eq!(display_scale(), 1.0);
        follow_cell(38);
        assert_eq!(display_scale(), 2.0);
        follow_cell(0);
        assert_eq!(display_scale(), 2.0);
    }
}
