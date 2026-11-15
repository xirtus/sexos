// SPDX-License-Identifier: GPL-3.0-only
//! Crisp `#![no_std]` software UI canvas, rounded cards, histograms, and cached
//! `sex-font` Geist Sans / Geist Mono anti-aliased typography for Gaze.

#![forbid(unsafe_code)]

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use sex_font::raster::scratch_len;
use sex_font::{Font, GEIST_MONO, GEIST_SANS};
use sex_image::Image;

#[derive(Clone, Copy, Debug)]
pub struct UiRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl UiRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub const fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }

    pub const fn center(&self) -> (i32, i32) {
        (self.x + self.w / 2, self.y + self.h / 2)
    }

    pub const fn inset(&self, dx: i32, dy: i32) -> Self {
        let nw = self.w - 2 * dx;
        let nh = self.h - 2 * dy;
        Self {
            x: self.x + dx,
            y: self.y + dy,
            w: if nw > 0 { nw } else { 0 },
            h: if nh > 0 { nh } else { 0 },
        }
    }
}

#[derive(Clone, Debug)]
pub struct HitControl {
    pub name: String,
    pub rect: UiRect,
}

#[derive(Clone)]
struct CachedGlyph {
    key: u64,
    w: u8,
    h: u8,
    advance_256: i32,
    asc_256: i32,
    coverage: Vec<u8>,
}

pub struct FontPainter {
    entries: Vec<Option<CachedGlyph>>,
    acc: Vec<i32>,
    out: Vec<u8>,
}

impl FontPainter {
    pub fn new() -> Self {
        let mut entries = Vec::with_capacity(256);
        entries.resize(256, None);
        Self {
            entries,
            acc: vec![0i32; scratch_len(40, 40)],
            out: vec![0u8; 40 * 40],
        }
    }

