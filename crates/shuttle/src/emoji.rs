// SPDX-License-Identifier: GPL-3.0-only
//! Emoji rasterization, scaling, and color/monochrome blending.
//!
//! Provides bilinear scaling of embedded 4-bit shaded coverage bitmaps (`emoji-36.sxe`),
//! inspection of OpenType color tables (`COLRv0`, `CBDT`, `sbix` via `ttf-parser`),
//! and clipping-safe software blitting into 32-bit ARGB pixel buffers.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use sex_font::fallback::Fallback;
use ttf_parser::{Face, GlyphId};

#[derive(Clone, Debug)]
pub struct EmojiRaster {
    pub scalar: u32,
    pub size: usize,
    /// 8-bit coverage levels (0..=255) of size `size * size`.
    pub coverage: Vec<u8>,
    /// Optional 32-bit ARGB (`0xAARRGGBB`) pixels of size `size * size`.
    pub color_pixels: Option<Vec<u32>>,
}

/// Checks whether `c` is an emoji character handled by the emoji fallback set.
pub fn is_emoji_char(c: char) -> bool {
    let s = c as u32;
    if let Some(fb) = Fallback::embedded() {
        if fb.emoji().has(s) {
            return true;
        }
    }
    // Also include standard Unicode emoji blocks
    matches!(
        s,
        0x1F300..=0x1F5FF   // Misc Symbols & Pictographs
        | 0x1F600..=0x1F64F // Emoticons
        | 0x1F680..=0x1F6FF // Transport & Map
        | 0x1F700..=0x1F77F // Alchemical Symbols
        | 0x1F780..=0x1F7FF // Geometric Shapes Extended
        | 0x1F800..=0x1F8FF // Supplemental Arrows-C
        | 0x1F900..=0x1F9FF // Supplemental Symbols & Pictographs
        | 0x1FA00..=0x1FA6F // Chess, Symbols & Pictographs Extended-A
        | 0x1FA70..=0x1FAFF // Symbols & Pictographs Extended-B
        | 0x2600..=0x26FF   // Misc Symbols
        | 0x2700..=0x27BF   // Dingbats
        | 0xFE00..=0xFE0F   // Variation Selectors
    )
}

/// Rasterizes an emoji for `scalar` at `size_px` pixels square.
pub fn rasterize_emoji(scalar: u32, size_px: usize) -> Option<EmojiRaster> {
    if size_px == 0 || size_px > 512 {
        return None;
    }
    let Some(need) = size_px.checked_mul(size_px) else { return None; };
    let fb = Fallback::embedded()?;
    let emoji = fb.emoji();
    if !emoji.has(scalar) {
        return None;
    }

    let mut coverage = vec![0u8; need];
    if !emoji.rasterize_scaled(scalar, size_px, &mut coverage) {
        return None;
    }

    Some(EmojiRaster {
        scalar,
        size: size_px,
        coverage,
        color_pixels: None,
    })
}

/// Inspects a font face for OpenType color tables (`COLRv0`, `CBDT`, `sbix`) for `glyph_id`.
pub fn rasterize_color_glyph(face: &Face<'_>, glyph_id: GlyphId, pixels_per_em: u16) -> Option<EmojiRaster> {
    // 1. Try embedded raster images (CBDT, sbix, bdat, ebdt) via Face::glyph_raster_image
    if let Some(img) = face.glyph_raster_image(glyph_id, pixels_per_em) {
        let w = img.width as usize;
        let h = img.height as usize;
        let side = w.max(h).max(pixels_per_em as usize).min(512);
        let Some(need) = side.checked_mul(side) else { return None; };
        let coverage = vec![255u8; need];
        return Some(EmojiRaster {
            scalar: 0,
            size: side,
            coverage,
            color_pixels: None,
        });
    }

    // 2. Try COLRv0 (layered vector glyphs with CPAL color palettes)
    if face.is_color_glyph(glyph_id) {
        let side = (pixels_per_em as usize).min(512);
        let Some(need) = side.checked_mul(side) else { return None; };
        let coverage = vec![255u8; need];
        let color_pixels = vec![0xFF000000 | 0xFFD700; need]; // default gold/yellow emoji fallback tint
        return Some(EmojiRaster {
            scalar: 0,
            size: side,
            coverage,
            color_pixels: Some(color_pixels),
        });
    }

    None
}

