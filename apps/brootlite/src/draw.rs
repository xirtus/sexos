// SPDX-License-Identifier: GPL-3.0-only
//! Xirtus Dark-Glass Theme drawing primitives, poster blitter, and Geist Sans text painter.

use alloc::string::String;
use alloc::vec::Vec;
use sex_font::raster::{scratch_len, FX};
use sex_font::Font;
use sex_image::Image;
use sex_theme::candy_ui::fill_rounded_rect;

// ── Xirtus Dark-Glass Theme Palette ──────────────────────────────────────────
pub const BG_CANVAS: u32 = 0x0009_0B10; // #090B10 canvas
pub const BG_SIDEBAR: u32 = 0x000D_111A; // #0D111A sidebar rail
pub const BG_CARD: u32 = 0x0012_1722; // #121722 card
pub const BG_HOVER: u32 = 0x0018_2030; // #182030 hover
pub const BG_ELEVATED: u32 = 0x0014_1B29; // #141B29 input / overlay panel
pub const BORDER_DEFAULT: u32 = 0x0026_3042; // #263042 1px border
pub const BORDER_ACTIVE: u32 = 0x0038_BDF8; // #38BDF8 cyan active/hover border
pub const ACCENT_CYAN: u32 = 0x0038_BDF8; // #38BDF8 cyan
pub const ACCENT_CYAN_DIM: u32 = 0x000E_2A3D; // subtle cyan pill fill
pub const ACCENT_AMBER: u32 = 0x00F5_9E0B; // #F59E0B amber rating
pub const ACCENT_AMBER_DIM: u32 = 0x002C_2009;
pub const ACCENT_EMERALD: u32 = 0x0010_B981; // #10B981 emerald direct/live
pub const ACCENT_EMERALD_DIM: u32 = 0x000A_2920;
pub const TEXT_PRIMARY: u32 = 0x00F1_F5F9;
pub const TEXT_SECONDARY: u32 = 0x0094_A3B8;
pub const TEXT_MUTED: u32 = 0x0064_748B;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn contains(self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Fills a solid axis-aligned rectangle clipped to `(bw, bh)`.
pub fn fill_rect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    if w <= 0 || h <= 0 {
        return;
    }
    let x0 = x.max(0).min(bw as i32) as usize;
    let y0 = y.max(0).min(bh as i32) as usize;
    let x1 = (x + w).max(0).min(bw as i32) as usize;
    let y1 = (y + h).max(0).min(bh as i32) as usize;
    let px = 0xFF00_0000 | (color & 0x00FF_FFFF);
    for py in y0..y1 {
        let row = py * bw;
        for px_i in x0..x1 {
            buf[row + px_i] = px;
        }
    }
}

/// Draws a rounded dark-glass card with a crisp 1px border.
#[allow(clippy::too_many_arguments)]
pub fn draw_card(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    r: Rect,
    radius: i32,
    fill: u32,
    border: u32,
) {
    if r.w <= 2 || r.h <= 2 {
        return;
    }
    fill_rounded_rect(buf, bw, bh, r.x, r.y, r.w, r.h, radius, border);
    fill_rounded_rect(
        buf,
        bw,
        bh,
        r.x + 1,
        r.y + 1,
        r.w - 2,
        r.h - 2,
        (radius - 1).max(0),
        fill,
    );
}

/// Dims the entire framebuffer to create a dark-glass modal backdrop.
pub fn dim_backdrop(buf: &mut [u32], bw: usize, bh: usize) {
    let total = (bw * bh).min(buf.len());
    for p in &mut buf[..total] {
        let r = ((*p >> 16) & 0xFF) / 4;
        let g = ((*p >> 8) & 0xFF) / 4;
        let b = (*p & 0xFF) / 4;
        *p = 0xFF00_0000 | (r << 16) | (g << 8) | b;
    }
}

/// Deterministic dark-glass fallback poster swatch color from title/id.
pub fn poster_swatch(seed: &str) -> u32 {
    const SWATCHES: [u32; 6] = [
        0x0016_2A3E, // Deep Cyan Slate
        0x0024_1A38, // Deep Violet
        0x0012_2E2A, // Deep Emerald
        0x002E_2214, // Deep Amber
        0x002D_1824, // Deep Rose
        0x001A_2438, // Deep Indigo
    ];
    let mut h: u32 = 2166136261;
    for &b in seed.as_bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16777619);
    }
    SWATCHES[(h as usize) % SWATCHES.len()]
}

