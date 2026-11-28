// SPDX-License-Identifier: GPL-3.0-only
//! macOS Noir Obsidian Glass UI primitives & typography helpers for pixel-buffer apps.
//!
//! Pure integer `#![no_std]` zero-alloc helpers operating on `0x00RRGGBB` pixel buffers.
//! Exports canonical [`NoirPalette`] (`CandyPalette` alias), `draw_noir_*` aliases for
//! [`crate::candy_ui`], plus higher-level macOS Noir widgets (soft drop shadows,
//! elevated glass cards, segmented controls, pill toggles, sidebar rows, search bars,
//! and Geist Medium / SemiBold linear-light text drawers when the `font` feature is on).

use crate::candy_ui::{clip_rect, draw_candy_button, draw_candy_selection_row, draw_candy_surface};
pub use crate::candy_ui::{
    button_text_color, fill_rounded_rect, mix_rgb, rgb, ButtonState, ButtonStyle, CandyPalette,
    CandyPalette as NoirPalette,
};
use crate::{Mode, NoirAccent};

pub use crate::candy_ui::draw_candy_badge as draw_noir_badge;
pub use crate::candy_ui::draw_candy_button as draw_noir_button;
pub use crate::candy_ui::draw_candy_panel as draw_noir_panel;
pub use crate::candy_ui::draw_candy_selection_row as draw_noir_selection_row;
pub use crate::candy_ui::draw_candy_swatch as draw_noir_swatch;
pub use crate::candy_ui::draw_candy_tab as draw_noir_tab;
pub use crate::candy_ui::draw_candy_toggle as draw_noir_toggle;
pub use crate::candy_ui::draw_candy_toolbar as draw_noir_toolbar;

/// Pure integer floor square root (`#![no_std]`, zero `libm` dependency).
#[inline]
const fn isqrt_u32(n: u32) -> u32 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Draws a soft rounded macOS Noir drop shadow beneath rectangle `(x, y, w, h)`.
/// `spread` controls shadow blur radius in pixels, and `strength` (0..=255) sets peak opacity.
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_shadow(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    spread: i32,
    strength: u8,
) {
    if w <= 0 || h <= 0 || spread <= 0 || strength == 0 || buf.len() < stride.saturating_mul(buf_h) {
        return;
    }
    let sy = y + (spread / 4).clamp(1, 3);
    let ox = x - spread;
    let oy = sy - spread;
    let ow = w + spread * 2;
    let oh = h + spread * 2;
    let Some((x0, y0, x1, y1)) = clip_rect(ox, oy, ow, oh, stride, buf_h) else {
        return;
    };

    let r = radius.clamp(0, w.min(h) / 2);
    let r2 = r * 2;
    let max_d = (spread * 2).max(1);

    for py in y0..y1 {
        let row = py as usize * stride;
        let py2 = (py - sy) * 2 + 1;
        let dy = if py2 < r2 {
            r2 - py2
        } else if py2 > h * 2 - r2 {
            py2 - (h * 2 - r2)
        } else {
            0
        };
        for px in x0..x1 {
            let px2 = (px - x) * 2 + 1;
            let dx = if px2 < r2 {
                r2 - px2
            } else if px2 > w * 2 - r2 {
                px2 - (w * 2 - r2)
            } else {
                0
            };

            let dist = if dx == 0 && dy == 0 {
                0
            } else {
                isqrt_u32((dx * dx + dy * dy) as u32) as i32 - r2
            };
            if dist >= max_d {
                continue;
            }
            let a = if dist <= 0 {
                strength
            } else {
                let fall = max_d - dist;
                // Quadratic falloff gives a realistic Gaussian-like soft macOS shadow.
                let q = (fall * fall * strength as i32) / (max_d * max_d);
                q.clamp(0, 255) as u8
            };
            if a > 0 {
                let idx = row + px as usize;
                buf[idx] = mix_rgb(buf[idx], 0x0002_0305, a);
            }
        }
    }
}

