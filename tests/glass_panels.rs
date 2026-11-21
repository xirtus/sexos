//! Host tests for frosted glass behind the desktop's popups (slice S6b): the
//! exact `servers/sexdisplay/src/glass.rs` Display compiles (blur, tint and
//! the luminance band), with the exact desktop theme colours
//! (`servers/sexdisplay/src/desk_theme.rs`) over `crates/sex-theme`, and the
//! Reduce transparency bit of `crates/sex-pdx/src/desktop_look.rs`.
//!
//! The crate root stands in for the crates desk_theme names, as in
//! `tests/desktop_theme.rs`.
extern crate self as sex_theme;
extern crate self as sex_pdx;
extern crate self as libm;

#[path = "../crates/sex-theme/src/lib.rs"]
mod theme_source;
pub use theme_source::*;
pub fn powf(x: f32, y: f32) -> f32 { x.powf(y) }

#[path = "../crates/sex-pdx/src/desktop_look.rs"]
pub mod desktop_look;
#[path = "../crates/sex-pdx/src/app_catalog.rs"]
pub mod app_catalog;
#[path = "../crates/sex-pdx/src/desktop_style.rs"]
pub mod desktop_style;
#[macro_export]
macro_rules! serial_record { ($($t:tt)*) => {{ let _ = format_args!($($t)*); }}; }

mod tray_icons {
    pub const FG: u32 = 0x00F1EEF4;
    pub const DIM: u32 = 0x006E6875;
    pub const WARN: u32 = 0x00F38BA8;
    pub const GOOD: u32 = 0x00A6E3A1;
}

#[path = "../servers/sexdisplay/src/desk_theme.rs"]
mod desk_theme;
#[path = "../servers/sexdisplay/src/glass.rs"]
mod glass;
use desk_theme::desk;
use glass::{Off, Params, Pixels, Scratch};

/// A frame in memory, as Display's is.
struct Buf { w: usize, px: Vec<u32> }
impl Pixels for Buf {
    fn get(&self, x: usize, y: usize) -> u32 { self.px[y * self.w + x] }
    fn set(&mut self, x: usize, y: usize, c: u32) { self.px[y * self.w + x] = c; }
}
fn scratch(band: usize, line: usize) -> (Vec<u32>, Vec<u32>, Vec<u32>) { (vec![0; band], vec![0; line], vec![0; line]) }
/// No tint and no band: the blur alone.
const PLAIN: Params = Params { tint: 0, alpha: 0, floor: 0, ceil: 10000 };
const ACCENTS: [u32; 4] = [DEFAULT_ACCENT, 0x3584E4, 0x2EC27E, 0xF6D32D];

fn frost(buf: &mut Buf, inner: glass::Rect, r: usize, p: &Params) -> Result<(), Off> {
    let (w, h) = (buf.w, buf.px.len() / buf.w);
    let (mut band, mut a, mut b) = scratch(glass::BAND_LEN, 4096);
    let mut s = Scratch { band: &mut band, a: &mut a, b: &mut b };
    glass::frost(buf, inner, r, (0, 0, w, h), p, &mut s)
}

/// WCAG's own formula in floating point, not glass.rs's table.
fn wcag(a: u32, b: u32) -> f32 { contrast(0xFF00_0000 | a, 0xFF00_0000 | b) }

#[test]
fn blur_of_a_flat_colour_is_that_colour() {
    for c in [0x0000_0000, 0x00FF_FFFF, 0x0033_2E2F, 0x00D6_336C] {
        let mut buf = Buf { w: 64, px: vec![c; 64 * 48] };
        frost(&mut buf, (10, 8, 40, 30), 6, &PLAIN).unwrap();
        assert!(buf.px.iter().all(|&p| p == c), "{c:06x}");
        // And the glass of it is one colour too: the tint blend of it.
        let p = glass::params(0x00FF_FFFF, &[0x001E_1B22], false, false).unwrap();
        let mut buf = Buf { w: 64, px: vec![c; 64 * 48] };
        frost(&mut buf, (10, 8, 40, 30), 6, &p).unwrap();
        let want = glass::frost_pixel(c, &p);
        for y in 8..38 { for x in 10..50 { assert_eq!(buf.get(x, y), want); } }
        assert_eq!(buf.get(9, 8), c, "outside the panel stays as it was");
    }
}

#[test]
fn a_step_edge_is_smoothed_and_monotone() {
    // Black left of x = 32, white from there; the panel spans the edge.
    let (w, h) = (64, 40);
    let px = (0..w * h).map(|i| if i % w < 32 { 0 } else { 0x00FF_FFFF }).collect();
    let mut buf = Buf { w, px };
    let r = 4;
    frost(&mut buf, (8, 4, 48, 32), r, &PLAIN).unwrap();
    for y in 4..36 {
        let row: Vec<u32> = (8..56).map(|x| buf.get(x, y) & 0xFF).collect();
        assert!(row.windows(2).all(|p| p[0] <= p[1]), "row {y} monotone: {row:?}");
        // Two passes of radius r spread the edge over 4r + 1 pixels.
        let mid = row.iter().filter(|&&v| v != 0 && v != 255).count();
        assert!(mid >= 2 * r && mid <= 4 * r, "row {y}: {mid} steps between, {row:?}");
        assert_eq!((row[0], row[47]), (0, 255), "far from the edge the colours stay");
        let jump = row.windows(2).map(|p| p[1] - p[0]).max().unwrap();
        assert!(jump < 64, "no hard step left: {jump}");
    }
}