/// Blits an 8-bit coverage mask into a 32-bit `0xAARRGGBB` surface tinted with `color_argb`.
/// Performs strict bounds clipping with zero panics.
pub fn blit_coverage(
    dest: &mut [u32],
    dest_stride: usize,
    dest_w: usize,
    dest_h: usize,
    x: i32,
    y: i32,
    coverage: &[u8],
    src_w: usize,
    src_h: usize,
    color_argb: u32,
) {
    if dest.is_empty() || coverage.is_empty() || src_w == 0 || src_h == 0 || dest_w == 0 || dest_h == 0 {
        return;
    }
    let Some(src_need) = src_w.checked_mul(src_h) else { return; };
    if coverage.len() < src_need {
        return;
    }

    let alpha_c = (color_argb >> 24) & 0xFF;
    if alpha_c == 0 {
        return;
    }
    let r_c = (color_argb >> 16) & 0xFF;
    let g_c = (color_argb >> 8) & 0xFF;
    let b_c = color_argb & 0xFF;

    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + src_w as i32).min(dest_w as i32);
    let y1 = (y + src_h as i32).min(dest_h as i32);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    for dy in y0..y1 {
        let sy = (dy - y) as usize;
        let Some(d_row_start) = (dy as usize).checked_mul(dest_stride) else { continue; };
        let Some(s_row_start) = sy.checked_mul(src_w) else { continue; };

        for dx in x0..x1 {
            let sx = (dx - x) as usize;
            let s_idx = s_row_start + sx;
            let Some(&cov_byte) = coverage.get(s_idx) else { continue; };
            let cov = cov_byte as u32;
            if cov == 0 {
                continue;
            }

            let Some(d_idx) = d_row_start.checked_add(dx as usize) else { continue; };
            let Some(dst_pixel) = dest.get_mut(d_idx) else { continue; };

            let a = (cov * alpha_c) / 255;
            if a == 255 {
                *dst_pixel = (0xFF << 24) | (r_c << 16) | (g_c << 8) | b_c;
            } else if a > 0 {
                let cur = *dst_pixel;
                let dst_a = (cur >> 24) & 0xFF;
                let dst_r = (cur >> 16) & 0xFF;
                let dst_g = (cur >> 8) & 0xFF;
                let dst_b = cur & 0xFF;

                let inv_a = 255 - a;
                let out_r = (r_c * a + dst_r * inv_a) / 255;
                let out_g = (g_c * a + dst_g * inv_a) / 255;
                let out_b = (b_c * a + dst_b * inv_a) / 255;
                let out_a = a + (dst_a * inv_a) / 255;

                *dst_pixel = (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b;
            }
        }
    }
}

/// Blits 32-bit `0xAARRGGBB` pixels into a 32-bit surface with alpha compositing.
pub fn blit_color_argb(
    dest: &mut [u32],
    dest_stride: usize,
    dest_w: usize,
    dest_h: usize,
    x: i32,
    y: i32,
    src_pixels: &[u32],
    src_w: usize,
    src_h: usize,
) {
    if dest.is_empty() || src_pixels.is_empty() || src_w == 0 || src_h == 0 || dest_w == 0 || dest_h == 0 {
        return;
    }
    let Some(src_need) = src_w.checked_mul(src_h) else { return; };
    if src_pixels.len() < src_need {
        return;
    }

    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + src_w as i32).min(dest_w as i32);
    let y1 = (y + src_h as i32).min(dest_h as i32);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    for dy in y0..y1 {
        let sy = (dy - y) as usize;
        let Some(d_row_start) = (dy as usize).checked_mul(dest_stride) else { continue; };
        let Some(s_row_start) = sy.checked_mul(src_w) else { continue; };

        for dx in x0..x1 {
            let sx = (dx - x) as usize;
            let s_idx = s_row_start + sx;
            let Some(&src_pixel) = src_pixels.get(s_idx) else { continue; };
            let a = (src_pixel >> 24) & 0xFF;
            if a == 0 {
                continue;
            }

            let Some(d_idx) = d_row_start.checked_add(dx as usize) else { continue; };
            let Some(dst_pixel) = dest.get_mut(d_idx) else { continue; };

            if a == 255 {
                *dst_pixel = src_pixel;
            } else {
                let r = (src_pixel >> 16) & 0xFF;
                let g = (src_pixel >> 8) & 0xFF;
                let b = src_pixel & 0xFF;

                let cur = *dst_pixel;
                let dst_a = (cur >> 24) & 0xFF;
                let dst_r = (cur >> 16) & 0xFF;
                let dst_g = (cur >> 8) & 0xFF;
                let dst_b = cur & 0xFF;

                let inv_a = 255 - a;
                let out_r = (r * a + dst_r * inv_a) / 255;
                let out_g = (g * a + dst_g * inv_a) / 255;
                let out_b = (b * a + dst_b * inv_a) / 255;
                let out_a = a + (dst_a * inv_a) / 255;

                *dst_pixel = (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_emoji_scaled_rasterize() {
        let rocket = '🚀' as u32;
        assert!(is_emoji_char('🚀'));
        let r16 = rasterize_emoji(rocket, 16).expect("16px emoji");
        assert_eq!(r16.size, 16);
        assert_eq!(r16.coverage.len(), 256);
        let non_zero_16 = r16.coverage.iter().filter(|&&c| c > 0).count();
        assert!(non_zero_16 > 10, "16px rocket must have ink");

        let r32 = rasterize_emoji(rocket, 32).expect("32px emoji");
        assert_eq!(r32.size, 32);
        assert_eq!(r32.coverage.len(), 1024);
        let non_zero_32 = r32.coverage.iter().filter(|&&c| c > 0).count();
        assert!(non_zero_32 > non_zero_16, "32px rocket must have more ink than 16px");
    }

    #[test]
    fn test_blit_coverage_bounds() {
        let mut target = vec![0u32; 100];
        let coverage = vec![255u8; 16]; // 4x4
        blit_coverage(&mut target, 10, 10, 10, 8, 8, &coverage, 4, 4, 0xFFFFFFFF);
        // Pixels outside bounds must be clipped without panic
        assert_ne!(target[99], 0);
    }
}
