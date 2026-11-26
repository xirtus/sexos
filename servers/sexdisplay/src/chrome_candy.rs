// SPDX-License-Identifier: GPL-3.0-only
//! Candy UI window chrome, clear 4-button micro-glyphs, and modern Apple-style
//! SilkBar glass helpers. Pure integer pixel functions — no heap, no state.

use sex_pdx::desktop_style::chrome::{Metrics, CONTROLS};

/// Theme mode IDs matching `sex_theme::Mode::setting()` (0=Light, 1=Dark, 2=HighContrast).
pub const MODE_LIGHT: u8 = 0;
pub const MODE_DARK: u8 = 1;
pub const MODE_HIGH_CONTRAST: u8 = 2;

/// Number of user-selectable candy swatches in the per-window color palette.
pub const CANDY_COUNT: usize = 8;

/// Harmonious 8-color candy palette (`0x00RRGGBB`).
/// Index 0..8 correspond to explicit swatches 1..=8 (0 = Auto per-window hue).
pub const CANDY_PALETTE: [u32; CANDY_COUNT] = [
    0x00D6_336C, // 1: Berry / SexOS Rose
    0x00A7_8BFA, // 2: Lavender / Mauve
    0x0060_A5FA, // 3: Sky / Sapphire
    0x0034_D399, // 4: Mint / Emerald
    0x00FB_923C, // 5: Peach / Apricot
    0x00FB_BF24, // 6: Honey / Lemon
    0x002D_D4BF, // 7: Aqua / Teal
    0x00F4_72B6, // 8: Coral / Flamingo
];

/// Returns the swatch RGB for `swatch` (1..=8), clamped into the palette.
#[inline]
pub const fn candy_swatch(swatch: u8) -> u32 {
    if swatch == 0 || swatch as usize > CANDY_COUNT {
        CANDY_PALETTE[0]
    } else {
        CANDY_PALETTE[(swatch - 1) as usize]
    }
}

/// Deterministic harmonious candy hue for a window (`0x00RRGGBB`).
/// `explicit_tint`: 0 = Auto per-window hue derived from `surface_id`,
/// 1..=8 = user-selected candy swatch from `CANDY_PALETTE`.
pub const fn window_candy_hue(surface_id: u64, explicit_tint: u8) -> u32 {
    if explicit_tint >= 1 && (explicit_tint as usize) <= CANDY_COUNT {
        return CANDY_PALETTE[(explicit_tint - 1) as usize];
    }
    // Signature per-window auto assignment so distinct open windows each get a
    // subtle, harmonious candy tint from the same cohesive palette.
    let idx = match surface_id {
        200 => 3, // Linen   -> Mint
        201 => 1, // Quil    -> Lavender
        202 => 6, // Mesh    -> Aqua
        203 => 4, // Collar  -> Peach
        204 => 0, // Bell    -> Berry
        153 => 2, // Spindle -> Sky
        _ => {
            // Stable multiplicative mix for dynamic program/app surfaces.
            let h = surface_id.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56;
            (h as usize) % CANDY_COUNT
        }
    };
    CANDY_PALETTE[idx]
}

/// Integer RGB blend: `a` toward `b` by `t`/255 (XRGB, upper byte 0).
#[inline]
pub const fn mix_u8(a: u32, b: u32, t: u8) -> u32 {
    let t = t as u32;
    let inv = 255 - t;
    let r = (((a >> 16) & 0xFF) * inv + ((b >> 16) & 0xFF) * t + 127) / 255;
    let g = (((a >> 8) & 0xFF) * inv + ((b >> 8) & 0xFF) * t + 127) / 255;
    let bl = ((a & 0xFF) * inv + (b & 0xFF) * t + 127) / 255;
    (r << 16) | (g << 8) | bl
}

