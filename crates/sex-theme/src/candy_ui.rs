// SPDX-License-Identifier: GPL-3.0-only
//! Shared Candy Lavender UI widget & surface drawing helpers for pixel-buffer apps.
//!
//! Pure integer `#![no_std]` routines (no heap, no floats) operating on `0x00RRGGBB`
//! row-major pixel buffers so every native SexOS app (Mesh, Flash, SexShop, Calendar,
//! Gossip, etc.) shares the exact same smooth anti-aliased rounded Candy Lavender
//! chrome as SilkBar and SexDisplay.

use crate::{noir as nr, Mode, NoirAccent};

/// Strip alpha channel to `0x00RRGGBB` used by native pixel-buffer windows.
#[inline]
pub const fn rgb(c: u32) -> u32 {
    c & 0x00FF_FFFF
}

/// Pure integer RGB blend: `a` toward `b` by `alpha` / 255 (`0x00RRGGBB`).
#[inline]
pub const fn mix_rgb(a: u32, b: u32, alpha: u8) -> u32 {
    let t = alpha as u32;
    let inv = 255 - t;
    let r = (((a >> 16) & 0xFF) * inv + ((b >> 16) & 0xFF) * t + 127) / 255;
    let g = (((a >> 8) & 0xFF) * inv + ((b >> 8) & 0xFF) * t + 127) / 255;
    let bl = ((a & 0xFF) * inv + (b & 0xFF) * t + 127) / 255;
    (r << 16) | (g << 8) | bl
}

/// Complete Candy Lavender color palette for native app surfaces and controls (`0x00RRGGBB`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CandyPalette {
    pub mode: Mode,
    /// Main window canvas background.
    pub bg: u32,
    /// Elevated sidebar / panel background.
    pub panel: u32,
    /// Frosted toolbar gradient top and bottom.
    pub toolbar_top: u32,
    pub toolbar_bottom: u32,
    /// Raised card / chart / tile surface.
    pub card: u32,
    /// Sunken input / track / well surface.
    pub well: u32,
    /// Default frosted candy pill control fill (rest / hover / pressed).
    pub control: u32,
    pub control_hover: u32,
    pub control_pressed: u32,
    /// Primary / active orchid accent fill (rest / hover / pressed).
    pub accent: u32,
    pub accent_hover: u32,
    pub accent_pressed: u32,
    /// Secondary candy rose/berry danger fill (rest / hover / pressed).
    pub danger: u32,
    pub danger_hover: u32,
    pub danger_pressed: u32,
    /// Soft orchid glow for active selections and highlights.
    pub selection: u32,
    /// Subtle orchid hairline divider and pill border rim.
    pub hairline: u32,
    pub rim: u32,
    pub rim_accent: u32,
    /// Top specular highlight sheen.
    pub specular: u32,
    /// Crisp foreground ink, muted ink, disabled ink, and ink on accent buttons.
    pub text: u32,
    pub text_dim: u32,
    pub text_disabled: u32,
    pub text_on_accent: u32,
    /// Semantic candy tones (good/mint, warn/honey, info/sky).
    pub mint: u32,
    pub honey: u32,
    pub sky: u32,
}

impl CandyPalette {
    /// macOS Noir Obsidian Glass palette (default for dark native apps, SexOS Crimson accent).
    pub const fn dark() -> Self {
        Self::noir_tint(NoirAccent::Crimson)
    }

    /// Alias for [`Self::dark`]: deep layered macOS Noir Obsidian Glass.
    pub const fn noir() -> Self {
        Self::dark()
    }

