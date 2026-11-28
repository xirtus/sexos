// SPDX-License-Identifier: GPL-3.0-only
//! macOS Noir pixel-buffer rendering primitives for Corset (`sex-font` + `sex-theme`).

use alloc::vec;
use alloc::vec::Vec;
use sex_app::Window;
use sex_font::raster::{scratch_len, FX};
use sex_font::{Font, Render, Synthetic};
use sex_theme::candy_ui::{
    draw_candy_button, fill_rounded_rect, mix_rgb, ButtonState, ButtonStyle, CandyPalette,
};
use sex_theme::Mode;

pub const BG: u32 = 0x000D_1017;
pub const HEADER: u32 = 0x000B_0E14;
pub const CARD: u32 = 0x0015_1924;
pub const CARD_ALT: u32 = 0x0019_1F2D;
pub const WELL: u32 = 0x0009_0B10;
pub const BORDER: u32 = 0x0025_2C3D;
pub const BORDER_ACTIVE: u32 = 0x003B_4661;
pub const TEXT: u32 = 0x00F1_F5F9;
pub const MUTED: u32 = 0x0094_A3B8;
pub const DIM: u32 = 0x0064_748B;

pub const ROSE: u32 = 0x00F4_3F5E;
pub const ROSE_BG: u32 = 0x002B_121B;
pub const ROSE_BORDER: u32 = 0x009F_1239;

pub const EMERALD: u32 = 0x0010_B981;
pub const EMERALD_BG: u32 = 0x000D_2620;
pub const EMERALD_BORDER: u32 = 0x0006_5F46;

pub const AMBER: u32 = 0x00F5_9E0B;
pub const AMBER_BG: u32 = 0x0029_1D0B;
pub const AMBER_BORDER: u32 = 0x0092_400E;

pub const SKY: u32 = 0x0038_BDF8;
pub const SKY_BG: u32 = 0x000C_2133;
pub const SKY_BORDER: u32 = 0x0007_5985;

pub const fn noir_palette() -> CandyPalette {
    CandyPalette {
        mode: Mode::Dark,
        bg: BG,
        panel: HEADER,
        toolbar_top: 0x0011_1520,
        toolbar_bottom: HEADER,
        card: CARD,
        well: WELL,
        control: 0x001B_2130,
        control_hover: 0x0023_2B3E,
        control_pressed: 0x0013_1824,
        accent: ROSE,
        accent_hover: 0x00FB_7185,
        accent_pressed: 0x00E1_1D48,
        danger: 0x00E1_1D48,
        danger_hover: 0x00F4_3F5E,
        danger_pressed: 0x00BE_123C,
        selection: 0x002B_1722,
        hairline: BORDER,
        rim: BORDER,
        rim_accent: 0x00FB_7185,
        specular: 0x0047_5569,
        text: TEXT,
        text_dim: MUTED,
        text_disabled: DIM,
        text_on_accent: 0x00FF_FFFF,
        mint: EMERALD,
        honey: AMBER,
        sky: SKY,
    }
}

pub fn window_dims(window: &Window) -> (usize, usize) {
    let (rw, rh) = window.size();
    let w = rw.clamp(320, 4096);
    let max_h = (window.room() / w).max(200);
    let h = rh.clamp(200, max_h);
    (w, h)
}

pub fn fill_rect(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, w: i32, h: i32, color: u32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x + w).min(bw as i32).max(0) as usize;
    let y1 = (y + h).min(bh as i32).max(0) as usize;
    for row in y0..y1 {
        let off = row * bw;
        buf[off + x0..off + x1].fill(color);
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
    fill_rounded_rect(
        buf,
        bw,
        bh,
        x + 1,
        y + 1,
        w - 2,
        h - 2,
        (radius - 1).max(0),
        fill,
    );
}

pub struct NoirPainter {
    pub sans: Option<Font<'static>>,
    pub mono: Option<Font<'static>>,
    acc: Vec<i32>,
    glyph: Vec<u8>,
}

impl NoirPainter {
    pub fn new() -> Self {
        Self {
            sans: Font::new(sex_font::GEIST_SANS),
            mono: Font::new(sex_font::GEIST_MONO),
            acc: vec![0; scratch_len(64, 64)],
            glyph: vec![0; 64 * 64],
        }
    }

    pub fn measure(&self, mono: bool, px_h: i32, text: &str) -> i32 {
        let font = if mono { self.mono.as_ref() } else { self.sans.as_ref() };
        let Some(font) = font else {
            return text.chars().count() as i32 * (px_h * 3 / 5).max(6);
        };
        let size = (px_h.max(9) as u32) * 64;
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
    pub fn text(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        mono: bool,
        x: i32,
        y: i32,
        max_w: i32,
        px_h: i32,
        text: &str,
        color: u32,
    ) {
        if max_w <= 4 {
            return;
        }
        let font = if mono { self.mono.as_ref() } else { self.sans.as_ref() };
        let Some(font) = font else { return };
        let px_size = px_h.max(9) as usize;
        let size = (px_size as u32) * 64;
        let asc = font.ascender(size);
        let line_h = ((font.ascender(size) - font.descender(size)) / FX + 2).max(px_size as i32) as usize;
        let limit = (x + max_w).min(bw as i32);
        let render = Render::ui_for(color);
        let pad = 2usize;
        let mut pen = x * FX;
        let mut prev = None;
        for ch in text.chars() {
            if pen / FX >= limit - 4 {
                break;
            }
            let Some(id) = font.glyph(ch as u32).or_else(|| font.glyph('?' as u32)) else {
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
                font.draw_rendered(
                    id,
                    size,
                    sub,
                    asc + FX,
                    gw,
                    line_h,
                    &mut self.acc[..slen],
                    &mut self.glyph[..gw * line_h],
                    Synthetic::NONE,
                    render,
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
                        let a = self.glyph[row * gw + col];
                        if a == 0 {
                            continue;
                        }
                        let idx = row_off + px as usize;
                        buf[idx] = mix_rgb(buf[idx], color, a);
                    }
                }
            }
            pen += adv;
            prev = Some(id);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pill(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        h: i32,
        mono: bool,
        label: &str,
        fg: u32,
        bg: u32,
        border: u32,
    ) -> i32 {
        let tw = self.measure(mono, 11, label);
        let pw = (tw + 16).max(36);
        draw_card(buf, bw, bh, x, y, pw, h, h / 2, bg, border);
        self.text(buf, bw, bh, mono, x + 8, y + (h - 12) / 2 - 1, pw - 16, 11, label, fg);
        pw
    }

    #[allow(clippy::too_many_arguments)]
    pub fn button(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        label: &str,
        style: ButtonStyle,
        state: ButtonState,
    ) {
        let pal = noir_palette();
        draw_candy_button(buf, bw, bh, x, y, w, h, (h / 2).min(8), style, state, &pal);
        let fg = match style {
            ButtonStyle::Primary | ButtonStyle::Danger => 0x00FF_FFFF,
            ButtonStyle::Active => ROSE,
            ButtonStyle::Disabled => DIM,
            _ => TEXT,
        };
        let px_h = if h <= 24 { 11 } else { 12 };
        let tw = self.measure(false, px_h, label);
        let tx = x + ((w - tw) / 2).max(6);
        let ty = y + (h - px_h) / 2 - 1;
        self.text(buf, bw, bh, false, tx, ty, w - 10, px_h, label, fg);
    }
}