#[test]
fn the_blur_reads_beyond_the_panel_and_stops_at_the_bounds() {
    // A white column just outside the panel shows inside it, blurred...
    let (w, h) = (40, 20);
    let mut px = vec![0u32; w * h];
    for y in 0..h { px[y * w + 9] = 0x00FF_FFFF; }
    let mut buf = Buf { w, px: px.clone() };
    frost(&mut buf, (10, 5, 20, 10), 3, &PLAIN).unwrap();
    assert!(buf.get(10, 8) & 0xFF > 0 && buf.get(10, 8) & 0xFF < 255);
    assert_eq!(buf.get(9, 8), 0x00FF_FFFF, "the column itself is not the panel's");
    // ...but not one outside the bounds (the bar).
    let mut buf = Buf { w, px: px.clone() };
    let (mut band, mut a, mut b) = scratch(glass::BAND_LEN, 4096);
    let mut s = Scratch { band: &mut band, a: &mut a, b: &mut b };
    glass::frost(&mut buf, (10, 5, 20, 10), 3, (10, 0, w, h), &PLAIN, &mut s).unwrap();
    assert!((10..30).all(|x| buf.get(x, 8) == 0));
}

#[test]
fn tint_blend_endpoints() {
    let (a, b) = (0x0012_3456, 0x00FE_DCBA);
    assert_eq!(glass::mix(a, b, 0), a);
    assert_eq!(glass::mix(a, b, 255), b);
    let p = Params { tint: b, alpha: 255, floor: 0, ceil: 10000 };
    assert_eq!(glass::frost_pixel(a, &p), b, "all tint");
    let p = Params { tint: b, alpha: 0, floor: 0, ceil: 10000 };
    assert_eq!(glass::frost_pixel(a, &p), a, "no tint");
    let half = glass::mix(0, 0x00FF_FFFF, 128);
    assert!((half & 0xFF).abs_diff(128) <= 1);
}

#[test]
fn the_table_matches_wcag() {
    for c in (0..=0xFF_FFFFu32).step_by(0x01_0305) {
        let exact = luminance(0xFF00_0000 | c) * 10000.0;
        assert!((glass::luminance(c) as f32 - exact).abs() <= 1.0, "{c:06x}: {} vs {exact}", glass::luminance(c));
    }
}

/// Backgrounds the glass may cover: every mix of 0, 51 ... 255 per channel.
fn backgrounds() -> Vec<u32> {
    let steps = [0u32, 51, 102, 153, 204, 255];
    let mut v = Vec::new();
    for r in steps { for g in steps { for b in steps { v.push(r << 16 | g << 8 | b); } } }
    v
}

#[test]
fn ink_reads_on_light_and_dark_glass_whatever_is_behind() {
    for mode in [Mode::Light, Mode::Dark] { for accent in ACCENTS {
        let d = desk(mode, accent);
        let p = glass::params(d.menu, &d.glass_inks(), false, false).unwrap_or_else(|e| panic!("{mode:?}: {e:?}"));
        let mut worst = f32::MAX;
        for bg in backgrounds() {
            let g = glass::frost_pixel(bg, &p);
            let l = glass::luminance(g);
            assert!(p.floor <= l && l <= p.ceil, "{mode:?} {bg:06x} -> {g:06x}");
            for ink in d.glass_inks() {
                let c = wcag(ink, g);
                worst = worst.min(c);
                assert!(c >= 4.5, "{mode:?} accent {accent:06x}: ink {ink:06x} on glass {g:06x} (over {bg:06x}) is {c:.2}:1");
            }
        }
        assert!(worst < 7.0 || mode == Mode::Light, "{mode:?}: the band is not needlessly tight ({worst:.2})");
    } }
}

#[test]
fn glass_still_shows_what_is_behind() {
    // Not a flat fill: different backgrounds give different glass.
    for mode in [Mode::Light, Mode::Dark] {
        let d = desk(mode, DEFAULT_ACCENT);
        let p = glass::params(d.menu, &d.glass_inks(), false, false).unwrap();
        let mut seen: Vec<u32> = backgrounds().into_iter().map(|bg| glass::frost_pixel(bg, &p)).collect();
        seen.sort(); seen.dedup();
        assert!(seen.len() > 20, "{mode:?}: only {} kinds of glass", seen.len());
    }
}

