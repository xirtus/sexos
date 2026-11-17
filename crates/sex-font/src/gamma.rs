// SPDX-License-Identifier: GPL-3.0-only
//! Gamma-correct blending of glyph coverage.
//!
//! Coverage says how much of a pixel the ink covers, which is a fraction of light,
//! not of sRGB code value. Mixing the sRGB bytes directly (what a plain alpha blend
//! does) makes dark text on a light background too thin and light text on a dark
//! background too heavy, and the weight changes with the theme. [`blend`] mixes in
//! linear light instead and then gives the coverage a little extra contrast,
//! more for dark ink than for light ink, the same correction DirectWrite and Skia
//! apply, so a word has one weight in both themes.
//!
//! Integer only, through the tables in `gamma_tables.rs`.
use crate::gamma_tables::{LINEAR_TO_SRGB, SRGB_TO_LINEAR};

/// Extra contrast for the darkest ink, in 1/256 (DirectWrite's "enhanced contrast").
pub const DARK_CONTRAST: i32 = 160;
/// Contrast for white ink: negative, because mixing in linear light already makes
/// light strokes on a dark ground heavier, and this takes that back.
pub const LIGHT_CONTRAST: i32 = -112;

/// sRGB byte to linear light, 0..=4095.
#[inline]
pub fn to_linear(c: u8) -> u32 { SRGB_TO_LINEAR[c as usize] as u32 }

/// Linear light 0..=4095 to an sRGB byte.
#[inline]
pub fn to_srgb(v: u32) -> u8 { LINEAR_TO_SRGB[v.min(4095) as usize] }

/// Relative luminance of `0xRRGGBB` in linear light, 0..=4095 (Rec. 709 weights).
pub fn luminance(rgb: u32) -> u32 {
    let ch = |s: u32| to_linear((rgb >> s) as u8);
    (ch(16) * 2126 + ch(8) * 7152 + ch(0) * 722) / 10000
}

/// The contrast for ink of `rgb`, in 1/256: dark ink is made heavier, light ink lighter.
pub fn contrast_for(rgb: u32) -> i32 {
    let lum = luminance(rgb) as i32;
    DARK_CONTRAST - (DARK_CONTRAST - LIGHT_CONTRAST) * lum / 4095
}

/// `coverage` with `k`/256 enhanced contrast: `a (1 + k) / (1 + k a)`. Keeps 0 and
/// 255; `k` above 0 raises partial coverage, below 0 lowers it (`k` > -256).
#[inline]
pub fn enhance(coverage: u8, k: i32) -> u8 {
    let (a, k) = (coverage as i64, k.max(-255) as i64);
    (a * (256 + k) * 255 / (256 * 255 + k * a)).clamp(0, 255) as u8
}

/// `fg` ink over `bg` at `coverage`, mixed in linear light. Both are `0x..RRGGBB`;
/// the top byte of `fg` is kept, as the plain blends in Display and Weft keep it.
pub fn blend(fg: u32, bg: u32, coverage: u8) -> u32 {
    blend_with(fg, bg, coverage, contrast_for(fg))
}

/// [`blend`] with the contrast boost already worked out (one per run of text).
#[inline]
pub fn blend_with(fg: u32, bg: u32, coverage: u8, k: i32) -> u32 {
    if coverage == 255 { return fg; }
    if coverage == 0 { return (fg & 0xFF00_0000) | (bg & 0xFF_FFFF); }
    let a = enhance(coverage, k) as u32;
    let mix = |s: u32| {
        let (f, b) = (to_linear((fg >> s) as u8), to_linear((bg >> s) as u8));
        (to_srgb((f * a + b * (255 - a) + 127) / 255) as u32) << s
    };
    (fg & 0xFF00_0000) | mix(16) | mix(8) | mix(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_are_exact_and_tables_round_trip() {
        assert_eq!(blend(0x00_102030, 0x00_f0e0d0, 255), 0x00_102030);
        assert_eq!(blend(0xff_102030, 0x00_f0e0d0, 0), 0xff_f0e0d0);
        for c in 0..=255u8 { assert_eq!(to_srgb(to_linear(c)), c); }
    }

    #[test]
    fn half_coverage_is_half_the_light_not_half_the_code() {
        // White over black at half coverage, no contrast: linear 0.5 is sRGB 188, not 128.
        assert_eq!(blend_with(0xffffff, 0x000000, 128, 0) & 0xff, 188);
        // Contrast makes dark ink on white heavier than the plain linear mix.
        let plain = blend_with(0x000000, 0xffffff, 128, 0) & 0xff;
        let tuned = blend(0x000000, 0xffffff, 128) & 0xff;
        assert!(tuned < plain, "{tuned} {plain}");
    }

    #[test]
    fn enhance_is_monotonic() {
        let mut last = 0;
        for a in 0..=255u8 { let e = enhance(a, DARK_CONTRAST); assert!(e >= last); assert!(e >= a); last = e; }
        assert_eq!(enhance(255, DARK_CONTRAST), 255);
    }
}