/// Matching chrome and subtle interior colors for one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowPalette {
    /// Active title-bar glass tint.
    pub title_bar: u32,
    /// Inactive tab fill harmonized with `title_bar`.
    pub inactive_tab: u32,
    /// Subtle 1px hairline separating title bar from content.
    pub title_divider: u32,
    /// Left, right, and bottom resize band — matches the title bar's candy hue.
    pub band: u32,
    /// Crisp 1px outer window border highlight.
    pub rim: u32,
    /// Coordinated subtle interior canvas tint (for native canvas windows).
    pub interior: u32,
}

/// Derives a cohesive, matching title-bar + full-frame + interior palette for a
/// window in `mode_id` (`0`=Light, `1`=Dark, `2`=HighContrast) with `candy_hue`.
pub const fn window_palette(
    mode_id: u8,
    candy_hue: u32,
    focused: bool,
    frame_hovered: bool,
) -> WindowPalette {
    let hue = candy_hue & 0x00FF_FFFF;
    match mode_id {
        MODE_DARK => {
            // Refined slightly-colored dark glass: rich charcoal base softly
            // tinted with the window's candy hue — never garish or muddy.
            let base_bar = 0x001A_1926;
            let base_band = 0x0014_131E;
            let base_canvas = 0x000D_1019;
            let bar_t: u8 = if focused { 66 } else { 36 };
            let band_t: u8 = if focused { 52 } else { 28 };
            let rim_t: u8 = if frame_hovered {
                180
            } else if focused {
                148
            } else {
                76
            };
            let title_bar = mix_u8(base_bar, hue, bar_t);
            let inactive_tab = mix_u8(0x0012_111C, hue, if focused { 32 } else { 18 });
            let band = mix_u8(base_band, hue, band_t);
            let rim = mix_u8(title_bar, hue, rim_t);
            let title_divider = mix_u8(title_bar, hue, if focused { 96 } else { 48 });
            let interior = mix_u8(base_canvas, hue, if focused { 20 } else { 12 });
            WindowPalette { title_bar, inactive_tab, title_divider, band, rim, interior }
        }
        MODE_HIGH_CONTRAST => WindowPalette {
            title_bar: 0x0000_0000,
            inactive_tab: 0x001A_1A1A,
            title_divider: 0x00FF_FFFF,
            band: 0x0000_0000,
            rim: if focused || frame_hovered { 0x00FF_E600 } else { 0x00FF_FFFF },
            interior: 0x0000_0000,
        },
        _ => {
            // Airy frosted pastel glass with crisp dark ink and cohesive frame.
            let base_bar = 0x00F4_F3F8;
            let base_band = 0x00EE_EDF4;
            let bar_t: u8 = if focused { 52 } else { 26 };
            let band_t: u8 = if focused { 40 } else { 20 };
            let rim_t: u8 = if frame_hovered {
                165
            } else if focused {
                128
            } else {
                68
            };
            let title_bar = mix_u8(base_bar, hue, bar_t);
            let inactive_tab = mix_u8(0x00E4_E2EC, hue, if focused { 24 } else { 12 });
            let band = mix_u8(base_band, hue, band_t);
            let rim = mix_u8(0x00B8_B4C4, hue, rim_t);
            let title_divider = mix_u8(0x00C8_C4D4, hue, if focused { 96 } else { 48 });
            let interior = mix_u8(0x00F8_F7FB, hue, if focused { 14 } else { 8 });
            WindowPalette { title_bar, inactive_tab, title_divider, band, rim, interior }
        }
    }
}

/// Refined 4-button candy colors: Close, Minimise, Maximise, Float.
pub const BTN_CLOSE_COLOR: u32 = 0x00FF_5F57;
pub const BTN_MINIMISE_COLOR: u32 = 0x00FE_BC2E;
pub const BTN_MAXIMISE_COLOR: u32 = 0x0028_C840;
pub const BTN_FLOAT_COLOR: u32 = 0x004D_A6FF;