#[test]
fn high_contrast_and_reduce_transparency_turn_glass_off() {
    let d = desk(Mode::HighContrast, DEFAULT_ACCENT);
    assert_eq!(glass::params(d.menu, &d.glass_inks(), d.mode == Mode::HighContrast, false), Err(Off::HighContrast));
    let d = desk(Mode::Light, DEFAULT_ACCENT);
    assert_eq!(glass::params(d.menu, &d.glass_inks(), false, true), Err(Off::Reduce));
    assert_eq!(Off::HighContrast.reason(), "high_contrast");
    assert_eq!(Off::Reduce.reason(), "reduce");
    // A tint the inks cannot read on is never glass.
    assert_eq!(glass::params(0x0080_8080, &[0x0000_0000, 0x00FF_FFFF], false, false), Err(Off::Contrast));
}

#[test]
fn reduce_transparency_rides_in_the_theme_word() {
    use desktop_look::*;
    let word = theme_word_reduced(1, 0x3584E4, true);
    assert_eq!(word, theme_word(1, 0x3584E4) | REDUCE_TRANSPARENCY);
    assert_eq!(decode_theme(word), Ok(Some((1, 0x3584E4))));
    assert!(reduces_transparency(word) && !reduces_transparency(theme_word(1, 0x3584E4)));
    assert_eq!(theme_word_reduced(0, DEFAULT_ACCENT, false), theme_word(0, DEFAULT_ACCENT));
    assert_eq!(decode_theme(word | 1 << 33), Err(()), "bits above it are still refused");
    assert!(!reduce_transparency());
    assert!(set_look(0, 0, word) && reduce_transparency());
    assert!(set_look(0, 0, 0) && reduce_transparency(), "a look without a theme keeps it");
    assert!(set_look(0, 0, theme_word(0, DEFAULT_ACCENT)) && !reduce_transparency());
}

#[test]
fn an_oversize_panel_falls_back_untouched() {
    let (w, h) = (64, 48);
    let px: Vec<u32> = (0..w * h).map(|i| (i as u32).wrapping_mul(2654435761) & 0x00FF_FFFF).collect();
    let mut buf = Buf { w, px: px.clone() };
    // Lines shorter than the region: nothing is written.
    let (mut band, mut a, mut b) = scratch(glass::BAND_LEN, 16);
    let mut s = Scratch { band: &mut band, a: &mut a, b: &mut b };
    assert_eq!(glass::frost(&mut buf, (8, 8, 40, 30), 6, (0, 0, w, h), &PLAIN, &mut s), Err(Off::Oversize));
    assert_eq!(buf.px, px);
    // A band too small for the rows above and below: the same.
    let (mut band, mut a, mut b) = scratch(10, 4096);
    let mut s = Scratch { band: &mut band, a: &mut a, b: &mut b };
    assert_eq!(glass::frost(&mut buf, (8, 20, 40, 10), 6, (0, 0, w, h), &PLAIN, &mut s), Err(Off::Oversize));
    assert_eq!(buf.px, px);
    // Reaching over the bar (outside the bounds): opaque, untouched.
    let (mut band, mut a, mut b) = scratch(glass::BAND_LEN, 4096);
    let mut s = Scratch { band: &mut band, a: &mut a, b: &mut b };
    assert_eq!(glass::frost(&mut buf, (8, 4, 40, 10), 6, (0, 6, w, h), &PLAIN, &mut s), Err(Off::Oversize));
    assert_eq!(buf.px, px);
    // Wider than any panel glass is drawn for.
    let wide = glass::MAX_INNER_W + 1;
    let s = Scratch { band: &mut vec![0; 1][..], a: &mut vec![0; 1][..], b: &mut vec![0; 1][..] };
    assert!(!glass::fits((0, 0, wide, 1), 1, (0, 0, wide, 1), &s));
    // Display's own scratch holds the largest popups at the largest scale:
    // the Bluetooth panel (420 x 14 rows of 26 + 24) and the window menu at 3x.
    let r = glass::radius(glass::MAX_SCALE);
    for (pw, ph) in [(420 * 3, (24 + 14 * 26) * 3), (200 * 3, (12 * 24 + 8) * 3), (320 * 3, (24 + 4 * 40) * 3)] {
        let (sw, sh) = (3840, 2160);
        let line = 8192 + 2 * glass::margin(r);
        let (mut band, mut a, mut b) = scratch(glass::BAND_LEN, line);
        let s = Scratch { band: &mut band, a: &mut a, b: &mut b };
        assert!(glass::fits((sw - pw - 24, 90, pw, ph), r, (0, 72, sw, sh), &s), "{pw}x{ph}");
    }
}

#[test]
fn the_radius_follows_the_scale() {
    assert_eq!(glass::radius(0), glass::RADIUS);
    assert_eq!(glass::radius(1), 6);
    assert_eq!(glass::radius(2), 12);
    assert_eq!(glass::radius(9), 18, "at most the largest scale");
    assert_eq!(glass::margin(6), 12);
}
