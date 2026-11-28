// SPDX-License-Identifier: GPL-3.0-only
//! Cached Geist Sans & Geist Mono text renderer and macOS Noir palette for Jot.

use alloc::collections::BTreeMap;
use alloc::vec;
use alloc::vec::Vec;
use sex_font::raster::{scratch_len, FX};
use sex_font::{gamma, Font, Render, Synthetic};
use sex_theme::candy_ui::fill_rounded_rect;

pub const BG_CANVAS: u32 = 0x000D_1017;
pub const BG_BAR: u32 = 0x0015_1924;
pub const BG_DOC: u32 = 0x000F_131D;
pub const BG_GUTTER: u32 = 0x0012_1622;
pub const BG_CURRENT: u32 = 0x0017_1C2B;
pub const BG_PILL: u32 = 0x001E_2433;
pub const BG_PILL_HOVER: u32 = 0x0026_2E42;
pub const BG_SELECTED: u32 = 0x003B_1826;
pub const BG_HIT: u32 = 0x0042_3217;
pub const BORDER: u32 = 0x0025_2C3D;
pub const ROSE: u32 = 0x00F4_3F5E;
pub const ROSE_DIM: u32 = 0x003B_1522;
pub const FG: u32 = 0x00F1_F5F9;
pub const DIM: u32 = 0x0094_A3B8;
pub const MUTED: u32 = 0x0064_748B;
pub const MISSPELT: u32 = 0x00F4_3F5E;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FaceKind {
    Sans = 0,
    SansBold = 1,
    Mono = 2,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CacheKey {
    face: u8,
    px: u16,
    glyph: u16,
    dark_ink: bool,
}

struct CachedGlyph {
    w: usize,
    h: usize,
    adv: i32,
    cov: Vec<u8>,
}

pub struct NoirFont {
    sans: Font<'static>,
    sans_bold: Font<'static>,
    mono: Font<'static>,
    cache: BTreeMap<CacheKey, CachedGlyph>,
    acc: Vec<i32>,
}

impl NoirFont {
    pub fn new() -> Self {
        Self {
            sans: Font::new(sex_font::GEIST_SANS).expect("GEIST_SANS"),
            sans_bold: Font::new(sex_font::GEIST_SANS_BOLD).expect("GEIST_SANS_BOLD"),
            mono: Font::new(sex_font::GEIST_MONO).expect("GEIST_MONO"),
            cache: BTreeMap::new(),
            acc: Vec::new(),
        }
    }

    fn font(&self, face: FaceKind) -> &Font<'static> {
        match face {
            FaceKind::Sans => &self.sans,
            FaceKind::SansBold => &self.sans_bold,
            FaceKind::Mono => &self.mono,
        }
    }

    pub fn line_height(&self, face: FaceKind, px_h: usize) -> usize {
        let size = (px_h.max(9) * 64) as u32;
        let f = self.font(face);
        ((f.ascender(size) - f.descender(size)) / FX + 4).max(px_h as i32 + 3) as usize
    }

    pub fn mono_advance(&self, px_h: usize) -> usize {
        let size = (px_h.max(9) * 64) as u32;
        if let Some(g) = self.mono.glyph(b'M' as u32) {
            ((self.mono.advance(g, size) / FX).max(6)) as usize
        } else {
            ((px_h * 6) / 10).max(6)
        }
    }

    pub fn measure(&self, face: FaceKind, px_h: usize, text: &str) -> i32 {
        let size = (px_h.max(9) * 64) as u32;
        let f = self.font(face);
        let mut pen = 0i32;
        let mut prev = None;
        for ch in text.chars() {
            let Some(id) = f.glyph(ch as u32) else {
                pen += (px_h as i32 / 2) * FX;
                prev = None;
                continue;
            };
            if let Some(p) = prev {
                pen += f.kerning(p, id, size);
            }
            pen += f.advance(id, size);
            prev = Some(id);
        }
        pen / FX
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_glyph_mono(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        px_h: usize,
        ch: char,
        color: u32,
        bg: u32,
    ) {
        let cw = self.mono_advance(px_h) as i32;
        let ch_h = self.line_height(FaceKind::Mono, px_h) as i32;
        if bg != BG_DOC {
            fill_rect(buf, bw, bh, x, y, cw, ch_h, bg);
        }
        if ch == ' ' || ch == '\t' {
            return;
        }
        let s = [ch];
        let mut tmp = [0u8; 4];
        let str_ch = s[0].encode_utf8(&mut tmp);
        self.draw_text(buf, bw, bh, FaceKind::Mono, x, y, cw + 4, px_h, str_ch, color);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        face: FaceKind,
        x: i32,
        y: i32,
        max_w: i32,
        px_h: usize,
        text: &str,
        color: u32,
    ) -> i32 {
        let px = px_h.max(9).min(48);
        let size = (px * 64) as u32;
        let line_h = self.line_height(face, px).min(64);
        let asc = self.font(face).ascender(size);
        let limit = (x + max_w).min(bw as i32);
        let k = gamma::contrast_for(color);
        let dark_ink = gamma::luminance(color) < 2048;
        let render = Render::ui_for(color);
        let pad = 2usize;

        let mut pen = x * FX;
        let mut prev = None;
        for ch in text.chars() {
            if pen / FX >= limit {
                break;
            }
            let (id, kern) = {
                let f = self.font(face);
                let Some(id) = f.glyph(ch as u32) else {
                    pen += (px as i32 / 2) * FX;
                    prev = None;
                    continue;
                };
                let kern = prev.map_or(0, |p| f.kerning(p, id, size));
                (id, kern)
            };
            pen += kern;
            let key = CacheKey {
                face: face as u8,
                px: px as u16,
                glyph: id.0,
                dark_ink,
            };
            if !self.cache.contains_key(&key) {
                if self.cache.len() > 2048 {
                    self.cache.clear();
                }
                let f = match face {
                    FaceKind::Sans => &self.sans,
                    FaceKind::SansBold => &self.sans_bold,
                    FaceKind::Mono => &self.mono,
                };
                let adv = f.advance(id, size);
                let gw = ((adv / FX).max(0) as usize + 2 * pad + 3).min(64);
                let slen = scratch_len(gw, line_h);
                if self.acc.len() < slen {
                    self.acc.resize(slen, 0);
                }
                let mut cov = vec![0u8; gw * line_h];
                f.draw_rendered(
                    id,
                    size,
                    (pad as i32) * FX,
                    asc + FX,
                    gw,
                    line_h,
                    &mut self.acc[..slen],
                    &mut cov,
                    Synthetic::NONE,
                    render,
                );
                self.cache.insert(key, CachedGlyph { w: gw, h: line_h, adv, cov });
            }
            let g = &self.cache[&key];
            let gx0 = (pen / FX) - pad as i32;
            for row in 0..g.h {
                let py = y + row as i32;
                if py < 0 || py >= bh as i32 {
                    continue;
                }
                let row_off = py as usize * bw;
                for col in 0..g.w {
                    let px_x = gx0 + col as i32;
                    if px_x < x.max(0) || px_x >= limit {
                        continue;
                    }
                    let a = g.cov[row * g.w + col];
                    if a == 0 {
                        continue;
                    }
                    let idx = row_off + px_x as usize;
                    buf[idx] = gamma::blend_with(color, buf[idx], a, k);
                }
            }
            pen += g.adv;
            prev = Some(id);
        }
        pen / FX
    }
}

pub fn fill_rect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, c: u32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x + w).min(bw as i32).max(0) as usize;
    let y1 = (y + h).min(bh as i32).max(0) as usize;
    for py in y0..y1 {
        let row = py * bw;
        buf[row + x0..row + x1].fill(c);
    }
}

pub fn draw_card(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    fill: u32,
    border: u32,
) {
    if w <= 2 || h <= 2 {
        return;
    }
    fill_rounded_rect(buf, bw, bh, x, y, w, h, radius, border);
    fill_rounded_rect(buf, bw, bh, x + 1, y + 1, w - 2, h - 2, (radius - 1).max(0), fill);
}