/// Returns whether local pixel `(gx, gy)` inside a 12x12 logical icon grid
/// belongs to the crisp micro-glyph for button `index`:
/// - 0 (Red / Close): `×` cross
/// - 1 (Amber / Minimise): `−` horizontal bar
/// - 2 (Green / Maximise): `+` expand cross
/// - 3 (Sky / Float): `□` floating window outline
#[inline]
const fn button_glyph_hit(index: u32, gx: i32, gy: i32) -> bool {
    // 12x12 grid, coordinates 0..12, center between 5 and 6.
    match index {
        0 => {
            // Crisp 'x' inside 3..9
            if gx < 3 || gx > 8 || gy < 3 || gy > 8 { return false; }
            let d1 = gx - gy;
            let d2 = (gx + gy) - 11;
            (d1 >= -1 && d1 <= 1 && (gx == gy)) || (d2 == 0)
        }
        1 => {
            // Crisp '-' horizontal bar at y=5..=6, x=3..=8
            gx >= 3 && gx <= 8 && (gy == 5 || gy == 6)
        }
        2 => {
            // Crisp '+' expand icon inside 3..=8
            let hbar = gx >= 3 && gx <= 8 && (gy == 5 || gy == 6);
            let vbar = gy >= 3 && gy <= 8 && (gx == 5 || gx == 6);
            hbar || vbar
        }
        _ => {
            // Crisp floating-window hollow square '□' inside 3..=8
            if gx < 3 || gx > 8 || gy < 3 || gy > 8 { return false; }
            gx == 3 || gx == 8 || gy == 3 || gy == 8
        }
    }
}

/// Renders the 4 colored window buttons with crisp anti-aliased circles,
/// unmistakable micro-glyphs (always subtly visible, full high-contrast on
/// hover), and a luminous hover ring. Shared by both `composite_pixel_source`
/// and `fast_chrome_row`.
#[inline]
pub fn control_button_pixel(
    lx: usize,
    ly: usize,
    bg: u32,
    m: Metrics,
    close_allowed: bool,
    light_hovered: bool,
    hovered_light_kind: u8,
    focused: bool,
) -> u32 {
    let scale = m.scale().max(1) as usize;
    let size = m.px(12) as usize;
    for index in 0..CONTROLS {
        let Some(r) = m.control(index) else { continue };
        let left = r.x as usize + (r.w as usize).saturating_sub(size) / 2;
        let top = r.y as usize + (r.h as usize).saturating_sub(size) / 2;
        // Include 1-logical-px halo for hover ring.
        let halo = scale;
        let hx0 = left.saturating_sub(halo);
        let hy0 = top.saturating_sub(halo);
        let h_size = size + 2 * halo;
        if lx < hx0 || lx >= hx0 + h_size || ly < hy0 || ly >= hy0 + h_size {
            continue;
        }
        let disabled = index == 0 && !close_allowed;
        let hovered = !disabled && light_hovered && hovered_light_kind as u32 == index;
        // Signed coordinates in half-pixel units relative to circle center.
        let cx2 = (left * 2 + size) as i32;
        let cy2 = (top * 2 + size) as i32;
        let dx = (lx * 2 + 1) as i32 - cx2;
        let dy = (ly * 2 + 1) as i32 - cy2;
        let dist2 = dx * dx + dy * dy;
        let rad = size as i32; // radius in half-pixel units == size
        let outer_rad = rad + (scale as i32) * 2;

        // Subtle luminous hover ring just outside the circle.
        if dist2 >= rad * rad {
            if hovered && dist2 < outer_rad * outer_rad {
                let ring_col = match index {
                    0 => BTN_CLOSE_COLOR,
                    1 => BTN_MINIMISE_COLOR,
                    2 => BTN_MAXIMISE_COLOR,
                    _ => BTN_FLOAT_COLOR,
                };
                return mix_u8(bg, mix_u8(ring_col, 0x00FF_FFFF, 128), 160);
            }
            continue;
        }

        let base_color = match index {
            0 => BTN_CLOSE_COLOR,
            1 => BTN_MINIMISE_COLOR,
            2 => BTN_MAXIMISE_COLOR,
            _ => BTN_FLOAT_COLOR,
        };
        // Focused or hovered buttons are vibrant; unfocused buttons soften slightly
        // while remaining clearly colored and legible.
        let fill_color = if disabled {
            mix_u8(bg, base_color, 72)
        } else if hovered {
            mix_u8(base_color, 0x00FF_FFFF, 48)
        } else if focused {
            base_color
        } else {
            mix_u8(bg, base_color, 185)
        };

        // Soft 1px inner border for definition against any light/dark title bar.
        let inner_rad = (rad - (scale as i32) * 2).max(1);
        let mut px = if dist2 >= inner_rad * inner_rad {
            let rim = if hovered { 0x00FF_FFFF } else { mix_u8(fill_color, 0x0000_0000, 64) };
            mix_u8(fill_color, rim, 140)
        } else {
            fill_color
        };

        // Crisp micro-glyph inside the circle: subtle always-on, full contrast on hover.
        if lx >= left && ly >= top {
            let gx = ((lx - left) / scale) as i32;
            let gy = ((ly - top) / scale) as i32;
            if button_glyph_hit(index, gx, gy) {
                let glyph_ink = if index == 0 || index == 3 { 0x001A_0A0A } else { 0x0014_1810 };
                let glyph_alpha: u8 = if disabled {
                    64
                } else if hovered {
                    235
                } else if focused {
                    150
                } else {
                    105
                };
                px = mix_u8(px, glyph_ink, glyph_alpha);
            }
        }

        // Smooth edge anti-aliasing on the outermost pixel ring.
        let aa_inner = (rad - 2).max(1);
        let cov: u8 = if dist2 <= aa_inner * aa_inner {
            255
        } else {
            // Linear falloff across the 1px boundary band.
            let num = (rad * rad - dist2).max(0) as u32;
            let den = (rad * rad - aa_inner * aa_inner).max(1) as u32;
            ((num * 255) / den).min(255) as u8
        };
        return mix_u8(bg, px, cov);
    }
    bg
}

