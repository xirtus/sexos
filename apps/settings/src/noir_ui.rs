// SPDX-License-Identifier: GPL-3.0-only
//! macOS Ventura/Sequoia Dark-Mode Noir rendering primitives, Geist Sans font
//! renderer, and vector icon rasterizer for Tatami Settings (`apps/settings`).
//! Zero heap allocation (`#![no_std]`).

use sex_font::cache::{GlyphCache, Key as GlyphKey};
use sex_font::gamma::{blend_with, contrast_for};
use sex_font::raster::{scratch_len, Raster, FX};
use sex_font::{Font, Render, Synthetic};
use sex_theme::candy_ui::{fill_rounded_rect, mix_rgb, rgb, ButtonState, CandyPalette};
use sex_theme::Mode;

/// macOS Ventura/Sequoia Dark-Mode Noir surface tokens.
pub const NOIR_SIDEBAR: u32 = 0x000B_0D12;
pub const NOIR_CANVAS: u32 = 0x0011_141D;
pub const NOIR_CARD: u32 = 0x0018_1C28;
pub const NOIR_TOP_SPECULAR: u32 = 0x0026_2D3F;
pub const NOIR_RIM: u32 = 0x001E_2434;
pub const NOIR_WELL: u32 = 0x000D_1017;
pub const NOIR_CONTROL: u32 = 0x0022_2839;
pub const NOIR_CONTROL_HOVER: u32 = 0x002B_3348;
pub const NOIR_CONTROL_PRESSED: u32 = 0x001A_1F2E;
pub const NOIR_HAIRLINE: u32 = 0x001E_2434;
pub const NOIR_TEXT: u32 = 0x00F1_F5F9;
pub const NOIR_TEXT_DIM: u32 = 0x0094_A3B8;
pub const NOIR_TEXT_DISABLED: u32 = 0x0064_748B;

/// The 4 flagship Noir Accent Tints.
pub const ACCENT_TINTS: [(&str, i32); 4] = [
    ("SexOS Rose", 0x00F4_3F5E),
    ("Graphite", 0x0064_748B),
    ("Noir Lavender", 0x00A7_8BFA),
    ("Ice Cyan", 0x0038_BDF8),
];

/// Builds a macOS Noir `CandyPalette` (for both Dark and Light setting values, Noir
/// keeps the signature `#0B0D12`/`#11141D`/`#181C28` surfaces while respecting High
/// Contrast accessibility mode and live accent tint).
pub fn noir_palette(mode: Mode, accent_raw: i32) -> CandyPalette {
    if mode == Mode::HighContrast {
        return CandyPalette::high_contrast();
    }
    let raw = (accent_raw as u32) & 0x00FF_FFFF;
    let accent = if raw == 0 || raw == 0x00D6_336C {
        0x00F4_3F5E
    } else {
        raw
    };
    CandyPalette {
        mode: Mode::Dark,
        bg: NOIR_CANVAS,
        panel: NOIR_SIDEBAR,
        toolbar_top: 0x0012_151F,
        toolbar_bottom: 0x0011_141D,
        card: NOIR_CARD,
        well: NOIR_WELL,
        control: NOIR_CONTROL,
        control_hover: NOIR_CONTROL_HOVER,
        control_pressed: NOIR_CONTROL_PRESSED,
        accent,
        accent_hover: mix_rgb(accent, 0x00FF_FFFF, 32),
        accent_pressed: mix_rgb(accent, 0x0008_0A10, 44),
        danger: 0x00EF_4444,
        danger_hover: 0x00F8_7171,
        danger_pressed: 0x00DC_2626,
        selection: mix_rgb(NOIR_CARD, accent, 54),
        hairline: NOIR_HAIRLINE,
        rim: NOIR_RIM,
        rim_accent: mix_rgb(accent, 0x00FF_FFFF, 72),
        specular: NOIR_TOP_SPECULAR,
        text: NOIR_TEXT,
        text_dim: NOIR_TEXT_DIM,
        text_disabled: NOIR_TEXT_DISABLED,
        text_on_accent: if contrast_on_dark(accent) { 0x000B_0D12 } else { 0x00FF_FFFF },
        mint: 0x0010_B981,
        honey: 0x00F5_9E0B,
        sky: 0x0038_BDF8,
    }
}

const fn contrast_on_dark(rgb_col: u32) -> bool {
    let r = ((rgb_col >> 16) & 0xFF) * 299;
    let g = ((rgb_col >> 8) & 0xFF) * 587;
    let b = (rgb_col & 0xFF) * 114;
    (r + g + b) >= 165_000
}