/// Draws a frosted macOS Noir Obsidian card with an optional soft drop shadow, subtle
/// top specular highlight, and `1px` `#2A3142`-style obsidian rim border.
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_card(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    elevated: bool,
    pal: &NoirPalette,
) {
    if pal.mode != Mode::HighContrast {
        draw_noir_shadow(
            buf,
            stride,
            buf_h,
            x,
            y,
            w,
            h,
            radius,
            if elevated { 8 } else { 5 },
            if pal.mode == Mode::Dark {
                if elevated { 96 } else { 64 }
            } else if elevated {
                40
            } else {
                24
            },
        );
    }
    let fill = if elevated {
        mix_rgb(pal.card, pal.control, if pal.mode == Mode::Dark { 108 } else { 40 })
    } else {
        pal.card
    };
    draw_candy_surface(
        buf,
        stride,
        buf_h,
        x,
        y,
        w,
        h,
        radius,
        fill,
        pal.rim,
        pal.specular,
        pal.mode == Mode::HighContrast,
        if pal.mode == Mode::Light { 128 } else if elevated { 48 } else { 36 },
        if pal.mode == Mode::Light { 116 } else { 196 },
    );
}

/// Draws a sleek macOS Noir pill toggle switch with accent track, subtle rim sheen,
/// and elevated circular knob.
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_pill_toggle(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    on: bool,
    state: ButtonState,
    pal: &NoirPalette,
) {
    draw_noir_toggle(buf, stride, buf_h, x, y, w, h, on, state, pal);
}

/// Computes `(seg_x, seg_w)` for segment `idx` inside a horizontal segmented control `(x, w)`.
#[inline]
pub const fn segment_rect(x: i32, w: i32, segments: usize, idx: usize) -> (i32, i32) {
    if segments == 0 || w <= 0 || idx >= segments {
        return (x, 0);
    }
    let pad = 2;
    let inner_w = w - pad * 2;
    if inner_w <= 0 {
        return (x, 0);
    }
    let n = segments as i32;
    let i = idx as i32;
    let sx = x + pad + (inner_w * i) / n;
    let ex = x + pad + (inner_w * (i + 1)) / n;
    (sx, ex - sx)
}

/// Draws a macOS Dark Mode segmented control (`segments` segments, `active_idx` selected,
/// optional `hovered_idx`). Use [`segment_rect`] to place segment labels.
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_segmented_control(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    segments: usize,
    active_idx: usize,
    hovered_idx: Option<usize>,
    pal: &NoirPalette,
) {
    if segments == 0 || w <= 4 || h <= 4 {
        return;
    }
    let track_r = (h / 2).clamp(4, 10);
    draw_candy_surface(
        buf,
        stride,
        buf_h,
        x,
        y,
        w,
        h,
        track_r,
        pal.well,
        pal.rim,
        pal.specular,
        pal.mode == Mode::HighContrast,
        24,
        150,
    );

    let thumb_r = (track_r - 2).max(3);
    if let Some(hidx) = hovered_idx {
        if hidx < segments && hidx != active_idx {
            let (hx, hw) = segment_rect(x, w, segments, hidx);
            if hw > 2 && h > 6 {
                fill_rounded_rect(buf, stride, buf_h, hx + 1, y + 3, hw - 2, h - 6, thumb_r, pal.control);
            }
        }
    }

    if active_idx < segments {
        let (ax, aw) = segment_rect(x, w, segments, active_idx);
        if aw > 4 && h > 4 {
            draw_candy_button(
                buf,
                stride,
                buf_h,
                ax + 1,
                y + 2,
                aw - 2,
                h - 4,
                thumb_r,
                ButtonStyle::Normal,
                ButtonState::Hover,
                pal,
            );
        }
    }
}

/// Draws a macOS Noir sidebar navigation pill row and returns the readable label ink color.
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_sidebar_row(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    selected: bool,
    hovered: bool,
    pal: &NoirPalette,
) -> u32 {
    if selected {
        draw_candy_selection_row(buf, stride, buf_h, x, y, w, h, radius, pal);
        pal.text
    } else if hovered {
        fill_rounded_rect(buf, stride, buf_h, x, y, w, h, radius, pal.control);
        pal.text
    } else {
        pal.text_dim
    }
}

/// Draws a sunken macOS Noir obsidian search input bar with accent focus rim when `focused`.
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_search_bar(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    focused: bool,
    pal: &NoirPalette,
) {
    let r = (h / 2).clamp(6, 12);
    let rim = if focused { pal.rim_accent } else { pal.rim };
    let rim_a = if focused { 210 } else { 150 };
    draw_candy_surface(
        buf,
        stride,
        buf_h,
        x,
        y,
        w,
        h,
        r,
        pal.well,
        rim,
        pal.specular,
        pal.mode == Mode::HighContrast,
        if focused { 48 } else { 20 },
        rim_a,
    );
}