/// Modern Apple-style translucent SilkBar glass pixel at row `y` (bar height `bar_h`).
/// Adapts cleanly to Dark, Light, HighContrast, and Reduce-Transparency modes.
#[inline]
pub const fn bar_glass_pixel(
    y: usize,
    bar_h: usize,
    blurred_wall: u32,
    mode_id: u8,
    panel_fill: u32,
    reduce_transparency: bool,
) -> u32 {
    if bar_h == 0 { return panel_fill; }
    match mode_id {
        MODE_HIGH_CONTRAST => {
            if y + 1 >= bar_h { 0x00FF_FFFF } else { 0x0000_0000 }
        }
        MODE_DARK => {
            // Sleek dark translucent obsidian glass blended smoothly over wallpaper.
            let tint = 0x0015_1520;
            let body = if reduce_transparency {
                panel_fill & 0x00FF_FFFF
            } else {
                mix_u8(blurred_wall & 0x00FF_FFFF, tint, 190)
            };
            if y == 0 {
                // Delicate specular sheen along the top edge.
                mix_u8(body, 0x00FF_FFFF, 22)
            } else if y + 1 >= bar_h {
                // Subtle translucent bottom hairline separator.
                mix_u8(body, 0x00FF_FFFF, 34)
            } else {
                body
            }
        }
        _ => {
            // Airy frosted white/pearl macOS-style menu bar.
            let tint = 0x00F5_F4F9;
            let body = if reduce_transparency {
                panel_fill & 0x00FF_FFFF
            } else {
                mix_u8(blurred_wall & 0x00FF_FFFF, tint, 224)
            };
            if y == 0 {
                mix_u8(body, 0x00FF_FFFF, 96)
            } else if y + 1 >= bar_h {
                // Crisp subtle hairline separator.
                mix_u8(body, 0x009E_9AA8, 120)
            } else {
                body
            }
        }
    }
}