    /// macOS Noir Obsidian Glass palette with one of the four curated [`NoirAccent`] tints.
    pub const fn noir_tint(tint: NoirAccent) -> Self {
        let accent = rgb(tint.fill());
        let primary = rgb(tint.primary());
        let glow = rgb(tint.glow());
        let on_accent = rgb(tint.on_accent());
        let bar_top = rgb(nr::DARK_BAR_TOP);
        let bar_bot = rgb(nr::DARK_BAR_BOTTOM);
        let pill = rgb(nr::DARK_PILL_FILL);
        let rim = rgb(nr::DARK_RIM);
        let spec = rgb(nr::DARK_SPECULAR);
        let hair = rgb(nr::DARK_HAIRLINE);
        let card = rgb(nr::DARK_SURFACE);
        let danger = 0x00E1_1D48;
        Self {
            mode: Mode::Dark,
            bg: rgb(nr::DARK_BASE),
            panel: rgb(nr::DARK_PANEL),
            toolbar_top: bar_top,
            toolbar_bottom: bar_bot,
            card,
            well: rgb(nr::DARK_WELL),
            control: pill,
            control_hover: rgb(nr::DARK_HOVER),
            control_pressed: mix_rgb(pill, rgb(nr::DARK_BASE), 72),
            accent,
            accent_hover: mix_rgb(primary, 0x00FF_FFFF, 24),
            accent_pressed: mix_rgb(accent, rgb(nr::DARK_BASE), 56),
            danger,
            danger_hover: 0x00F4_3F5E,
            danger_pressed: 0x009F_1239,
            selection: mix_rgb(card, primary, 48),
            hairline: hair,
            rim,
            rim_accent: glow,
            specular: spec,
            text: rgb(nr::DARK_TEXT),
            text_dim: rgb(nr::DARK_SUBTEXT),
            text_disabled: rgb(nr::DARK_DISABLED),
            text_on_accent: on_accent,
            mint: 0x0034_D399,
            honey: 0x00FB_BF24,
            sky: 0x0038_BDF8,
        }
    }

    /// Returns a copy of `self` re-tinted with `tint` (preserving HighContrast accessibility).
    pub const fn with_noir_accent(mut self, tint: NoirAccent) -> Self {
        if matches!(self.mode, Mode::HighContrast) {
            return self;
        }
        if matches!(self.mode, Mode::Dark) {
            return Self::noir_tint(tint);
        }
        let accent = rgb(tint.fill());
        let primary = rgb(tint.primary());
        self.accent = accent;
        self.accent_hover = mix_rgb(primary, 0x00FF_FFFF, 24);
        self.accent_pressed = mix_rgb(accent, 0x0011_1318, 44);
        self.rim_accent = primary;
        self.selection = mix_rgb(self.card, primary, 38);
        self.text_on_accent = rgb(tint.on_accent());
        self
    }

    /// Returns a copy of `self` re-tinted to an arbitrary `0x00RRGGBB` accent color,
    /// picking dark or light `text_on_accent` automatically for readable contrast.
    pub const fn with_accent_rgb(mut self, color: u32) -> Self {
        if matches!(self.mode, Mode::HighContrast) {
            return self;
        }
        let accent = rgb(color);
        let r = (accent >> 16) & 0xFF;
        let g = (accent >> 8) & 0xFF;
        let b = accent & 0xFF;
        // Fast perceived brightness threshold aligned with WCAG 4.5:1 midpoint.
        let luma = (r * 2126 + g * 7152 + b * 722) / 10_000;
        self.accent = accent;
        self.accent_hover = mix_rgb(accent, 0x00FF_FFFF, 32);
        self.accent_pressed = mix_rgb(accent, 0x000B_0D12, 48);
        self.rim_accent = mix_rgb(accent, self.specular, 72);
        self.selection = mix_rgb(self.card, accent, if matches!(self.mode, Mode::Light) { 40 } else { 52 });
        self.text_on_accent = if luma >= 145 { 0x000B_0D12 } else { 0x00FF_FFFF };
        self
    }