const MAX_GLYPH_W: usize = 32;
const MAX_GLYPH_H: usize = 32;
const GLYPH_SLOTS: usize = 192;
const GLYPH_ARENA: usize = 192 * 18 * 20;

/// Zero-heap proportional Geist Sans + Geist Mono glyph cache.
pub struct Glyphs {
    sans_reg: Option<Font<'static>>,
    sans_bold: Option<Font<'static>>,
    mono: Option<Font<'static>>,
    loaded: bool,
    cache: GlyphCache<GLYPH_SLOTS, GLYPH_ARENA>,
    acc: [i32; scratch_len(MAX_GLYPH_W, MAX_GLYPH_H)],
    cover: [u8; MAX_GLYPH_W * MAX_GLYPH_H],
    icon_acc: [i32; scratch_len(20, 20)],
    icon_mask: [u8; 20 * 20],
}

impl Glyphs {
    pub const fn new() -> Self {
        Self {
            sans_reg: None,
            sans_bold: None,
            mono: None,
            loaded: false,
            cache: GlyphCache::new(),
            acc: [0; scratch_len(MAX_GLYPH_W, MAX_GLYPH_H)],
            cover: [0; MAX_GLYPH_W * MAX_GLYPH_H],
            icon_acc: [0; scratch_len(20, 20)],
            icon_mask: [0; 20 * 20],
        }
    }

    fn ensure_fonts(&mut self) {
        if !self.loaded {
            self.sans_reg = Font::new(sex_font::GEIST_SANS);
            self.sans_bold = Font::new(sex_font::GEIST_SANS_BOLD);
            self.mono = Font::new(sex_font::GEIST_MONO);
            self.loaded = true;
        }
    }