/// Draws a horizontal 1px macOS Noir hairline divider.
pub fn draw_noir_divider(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    pal: &NoirPalette,
) {
    let Some((x0, y0, x1, _)) = clip_rect(x, y, w, 1, stride, buf_h) else {
        return;
    };
    if buf.len() < stride.saturating_mul(buf_h) {
        return;
    }
    let row = y0 as usize * stride;
    buf[row + x0 as usize..row + x1 as usize].fill(pal.hairline);
}

/// Alpha-blends a glyph coverage bitmap (`gw * gh` bytes, `0..=255`) onto a `0x00RRGGBB`
/// pixel buffer at `(dst_x, dst_y)` with ink `fg`.
#[allow(clippy::too_many_arguments)]
pub fn blit_glyph_coverage(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    dst_x: i32,
    dst_y: i32,
    gw: usize,
    gh: usize,
    cov: &[u8],
    fg: u32,
) {
    if gw == 0 || gh == 0 || cov.len() < gw * gh || buf.len() < stride.saturating_mul(buf_h) {
        return;
    }
    let fg = rgb(fg);
    for gy in 0..gh {
        let py = dst_y + gy as i32;
        if py < 0 || py >= buf_h as i32 {
            continue;
        }
        let row = py as usize * stride;
        let grow = gy * gw;
        for gx in 0..gw {
            let a = cov[grow + gx];
            if a == 0 {
                continue;
            }
            let px = dst_x + gx as i32;
            if px < 0 || px >= stride as i32 {
                continue;
            }
            let idx = row + px as usize;
            buf[idx] = if a == 255 {
                fg
            } else {
                mix_rgb(buf[idx], fg, a)
            };
        }
    }
}

/// Convenience helper returning `(NoirPalette::noir_tint(tint), tint.primary())`.
pub const fn noir_theme_pair(idx: u8) -> (NoirPalette, u32) {
    let tint = NoirAccent::from_index(idx);
    (NoirPalette::noir_tint(tint), rgb(tint.primary()))
}

#[cfg(feature = "font")]
const MAX_TEXT_BOX: usize = 40;
#[cfg(feature = "font")]
const TEXT_ACC_LEN: usize = sex_font::raster::scratch_len(MAX_TEXT_BOX, MAX_TEXT_BOX);

/// Measures the integer pixel width of `text` in `Geist` (`weight`) at `px_size` pixels.
#[cfg(feature = "font")]
pub fn measure_noir_text(px_size: u16, weight: sex_font::Weight, text: &str) -> i32 {
    let Some(font) = sex_font::Font::new(sex_font::geist_weight(false, weight, false)) else {
        return 0;
    };
    let size = (px_size as u32).clamp(6, 28) * 64;
    let mut pen: i32 = 0;
    let mut prev = None;
    for ch in text.chars() {
        let Some(gid) = font.glyph(ch as u32) else {
            continue;
        };
        if let Some(p) = prev {
            pen += font.kerning(p, gid, size);
        }
        pen += font.advance(gid, size);
        prev = Some(gid);
    }
    (pen + 255) >> 8
}

/// Draws `text` with `Geist` at `weight` using Apple CoreText-style stem darkening
/// and linear-light contrast preservation at `(x, top_y)`. Returns advance width in pixels.
#[cfg(feature = "font")]
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_text_weight(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    top_y: i32,
    px_size: u16,
    weight: sex_font::Weight,
    fg: u32,
    text: &str,
) -> i32 {
    if buf.len() < stride.saturating_mul(buf_h) {
        return 0;
    }
    let Some(font) = sex_font::Font::new(sex_font::geist_weight(false, weight, false)) else {
        return 0;
    };
    let px = (px_size as usize).clamp(6, 28);
    let size = px as u32 * 64;
    let box_w = (px + 10).min(MAX_TEXT_BOX);
    let box_h = (px + 12).min(MAX_TEXT_BOX);
    let ascender = font.ascender(size);
    let baseline = ascender.max(px as i32 * 192);
    let fg_rgb = rgb(fg);

    let mut acc = [0i32; TEXT_ACC_LEN];
    let mut cov = [0u8; MAX_TEXT_BOX * MAX_TEXT_BOX];
    let mut pen: i32 = 0;
    let mut prev = None;

    for ch in text.chars() {
        let Some(gid) = font.glyph(ch as u32) else {
            continue;
        };
        if let Some(p) = prev {
            pen += font.kerning(p, gid, size);
        }
        let gx_px = pen >> 8;
        let frac_x = pen & 0xFF;
        let out_slice = &mut cov[..box_w * box_h];
        if font.draw_humanized(
            gid,
            size,
            frac_x + 256,
            baseline,
            box_w,
            box_h,
            &mut acc,
            out_slice,
            sex_font::Synthetic::NONE,
            32 * 64,
        ) {
            let dst_x0 = x + gx_px - 1;
            for ry in 0..box_h {
                let py = top_y + ry as i32;
                if py < 0 || py >= buf_h as i32 {
                    continue;
                }
                let row = py as usize * stride;
                let grow = ry * box_w;
                for rx in 0..box_w {
                    let c = out_slice[grow + rx];
                    if c == 0 {
                        continue;
                    }
                    let px_x = dst_x0 + rx as i32;
                    if px_x < 0 || px_x >= stride as i32 {
                        continue;
                    }
                    let idx = row + px_x as usize;
                    buf[idx] = sex_font::gamma::blend_humanized(fg_rgb, buf[idx], c);
                }
            }
        }
        pen += font.advance(gid, size);
        prev = Some(gid);
    }
    (pen + 255) >> 8
}