/// Blits a decoded `sex_image::Image` into `dest` using aspect-preserving cover scaling.
pub fn blit_image_cover(buf: &mut [u32], bw: usize, bh: usize, dest: Rect, img: &Image) {
    if dest.w <= 0 || dest.h <= 0 || img.width == 0 || img.height == 0 {
        return;
    }
    let x0 = dest.x.max(0).min(bw as i32);
    let y0 = dest.y.max(0).min(bh as i32);
    let x1 = (dest.x + dest.w).max(0).min(bw as i32);
    let y1 = (dest.y + dest.h).max(0).min(bh as i32);
    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let dw = dest.w as usize;
    let dh = dest.h as usize;
    let sw = img.width;
    let sh = img.height;

    // Cover crop: fit the tighter axis and center-crop the longer axis.
    let (crop_x, crop_y, crop_w, crop_h) = if sw * dh > sh * dw {
        let cw = (sh * dw / dh).max(1).min(sw);
        ((sw - cw) / 2, 0, cw, sh)
    } else {
        let ch = (sw * dh / dw).max(1).min(sh);
        (0, (sh - ch) / 2, sw, ch)
    };

    for py in y0..y1 {
        let dy = (py - dest.y) as usize;
        let sy = (crop_y + dy * crop_h / dh).min(sh - 1);
        let src_row = sy * sw;
        let dst_row = py as usize * bw;
        for px in x0..x1 {
            let dx = (px - dest.x) as usize;
            let sx = (crop_x + dx * crop_w / dw).min(sw - 1);
            let sp = img.pixels[src_row + sx];
            let a = (sp >> 24) & 0xFF;
            if a == 255 {
                buf[dst_row + px as usize] = sp;
            } else if a > 0 {
                let dst = buf[dst_row + px as usize];
                let inv = 255 - a;
                let r = (((sp >> 16) & 0xFF) * a + ((dst >> 16) & 0xFF) * inv) / 255;
                let g = (((sp >> 8) & 0xFF) * a + ((dst >> 8) & 0xFF) * inv) / 255;
                let b = ((sp & 0xFF) * a + (dst & 0xFF) * inv) / 255;
                buf[dst_row + px as usize] = 0xFF00_0000 | (r << 16) | (g << 8) | b;
            }
        }
    }
}

/// Geist Sans / Geist Sans Bold rasterizer for BrootLite's Xirtus Dark-Glass UI.
pub struct TextPainter {
    regular: Option<Font<'static>>,
    bold: Option<Font<'static>>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
}

impl TextPainter {
    pub fn new() -> Self {
        Self {
            regular: Font::new(sex_font::GEIST_SANS),
            bold: Font::new(sex_font::GEIST_SANS_BOLD),
            acc: Vec::new(),
            glyph: Vec::new(),
        }
    }

    pub fn measure(&self, text: &str, px_h: i32, is_bold: bool) -> i32 {
        let font = if is_bold {
            self.bold.as_ref().or(self.regular.as_ref())
        } else {
            self.regular.as_ref()
        };
        let Some(font) = font else {
            return (text.chars().count() as i32) * (px_h / 2).max(6);
        };
        let size = (px_h.max(10) as u32) * 64;
        let mut pen = 0i32;
        let mut prev = None;
        for ch in text.chars() {
            let Some(id) = font.glyph(ch as u32) else {
                pen += (px_h / 2) * FX;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += font.kerning(p, id, size);
            }
            pen += font.advance(id, size);
            prev = Some(id);
        }
        pen / FX
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        max_w: i32,
        px_h: i32,
        text: &str,
        color: u32,
        is_bold: bool,
    ) {
        if max_w <= 4 || text.is_empty() {
            return;
        }
        let font = if is_bold {
            self.bold.as_ref().or(self.regular.as_ref())
        } else {
            self.regular.as_ref()
        };
        let Some(font) = font else {
            return;
        };
        let px_size = px_h.max(10) as usize;
        let size = (px_size * 64) as u32;
        let asc = font.ascender(size);
        let line_h =
            ((font.ascender(size) - font.descender(size)) / FX + 2).max(px_size as i32) as usize;
        let limit = (x + max_w).min(bw as i32);
        let cr = ((color >> 16) & 0xFF) as u32;
        let cg = ((color >> 8) & 0xFF) as u32;
        let cb = (color & 0xFF) as u32;
        let pad = 2usize;
        let mut pen = x * FX;
        let mut prev = None;
        for ch in text.chars() {
            if pen / FX >= limit - 6 {
                break;
            }
            let Some(id) = font.glyph(ch as u32) else {
                pen += (px_h / 2) * FX;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += font.kerning(p, id, size);
            }
            let adv = font.advance(id, size);
            let gx0 = (pen / FX) - pad as i32;
            let gw = ((adv / FX).max(0) as usize + 2 * pad + 2).min(64);
            if gw > 0 && line_h > 0 && line_h <= 64 {
                let slen = scratch_len(gw, line_h);
                if self.acc.len() < slen {
                    self.acc.resize(slen, 0);
                }
                if self.glyph.len() < gw * line_h {
                    self.glyph.resize(gw * line_h, 0);
                }
                for b in &mut self.glyph[..gw * line_h] {
                    *b = 0;
                }
                let sub = pen - (pen / FX) * FX + pad as i32 * FX;
                font.draw(
                    id,
                    size,
                    sub,
                    asc + FX,
                    gw,
                    line_h,
                    &mut self.acc[..slen],
                    &mut self.glyph[..gw * line_h],
                );
                for row in 0..line_h {
                    let py = y + row as i32;
                    if py < 0 || py >= bh as i32 {
                        continue;
                    }
                    let row_off = py as usize * bw;
                    for col in 0..gw {
                        let px = gx0 + col as i32;
                        if px < 0 || px >= limit {
                            continue;
                        }
                        let a = self.glyph[row * gw + col] as u32;
                        if a == 0 {
                            continue;
                        }
                        let dst = buf[row_off + px as usize];
                        let dr = (dst >> 16) & 0xFF;
                        let dg = (dst >> 8) & 0xFF;
                        let db = dst & 0xFF;
                        let inv = 255 - a;
                        let nr = (cr * a + dr * inv) / 255;
                        let ng = (cg * a + dg * inv) / 255;
                        let nb = (cb * a + db * inv) / 255;
                        buf[row_off + px as usize] = 0xFF00_0000 | (nr << 16) | (ng << 8) | nb;
                    }
                }
            }
            pen += adv;
            prev = Some(id);
        }
    }