    fn font_ref(&mut self, kind: u8) -> Option<&Font<'static>> {
        self.ensure_fonts();
        match kind {
            1 => self.sans_bold.as_ref().or(self.sans_reg.as_ref()),
            2 => self.mono.as_ref().or(self.sans_reg.as_ref()),
            _ => self.sans_reg.as_ref(),
        }
    }

    /// Measures pixel width of UTF-8/ASCII bytes in Geist Sans/Mono at `px_h`.
    pub fn measure(&mut self, kind: u8, px_h: i32, text: &[u8]) -> i32 {
        let s = core::str::from_utf8(text).unwrap_or("");
        let Some(font) = self.font_ref(kind) else {
            return (text.len() as i32) * (px_h * 6 / 10);
        };
        let size = (px_h.max(9) as u32) * 64;
        let mut pen = 0i32;
        let mut prev = None;
        for ch in s.chars() {
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

    /// Draws crisp proportional Geist Sans/Mono text at `(x, y)` top-left, clipped to `max_w`.
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
        text: &[u8],
        color: u32,
    ) {
        if max_w <= 4 || text.is_empty() || buf.len() < bw.saturating_mul(bh) {
            return;
        }
        self.ensure_fonts();
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

        let s = core::str::from_utf8(text).unwrap_or("");
        let mut pen = x * FX;
        let mut prev = None;

        for ch in s.chars() {
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

pub fn draw_sans(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    max_w: i32,
    text: &[u8],
    color: u32,
) {
    g.draw_text(buf, w, h, x, y, max_w, 13, 0, text, color);
}

pub fn draw_sans_bold(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    max_w: i32,
    px_h: i32,
    text: &[u8],
    color: u32,
) {
    g.draw_text(buf, w, h, x, y, max_w, px_h, 1, text, color);
}

pub fn draw_sans_sm(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    max_w: i32,
    text: &[u8],
    color: u32,
) {
    g.draw_text(buf, w, h, x, y, max_w, 11, 0, text, color);
}

pub fn draw_mono(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    max_w: i32,
    text: &[u8],
    color: u32,
) {
    g.draw_text(buf, w, h, x, y, max_w, 12, 2, text, color);
}

pub fn fill_solid(buf: &mut [u32], w: usize, h: usize, x: i32, y: i32, rw: i32, rh: i32, color: u32) {
    let x0 = x.max(0) as usize;
    let y0 = y.max(0) as usize;
    let x1 = x.saturating_add(rw).clamp(0, w as i32) as usize;
    let y1 = y.saturating_add(rh).clamp(0, h as i32) as usize;
    if x0 >= x1 || y0 >= y1 || buf.len() < w.saturating_mul(h) {
        return;
    }
    let col = rgb(color);
    for row in y0..y1 {
        buf[row * w + x0..row * w + x1].fill(col);
    }
}

/// Draws a macOS Ventura/Sequoia grouped rounded card (`#181C28`) with subtle `#1E2434`
/// perimeter border and a crisp `1px` `#262D3F` top specular highlight edge.
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_card(
    buf: &mut [u32],
    w: usize,
    h: usize,
    x: i32,
    y: i32,
    cw: i32,
    ch: i32,
    radius: i32,
    pal: &CandyPalette,
) {
    if cw <= 2 || ch <= 2 {
        return;
    }
    let r = radius.clamp(2, cw.min(ch) / 2);
    fill_rounded_rect(buf, w, h, x, y, cw, ch, r, pal.rim);
    fill_rounded_rect(buf, w, h, x + 1, y + 1, cw - 2, ch - 2, (r - 1).max(1), pal.card);
    // Crisp 1px top specular border (`#262D3F` in Noir Dark).
    let spec_x0 = x + r.min(cw / 2);
    let spec_w = (cw - 2 * r.min(cw / 2)).max(0);
    if spec_w > 0 {
        fill_solid(buf, w, h, spec_x0, y, spec_w, 1, pal.specular);
    }
}

/// Draws a sunken Noir input / slider track / segmented bar well with 1px border.
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_well(
    buf: &mut [u32],
    w: usize,
    h: usize,
    x: i32,
    y: i32,
    cw: i32,
    ch: i32,
    radius: i32,
    border_col: u32,
    pal: &CandyPalette,
) {
    if cw <= 2 || ch <= 2 {
        return;
    }
    let r = radius.clamp(2, cw.min(ch) / 2);
    fill_rounded_rect(buf, w, h, x, y, cw, ch, r, border_col);
    fill_rounded_rect(buf, w, h, x + 1, y + 1, cw - 2, ch - 2, (r - 1).max(1), pal.well);
}

/// Draws a smooth macOS pill toggle switch (accent-filled when `on`, crisp white thumb).
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_toggle(
    buf: &mut [u32],
    w: usize,
    h: usize,
    x: i32,
    y: i32,
    tw: i32,
    th: i32,
    on: bool,
    state: ButtonState,
    pal: &CandyPalette,
) {
    let r = (th / 2).max(2);
    let track = if on {
        match state {
            ButtonState::Rest => pal.accent,
            ButtonState::Hover => pal.accent_hover,
            ButtonState::Pressed => pal.accent_pressed,
        }
    } else {
        match state {
            ButtonState::Rest => pal.control,
            ButtonState::Hover => pal.control_hover,
            ButtonState::Pressed => pal.control_pressed,
        }
    };
    let rim = if on { mix_rgb(track, pal.specular, 72) } else { pal.specular };
    fill_rounded_rect(buf, w, h, x, y, tw, th, r, rim);
    fill_rounded_rect(buf, w, h, x + 1, y + 1, tw - 2, th - 2, (r - 1).max(1), track);

    let pad = 3;
    let thumb = (th - pad * 2).max(6);
    let tx = if on { x + tw - pad - thumb } else { x + pad };
    let ty = y + (th - thumb) / 2;
    let thumb_col = if pal.mode == Mode::HighContrast && on {
        0x0000_0000
    } else {
        0x00FF_FFFF
    };
    fill_rounded_rect(buf, w, h, tx, ty, thumb, thumb, thumb / 2, thumb_col);
}

/// Draws a horizontal macOS Noir interactive slider (`val` in `min..=max`).
#[allow(clippy::too_many_arguments)]
pub fn draw_noir_slider(
    buf: &mut [u32],
    w: usize,
    h: usize,
    x: i32,
    y: i32,
    sw: i32,
    val: i32,
    min: i32,
    max: i32,
    hovered: bool,
    pal: &CandyPalette,
) {
    if sw < 24 {
        return;
    }
    let track_h = 6;
    let track_y = y + 8;
    fill_rounded_rect(buf, w, h, x, track_y, sw, track_h, 3, pal.well);
    fill_rounded_rect(buf, w, h, x, track_y, sw, 1, 0, pal.rim);

    let span = (max - min).max(1) as i64;
    let clamped = val.clamp(min, max) as i64 - min as i64;
    let knob = 16;
    let usable = (sw - knob).max(1) as i64;
    let fill_w = ((clamped * usable) / span) as i32 + knob / 2;
    fill_rounded_rect(buf, w, h, x, track_y, fill_w.clamp(6, sw), track_h, 3, pal.accent);

    let kx = x + ((clamped * usable) / span) as i32;
    let ky = y + 3;
    let halo = if hovered { pal.rim_accent } else { pal.specular };
    fill_rounded_rect(buf, w, h, kx - 1, ky - 1, knob + 2, knob + 2, (knob + 2) / 2, halo);
    fill_rounded_rect(buf, w, h, kx, ky, knob, knob, knob / 2, 0x00FF_FFFF);
}

/// Vector icon identifiers for sidebar color badges and UI pills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Search,
    Appearance,
    Network,
    Sound,
    Display,
    Trackpad,
    Apps,
    Keyboard,
    Privacy,
    Storage,
    Update,
    About,
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

fn fill_arc(r: &mut Raster<'_>, cx: f32, cy: f32, rad: f32, a_start: f32, a_end: f32, half: f32) {
    const N: usize = 8;
    for i in 0..N {
        let t0 = a_start + (a_end - a_start) * (i as f32 / N as f32);
        let t1 = a_start + (a_end - a_start) * ((i + 1) as f32 / N as f32);
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
    const N: usize = 14;
    const STEP: f32 = core::f32::consts::TAU / N as f32;
    for i in 0..N {
        let a0 = i as f32 * STEP;
        let a1 = (i + 1) as f32 * STEP;
        let p0 = (
            ((cx + rad * fast_cosf(a0)) * FX as f32 + 0.5) as i32,
            ((cy + rad * fast_sinf(a0)) * FX as f32 + 0.5) as i32,
        );
        let p1 = (
            ((cx + rad * fast_cosf(a1)) * FX as f32 + 0.5) as i32,
            ((cy + rad * fast_sinf(a1)) * FX as f32 + 0.5) as i32,
        );
        r.line(p0.0, p0.1, p1.0, p1.1);
    }
}

/// Draws a crisp 14x14 anti-aliased vector icon at `(x, y)` in `color`.
pub fn draw_icon(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    icon: Icon,
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
        Icon::Search => {
            fill_ring(&mut r, 5.8, 5.8, 3.6, 1.45);
            stroke_line(&mut r, 8.4, 8.4, 12.2, 12.2, 0.85);
        }
        Icon::Appearance => {
            stroke_line(&mut r, 2.2, 4.2, 11.8, 4.2, h);
            stroke_line(&mut r, 2.2, 9.8, 11.8, 9.8, h);
            fill_disc(&mut r, 5.0, 4.2, 1.8);
            fill_disc(&mut r, 9.2, 9.8, 1.8);
        }
        Icon::Network => {
            fill_arc(&mut r, 7.0, 11.4, 7.8, -2.35, -0.79, h);
            fill_arc(&mut r, 7.0, 11.4, 4.8, -2.35, -0.79, h);
            fill_disc(&mut r, 7.0, 11.2, 1.3);
        }
        Icon::Sound => {
            stroke_line(&mut r, 2.4, 5.4, 4.8, 5.4, h);
            stroke_line(&mut r, 4.8, 5.4, 7.6, 3.0, h);
            stroke_line(&mut r, 7.6, 3.0, 7.6, 11.0, h);
            stroke_line(&mut r, 7.6, 11.0, 4.8, 8.6, h);
            stroke_line(&mut r, 4.8, 8.6, 2.4, 8.6, h);
            stroke_line(&mut r, 2.4, 8.6, 2.4, 5.4, h);
            fill_arc(&mut r, 7.6, 7.0, 3.2, -0.8, 0.8, h * 0.9);
            fill_arc(&mut r, 7.6, 7.0, 5.0, -0.8, 0.8, h * 0.9);
        }
        Icon::Display => {
            stroke_line(&mut r, 2.0, 3.0, 12.0, 3.0, h);
            stroke_line(&mut r, 12.0, 3.0, 12.0, 9.6, h);
            stroke_line(&mut r, 12.0, 9.6, 2.0, 9.6, h);
            stroke_line(&mut r, 2.0, 9.6, 2.0, 3.0, h);
            stroke_line(&mut r, 5.0, 11.6, 9.0, 11.6, h);
        }
        Icon::Trackpad => {
            stroke_line(&mut r, 2.2, 3.0, 11.8, 3.0, h);
            stroke_line(&mut r, 11.8, 3.0, 11.8, 11.0, h);
            stroke_line(&mut r, 11.8, 11.0, 2.2, 11.0, h);
            stroke_line(&mut r, 2.2, 11.0, 2.2, 3.0, h);
            stroke_line(&mut r, 2.2, 8.4, 11.8, 8.4, 0.55);
            stroke_line(&mut r, 7.0, 8.4, 7.0, 11.0, 0.55);
        }
        Icon::Apps => {
            for (gx, gy) in [(3.2, 3.2), (8.2, 3.2), (3.2, 8.2), (8.2, 8.2)] {
                stroke_line(&mut r, gx, gy, gx + 2.6, gy, 0.85);
                stroke_line(&mut r, gx, gy + 2.2, gx + 2.6, gy + 2.2, 0.85);
            }
        }
        Icon::Keyboard => {
            stroke_line(&mut r, 1.8, 3.6, 12.2, 3.6, h);
            stroke_line(&mut r, 12.2, 3.6, 12.2, 10.4, h);
            stroke_line(&mut r, 12.2, 10.4, 1.8, 10.4, h);
            stroke_line(&mut r, 1.8, 10.4, 1.8, 3.6, h);
            stroke_line(&mut r, 4.2, 8.4, 9.8, 8.4, 0.65);
            fill_disc(&mut r, 4.2, 6.0, 0.75);
            fill_disc(&mut r, 7.0, 6.0, 0.75);
            fill_disc(&mut r, 9.8, 6.0, 0.75);
        }
        Icon::Privacy => {
            stroke_line(&mut r, 3.4, 6.4, 10.6, 6.4, h);
            stroke_line(&mut r, 10.6, 6.4, 10.6, 11.6, h);
            stroke_line(&mut r, 10.6, 11.6, 3.4, 11.6, h);
            stroke_line(&mut r, 3.4, 11.6, 3.4, 6.4, h);
            fill_arc(&mut r, 7.0, 6.2, 2.5, -3.14, 0.0, h);
        }
        Icon::Storage => {
            stroke_line(&mut r, 2.4, 3.8, 11.6, 3.8, h);
            stroke_line(&mut r, 11.6, 3.8, 11.6, 10.2, h);
            stroke_line(&mut r, 11.6, 10.2, 2.4, 10.2, h);
            stroke_line(&mut r, 2.4, 10.2, 2.4, 3.8, h);
            fill_disc(&mut r, 9.4, 7.8, 1.0);
        }
        Icon::Update => {
            fill_arc(&mut r, 7.0, 7.0, 4.2, -2.5, 0.4, h);
            fill_arc(&mut r, 7.0, 7.0, 4.2, 0.7, 3.5, h);
            stroke_line(&mut r, 9.8, 3.2, 11.0, 5.2, h);
            stroke_line(&mut r, 8.8, 5.6, 11.0, 5.2, h);
        }
        Icon::About => {
            fill_ring(&mut r, 7.0, 7.0, 5.0, 1.35);
            fill_disc(&mut r, 7.0, 4.5, 0.85);
            stroke_line(&mut r, 7.0, 6.6, 7.0, 10.0, 0.85);
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

/// Draws a vibrant macOS System Settings 20x20 rounded-square icon badge.
#[allow(clippy::too_many_arguments)]
pub fn draw_sidebar_badge(
    buf: &mut [u32],
    w: usize,
    h: usize,
    g: &mut Glyphs,
    x: i32,
    y: i32,
    badge_color: u32,
    icon: Icon,
) {
    let top = mix_rgb(badge_color, 0x00FF_FFFF, 28);
    fill_rounded_rect(buf, w, h, x, y, 20, 20, 5, top);
    fill_rounded_rect(buf, w, h, x, y + 1, 20, 19, 5, badge_color);
    let ink = if contrast_on_dark(badge_color) { 0x000B_0D12 } else { 0x00FF_FFFF };
    draw_icon(buf, w, h, g, x + 3, y + 3, icon, ink);
}

/// Renders the SexOS Rose-Gold Lips emblem centered at `(cx, cy)` with target `width`.
pub fn draw_sexos_lips(
    buf: &mut [u32],
    bw: usize,
    bh: usize,
    cx: i32,
    cy: i32,
    width: u32,
    bright: bool,
) {
    let width = width.min(256);
    if width == 0 {
        return;
    }
    let h = sex_pdx::brand::height_for(width) as i32;
    let x0 = cx - (width as i32) / 2;
    let y0 = cy - h / 2;
    let tone = if bright {
        sex_pdx::brand::Tone::Bright
    } else {
        sex_pdx::brand::Tone::Rich
    };
    let mut row = [0u8; 256];
    for dy in 0..h {
        let py = y0 + dy;
        if py < 0 || (py as usize) >= bh {
            continue;
        }
        sex_pdx::brand::coverage_row(width, dy as u32, &mut row[..width as usize]);
        let row_off = py as usize * bw;
        for dx in 0..width as i32 {
            let cov = row[dx as usize];
            if cov == 0 {
                continue;
            }
            let px = x0 + dx;
            if px < 0 || (px as usize) >= bw {
                continue;
            }
            let fg = sex_pdx::brand::color_at(tone, dx as u32, width);
            let idx = row_off + px as usize;
            buf[idx] = sex_pdx::brand::blend(fg, buf[idx], cov) & 0x00FF_FFFF;
        }
    }
}