    /// Airy frosted pearl palette for Light mode.
    pub const fn light() -> Self {
        let accent = 0x00E1_1D48;
        let bar_top = rgb(nr::LIGHT_BAR_TOP);
        let bar_bot = rgb(nr::LIGHT_BAR_BOTTOM);
        let pill = rgb(nr::LIGHT_PILL_FILL);
        let rim = rgb(nr::LIGHT_MENU_RIM);
        let spec = rgb(nr::LIGHT_SPECULAR);
        let hair = rgb(nr::LIGHT_MENU_DIVIDER);
        let danger = 0x00D6_336C;
        Self {
            mode: Mode::Light,
            bg: 0x00F5_F6FA,
            panel: 0x00ED_EFF5,
            toolbar_top: bar_top,
            toolbar_bottom: bar_bot,
            card: 0x00FF_FFFF,
            well: 0x00E6_E9F2,
            control: pill,
            control_hover: mix_rgb(pill, accent, 20),
            control_pressed: mix_rgb(pill, accent, 42),
            accent,
            accent_hover: 0x00F4_3F5E,
            accent_pressed: 0x00BE_123C,
            danger,
            danger_hover: mix_rgb(danger, 0x00FF_FFFF, 28),
            danger_pressed: mix_rgb(danger, 0x001E_162E, 36),
            selection: mix_rgb(0x00FF_FFFF, accent, 36),
            hairline: hair,
            rim,
            rim_accent: accent,
            specular: spec,
            text: rgb(nr::LIGHT_TEXT),
            text_dim: 0x0046_4E61,
            text_disabled: 0x008C_94A8,
            text_on_accent: 0x00FF_FFFF,
            mint: 0x000F_9D6B,
            honey: 0x00C2_7803,
            sky: 0x0025_63EB,
        }
    }

    /// High-contrast Candy palette (WCAG AAA crisp outlines).
    pub const fn high_contrast() -> Self {
        Self {
            mode: Mode::HighContrast,
            bg: 0x0000_0000,
            panel: 0x000A_0A0A,
            toolbar_top: 0x0000_0000,
            toolbar_bottom: 0x0000_0000,
            card: 0x0000_0000,
            well: 0x0000_0000,
            control: 0x0000_0000,
            control_hover: 0x001F_1F1F,
            control_pressed: 0x0033_3333,
            accent: 0x00FF_E600,
            accent_hover: 0x00FF_F066,
            accent_pressed: 0x00CC_B800,
            danger: 0x00FF_6E6E,
            danger_hover: 0x00FF_9999,
            danger_pressed: 0x00CC_4444,
            selection: 0x0026_2200,
            hairline: 0x00FF_FFFF,
            rim: 0x00FF_FFFF,
            rim_accent: 0x00FF_E600,
            specular: 0x00FF_FFFF,
            text: 0x00FF_FFFF,
            text_dim: 0x00E0_E0E0,
            text_disabled: 0x009E_9E9E,
            text_on_accent: 0x0000_0000,
            mint: 0x0000_FF99,
            honey: 0x00FF_E600,
            sky: 0x0000_FFFF,
        }
    }

    /// Returns the Candy palette for a given `Mode`.
    pub const fn for_mode(mode: Mode) -> Self {
        match mode {
            Mode::Light => Self::light(),
            Mode::Dark => Self::dark(),
            Mode::HighContrast => Self::high_contrast(),
        }
    }
}

/// Visual role of a Candy button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Standard frosted lavender pill button.
    Normal,
    /// Primary action button (orchid accent fill).
    Primary,
    /// Active / toggled-on pill (soft orchid glow + accent rim).
    Active,
    /// Destructive / recording / shutter button (candy berry-coral).
    Danger,
    /// Subtle / ghost pill (minimal fill at rest, brightens on hover).
    Subtle,
    /// Non-interactive muted pill.
    Disabled,
}

/// Interactive pointer state of a Candy button or tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ButtonState {
    #[default]
    Rest,
    Hover,
    Pressed,
}