    fn lookup_or_draw(&mut self, mono: bool, px: u16, ch: char) -> Option<usize> {
        let scalar = ch as u32;
        let key = ((mono as u64) << 48) | ((px as u64) << 32) | (scalar as u64);
        let slot = ((key.wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 56) as usize & 0xFF;

        if let Some(existing) = &self.entries[slot] {
            if existing.key == key {
                return Some(slot);
            }
        }

        let font_bytes = if mono { GEIST_MONO } else { GEIST_SANS };
        let font = Font::new(font_bytes)?;
        let glyph = font.glyph(scalar)?;
        let size_arg = (px as u32) * 64;
        let asc_256 = font.ascender(size_arg);
        let advance_256 = font.advance(glyph, size_arg);

        let bw = ((px as usize * 3) / 2).clamp(10, 40);
        let bh = ((px as usize * 3) / 2).clamp(12, 40);
        let need_acc = scratch_len(bw, bh);
        if self.acc.len() < need_acc {
            self.acc.resize(need_acc, 0);
        }
        if self.out.len() < bw * bh {
            self.out.resize(bw * bh, 0);
        }
        self.acc[..need_acc].fill(0);
        self.out[..bw * bh].fill(0);

        if !font.draw(
            glyph,
            size_arg,
            0,
            asc_256,
            bw,
            bh,
            &mut self.acc[..need_acc],
            &mut self.out[..bw * bh],
        ) {
            return None;
        }

        self.entries[slot] = Some(CachedGlyph {
            key,
            w: bw as u8,
            h: bh as u8,
            advance_256,
            asc_256,
            coverage: self.out[..bw * bh].to_vec(),
        });
        Some(slot)
    }

    pub fn measure_text(&mut self, text: &str, px: u16, mono: bool) -> i32 {
        let mut x_256 = 0i32;
        for ch in text.chars() {
            if ch == ' ' {
                x_256 += (px as i32) * 90;
                continue;
            }
            if let Some(slot) = self.lookup_or_draw(mono, px, ch) {
                if let Some(g) = &self.entries[slot] {
                    x_256 += g.advance_256;
                }
            } else {
                x_256 += (px as i32) * 128;
            }
        }
        (x_256 + 128) >> 8
    }

    /// Draws `text` clipped strictly inside `clip`, truncating with `..` if wider than `clip.w`.
    pub fn draw_text_clipped(
        &mut self,
        fb: &mut [u32],
        fb_w: usize,
        fb_h: usize,
        x: i32,
        y: i32,
        text: &str,
        px: u16,
        color: u32,
        mono: bool,
        bold: bool,
        clip: UiRect,
    ) -> i32 {
        if clip.w <= 2 || clip.h <= 2 || fb_w == 0 || fb_h == 0 {
            return x;
        }
        let max_x = (clip.x + clip.w).min(fb_w as i32);
        let min_x = clip.x.max(0);
        let min_y = clip.y.max(0);
        let max_y = (clip.y + clip.h).min(fb_h as i32);
        if min_x >= max_x || min_y >= max_y {
            return x;
        }

        let cr = ((color >> 16) & 0xFF) as u32;
        let cg = ((color >> 8) & 0xFF) as u32;
        let cb = (color & 0xFF) as u32;

        let mut cur_256 = x * 256;
        let baseline_y = y + (px as i32 * 85) / 100;

        for ch in text.chars() {
            if ch == ' ' {
                cur_256 += (px as i32) * 90;
                if (cur_256 >> 8) >= max_x {
                    break;
                }
                continue;
            }
            let Some(slot) = self.lookup_or_draw(mono, px, ch) else {
                cur_256 += (px as i32) * 110;
                continue;
            };
            let Some(g) = &self.entries[slot] else {
                continue;
            };
            let next_px = (cur_256 + g.advance_256) >> 8;
            if next_px > max_x {
                break;
            }
            let gx0 = cur_256 >> 8;
            let gy0 = baseline_y - (g.asc_256 >> 8);
            let gw = g.w as usize;
            let gh = g.h as usize;

            let passes = if bold { 2 } else { 1 };
            for pass in 0..passes {
                let ox = gx0 + pass;
                for row in 0..gh {
                    let dy = gy0 + row as i32;
                    if dy < min_y || dy >= max_y {
                        continue;
                    }
                    let fb_row = dy as usize * fb_w;
                    let cov_row = row * gw;
                    for col in 0..gw {
                        let dx = ox + col as i32;
                        if dx < min_x || dx >= max_x {
                            continue;
                        }
                        let alpha = g.coverage[cov_row + col] as u32;
                        if alpha == 0 {
                            continue;
                        }
                        let alpha = if bold && pass == 1 { alpha * 3 / 5 } else { alpha };
                        if alpha == 0 {
                            continue;
                        }
                        let dst = fb[fb_row + dx as usize];
                        let dr = (dst >> 16) & 0xFF;
                        let dg = (dst >> 8) & 0xFF;
                        let db = dst & 0xFF;
                        let inv = 255 - alpha;
                        let nr = (cr * alpha + dr * inv) / 255;
                        let ng = (cg * alpha + dg * inv) / 255;
                        let nb = (cb * alpha + db * inv) / 255;
                        fb[fb_row + dx as usize] = (nr << 16) | (ng << 8) | nb;
                    }
                }
            }
            cur_256 += g.advance_256;
        }
        cur_256 >> 8
    }
}

pub fn fill_rect(fb: &mut [u32], fb_w: usize, fb_h: usize, r: UiRect, color: u32) {
    let x0 = r.x.max(0) as usize;
    let y0 = r.y.max(0) as usize;
    let x1 = (r.x + r.w).max(0).min(fb_w as i32) as usize;
    let y1 = (r.y + r.h).max(0).min(fb_h as i32) as usize;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let c = color & 0x00FF_FFFF;
    for y in y0..y1 {
        fb[y * fb_w + x0..y * fb_w + x1].fill(c);
    }
}

pub fn fill_round_rect(fb: &mut [u32], fb_w: usize, fb_h: usize, r: UiRect, radius: i32, color: u32) {
    if r.w <= 0 || r.h <= 0 {
        return;
    }
    let rad = radius.clamp(0, r.w.min(r.h) / 2);
    if rad <= 1 {
        fill_rect(fb, fb_w, fb_h, r, color);
        return;
    }
    let c = color & 0x00FF_FFFF;
    let y0 = r.y.max(0);
    let y1 = (r.y + r.h).min(fb_h as i32);
    let rad_sq = rad * rad;

    for y in y0..y1 {
        let ly = y - r.y;
        let dy = if ly < rad {
            rad - 1 - ly
        } else if ly >= r.h - rad {
            ly - (r.h - rad)
        } else {
            0
        };
        let inset = if dy > 0 {
            let mut dx = 0;
            while (rad - 1 - dx) * (rad - 1 - dx) + dy * dy > rad_sq && dx < rad {
                dx += 1;
            }
            dx
        } else {
            0
        };
        let sx0 = (r.x + inset).max(0) as usize;
        let sx1 = (r.x + r.w - inset).max(0).min(fb_w as i32) as usize;
        if sx0 < sx1 {
            fb[y as usize * fb_w + sx0..y as usize * fb_w + sx1].fill(c);
        }
    }
}

pub fn stroke_rect(fb: &mut [u32], fb_w: usize, fb_h: usize, r: UiRect, thickness: i32, color: u32) {
    if r.w <= 0 || r.h <= 0 || thickness <= 0 {
        return;
    }
    let t = thickness.min(r.w / 2).min(r.h / 2).max(1);
    fill_rect(fb, fb_w, fb_h, UiRect::new(r.x, r.y, r.w, t), color);
    fill_rect(fb, fb_w, fb_h, UiRect::new(r.x, r.y + r.h - t, r.w, t), color);
    fill_rect(fb, fb_w, fb_h, UiRect::new(r.x, r.y + t, t, r.h - 2 * t), color);
    fill_rect(
        fb,
        fb_w,
        fb_h,
        UiRect::new(r.x + r.w - t, r.y + t, t, r.h - 2 * t),
        color,
    );
}

pub fn stroke_round_rect(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    r: UiRect,
    radius: i32,
    thickness: i32,
    border_color: u32,
    fill_color: u32,
) {
    fill_round_rect(fb, fb_w, fb_h, r, radius, border_color);
    let inner = r.inset(thickness, thickness);
    if inner.w > 0 && inner.h > 0 {
        fill_round_rect(fb, fb_w, fb_h, inner, (radius - thickness).max(0), fill_color);
    }
}

/// Aspect-fits `img` centered inside `dest` and blits it cleanly onto `fb`, returning the drawn `UiRect`.
pub fn blit_image_fitted(
    fb: &mut [u32],
    fb_w: usize,
    fb_h: usize,
    img: &Image,
    dest: UiRect,
    bg_color: u32,
) -> UiRect {
    fill_rect(fb, fb_w, fb_h, dest, bg_color);
    if img.width == 0 || img.height == 0 || dest.w <= 2 || dest.h <= 2 {
        return dest;
    }
    let dw = dest.w as usize;
    let dh = dest.h as usize;
    let (tw, th) = if img.width * dh >= img.height * dw {
        (dw, (img.height * dw / img.width).max(1))
    } else {
        ((img.width * dh / img.height).max(1), dh)
    };
    let ox = dest.x + ((dest.w - tw as i32) / 2);
    let oy = dest.y + ((dest.h - th as i32) / 2);

    for y in 0..th {
        let dy = oy + y as i32;
        if dy < 0 || dy >= fb_h as i32 || dy < dest.y || dy >= dest.y + dest.h {
            continue;
        }
        let sy = (y * img.height / th).min(img.height - 1);
        let src_row = &img.pixels[sy * img.width..(sy + 1) * img.width];
        let fb_row = dy as usize * fb_w;
        for x in 0..tw {
            let dx = ox + x as i32;
            if dx < 0 || dx >= fb_w as i32 || dx < dest.x || dx >= dest.x + dest.w {
                continue;
            }
            let sx = (x * img.width / tw).min(img.width - 1);
            let p = src_row[sx];
            let a = (p >> 24) & 0xFF;
            let rgb = if a == 255 {
                p & 0x00FF_FFFF
            } else {
                let mix = |s: u32| ((((p >> s) & 0xFF) * a + ((bg_color >> s) & 0xFF) * (255 - a)) / 255) << s;
                mix(16) | mix(8) | mix(0)
            };
            fb[fb_row + dx as usize] = rgb;
        }
    }

    UiRect::new(ox, oy, tw as i32, th as i32)
}

/// Draws a live 48-bin RGB luminance histogram inside `rect` for the Inspector panel.
pub fn draw_luma_histogram(fb: &mut [u32], fb_w: usize, fb_h: usize, img: &Image, rect: UiRect) {
    stroke_round_rect(fb, fb_w, fb_h, rect, 6, 1, 0x243148, 0x0B1019);
    let inner = rect.inset(6, 6);
    if inner.w <= 8 || inner.h <= 8 || img.pixels.is_empty() {
        return;
    }
    const BINS: usize = 48;
    let mut hist_r = [0u32; BINS];
    let mut hist_g = [0u32; BINS];
    let mut hist_b = [0u32; BINS];

    let step = (img.pixels.len() / 2048).max(1);
    for &p in img.pixels.iter().step_by(step) {
        let r = ((p >> 16) & 0xFF) as usize;
        let g = ((p >> 8) & 0xFF) as usize;
        let b = (p & 0xFF) as usize;
        hist_r[(r * (BINS - 1)) / 255] += 1;
        hist_g[(g * (BINS - 1)) / 255] += 1;
        hist_b[(b * (BINS - 1)) / 255] += 1;
    }
    let peak = hist_r
        .iter()
        .chain(hist_g.iter())
        .chain(hist_b.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);

    for x in 0..inner.w {
        let bin = ((x as usize) * BINS / (inner.w as usize).max(1)).min(BINS - 1);
        let hr = ((hist_r[bin] as i64 * inner.h as i64) / peak as i64) as i32;
        let hg = ((hist_g[bin] as i64 * inner.h as i64) / peak as i64) as i32;
        let hb = ((hist_b[bin] as i64 * inner.h as i64) / peak as i64) as i32;
        for y in 0..inner.h {
            let from_bottom = inner.h - 1 - y;
            let mut col = 0x0B1019u32;
            let mut r_ch = (col >> 16) & 0xFF;
            let mut g_ch = (col >> 8) & 0xFF;
            let mut b_ch = col & 0xFF;
            if from_bottom < hr {
                r_ch = (r_ch + 0xB8).min(255);
            }
            if from_bottom < hg {
                g_ch = (g_ch + 0x9E).min(255);
            }
            if from_bottom < hb {
                b_ch = (b_ch + 0xCE).min(255);
            }
            col = (r_ch << 16) | (g_ch << 8) | b_ch;
            let dx = inner.x + x;
            let dy = inner.y + y;
            if dx >= 0 && dx < fb_w as i32 && dy >= 0 && dy < fb_h as i32 {
                fb[dy as usize * fb_w + dx as usize] = col;
            }
        }
    }
}