/// Modern rounded pill/dot workspace indicator inside box `(bx, by, bw, bh)`.
/// Active/urgent workspaces render as sleek rounded capsules; inactive ones as
/// clean circular dots.
#[inline]
pub fn workspace_pill_pixel(
    x: usize,
    y: usize,
    bx: usize,
    by: usize,
    bw: usize,
    bh: usize,
    scale: usize,
    active: bool,
    urgent: bool,
    color: u32,
    bg: u32,
) -> Option<u32> {
    let s = if scale == 0 { 1 } else { scale };
    let (pw, ph) = if active || urgent {
        ((14 * s).min(bw.saturating_sub(2 * s)), 6 * s)
    } else {
        (5 * s, 5 * s)
    };
    if pw == 0 || ph == 0 || bw < pw || bh < ph { return None; }
    let left = bx + (bw - pw) / 2;
    let top = by + (bh - ph) / 2;
    if x < left || x >= left + pw || y < top || y >= top + ph { return None; }
    // Stadium / capsule SDF in half-pixel coordinates.
    let r = ph as i32; // radius in half-pixel units == ph
    let cy2 = (top * 2 + ph) as i32;
    let x_l = (left * 2 + ph) as i32;
    let x_r = ((left + pw) * 2 - ph) as i32;
    let px2 = (x * 2 + 1) as i32;
    let py2 = (y * 2 + 1) as i32;
    let cx2 = if px2 < x_l { x_l } else if px2 > x_r { x_r } else { px2 };
    let dx = px2 - cx2;
    let dy = py2 - cy2;
    let d2 = dx * dx + dy * dy;
    if d2 >= r * r { return None; }
    let aa = (r - 2).max(1);
    let cov: u8 = if d2 <= aa * aa {
        255
    } else {
        let num = (r * r - d2).max(0) as u32;
        let den = (r * r - aa * aa).max(1) as u32;
        ((num * 255) / den).min(255) as u8
    };
    Some(mix_u8(bg, color & 0x00FF_FFFF, cov))
}

/// Clean 12x12 logical bell icon silhouette inside box `(bx, by, bw, bh)`,
/// replacing the flat 90s square block. Adds an amber/gold badge dot when
/// notifications are present.
#[inline]
pub const fn bell_icon_pixel(
    x: usize,
    y: usize,
    bx: usize,
    by: usize,
    bw: usize,
    bh: usize,
    scale: usize,
    available: bool,
    visible_events: u8,
    redacted_events: u8,
    fg_ink: u32,
    muted_ink: u32,
    bg: u32,
) -> Option<u32> {
    let s = if scale == 0 { 1 } else { scale };
    let sz = 12 * s;
    if bw < sz || bh < sz { return None; }
    let left = bx + (bw - sz) / 2;
    let top = by + (bh - sz) / 2;
    if x < left || x >= left + sz || y < top || y >= top + sz { return None; }
    let gx = (x - left) / s;
    let gy = (y - top) / s;
    // Notification badge dot at top-right (gx 8..=10, gy 1..=3) when events exist.
    if available && visible_events > 0 && gx >= 8 && gx <= 10 && gy >= 1 && gy <= 3 {
        let badge = if redacted_events > 0 { 0x00FF_AA44 } else { 0x00FF_D700 };
        return Some(badge);
    }
    // 12x12 bell bitmap (MSB = bit 11).
    const BELL: [u16; 12] = [
        0b0000_0110_0000, // 0: handle
        0b0000_1111_0000, // 1: crown
        0b0001_1111_1000, // 2: dome
        0b0001_1111_1000, // 3: body
        0b0001_1111_1000, // 4: body
        0b0001_1111_1000, // 5: body
        0b0011_1111_1100, // 6: flare
        0b0111_1111_1110, // 7: lip
        0b0111_1111_1110, // 8: rim
        0b0000_0000_0000, // 9: gap
        0b0000_1111_0000, // 10: clapper
        0b0000_0110_0000, // 11: clapper tip
    ];
    if (BELL[gy] >> (11 - gx)) & 1 != 0 {
        let ink = if available && visible_events > 0 { fg_ink } else { muted_ink };
        return Some(mix_u8(bg, ink & 0x00FF_FFFF, 220));
    }
    None
}