/// Computes `(coverage_u8, on_rim, on_top_edge)` for pixel `(px, py)` inside
/// rounded rectangle `(x, y, w, h)` with corner radius `r`.
#[inline]
pub(crate) fn rounded_rect_sample(
    px: i32,
    py: i32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
) -> (u8, bool, bool) {
    let on_top_edge = py == y;
    let on_box_rim = px == x || px == x + w - 1 || py == y || py == y + h - 1;
    if r <= 0 {
        return (255, on_box_rim, on_top_edge);
    }

    let r2 = r * 2;
    let px2 = (px - x) * 2 + 1;
    let py2 = (py - y) * 2 + 1;
    let cx_l = r2;
    let cx_r = w * 2 - r2;
    let cy_t = r2;
    let cy_b = h * 2 - r2;

    let dx = if px2 < cx_l {
        cx_l - px2
    } else if px2 > cx_r {
        px2 - cx_r
    } else {
        0
    };
    let dy = if py2 < cy_t {
        cy_t - py2
    } else if py2 > cy_b {
        py2 - cy_b
    } else {
        0
    };

    if dx == 0 && dy == 0 {
        return (255, on_box_rim, on_top_edge);
    }

    let d2 = dx * dx + dy * dy;
    let outer = r2 * r2;
    if d2 >= outer {
        return (0, false, false);
    }

    let aa = (r2 - 2).max(0);
    let inner_aa = aa * aa;
    let cov = if d2 <= inner_aa {
        255
    } else {
        let num = (outer - d2) as u32;
        let den = (outer - inner_aa).max(1) as u32;
        ((num * 255) / den).min(255) as u8
    };

    let rim_in = (r2 - 3).max(0);
    let on_rim = on_box_rim || d2 >= rim_in * rim_in;
    let top_rim = on_top_edge || (py2 < cy_t && on_rim && dy >= dx);
    (cov, on_rim, top_rim)
}

#[inline]
pub(crate) fn clip_rect(
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    stride: usize,
    buf_h: usize,
) -> Option<(i32, i32, i32, i32)> {
    if w <= 0 || h <= 0 || stride == 0 || buf_h == 0 {
        return None;
    }
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = x.saturating_add(w).min(stride as i32);
    let y1 = y.saturating_add(h).min(buf_h as i32);
    if x0 >= x1 || y0 >= y1 {
        return None;
    }
    Some((x0, y0, x1, y1))
}

/// Fills an anti-aliased rounded rectangle with a solid `color` (`0x00RRGGBB`).
pub fn fill_rounded_rect(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    color: u32,
) {
    let Some((x0, y0, x1, y1)) = clip_rect(x, y, w, h, stride, buf_h) else {
        return;
    };
    if buf.len() < stride.saturating_mul(buf_h) {
        return;
    }
    let r = radius.clamp(0, w.min(h) / 2);
    let col = rgb(color);
    for py in y0..y1 {
        let row = py as usize * stride;
        for px in x0..x1 {
            let (cov, _, _) = rounded_rect_sample(px, py, x, y, w, h, r);
            if cov == 0 {
                continue;
            }
            let idx = row + px as usize;
            buf[idx] = if cov == 255 { col } else { mix_rgb(buf[idx], col, cov) };
        }
    }
}

/// Draws a frosted Candy Lavender gradient toolbar/header bar with a 1px top specular
/// highlight and a 1px bottom orchid hairline divider.
pub fn draw_candy_toolbar(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    pal: &CandyPalette,
) {
    let Some((x0, y0, x1, y1)) = clip_rect(x, y, w, h, stride, buf_h) else {
        return;
    };
    if buf.len() < stride.saturating_mul(buf_h) {
        return;
    }
    let denom = (h - 1).max(1);
    for py in y0..y1 {
        let t = (((py - y) * 255) / denom).clamp(0, 255) as u8;
        let base = mix_rgb(pal.toolbar_top, pal.toolbar_bottom, t);
        let col = if py == y && pal.mode != Mode::HighContrast {
            mix_rgb(base, pal.specular, if pal.mode == Mode::Light { 150 } else { 42 })
        } else if py == y + h - 1 {
            pal.hairline
        } else {
            base
        };
        let row = py as usize * stride;
        buf[row + x0 as usize..row + x1 as usize].fill(col);
    }
}