/// Draws standard body/label UI text with `Geist-Medium` (500) + subtle CoreText-style
/// stem darkening.
#[cfg(feature = "font")]
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_text_medium(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    top_y: i32,
    px_size: u16,
    fg: u32,
    text: &str,
) -> i32 {
    draw_noir_text_weight(buf, stride, buf_h, x, top_y, px_size, sex_font::Weight::Medium, fg, text)
}

/// Draws UI chrome/heading/button text with `Geist-SemiBold` (600 "thinner bold") + subtle
/// CoreText-style stem darkening.
#[cfg(feature = "font")]
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_text_semibold(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    top_y: i32,
    px_size: u16,
    fg: u32,
    text: &str,
) -> i32 {
    draw_noir_text_weight(buf, stride, buf_h, x, top_y, px_size, sex_font::Weight::SemiBold, fg, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noir_widgets_render_cleanly_without_panics() {
        let (pal, _accent) = noir_theme_pair(0);
        let mut buf = [pal.bg; 200 * 120];
        draw_noir_card(&mut buf, 200, 120, 12, 12, 176, 96, 12, true, &pal);
        draw_noir_search_bar(&mut buf, 200, 120, 20, 20, 160, 24, true, &pal);
        draw_noir_segmented_control(&mut buf, 200, 120, 20, 52, 120, 22, 3, 1, Some(2), &pal);
        let row_ink = draw_noir_sidebar_row(&mut buf, 200, 120, 20, 80, 120, 18, 6, true, false, &pal);
        assert_eq!(row_ink, pal.text);
        draw_noir_pill_toggle(&mut buf, 200, 120, 148, 52, 32, 18, true, ButtonState::Rest, &pal);
        draw_noir_divider(&mut buf, 200, 120, 20, 48, 160, &pal);
        // Center of active segmented control thumb was drawn over background.
        let (sx, sw) = segment_rect(20, 120, 3, 1);
        assert!(sw > 20);
        assert_ne!(buf[63 * 200 + (sx + sw / 2) as usize], pal.bg);
        // Drop shadow outside card top-left slightly darkens background.
        assert_ne!(buf[18 * 200 + 8], pal.bg);
    }

    #[test]
    fn segment_rect_partitions_track_cleanly() {
        let mut total = 0;
        for i in 0..4 {
            let (sx, sw) = segment_rect(10, 100, 4, i);
            assert!(sx >= 12);
            total += sw;
        }
        assert_eq!(total, 96);
        assert_eq!(segment_rect(10, 100, 4, 9).1, 0);
    }

    #[cfg(feature = "font")]
    #[test]
    fn noir_font_text_helpers_render_medium_and_semibold() {
        let pal = NoirPalette::noir();
        let mut buf = [pal.card; 160 * 48];
        let w_med = draw_noir_text_medium(&mut buf, 160, 48, 8, 6, 13, pal.text, "Body Medium");
        let w_semi = draw_noir_text_semibold(&mut buf, 160, 48, 8, 24, 13, pal.text, "Chrome Title");
        assert!(w_med > 40);
        assert!(w_semi > 40);
        let ink_pixels = buf.iter().filter(|&&px| px != pal.card).count();
        assert!(ink_pixels > 80);
    }
}