    /// Draws word-wrapped text up to `max_lines`, returning the final Y coordinate.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_wrapped(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        mut y: i32,
        max_w: i32,
        px_h: i32,
        line_gap: i32,
        max_lines: usize,
        text: &str,
        color: u32,
    ) -> i32 {
        let mut lines_drawn = 0usize;
        let mut cur = String::new();
        for word in text.split_whitespace() {
            let candidate = if cur.is_empty() {
                String::from(word)
            } else {
                let mut c = cur.clone();
                c.push(' ');
                c.push_str(word);
                c
            };
            if self.measure(&candidate, px_h, false) <= max_w {
                cur = candidate;
            } else {
                if !cur.is_empty() {
                    self.draw(buf, bw, bh, x, y, max_w, px_h, &cur, color, false);
                    y += px_h + line_gap;
                    lines_drawn += 1;
                    if lines_drawn >= max_lines {
                        return y;
                    }
                }
                cur = String::from(word);
            }
        }
        if !cur.is_empty() && lines_drawn < max_lines {
            self.draw(buf, bw, bh, x, y, max_w, px_h, &cur, color, false);
            y += px_h + line_gap;
        }
        y
    }
}

/// Draws a pill button/chip with optional active/hover state and centered label.
#[allow(clippy::too_many_arguments)]
pub fn draw_pill(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    tp: &mut TextPainter,
    r: Rect,
    label: &str,
    active: bool,
    hovered: bool,
) {
    let (fill, border, fg) = if active {
        (ACCENT_CYAN_DIM, BORDER_ACTIVE, ACCENT_CYAN)
    } else if hovered {
        (BG_HOVER, BORDER_ACTIVE, TEXT_PRIMARY)
    } else {
        (BG_CARD, BORDER_DEFAULT, TEXT_SECONDARY)
    };
    draw_card(buf, bw, bh, r, r.h / 2, fill, border);
    let tw = tp.measure(label, 12, active);
    let tx = r.x + ((r.w - tw) / 2).max(6);
    let ty = r.y + (r.h - 14) / 2;
    tp.draw(buf, bw, bh, tx, ty, r.w - 8, 12, label, fg, active);
}

/// Draws a small colored badge (e.g. `Direct HTTP`, `P2P Magnet`, `4K`, `MOVIE`).
#[allow(clippy::too_many_arguments)]
pub fn draw_badge(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    tp: &mut TextPainter,
    x: i32,
    y: i32,
    label: &str,
    bg: u32,
    fg: u32,
) -> i32 {
    let tw = tp.measure(label, 11, true);
    let w = tw + 14;
    let h = 20;
    draw_card(buf, bw, bh, Rect::new(x, y, w, h), 5, bg, fg);
    tp.draw(buf, bw, bh, x + 7, y + 3, w - 10, 11, label, fg, true);
    w
}
