// SPDX-License-Identifier: GPL-3.0-only
//! macOS Bluetooth & Nearby Device Pairing Noir pixel drawing helpers (`sex-font` + `sex-theme`).

use sex_font::raster::{scratch_len, FX};
use sex_font::Font;
use sex_theme::candy_ui::fill_rounded_rect;

#[derive(Clone, Copy)]
pub enum FontKind {
    Sans,
    Bold,
    Mono,
}

pub struct Fonts {
    sans: Option<Font<'static>>,
    bold: Option<Font<'static>>,
    mono: Option<Font<'static>>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
}

impl Fonts {
    pub fn new() -> Self {
        Self {
            sans: Font::new(sex_font::GEIST_SANS),
            bold: Font::new(sex_font::GEIST_SANS_BOLD).or_else(|| Font::new(sex_font::GEIST_SANS)),
            mono: Font::new(sex_font::GEIST_MONO).or_else(|| Font::new(sex_font::GEIST_SANS)),
            acc: Vec::new(),
            glyph: Vec::new(),
        }
    }

    fn font(&self, kind: FontKind) -> Option<&Font<'static>> {
        match kind {
            FontKind::Sans => self.sans.as_ref(),
            FontKind::Bold => self.bold.as_ref(),
            FontKind::Mono => self.mono.as_ref(),
        }
    }

    pub fn measure(&self, kind: FontKind, px_h: i32, text: &str) -> i32 {
        let Some(font) = self.font(kind) else {
            return text.chars().count() as i32 * (px_h / 2).max(6);
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
        kind: FontKind,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        max_w: i32,
        px_h: i32,
        text: &str,
        color: u32,
    ) -> i32 {
        let font = match kind {
            FontKind::Sans => self.sans.as_ref(),
            FontKind::Bold => self.bold.as_ref(),
            FontKind::Mono => self.mono.as_ref(),
        };
        let Some(font) = font else {
            return x;
        };
        let px_size = px_h.max(10) as usize;
        let size = (px_size * 64) as u32;
        let asc = font.ascender(size);
        let line_h = ((font.ascender(size) - font.descender(size)) / FX + 2).max(px_size as i32) as usize;
        let limit = (x + max_w).min(bw as i32);
        let cr = (color >> 16) & 0xFF;
        let cg = (color >> 8) & 0xFF;
        let cb = color & 0xFF;
        let pad = 2usize;
        let mut pen = x * FX;
        let mut prev = None;
        for ch in text.chars() {
            if pen / FX >= limit - 4 {
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
                self.glyph[..gw * line_h].fill(0);
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
                        buf[row_off + px as usize] = (((cr * a + dr * inv) / 255) << 16)
                            | (((cg * a + dg * inv) / 255) << 8)
                            | ((cb * a + db * inv) / 255);
                    }
                }
            }
            pen += adv;
            prev = Some(id);
        }
        pen / FX
    }
}

pub fn fill_rect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x + w).clamp(0, bw as i32) as usize;
    let y1 = (y + h).clamp(0, bh as i32) as usize;
    if x0 >= x1 {
        return;
    }
    for row in y0..y1 {
        buf[row * bw + x0..row * bw + x1].fill(color);
    }
}

pub fn fill_bordered_card(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
    fill: u32,
    border: u32,
) {
    fill_rounded_rect(buf, bw, bh, x, y, w, h, r, border);
    if w > 2 && h > 2 {
        fill_rounded_rect(buf, bw, bh, x + 1, y + 1, w - 2, h - 2, (r - 1).max(0), fill);
    }
}

/// Draws a 4-bar RSSI / link signal meter.
pub fn draw_rssi_bars(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    x: i32,
    y: i32,
    bars_lit: usize,
    lit_color: u32,
    dim_color: u32,
) {
    for i in 0..4 {
        let bh_bar = 4 + (i as i32) * 3;
        let bx = x + (i as i32) * 4;
        let by = y + (13 - bh_bar);
        let col = if i < bars_lit { lit_color } else { dim_color };
        fill_rounded_rect(buf, bw, bh, bx, by, 3, bh_bar, 1, col);
    }
}

pub fn clip(text: &str, n: usize) -> &str {
    match text.char_indices().nth(n) {
        Some((i, _)) => &text[..i],
        None => text,
    }
}