/// Draws a smooth anti-aliased rounded Candy card/panel with subtle top sheen and
/// a crisp 1px orchid border rim.
pub fn draw_candy_panel(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    pal: &CandyPalette,
) {
    draw_candy_surface(
        buf,
        stride,
        buf_h,
        x,
        y,
        w,
        h,
        radius,
        pal.card,
        pal.rim,
        pal.specular,
        pal.mode == Mode::HighContrast,
        if pal.mode == Mode::Light { 120 } else { 36 },
        if pal.mode == Mode::Light { 110 } else { 180 },
    );
}

/// Draws a smooth anti-aliased rounded Candy button/pill with subtle vertical top-lit
/// sheen, specular top rim, orchid hairline border, and distinct hover/pressed states.
pub fn draw_candy_button(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    style: ButtonStyle,
    state: ButtonState,
    pal: &CandyPalette,
) {
    let (base, rim_col, spec_a, rim_a, top_boost) = match style {
        ButtonStyle::Primary => {
            let fill = match state {
                ButtonState::Rest => pal.accent,
                ButtonState::Hover => pal.accent_hover,
                ButtonState::Pressed => pal.accent_pressed,
            };
            (fill, pal.rim_accent, 96u8, 120u8, 34u8)
        }
        ButtonStyle::Active => {
            let active_base = mix_rgb(pal.control, pal.accent, 64);
            let fill = match state {
                ButtonState::Rest => active_base,
                ButtonState::Hover => mix_rgb(active_base, pal.specular, 32),
                ButtonState::Pressed => mix_rgb(active_base, pal.bg, 44),
            };
            (fill, pal.rim_accent, 110u8, 140u8, 30u8)
        }
        ButtonStyle::Danger => {
            let fill = match state {
                ButtonState::Rest => pal.danger,
                ButtonState::Hover => pal.danger_hover,
                ButtonState::Pressed => pal.danger_pressed,
            };
            (fill, mix_rgb(pal.danger, pal.specular, 128), 96u8, 110u8, 32u8)
        }
        ButtonStyle::Subtle => {
            let fill = match state {
                ButtonState::Rest => mix_rgb(pal.panel, pal.control, 90),
                ButtonState::Hover => pal.control_hover,
                ButtonState::Pressed => pal.control_pressed,
            };
            let ra = match state {
                ButtonState::Rest => 48u8,
                ButtonState::Hover => 104u8,
                ButtonState::Pressed => 88u8,
            };
            (fill, pal.rim, 56u8, ra, 18u8)
        }
        ButtonStyle::Disabled => {
            let fill = mix_rgb(pal.panel, pal.control, 70);
            (fill, pal.hairline, 24u8, 52u8, 8u8)
        }
        ButtonStyle::Normal => {
            let fill = match state {
                ButtonState::Rest => pal.control,
                ButtonState::Hover => pal.control_hover,
                ButtonState::Pressed => pal.control_pressed,
            };
            let ra = match state {
                ButtonState::Rest => 84u8,
                ButtonState::Hover => 136u8,
                ButtonState::Pressed => 100u8,
            };
            (fill, pal.rim, 80u8, ra, 24u8)
        }
    };

    let Some((x0, y0, x1, y1)) = clip_rect(x, y, w, h, stride, buf_h) else {
        return;
    };
    if buf.len() < stride.saturating_mul(buf_h) {
        return;
    }

    let r = radius.clamp(0, w.min(h) / 2);
    let denom = (h - 1).max(1);
    let hc = pal.mode == Mode::HighContrast;
    let (top_col, bot_col) = if hc {
        (base, base)
    } else if state == ButtonState::Pressed {
        (mix_rgb(base, pal.bg, 26), mix_rgb(base, pal.specular, 12))
    } else {
        (mix_rgb(base, pal.specular, top_boost), base)
    };

    for py in y0..y1 {
        let local_t = (((py - y) * 255) / denom).clamp(0, 255) as u8;
        let fill = mix_rgb(top_col, bot_col, local_t);
        let row = py as usize * stride;
        for px in x0..x1 {
            let (cov, on_rim, on_top) = rounded_rect_sample(px, py, x, y, w, h, r);
            if cov == 0 {
                continue;
            }
            let pixel = if on_rim {
                if hc {
                    rim_col
                } else if on_top && state != ButtonState::Pressed {
                    mix_rgb(fill, pal.specular, spec_a)
                } else {
                    mix_rgb(fill, rim_col, rim_a)
                }
            } else {
                fill
            };
            let idx = row + px as usize;
            buf[idx] = if cov == 255 {
                pixel
            } else {
                mix_rgb(buf[idx], pixel, cov)
            };
        }
    }
}

