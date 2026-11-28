// SPDX-License-Identifier: GPL-3.0-only
//! macOS App Store Dark-Mode Noir rendering primitives, Geist Sans/Bold/Mono
//! proportional font renderer, and vector icons for `apps/store` (`sexshop`).

use alloc::vec;
use alloc::vec::Vec;
use sex_font::cache::{GlyphCache, Key as GlyphKey};
use sex_font::gamma::{blend_with, contrast_for};
use sex_font::raster::{scratch_len, Raster, FX};
use sex_font::{Font, Render, Synthetic};
use sex_theme::candy_ui::{
    button_text_color, draw_candy_badge, draw_candy_button, fill_rounded_rect, mix_rgb,
    ButtonState, ButtonStyle, CandyPalette,
};
use sex_theme::Mode;

pub const NOIR_SIDEBAR: u32 = 0x000B_0D12;
pub const NOIR_CANVAS: u32 = 0x0011_141D;
pub const NOIR_CARD: u32 = 0x0018_1C28;
pub const NOIR_TOP_SPECULAR: u32 = 0x0026_2D3F;
pub const NOIR_HAIRLINE: u32 = 0x001E_2434;
pub const NOIR_WELL: u32 = 0x0012_1621;
pub const NOIR_CONTROL: u32 = 0x0022_2839;
pub const NOIR_CONTROL_HOVER: u32 = 0x002B_3348;
pub const NOIR_CONTROL_PRESSED: u32 = 0x001A_1F2E;
pub const NOIR_TEXT: u32 = 0x00F1_F5F9;
pub const NOIR_TEXT_DIM: u32 = 0x0094_A3B8;
pub const NOIR_TEXT_DISABLED: u32 = 0x0064_748B;
pub const NOIR_ROSE: u32 = 0x00F4_3F5E;
pub const NOIR_MINT: u32 = 0x0010_B981;
pub const NOIR_SKY: u32 = 0x0038_BDF8;
pub const NOIR_AMBER: u32 = 0x00F5_9E0B;
pub const NOIR_INDIGO: u32 = 0x0063_66F1;
pub const NOIR_PURPLE: u32 = 0x00A8_55F7;

pub fn noir_palette() -> CandyPalette {
    CandyPalette {
        mode: Mode::Dark,
        bg: NOIR_CANVAS,
        panel: NOIR_SIDEBAR,
        toolbar_top: 0x0012_151F,
        toolbar_bottom: NOIR_CANVAS,
        card: NOIR_CARD,
        well: NOIR_WELL,
        control: NOIR_CONTROL,
        control_hover: NOIR_CONTROL_HOVER,
        control_pressed: NOIR_CONTROL_PRESSED,
        accent: NOIR_ROSE,
        accent_hover: 0x00FB_7185,
        accent_pressed: 0x00E1_1D48,
        danger: 0x00EF_4444,
        danger_hover: 0x00F8_7171,
        danger_pressed: 0x00DC_2626,
        selection: mix_rgb(NOIR_CARD, NOIR_ROSE, 50),
        hairline: NOIR_HAIRLINE,
        rim: NOIR_HAIRLINE,
        rim_accent: mix_rgb(NOIR_ROSE, 0x00FF_FFFF, 72),
        specular: NOIR_TOP_SPECULAR,
        text: NOIR_TEXT,
        text_dim: NOIR_TEXT_DIM,
        text_disabled: NOIR_TEXT_DISABLED,
        text_on_accent: 0x00FF_FFFF,
        mint: NOIR_MINT,
        honey: NOIR_AMBER,
        sky: NOIR_SKY,
    }
}

const MAX_GLYPH_W: usize = 32;
const MAX_GLYPH_H: usize = 32;
const GLYPH_SLOTS: usize = 256;
const GLYPH_ARENA: usize = 256 * 18 * 20;

/// Heap-backed proportional Geist Sans / Geist Bold / Geist Mono renderer with glyph cache.
pub struct Glyphs {
    sans_reg: Option<Font<'static>>,
    sans_bold: Option<Font<'static>>,
    mono: Option<Font<'static>>,
    cache: GlyphCache<GLYPH_SLOTS, GLYPH_ARENA>,
    acc: Vec<i32>,
    cover: Vec<u8>,
    icon_acc: Vec<i32>,
    icon_mask: Vec<u8>,
}