/// Draws a rounded Candy tab pill with an orchid active glow and bottom accent bar.
pub fn draw_candy_tab(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    active: bool,
    state: ButtonState,
    pal: &CandyPalette,
) {
    let style = if active { ButtonStyle::Active } else { ButtonStyle::Subtle };
    let r = (h / 3).clamp(4, 10);
    draw_candy_button(buf, stride, buf_h, x, y, w, h, r, style, state, pal);
    if active && w >= 16 && h >= 8 {
        let bar_w = (w - 16).max(8);
        let bar_x = x + (w - bar_w) / 2;
        fill_rounded_rect(buf, stride, buf_h, bar_x, y + h - 3, bar_w, 2, 1, pal.accent);
    }
}

/// Draws a rounded Candy selection highlight row with a soft orchid left accent pill.
pub fn draw_candy_selection_row(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    pal: &CandyPalette,
) {
    fill_rounded_rect(buf, stride, buf_h, x, y, w, h, radius, pal.selection);
    if w >= 8 && h >= 6 {
        fill_rounded_rect(buf, stride, buf_h, x + 2, y + 3, 3, h - 6, 2, pal.accent);
    }
}

/// Draws a full-capsule Candy badge pill with a custom accent `color` (`0x00RRGGBB`).
pub fn draw_candy_badge(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: u32,
    pal: &CandyPalette,
) {
    let col = rgb(color);
    let r = (h / 2).max(2);
    let rim = mix_rgb(col, pal.specular, 110);
    draw_candy_surface(
        buf,
        stride,
        buf_h,
        x,
        y,
        w,
        h,
        r,
        col,
        rim,
        pal.specular,
        pal.mode == Mode::HighContrast,
        88,
        96,
    );
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_candy_surface(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    radius: i32,
    fill_col: u32,
    rim_col: u32,
    spec_col: u32,
    high_contrast: bool,
    spec_a: u8,
    rim_a: u8,
) {
    let Some((x0, y0, x1, y1)) = clip_rect(x, y, w, h, stride, buf_h) else {
        return;
    };
    if buf.len() < stride.saturating_mul(buf_h) {
        return;
    }
    let r = radius.clamp(0, w.min(h) / 2);
    let denom = (h - 1).max(1);
    let top_col = if high_contrast {
        fill_col
    } else {
        mix_rgb(fill_col, spec_col, 16)
    };
    for py in y0..y1 {
        let t = (((py - y) * 255) / denom).clamp(0, 255) as u8;
        let base = mix_rgb(top_col, fill_col, t);
        let row = py as usize * stride;
        for px in x0..x1 {
            let (cov, on_rim, on_top) = rounded_rect_sample(px, py, x, y, w, h, r);
            if cov == 0 {
                continue;
            }
            let pixel = if on_rim {
                if high_contrast {
                    rim_col
                } else if on_top {
                    mix_rgb(base, spec_col, spec_a)
                } else {
                    mix_rgb(base, rim_col, rim_a)
                }
            } else {
                base
            };
            let idx = row + px as usize;
            buf[idx] = if cov == 255 {
                pixel
            } else {
                mix_rgb(buf[idx], pixel, cov)
            };
        }
    }
}

/// Draws a smooth rounded Candy toggle switch (pill track + sliding circular thumb).
pub fn draw_candy_toggle(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    on: bool,
    state: ButtonState,
    pal: &CandyPalette,
) {
    let r = (h / 2).max(2);
    let track_style = if on { ButtonStyle::Primary } else { ButtonStyle::Normal };
    draw_candy_button(buf, stride, buf_h, x, y, w, h, r, track_style, state, pal);
    let pad = 3;
    let thumb = (h - pad * 2).max(4);
    let tx = if on { x + w - pad - thumb } else { x + pad };
    let ty = y + (h - thumb) / 2;
    let thumb_col = if on {
        if matches!(pal.mode, Mode::HighContrast) { 0x0000_0000 } else { 0x00FF_FFFF }
    } else {
        pal.text
    };
    fill_rounded_rect(buf, stride, buf_h, tx, ty, thumb, thumb, thumb / 2, thumb_col);
}

/// Draws a smooth rounded Candy color swatch pill with an outer selection halo when `selected`.
pub fn draw_candy_swatch(
    buf: &mut [u32],
    stride: usize,
    buf_h: usize,
    x: i32,
    y: i32,
    size: i32,
    color: u32,
    selected: bool,
    hovered: bool,
    pal: &CandyPalette,
) {
    let r = (size / 2).max(4);
    if selected || hovered {
        let halo = if selected { pal.rim_accent } else { pal.rim };
        fill_rounded_rect(buf, stride, buf_h, x - 2, y - 2, size + 4, size + 4, r + 2, halo);
        fill_rounded_rect(buf, stride, buf_h, x - 1, y - 1, size + 2, size + 2, r + 1, pal.bg);
    }
    let col = rgb(color);
    let rim = mix_rgb(col, pal.specular, 96);
    draw_candy_surface(
        buf,
        stride,
        buf_h,
        x,
        y,
        size,
        size,
        r,
        col,
        rim,
        pal.specular,
        matches!(pal.mode, Mode::HighContrast),
        104,
        92,
    );
}

/// Returns the readable label ink color for a given `ButtonStyle` in `pal`.
#[inline]
pub const fn button_text_color(style: ButtonStyle, pal: &CandyPalette) -> u32 {
    match style {
        ButtonStyle::Primary => pal.text_on_accent,
        ButtonStyle::Danger => match pal.mode {
            Mode::HighContrast => 0x0000_0000,
            _ => 0x00FF_FFFF,
        },
        ButtonStyle::Disabled => pal.text_disabled,
        ButtonStyle::Subtle => pal.text_dim,
        ButtonStyle::Normal | ButtonStyle::Active => pal.text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contrast;

    #[test]
    fn candy_palettes_meet_wcag_contrast() {
        for mode in [Mode::Dark, Mode::Light, Mode::HighContrast] {
            let p = CandyPalette::for_mode(mode);
            assert!(contrast(p.text, p.bg) >= 7.0, "text on bg {mode:?}");
            assert!(contrast(p.text, p.panel) >= 7.0, "text on panel {mode:?}");
            assert!(contrast(p.text, p.card) >= 7.0, "text on card {mode:?}");
            assert!(contrast(p.text, p.control) >= 7.0, "text on control {mode:?}");
            assert!(contrast(p.text_dim, p.bg) >= 4.5, "dim on bg {mode:?}");
            assert!(contrast(p.text_on_accent, p.accent) >= 4.5, "on_accent {mode:?}");
        }
    }

    #[test]
    fn rounded_rect_clips_corners_and_fills_center() {
        let mut buf = [0x0010_1010u32; 40 * 24];
        fill_rounded_rect(&mut buf, 40, 24, 4, 2, 32, 20, 8, 0x00A7_8BFA);
        // Outer corner of bounding box must remain background (rounded, not square).
        assert_eq!(buf[2 * 40 + 4], 0x0010_1010);
        assert_eq!(buf[2 * 40 + 35], 0x0010_1010);
        assert_eq!(buf[21 * 40 + 4], 0x0010_1010);
        assert_eq!(buf[21 * 40 + 35], 0x0010_1010);
        // Center must be filled with exact color.
        assert_eq!(buf[12 * 40 + 20], 0x00A7_8BFA);
    }

    #[test]
    fn candy_button_states_visually_differ_and_stay_rounded() {
        let pal = CandyPalette::dark();
        let mut rest = [pal.bg; 80 * 32];
        let mut hover = [pal.bg; 80 * 32];
        let mut pressed = [pal.bg; 80 * 32];
        draw_candy_button(&mut rest, 80, 32, 4, 4, 72, 24, 8, ButtonStyle::Normal, ButtonState::Rest, &pal);
        draw_candy_button(&mut hover, 80, 32, 4, 4, 72, 24, 8, ButtonStyle::Normal, ButtonState::Hover, &pal);
        draw_candy_button(&mut pressed, 80, 32, 4, 4, 72, 24, 8, ButtonStyle::Normal, ButtonState::Pressed, &pal);

        // Corner remains untouched background -> anti-aliased rounded shape.
        assert_eq!(rest[4 * 80 + 4], pal.bg);
        // Center pixels differ across Rest, Hover, Pressed.
        let c = 16 * 80 + 40;
        assert_ne!(rest[c], pal.bg);
        assert_ne!(rest[c], hover[c]);
        assert_ne!(rest[c], pressed[c]);
        assert_ne!(hover[c], pressed[c]);
    }

    #[test]
    fn candy_toolbar_has_gradient_and_hairline() {
        let pal = CandyPalette::dark();
        let mut buf = [0u32; 32 * 24];
        draw_candy_toolbar(&mut buf, 32, 24, 0, 0, 32, 24, &pal);
        // Top row has specular sheen brighter than plain toolbar_top.
        assert_ne!(buf[0], pal.toolbar_top);
        // Bottom row is exact orchid hairline.
        assert_eq!(buf[23 * 32 + 10], pal.hairline);
        // Middle row is smoothly interpolated between top and bottom.
        assert_ne!(buf[4 * 32 + 10], buf[18 * 32 + 10]);
    }

    #[test]
    fn noir_accent_tints_meet_wcag_and_preserve_high_contrast() {
        for tint in NoirAccent::ALL {
            let p = CandyPalette::noir_tint(tint);
            assert!(contrast(p.text, p.bg) >= 7.0, "{tint:?} text on bg");
            assert!(contrast(p.text, p.card) >= 7.0, "{tint:?} text on card");
            assert!(contrast(p.text_dim, p.card) >= 4.5, "{tint:?} dim on card");
            assert!(contrast(p.text_on_accent, p.accent) >= 4.5, "{tint:?} on accent");
            assert!(contrast(p.text, mix_rgb(p.control, p.accent, 64)) >= 4.5, "{tint:?} text on active button");

            let re = CandyPalette::light().with_noir_accent(tint);
            assert!(contrast(re.text_on_accent, re.accent) >= 4.5, "light {tint:?}");
        }
        let hc = CandyPalette::high_contrast();
        assert_eq!(hc.with_noir_accent(NoirAccent::IceCyan), hc);
        assert_eq!(hc.with_accent_rgb(0x0038_BDF8), hc);
        let dark = CandyPalette::noir();
        assert_eq!(dark.accent, 0x00E1_1D48);
        assert_eq!(dark.bg, 0x000B_0D12);
        assert_eq!(dark.card, 0x0013_161F);
        assert_eq!(dark.control_hover, 0x0023_2938);
        assert_eq!(dark.rim, 0x002A_3142);
    }

    #[test]
    fn clipping_out_of_bounds_never_panics() {
        let pal = CandyPalette::light();
        let mut buf = [0u32; 16 * 16];
        draw_candy_button(&mut buf, 16, 16, -10, -10, 40, 40, 8, ButtonStyle::Primary, ButtonState::Hover, &pal);
        draw_candy_panel(&mut buf, 16, 16, 100, 100, 20, 20, 6, &pal);
        draw_candy_tab(&mut buf, 16, 16, 0, 0, 0, 0, true, ButtonState::Rest, &pal);
        draw_candy_selection_row(&mut buf, 16, 16, -4, 4, 24, 10, 4, &pal);
        draw_candy_badge(&mut buf, 16, 16, 2, 2, 12, 10, pal.mint, &pal);
    }
}