impl Glyphs {
    pub fn new() -> Self {
        Self {
            sans_reg: Font::new(sex_font::GEIST_SANS),
            sans_bold: Font::new(sex_font::GEIST_SANS_BOLD),
            mono: Font::new(sex_font::GEIST_MONO),
            cache: GlyphCache::new(),
            acc: vec![0; scratch_len(MAX_GLYPH_W, MAX_GLYPH_H)],
            cover: vec![0; MAX_GLYPH_W * MAX_GLYPH_H],
            icon_acc: vec![0; scratch_len(20, 20)],
            icon_mask: vec![0; 20 * 20],
        }
    }

    fn font_ref(&self, kind: u8) -> Option<&Font<'static>> {
        match kind {
            1 => self.sans_bold.as_ref().or(self.sans_reg.as_ref()),
            2 => self.mono.as_ref().or(self.sans_reg.as_ref()),
            _ => self.sans_reg.as_ref(),
        }
    }

    pub fn measure(&self, kind: u8, px_h: i32, text: &str) -> i32 {
        let Some(font) = self.font_ref(kind) else {
            return (text.len() as i32) * (px_h * 6 / 10);
        };
        let size = (px_h.max(9) as u32) * 64;
        let mut pen = 0i32;
        let mut prev = None;
        for ch in text.chars() {
            let Some(id) = font.glyph(ch as u32).or_else(|| font.glyph('?' as u32)) else {
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
        (pen + FX - 1) / FX
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
        &mut self,
        buf: &mut [u32],
        bw: usize,
        bh: usize,
        x: i32,
        y: i32,
        max_w: i32,
        px_h: i32,
        kind: u8,
        text: &str,
        color: u32,
    ) {
        if max_w <= 4 || text.is_empty() || buf.len() < bw.saturating_mul(bh) {
            return;
        }
        let font_opt = match kind {
            1 => self.sans_bold.as_ref().or(self.sans_reg.as_ref()),
            2 => self.mono.as_ref().or(self.sans_reg.as_ref()),
            _ => self.sans_reg.as_ref(),
        };
        let Some(font) = font_opt else { return };

        let px_size = px_h.clamp(9, 26) as usize;
        let size = (px_size * 64) as u32;
        let asc = font.ascender(size);
        let desc = font.descender(size);
        let line_h = (((asc - desc) / FX) + 2).clamp(px_size as i32, MAX_GLYPH_H as i32) as usize;
        let limit = (x + max_w).min(bw as i32);
        let pad = 2usize;
        let render = Render::ui_for(color);
        let k = contrast_for(color);
        let font_key = kind * 2 + render.darken as u8;

        let mut pen = x * FX;
        let mut prev = None;

        for ch in text.chars() {
            if pen / FX >= limit - 2 {
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
            if (pen + adv) / FX > limit + 1 && pen > x * FX {
                break;
            }
            if ch != ' ' {
                let gx0 = (pen / FX) - pad as i32;
                let gw = ((adv / FX).max(1) as usize + 2 * pad + 2).min(MAX_GLYPH_W);
                let frac = (pen - (pen / FX) * FX).clamp(0, FX - 1);
                let phase = ((frac * 4) / FX).clamp(0, 3) as u8;
                let sub = (phase as i32 * FX / 4) + pad as i32 * FX;
                let gkey = GlyphKey {
                    font: font_key,
                    scalar: ch as u32,
                    size: px_size as u16,
                    phase,
                    w: gw as u8,
                    h: line_h as u8,
                };
                let acc = &mut self.acc;
                let cover = &mut self.cover;
                let bitmap_opt = if gw * line_h <= 18 * 20 {
                    self.cache.get(gkey, |out| {
                        out.fill(0);
                        font.draw_rendered(
                            id,
                            size,
                            sub,
                            asc + FX,
                            gw,
                            line_h,
                            acc,
                            out,
                            Synthetic::NONE,
                            render,
                        )
                    })
                } else {
                    let len = gw * line_h;
                    cover[..len].fill(0);
                    if font.draw_rendered(
                        id,
                        size,
                        sub,
                        asc + FX,
                        gw,
                        line_h,
                        acc,
                        &mut cover[..len],
                        Synthetic::NONE,
                        render,
                    ) {
                        Some(&cover[..len])
                    } else {
                        None
                    }
                };

                if let Some(cov) = bitmap_opt {
                    for row in 0..line_h {
                        let py = y + row as i32;
                        if py < 0 || py >= bh as i32 {
                            continue;
                        }
                        let row_off = py as usize * bw;
                        for col in 0..gw {
                            let px = gx0 + col as i32;
                            if px < x || px >= limit || px < 0 {
                                continue;
                            }
                            let a = cov[row * gw + col];
                            if a == 0 {
                                continue;
                            }
                            let idx = row_off + px as usize;
                            buf[idx] = blend_with(color, buf[idx], a, k) & 0x00FF_FFFF;
                        }
                    }
                }
            }
            pen += adv;
            prev = Some(id);
        }
    }
}

pub fn draw_sans(buf: &mut [u32], w: usize, h: usize, g: &mut Glyphs, x: i32, y: i32, mw: i32, s: &str, c: u32) {
    g.draw_text(buf, w, h, x, y, mw, 13, 0, s, c);
}
pub fn draw_sans_sm(buf: &mut [u32], w: usize, h: usize, g: &mut Glyphs, x: i32, y: i32, mw: i32, s: &str, c: u32) {
    g.draw_text(buf, w, h, x, y, mw, 11, 0, s, c);
}
pub fn draw_sans_bold(buf: &mut [u32], w: usize, h: usize, g: &mut Glyphs, x: i32, y: i32, mw: i32, sz: i32, s: &str, c: u32) {
    g.draw_text(buf, w, h, x, y, mw, sz, 1, s, c);
}
pub fn draw_mono(buf: &mut [u32], w: usize, h: usize, g: &mut Glyphs, x: i32, y: i32, mw: i32, s: &str, c: u32) {
    g.draw_text(buf, w, h, x, y, mw, 11, 2, s, c);
}

pub fn fill_solid(buf: &mut [u32], bw: usize, bh: usize, x: i32, y: i32, rw: i32, rh: i32, color: u32) {
    if rw <= 0 || rh <= 0 || buf.len() < bw.saturating_mul(bh) {
        return;
    }
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = (x + rw).clamp(0, bw as i32) as usize;
    let y1 = (y + rh).clamp(0, bh as i32) as usize;
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    for row in y0..y1 {
        buf[row * bw + x0..row * bw + x1].fill(color);
    }
}

/// Draws a `#181C28` rounded card with `1px` `#262D3F` top specular rim and `#1E2434` body rim.
pub fn draw_noir_card(
    buf: &mut [u32],
    w: usize,
    h: usize,
    x: i32,
    y: i32,
    cw: i32,
    ch: i32,
    r: i32,
    pal: &CandyPalette,
    border_override: Option<u32>,
) {
    if cw <= 4 || ch <= 4 {
        return;
    }
    let rim = border_override.unwrap_or(pal.hairline);
    fill_rounded_rect(buf, w, h, x, y, cw, ch, r, rim);
    fill_rounded_rect(buf, w, h, x + 1, y + 1, cw - 2, ch - 2, (r - 1).max(2), pal.card);
    // 1px #262D3F top specular highlight inset from rounded corners.
    let top_spec = border_override.unwrap_or(pal.specular);
    let pad = (r / 2).max(4).min(cw / 3);
    fill_solid(buf, w, h, x + pad, y + 1, (cw - 2 * pad).max(1), 1, top_spec);
}

pub fn draw_noir_well(
    buf: &mut [u32],
    w: usize,
    h: usize,
    x: i32,
    y: i32,
    cw: i32,
    ch: i32,
    r: i32,
    border: u32,
    pal: &CandyPalette,
) {
    fill_rounded_rect(buf, w, h, x, y, cw, ch, r, border);
    fill_rounded_rect(buf, w, h, x + 1, y + 1, cw - 2, ch - 2, (r - 1).max(2), pal.well);
}

pub fn draw_pill_button(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    bw: i32,
    bh: i32,
    label: &str,
    style: ButtonStyle,
    state: ButtonState,
    pal: &CandyPalette,
) {
    let r = (bh / 2).min(9);
    draw_candy_button(buf, w, h, x, y, bw, bh, r, style, state, pal);
    let ink = button_text_color(style, pal);
    let tw = g.measure(1, 11, label).min(bw - 10);
    let tx = x + ((bw - tw) / 2).max(4);
    draw_sans_bold(buf, w, h, g, tx, y + (bh - 13) / 2, bw - 8, 11, label, ink);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreIcon {
    Search,
    Discover,
    AllApps,
    Creative,
    Productivity,
    System,
    Updates,
    Repos,
    Shield,
    Back,
    Refresh,
}

fn fast_sqrtf(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut g = f32::from_bits((x.to_bits() >> 1) + 0x1FC0_0000);
    for _ in 0..4 {
        g = 0.5 * (g + x / g);
    }
    g
}

fn fast_sinf(mut x: f32) -> f32 {
    use core::f32::consts::{FRAC_PI_2, PI, TAU};
    while x > PI {
        x -= TAU;
    }
    while x < -PI {
        x += TAU;
    }
    if x > FRAC_PI_2 {
        x = PI - x;
    } else if x < -FRAC_PI_2 {
        x = -PI - x;
    }
    let x2 = x * x;
    x * (1.0 - x2 * (1.0 / 6.0 - x2 * (1.0 / 120.0 - x2 / 5040.0)))
}

fn fast_cosf(x: f32) -> f32 {
    fast_sinf(x + core::f32::consts::FRAC_PI_2)
}

fn stroke_line(r: &mut Raster<'_>, x0: f32, y0: f32, x1: f32, y1: f32, half: f32) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = fast_sqrtf(dx * dx + dy * dy);
    if len < 0.05 {
        return;
    }
    let nx = -dy / len * half;
    let ny = dx / len * half;
    let p = |x: f32, y: f32| ((x * FX as f32 + 0.5) as i32, (y * FX as f32 + 0.5) as i32);
    let a = p(x0 + nx, y0 + ny);
    let b = p(x1 + nx, y1 + ny);
    let c = p(x1 - nx, y1 - ny);
    let d = p(x0 - nx, y0 - ny);
    r.line(a.0, a.1, b.0, b.1);
    r.line(b.0, b.1, c.0, c.1);
    r.line(c.0, c.1, d.0, d.1);
    r.line(d.0, d.1, a.0, a.1);
}

fn fill_ring(r: &mut Raster<'_>, cx: f32, cy: f32, rad: f32, thick: f32) {
    const N: usize = 16;
    const STEP: f32 = core::f32::consts::TAU / N as f32;
    let half = thick * 0.5;
    for i in 0..N {
        let a0 = i as f32 * STEP;
        let a1 = (i + 1) as f32 * STEP;
        stroke_line(
            r,
            cx + rad * fast_cosf(a0),
            cy + rad * fast_sinf(a0),
            cx + rad * fast_cosf(a1),
            cy + rad * fast_sinf(a1),
            half,
        );
    }
}

fn fill_arc(r: &mut Raster<'_>, cx: f32, cy: f32, rad: f32, a0: f32, a1: f32, half: f32) {
    const N: usize = 8;
    for i in 0..N {
        let t0 = a0 + (a1 - a0) * (i as f32 / N as f32);
        let t1 = a0 + (a1 - a0) * ((i + 1) as f32 / N as f32);
        stroke_line(
            r,
            cx + rad * fast_cosf(t0),
            cy + rad * fast_sinf(t0),
            cx + rad * fast_cosf(t1),
            cy + rad * fast_sinf(t1),
            half,
        );
    }
}

fn fill_disc(r: &mut Raster<'_>, cx: f32, cy: f32, rad: f32) {
    const N: usize = 12;
    const STEP: f32 = core::f32::consts::TAU / N as f32;
    for i in 0..N {
        let a0 = i as f32 * STEP;
        let a1 = (i + 1) as f32 * STEP;
        r.line(
            ((cx + rad * fast_cosf(a0)) * FX as f32 + 0.5) as i32,
            ((cy + rad * fast_sinf(a0)) * FX as f32 + 0.5) as i32,
            ((cx + rad * fast_cosf(a1)) * FX as f32 + 0.5) as i32,
            ((cy + rad * fast_sinf(a1)) * FX as f32 + 0.5) as i32,
        );
    }
}

pub fn draw_icon(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    icon: StoreIcon,
    color: u32,
) {
    let side = 14usize;
    let mask = &mut g.icon_mask[..side * side];
    mask.fill(0);
    let Some(mut r) = Raster::new(side, side, &mut g.icon_acc) else {
        return;
    };
    let h = 0.72;
    match icon {
        StoreIcon::Search => {
            fill_ring(&mut r, 5.8, 5.8, 3.6, 1.45);
            stroke_line(&mut r, 8.4, 8.4, 12.2, 12.2, 0.85);
        }
        StoreIcon::Discover => {
            // 4-point sparkle star.
            stroke_line(&mut r, 7.0, 1.8, 7.0, 12.2, 0.9);
            stroke_line(&mut r, 1.8, 7.0, 12.2, 7.0, 0.9);
            stroke_line(&mut r, 3.6, 3.6, 10.4, 10.4, 0.65);
            stroke_line(&mut r, 10.4, 3.6, 3.6, 10.4, 0.65);
        }
        StoreIcon::AllApps => {
            for (gx, gy) in [(3.0, 3.0), (8.2, 3.0), (3.0, 8.2), (8.2, 8.2)] {
                stroke_line(&mut r, gx, gy, gx + 2.8, gy, 0.95);
                stroke_line(&mut r, gx, gy + 2.0, gx + 2.8, gy + 2.0, 0.95);
            }
        }
        StoreIcon::Creative => {
            stroke_line(&mut r, 2.5, 11.5, 11.5, 2.5, 0.95);
            fill_disc(&mut r, 3.4, 10.6, 1.5);
        }
        StoreIcon::Productivity => {
            stroke_line(&mut r, 2.6, 4.0, 11.4, 4.0, h);
            stroke_line(&mut r, 2.6, 7.0, 11.4, 7.0, h);
            stroke_line(&mut r, 2.6, 10.0, 8.6, 10.0, h);
        }
        StoreIcon::System => {
            fill_ring(&mut r, 7.0, 7.0, 3.6, 1.5);
            fill_disc(&mut r, 7.0, 7.0, 1.2);
        }
        StoreIcon::Updates | StoreIcon::Refresh => {
            fill_arc(&mut r, 7.0, 7.0, 4.2, -2.5, 0.4, h);
            fill_arc(&mut r, 7.0, 7.0, 4.2, 0.7, 3.5, h);
            stroke_line(&mut r, 9.8, 3.2, 11.0, 5.2, h);
            stroke_line(&mut r, 8.8, 5.6, 11.0, 5.2, h);
        }
        StoreIcon::Repos => {
            stroke_line(&mut r, 2.5, 3.6, 11.5, 3.6, h);
            stroke_line(&mut r, 2.5, 7.0, 11.5, 7.0, h);
            stroke_line(&mut r, 2.5, 10.4, 11.5, 10.4, h);
        }
        StoreIcon::Shield => {
            stroke_line(&mut r, 7.0, 2.0, 11.4, 3.6, h);
            stroke_line(&mut r, 11.4, 3.6, 10.6, 9.0, h);
            stroke_line(&mut r, 10.6, 9.0, 7.0, 12.2, h);
            stroke_line(&mut r, 7.0, 12.2, 3.4, 9.0, h);
            stroke_line(&mut r, 3.4, 9.0, 2.6, 3.6, h);
            stroke_line(&mut r, 2.6, 3.6, 7.0, 2.0, h);
            stroke_line(&mut r, 4.8, 7.2, 6.4, 8.8, 0.75);
            stroke_line(&mut r, 6.4, 8.8, 9.4, 5.4, 0.75);
        }
        StoreIcon::Back => {
            stroke_line(&mut r, 9.5, 2.5, 4.5, 7.0, 0.85);
            stroke_line(&mut r, 4.5, 7.0, 9.5, 11.5, 0.85);
        }
    }
    r.finish(mask);
    for ry in 0..side {
        let py = y + ry as i32;
        if py < 0 || py >= bh as i32 {
            continue;
        }
        for rx in 0..side {
            let px = x + rx as i32;
            if px < 0 || px >= bw as i32 {
                continue;
            }
            let a = mask[ry * side + rx];
            if a == 0 {
                continue;
            }
            let idx = py as usize * bw + px as usize;
            buf[idx] = mix_rgb(buf[idx], color, a);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_sidebar_badge(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    color: u32,
    icon: StoreIcon,
) {
    let top = mix_rgb(color, 0x00FF_FFFF, 28);
    fill_rounded_rect(buf, w, h, x, y, 20, 20, 5, top);
    fill_rounded_rect(buf, w, h, x, y + 1, 20, 19, 5, color);
    let r = ((color >> 16) & 0xFF) * 299;
    let gn = ((color >> 8) & 0xFF) * 587;
    let b = (color & 0xFF) * 114;
    let ink = if r + gn + b >= 165_000 { 0x000B_0D12 } else { 0x00FF_FFFF };
    draw_icon(buf, w, h, g, x + 3, y + 3, icon, ink);
}

/// Deterministic rich gradient monogram app icon tile for packages without raster icons.
#[allow(clippy::too_many_arguments)]
pub fn draw_monogram_tile(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    side: i32,
    name: &str,
    category: &str,
    pal: &CandyPalette,
) {
    let base = match category {
        "games" | "creative" | "art" | "media" => NOIR_PURPLE,
        "tools" | "productivity" | "dev" | "office" => 0x003B_82F6,
        "system" | "network" | "security" => NOIR_MINT,
        _ => {
            const SWATCHES: [u32; 5] = [NOIR_ROSE, 0x003B_82F6, NOIR_PURPLE, NOIR_MINT, NOIR_AMBER];
            let hval = name.bytes().fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
            SWATCHES[hval % SWATCHES.len()]
        }
    };
    let radius = (side / 4).clamp(8, 16);
    draw_candy_badge(buf, w, h, x, y, side, side, mix_rgb(base, 0x0008_0A10, 42), pal);
    fill_rounded_rect(buf, w, h, x + 1, y + 1, side - 2, side - 2, radius - 1, base);
    fill_rounded_rect(
        buf,
        w,
        h,
        x + 2,
        y + 2,
        side - 4,
        (side / 2).max(4),
        (radius - 2).max(2),
        mix_rgb(base, 0x00FF_FFFF, 28),
    );
    let mut mono = [b'A'; 2];
    let mut n = 0;
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() {
            mono[n] = b.to_ascii_uppercase();
            n += 1;
            if n == 2 {
                break;
            }
        }
    }
    let label = if n == 0 { "PX" } else { core::str::from_utf8(&mono[..n]).unwrap_or("PX") };
    let px_h = (side / 2).clamp(13, 24);
    let tw = g.measure(1, px_h, label);
    draw_sans_bold(
        buf,
        w,
        h,
        g,
        x + (side - tw) / 2,
        y + (side - px_h) / 2 - 1,
        side - 4,
        px_h,
        label,
        0x00FF_FFFF,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noir_store_palette_icons_and_cards_render_cleanly() {
        let pal = noir_palette();
        assert_eq!(pal.panel, NOIR_SIDEBAR);
        assert_eq!(pal.bg, NOIR_CANVAS);
        assert_eq!(pal.card, NOIR_CARD);
        assert_eq!(pal.specular, NOIR_TOP_SPECULAR);
        assert_eq!(pal.accent, NOIR_ROSE);

        let mut g = Glyphs::new();
        let mut buf = vec![NOIR_CANVAS; 420 * 240];
        draw_noir_card(&mut buf, 420, 240, 16, 16, 380, 140, 10, &pal, None);
        draw_monogram_tile(&mut buf, 420, 240, &mut g, 28, 30, 48, "chello", "tools", &pal);
        draw_pill_button(&mut buf, 420, 240, &mut g, 300, 120, 80, 24, "GET", ButtonStyle::Primary, ButtonState::Rest, &pal);
        for (idx, icon) in [
            StoreIcon::Search,
            StoreIcon::Discover,
            StoreIcon::AllApps,
            StoreIcon::Creative,
            StoreIcon::Productivity,
            StoreIcon::System,
            StoreIcon::Updates,
            StoreIcon::Repos,
            StoreIcon::Shield,
            StoreIcon::Back,
            StoreIcon::Refresh,
        ]
        .iter()
        .enumerate()
        {
            draw_sidebar_badge(&mut buf, 420, 240, &mut g, 20 + idx as i32 * 26, 180, NOIR_ROSE, *icon);
        }
        assert_ne!(buf[190 * 420 + 30], NOIR_CANVAS);
    }
}
