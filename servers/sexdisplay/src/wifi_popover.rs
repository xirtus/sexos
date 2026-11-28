// SPDX-License-Identifier: GPL-3.0-only
//! Frosted Noir / Candy Lavender renderer for SilkBar's Live Wi-Fi Scanner Popover.
//!
//! Shell drives all state over `OP_WIFI_POPOVER` (`sex_pdx::wifi_popover`);
//! Display renders using the exact shared `WifiLayout` so pointer hits match pixels 1:1.

use sex_pdx::wifi_popover::{
    self as wp, phase, security, signal_bars, Assembler, HitTarget, PopoverView, Rect,
    WifiLayout, SHELL_PD,
};
use sex_theme::{candy_lavender as cl, Mode};
use crate::desk_theme::current as desk;

static mut ASSEMBLER: Assembler = Assembler::new();
static mut SHOWN: Option<PopoverView> = None;

/// Handles one `OP_WIFI_POPOVER` chunk from Shell. Returns true when display changed.
pub fn receive(caller: u32, w0: u64, w1: u64, w2: u64) -> bool {
    if caller != SHELL_PD {
        return false;
    }
    let asm = unsafe { &mut *core::ptr::addr_of_mut!(ASSEMBLER) };
    let Some(next) = asm.push(w0, w1, w2) else {
        return false;
    };
    let next_opt = next.open.then_some(next);
    unsafe {
        let slot = &mut *core::ptr::addr_of_mut!(SHOWN);
        if *slot == next_opt {
            return false;
        }
        *slot = next_opt;
    }
    static mut BUDGET: u32 = 32;
    unsafe {
        if BUDGET > 0 {
            BUDGET -= 1;
            match next_opt {
                Some(v) => {
                    sex_pdx::serial_record!(
                        "[display.wifi_popover] open=1 phase={} radio={} nets={} pw_open={}",
                        v.phase,
                        v.radio_on as u8,
                        v.net_count,
                        v.password_open as u8
                    );
                }
                None => {
                    sex_pdx::serial_record!("[display.wifi_popover] open=0");
                }
            }
        }
    }
    true
}

pub fn is_open() -> bool {
    unsafe { (*core::ptr::addr_of!(SHOWN)).is_some() }
}

pub fn glass_rect(w: usize, h: usize, scale: u32) -> Option<Rect> {
    let view = unsafe { *core::ptr::addr_of!(SHOWN) }?;
    WifiLayout::new(&view, w, h, scale).map(|l| l.panel)
}

#[derive(Clone, Copy)]
struct Palette {
    bg: u32,
    rim: u32,
    specular: u32,
    hover: u32,
    divider: u32,
    card_bg: u32,
    card_rim: u32,
    field_bg: u32,
    ink: u32,
    title: u32,
    subtext: u32,
    dim: u32,
    accent: u32,
    on_accent: u32,
    good: u32,
    warn: u32,
    danger: u32,
    btn_bg: u32,
    btn_hover: u32,
    badge_bg: u32,
}

fn palette() -> Palette {
    let d = desk();
    match d.mode {
        Mode::HighContrast => Palette {
            bg: d.menu,
            rim: d.menu_rim,
            specular: d.menu_rim,
            hover: d.menu_picked,
            divider: d.menu_rim,
            card_bg: d.menu,
            card_rim: d.menu_rim,
            field_bg: d.menu_field,
            ink: d.menu_ink,
            title: d.menu_title,
            subtext: d.menu_soft,
            dim: d.menu_dim,
            accent: d.focus,
            on_accent: d.menu_on_send,
            good: d.good,
            warn: d.focus,
            danger: d.menu_close,
            btn_bg: d.menu_button,
            btn_hover: d.menu_picked,
            badge_bg: d.menu_button,
        },
        Mode::Dark => Palette {
            bg: cl::DARK_MENU_BG & 0x00FF_FFFF,
            rim: cl::DARK_MENU_RIM & 0x00FF_FFFF,
            specular: cl::DARK_SPECULAR & 0x00FF_FFFF,
            hover: cl::DARK_MENU_HOVER & 0x00FF_FFFF,
            divider: cl::DARK_MENU_DIVIDER & 0x00FF_FFFF,
            card_bg: 0x0026_1A3D,
            card_rim: 0x0058_3E8A,
            field_bg: 0x0012_0D1F,
            ink: cl::DARK_TEXT & 0x00FF_FFFF,
            title: 0x00FF_FFFF,
            subtext: cl::DARK_MENU_SUBTEXT & 0x00FF_FFFF,
            dim: 0x006C_5E8A,
            accent: d.focus,
            on_accent: d.menu_on_send,
            good: d.good,
            warn: 0x00F9_E2AF,
            danger: cl::DARK_MENU_DANGER & 0x00FF_FFFF,
            btn_bg: cl::DARK_PILL_FILL & 0x00FF_FFFF,
            btn_hover: 0x004E_3877,
            badge_bg: 0x0034_2454,
        },
        Mode::Light => Palette {
            bg: cl::LIGHT_MENU_BG & 0x00FF_FFFF,
            rim: cl::LIGHT_MENU_RIM & 0x00FF_FFFF,
            specular: cl::LIGHT_SPECULAR & 0x00FF_FFFF,
            hover: cl::LIGHT_MENU_HOVER & 0x00FF_FFFF,
            divider: cl::LIGHT_MENU_DIVIDER & 0x00FF_FFFF,
            card_bg: 0x00FF_FFFF,
            card_rim: cl::LIGHT_MENU_RIM & 0x00FF_FFFF,
            field_bg: 0x00FF_FFFF,
            ink: cl::LIGHT_TEXT & 0x00FF_FFFF,
            title: 0x0016_1024,
            subtext: cl::LIGHT_MENU_SUBTEXT & 0x00FF_FFFF,
            dim: 0x009B_8CB8,
            accent: d.focus,
            on_accent: d.menu_on_send,
            good: d.good,
            warn: 0x00B4_5309,
            danger: cl::LIGHT_MENU_DANGER & 0x00FF_FFFF,
            btn_bg: cl::LIGHT_PILL_FILL & 0x00FF_FFFF,
            btn_hover: 0x00E4_D5FF,
            badge_bg: 0x00EC_E0FF,
        },
    }
}

// ── Icon Cache (`sex_icons` line icons rasterized once per size) ────────────

const ICON_CAP: usize = 48;
const ICON_SLOTS: usize = 6;
const ICON_NAMES: [&str; ICON_SLOTS] = ["lock", "star", "eye", "refresh", "settings", "check"];

struct IconCache {
    sides: [usize; ICON_SLOTS],
    masks: [[u8; ICON_CAP * ICON_CAP]; ICON_SLOTS],
    acc: [i32; sex_font::raster::scratch_len(ICON_CAP, ICON_CAP)],
}

static mut ICONS: IconCache = IconCache {
    sides: [0; ICON_SLOTS],
    masks: [[0; ICON_CAP * ICON_CAP]; ICON_SLOTS],
    acc: [0; sex_font::raster::scratch_len(ICON_CAP, ICON_CAP)],
};

#[allow(static_mut_refs)]
fn draw_icon(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    name_idx: usize,
    x0: usize,
    y0: usize,
    side: usize,
    ink: u32,
) {
    let side = side.clamp(8, ICON_CAP);
    if name_idx >= ICON_SLOTS {
        return;
    }
    let cache = unsafe { &mut *core::ptr::addr_of_mut!(ICONS) };
    if cache.sides[name_idx] != side {
        let Some(data) = sex_icons::find(sex_icons::Kind::Line, ICON_NAMES[name_idx]) else {
            return;
        };
        let place = sex_icons::Place::square(side);
        let stroke = (1.8 * place.scale).max(1.2) / place.scale;
        let mask = &mut cache.masks[name_idx][..side * side];
        sex_icons::coverage(data, place, stroke, side, side, &mut cache.acc, mask);
        cache.sides[name_idx] = side;
    }
    let mask = &cache.masks[name_idx][..side * side];
    for gy in 0..side {
        let py = y0 + gy;
        if py >= h {
            break;
        }
        for gx in 0..side {
            let px = x0 + gx;
            if px >= w {
                break;
            }
            let a = mask[gy * side + gx];
            if a == 0 {
                continue;
            }
            let at = py * w + px;
            if at < total {
                unsafe {
                    let p = fb.add(at);
                    core::ptr::write_volatile(
                        p,
                        super::coverage_blend(ink, core::ptr::read_volatile(p), a),
                    );
                }
            }
        }
    }
}

// ── Pixel Drawing Helpers ───────────────────────────────────────────────────

#[inline]
fn put_px(fb: *mut u32, w: usize, h: usize, total: usize, x: usize, y: usize, c: u32) {
    if x < w && y < h {
        let i = y * w + x;
        if i < total {
            unsafe { core::ptr::write_volatile(fb.add(i), c); }
        }
    }
}

#[inline]
fn blend_px(fb: *mut u32, w: usize, h: usize, total: usize, x: usize, y: usize, c: u32, alpha: u8) {
    if alpha == 0 {
        return;
    }
    if alpha == 255 {
        put_px(fb, w, h, total, x, y, c);
        return;
    }
    if x < w && y < h {
        let i = y * w + x;
        if i < total {
            unsafe {
                let p = fb.add(i);
                core::ptr::write_volatile(p, super::coverage_blend(c, core::ptr::read_volatile(p), alpha));
            }
        }
    }
}

fn fill_rect(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, c: u32) {
    for y in r.1..(r.1 + r.3).min(h) {
        for x in r.0..(r.0 + r.2).min(w) {
            put_px(fb, w, h, total, x, y, c);
        }
    }
}

fn tint_glass(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, tint: u32, alpha: u8) {
    for y in r.1..(r.1 + r.3).min(h) {
        for x in r.0..(r.0 + r.2).min(w) {
            let i = y * w + x;
            if i < total {
                unsafe {
                    let p = fb.add(i);
                    let under = core::ptr::read_volatile(p);
                    core::ptr::write_volatile(p, crate::chrome_candy::mix_u8(under, tint, alpha));
                }
            }
        }
    }
}

fn inside_round(dx: usize, dy: usize, rw: usize, rh: usize, rad: usize) -> bool {
    if rad == 0 {
        return true;
    }
    let rx = if dx < rad {
        rad - 1 - dx
    } else if dx >= rw.saturating_sub(rad) {
        dx - (rw - rad)
    } else {
        return true;
    };
    let ry = if dy < rad {
        rad - 1 - dy
    } else if dy >= rh.saturating_sub(rad) {
        dy - (rh - rad)
    } else {
        return true;
    };
    rx * rx + ry * ry <= rad * rad
}

fn fill_round(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, rad: usize, c: u32) {
    if r.2 == 0 || r.3 == 0 {
        return;
    }
    let rad = rad.min(r.2 / 2).min(r.3 / 2);
    for dy in 0..r.3 {
        let py = r.1 + dy;
        if py >= h {
            break;
        }
        for dx in 0..r.2 {
            let px = r.0 + dx;
            if px >= w {
                break;
            }
            if inside_round(dx, dy, r.2, r.3, rad) {
                put_px(fb, w, h, total, px, py, c);
            }
        }
    }
}

fn stroke_round(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    r: Rect,
    rad: usize,
    thick: usize,
    c: u32,
) {
    if r.2 < 2 * thick || r.3 < 2 * thick {
        return;
    }
    let rad = rad.min(r.2 / 2).min(r.3 / 2);
    let inner_w = r.2 - 2 * thick;
    let inner_h = r.3 - 2 * thick;
    let inner_rad = rad.saturating_sub(thick);
    for dy in 0..r.3 {
        let py = r.1 + dy;
        if py >= h {
            break;
        }
        for dx in 0..r.2 {
            let px = r.0 + dx;
            if px >= w {
                break;
            }
            if !inside_round(dx, dy, r.2, r.3, rad) {
                continue;
            }
            let in_hole = dx >= thick
                && dx < r.2 - thick
                && dy >= thick
                && dy < r.3 - thick
                && inside_round(dx - thick, dy - thick, inner_w, inner_h, inner_rad);
            if !in_hole {
                put_px(fb, w, h, total, px, py, c);
            }
        }
    }
}

fn fill_circle(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    cx: usize,
    cy: usize,
    d: usize,
    c: u32,
) {
    if d == 0 {
        return;
    }
    let rad2 = (d * d) as i32; // compare against (2*dx - (d-1))^2 + (2*dy - (d-1))^2 <= (d-1)^2 or AA
    for dy in 0..d {
        for dx in 0..d {
            let vx = (2 * dx + 1) as i32 - d as i32;
            let vy = (2 * dy + 1) as i32 - d as i32;
            let dist2 = vx * vx + vy * vy;
            if dist2 <= rad2 {
                let edge = rad2 - dist2;
                let alpha = if edge >= (d as i32) {
                    255
                } else {
                    ((edge * 255) / d.max(1) as i32).clamp(0, 255) as u8
                };
                blend_px(fb, w, h, total, cx + dx, cy + dy, c, alpha);
            }
        }
    }
}

fn text_core(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    t: &[u8],
    x0: usize,
    y0: usize,
    ink: u32,
    scale: u32,
    max_w: usize,
    semibold: bool,
) {
    let width = super::glyphs::ui_width(t, scale).unwrap_or(0).min(max_w);
    let lh = super::glyphs::UI_H * scale as usize;
    let s_px = scale.max(1) as usize;
    for y in y0..(y0 + lh).min(h) {
        for x in x0..(x0 + width).min(w) {
            let Some(alpha) = super::glyphs::ui_coverage(t, (x - x0) as i32, y - y0, scale) else {
                break;
            };
            if alpha == 0 {
                continue;
            }
            blend_px(fb, w, h, total, x, y, ink, alpha);
            if semibold && x + s_px < (x0 + width + s_px).min(w) {
                let a2 = ((alpha as u16 * 105) / 255) as u8;
                blend_px(fb, w, h, total, x + 1, y, ink, a2);
            }
        }
    }
}

fn text(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    t: &[u8],
    x0: usize,
    y0: usize,
    ink: u32,
    scale: u32,
    max_w: usize,
) {
    text_core(fb, w, h, total, t, x0, y0, ink, scale, max_w, false);
}

fn text_semibold(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    t: &[u8],
    x0: usize,
    y0: usize,
    ink: u32,
    scale: u32,
    max_w: usize,
) {
    text_core(fb, w, h, total, t, x0, y0, ink, scale, max_w, true);
}

fn text_fit(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    src: &[u8],
    x0: usize,
    y0: usize,
    ink: u32,
    scale: u32,
    max_w: usize,
    semibold: bool,
) {
    let full_w = super::glyphs::ui_width(src, scale).unwrap_or(src.len() * 7 * scale as usize);
    if full_w <= max_w {
        text_core(fb, w, h, total, src, x0, y0, ink, scale, max_w, semibold);
        return;
    }
    let mut buf = [0u8; 64];
    let ell = b"...";
    let mut end = src.len().min(buf.len() - 3);
    while end > 0 {
        buf[..end].copy_from_slice(&src[..end]);
        buf[end..end + 3].copy_from_slice(ell);
        let tw = super::glyphs::ui_width(&buf[..end + 3], scale).unwrap_or((end + 3) * 7 * scale as usize);
        if tw <= max_w {
            text_core(fb, w, h, total, &buf[..end + 3], x0, y0, ink, scale, max_w, semibold);
            return;
        }
        end -= 1;
    }
}

fn draw_signal_bars(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    x0: usize,
    y0: usize,
    s: usize,
    bars: u8,
    active_col: u32,
    dim_col: u32,
) {
    let bar_w = 2 * s;
    let gap = s;
    let max_h = 12 * s;
    let heights = [3 * s, 6 * s, 9 * s, 12 * s];
    for (idx, &bh) in heights.iter().enumerate() {
        let bx = x0 + idx * (bar_w + gap);
        let by = y0 + max_h - bh;
        let col = if (idx as u8) < bars { active_col } else { dim_col };
        fill_rect(fb, w, h, total, (bx, by, bar_w, bh), col);
    }
}

fn draw_spinner(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    x0: usize,
    y0: usize,
    s: usize,
    spin_phase: u8,
    accent: u32,
    dim: u32,
) {
    // 8 points in a clean 12*s x 12*s circle: offsets around center (5*s, 5*s), radius 4.5*s.
    let pts: [(usize, usize); 8] = [
        (5, 0),
        (9, 1),
        (10, 5),
        (9, 9),
        (5, 10),
        (1, 9),
        (0, 5),
        (1, 1),
    ];
    let dot = (2 * s).max(2);
    for (i, &(ox, oy)) in pts.iter().enumerate() {
        let dist = (spin_phase as usize + 8 - i) % 8;
        let col = match dist {
            0 => accent,
            1 => crate::chrome_candy::mix_u8(accent, dim, 80),
            2 => crate::chrome_candy::mix_u8(accent, dim, 160),
            _ => dim,
        };
        fill_rect(fb, w, h, total, (x0 + ox * s, y0 + oy * s, dot, dot), col);
    }
}

fn fmt_ipv4(ip: [u8; 4], out: &mut [u8; 20]) -> usize {
    let mut n = 0usize;
    for (idx, &oct) in ip.iter().enumerate() {
        if idx > 0 && n < out.len() {
            out[n] = b'.';
            n += 1;
        }
        let mut d = [0u8; 3];
        let mut v = oct;
        let mut k = 0usize;
        loop {
            d[k] = b'0' + (v % 10);
            k += 1;
            v /= 10;
            if v == 0 {
                break;
            }
        }
        while k > 0 && n < out.len() {
            k -= 1;
            out[n] = d[k];
            n += 1;
        }
    }
    n
}

fn fmt_u8(v: u8, out: &mut [u8; 4]) -> usize {
    let mut digits = [0u8; 3];
    let mut n = 0usize;
    let mut rem = v;
    loop {
        digits[n] = b'0' + (rem % 10);
        n += 1;
        rem /= 10;
        if rem == 0 {
            break;
        }
    }
    let mut pos = 0usize;
    while n > 0 {
        n -= 1;
        out[pos] = digits[n];
        pos += 1;
    }
    pos
}

/// Renders the Live Wi-Fi Scanner popover when open. Returns `true` if drawn.
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, scale: u32, bar: usize) -> bool {
    let Some(view) = (unsafe { *core::ptr::addr_of!(SHOWN) }) else {
        return false;
    };
    let Some(l) = WifiLayout::new(&view, w, h, scale) else {
        return true;
    };
    let sc = l.scale;
    let s = sc as usize;
    let p = palette();
    let inner = crate::frost::inside(l.panel, s);

    // 1. Frosted Candy Lavender / Noir glass backdrop + specular top sheen + rim.
    crate::frost::glass_or(fb, w, h, total, crate::frost::Panel::SilkBarMenu, inner, sc, bar, p.bg);
    if desk().mode != Mode::HighContrast {
        tint_glass(fb, w, h, total, inner, p.bg, 72);
        tint_glass(fb, w, h, total, (inner.0, inner.1, inner.2, s), p.specular, 80);
    }
    crate::frost::rim(fb, w, h, total, l.panel, s, p.rim);

    let line = super::glyphs::UI_H * s;

    // 2. Header Row: `"Wi-Fi"` title + spinner / Rescan pill + macOS On/Off Pill Toggle.
    let hr = l.header;
    let hy = hr.1 + hr.3.saturating_sub(line) / 2;
    text_semibold(fb, w, h, total, b"Wi-Fi", hr.0 + 2 * s, hy, p.title, sc, 72 * s);
    if view.scanning {
        draw_spinner(fb, w, h, total, hr.0 + 48 * s, hr.1 + (hr.3.saturating_sub(12 * s)) / 2, s, view.spin_phase, p.accent, p.dim);
    }

    if l.rescan.2 > 0 {
        let rr = l.rescan;
        let hovered = view.hover == HitTarget::Rescan;
        fill_round(fb, w, h, total, rr, 6 * s, if hovered { p.btn_hover } else { p.btn_bg });
        stroke_round(fb, w, h, total, rr, 6 * s, s, if hovered { p.accent } else { p.rim });
        if view.scanning {
            draw_spinner(fb, w, h, total, rr.0 + 6 * s, rr.1 + (rr.3.saturating_sub(12 * s)) / 2, s, view.spin_phase, p.accent, p.dim);
        } else {
            draw_icon(fb, w, h, total, 3 /* refresh */, rr.0 + 6 * s, rr.1 + (rr.3.saturating_sub(12 * s)) / 2, 12 * s, p.ink);
        }
        let lbl = b"Scan";
        text(fb, w, h, total, lbl, rr.0 + 22 * s, rr.1 + rr.3.saturating_sub(line) / 2, p.ink, sc, rr.2.saturating_sub(24 * s));
    }

    // macOS On/Off pill switch
    let tr = l.toggle;
    let rad = tr.3 / 2;
    let track_col = if view.radio_on { p.accent } else { p.divider };
    fill_round(fb, w, h, total, tr, rad, track_col);
    if view.hover == HitTarget::Toggle {
        stroke_round(fb, w, h, total, tr, rad, s, p.title);
    } else {
        stroke_round(fb, w, h, total, tr, rad, s, p.rim);
    }
    let knob_d = tr.3.saturating_sub(4 * s);
    let knob_x = if view.radio_on { tr.0 + tr.2 - knob_d - 2 * s } else { tr.0 + 2 * s };
    let knob_y = tr.1 + 2 * s;
    fill_circle(fb, w, h, total, knob_x, knob_y, knob_d, 0x00FF_FFFF);

    // Header divider
    fill_rect(fb, w, h, total, (hr.0, hr.1 + hr.3 + 2 * s, hr.2, s), p.divider);

    // 3. Optional Status / Error Banner (when inline password drawer is closed)
    if let Some(br) = l.status_banner {
        let st_col = if view.phase == phase::FAILED { p.danger } else { p.warn };
        text_fit(fb, w, h, total, view.status(), br.0 + 2 * s, br.1 + br.3.saturating_sub(line) / 2, st_col, sc, br.2.saturating_sub(4 * s), false);
    }

    // 4. Current Connection Card (Active SSID / Connecting / Wired LAN)
    if let Some(cr) = l.card {
        fill_round(fb, w, h, total, cr, 8 * s, p.card_bg);
        let border_col = if phase::is_connected(view.phase) || view.active_wired {
            crate::chrome_candy::mix_u8(p.card_rim, p.good, 90)
        } else if view.phase == phase::FAILED {
            p.danger
        } else {
            p.card_rim
        };
        stroke_round(fb, w, h, total, cr, 8 * s, s, border_col);

        let dot_col = if phase::is_connected(view.phase) || view.active_wired {
            p.good
        } else if view.phase == phase::FAILED {
            p.danger
        } else {
            p.warn
        };
        fill_circle(fb, w, h, total, cr.0 + 10 * s, cr.1 + (cr.3.saturating_sub(7 * s)) / 2, 7 * s, dot_col);

        let text_left = if view.active_wired {
            cr.0 + 24 * s
        } else {
            draw_signal_bars(fb, w, h, total, cr.0 + 22 * s, cr.1 + (cr.3.saturating_sub(12 * s)) / 2, s, signal_bars(view.active_rssi), p.ink, p.dim);
            cr.0 + 40 * s
        };
        let right_edge = l.disconnect.map_or(cr.0 + cr.2 - 10 * s, |d| d.0.saturating_sub(8 * s));
        let text_max_w = right_edge.saturating_sub(text_left);

        let title_bytes: &[u8] = if view.active_wired {
            b"Wired Ethernet (eth0)"
        } else if !view.active_ssid().is_empty() {
            view.active_ssid()
        } else {
            b"Connecting..."
        };
        let lock_room = if !view.active_wired && view.active_security != security::OPEN { 16 * s } else { 0 };
        text_fit(fb, w, h, total, title_bytes, text_left, cr.1 + 7 * s, p.title, sc, text_max_w.saturating_sub(lock_room), true);
        if lock_room > 0 {
            let tw = super::glyphs::ui_width(title_bytes, sc).unwrap_or(0).min(text_max_w.saturating_sub(lock_room));
            draw_icon(fb, w, h, total, 0 /* lock */, text_left + tw + 4 * s, cr.1 + 9 * s, 11 * s, p.subtext);
        }

        // Subtitle line: status / security / IPv4
        let mut sub = [0u8; 56];
        let mut sn = 0usize;
        let put = |sub: &mut [u8], sn: &mut usize, bytes: &[u8]| {
            for &b in bytes {
                if *sn < sub.len() {
                    sub[*sn] = b;
                    *sn += 1;
                }
            }
        };
        if view.active_wired {
            put(&mut sub, &mut sn, b"Active");
        } else if phase::is_connected(view.phase) {
            put(&mut sub, &mut sn, b"Connected");
        } else if view.phase == phase::HANDSHAKE {
            put(&mut sub, &mut sn, b"Authenticating...");
        } else if view.phase == phase::FAILED {
            put(&mut sub, &mut sn, b"Failed");
        } else {
            put(&mut sub, &mut sn, b"Connecting...");
        }
        if !view.active_wired {
            put(&mut sub, &mut sn, b" - ");
            put(&mut sub, &mut sn, security::label(view.active_security).as_bytes());
        }
        if view.active_ipv4 != [0; 4] {
            let mut ipb = [0u8; 20];
            let iplen = fmt_ipv4(view.active_ipv4, &mut ipb);
            put(&mut sub, &mut sn, b" - ");
            put(&mut sub, &mut sn, &ipb[..iplen]);
        }
        text_fit(fb, w, h, total, &sub[..sn], text_left, cr.1 + 28 * s, p.subtext, sc, text_max_w, false);

        if let Some(dr) = l.disconnect {
            let hovered = view.hover == HitTarget::Disconnect;
            let dbg = if hovered { crate::chrome_candy::mix_u8(p.btn_bg, p.danger, 70) } else { p.btn_bg };
            fill_round(fb, w, h, total, dr, 6 * s, dbg);
            stroke_round(fb, w, h, total, dr, 6 * s, s, if hovered { p.danger } else { p.rim });
            let lbl: &[u8] = if phase::is_connected(view.phase) { b"Disconnect" } else { b"Cancel" };
            let lw = super::glyphs::ui_width(lbl, sc).unwrap_or(0);
            let lx = dr.0 + dr.2.saturating_sub(lw) / 2;
            let ly = dr.1 + dr.3.saturating_sub(line) / 2;
            text(fb, w, h, total, lbl, lx, ly, if hovered { p.danger } else { p.ink }, sc, dr.2);
        }
    }

    // 5. Wi-Fi Turned Off Message
    if let Some(or) = l.off_box {
        fill_round(fb, w, h, total, or, 6 * s, p.card_bg);
        stroke_round(fb, w, h, total, or, 6 * s, s, p.divider);
        text(fb, w, h, total, b"Wi-Fi is Turned Off", or.0 + 10 * s, or.1 + 8 * s, p.ink, sc, or.2.saturating_sub(20 * s));
        text(fb, w, h, total, b"Turn On Wi-Fi to scan local networks", or.0 + 10 * s, or.1 + 27 * s, p.subtext, sc, or.2.saturating_sub(20 * s));
    }

    // 6. Other Networks Section
    if let Some(sh) = l.section_hdr {
        let sy = sh.1 + sh.3.saturating_sub(line) / 2;
        text(fb, w, h, total, b"Other Networks", sh.0 + 2 * s, sy, p.subtext, sc, 150 * s);
        let right_lbl: &[u8] = if view.phase == phase::NONE {
            b"No Wi-Fi card"
        } else if view.scanning {
            b"Scanning..."
        } else {
            b""
        };
        if !right_lbl.is_empty() {
            let rw = super::glyphs::ui_width(right_lbl, sc).unwrap_or(0);
            text(fb, w, h, total, right_lbl, sh.0 + sh.2.saturating_sub(rw + 2 * s), sy, p.dim, sc, rw);
        } else if l.visible_nets > 0 {
            let mut cnt = [0u8; 16];
            let mut cb = [0u8; 4];
            let cn = fmt_u8(l.visible_nets as u8, &mut cb);
            cnt[..cn].copy_from_slice(&cb[..cn]);
            let suf = if l.visible_nets == 1 { b" found".as_slice() } else { b" found".as_slice() };
            cnt[cn..cn + suf.len()].copy_from_slice(suf);
            let lbl = &cnt[..cn + suf.len()];
            let rw = super::glyphs::ui_width(lbl, sc).unwrap_or(0);
            text(fb, w, h, total, lbl, sh.0 + sh.2.saturating_sub(rw + 2 * s), sy, p.dim, sc, rw);
        }
    }

    if let Some(er) = l.empty_row {
        let msg: &[u8] = if view.phase == phase::NONE {
            b"No wireless adapter detected"
        } else if view.scanning || view.phase == phase::STARTING {
            b"Searching for nearby networks..."
        } else {
            b"No networks found - click Scan"
        };
        text(fb, w, h, total, msg, er.0 + 8 * s, er.1 + er.3.saturating_sub(line) / 2, p.dim, sc, er.2.saturating_sub(16 * s));
    }

    for i in 0..l.visible_nets {
        let entry = &view.networks[i];
        let rr = l.net_rows[i];
        let is_hover = view.hover == HitTarget::NetworkRow(i as u8);
        let is_pw_target = view.password_open && view.password_target == Some(i as u8);
        if is_hover || is_pw_target {
            fill_round(fb, w, h, total, rr, 6 * s, p.hover);
            fill_rect(fb, w, h, total, (rr.0 + 2 * s, rr.1 + 5 * s, 3 * s, rr.3.saturating_sub(10 * s)), p.accent);
        }

        // 4-bar signal strength icon
        draw_signal_bars(fb, w, h, total, rr.0 + 10 * s, rr.1 + (rr.3.saturating_sub(12 * s)) / 2, s, signal_bars(entry.rssi), p.ink, p.dim);

        // Right-aligned badges: Lock icon + Known badge
        let mut right_cursor = rr.0 + rr.2 - 8 * s;
        if entry.security != security::OPEN {
            right_cursor = right_cursor.saturating_sub(14 * s);
            draw_icon(fb, w, h, total, 0 /* lock */, right_cursor + 2 * s, rr.1 + (rr.3.saturating_sub(11 * s)) / 2, 11 * s, p.subtext);
            right_cursor = right_cursor.saturating_sub(4 * s);
        }
        if entry.known {
            let bw = 54 * s;
            let bh = 18 * s;
            right_cursor = right_cursor.saturating_sub(bw);
            let br = (right_cursor, rr.1 + (rr.3.saturating_sub(bh)) / 2, bw, bh);
            fill_round(fb, w, h, total, br, 5 * s, p.badge_bg);
            stroke_round(fb, w, h, total, br, 5 * s, s, p.rim);
            draw_icon(fb, w, h, total, 1 /* star */, br.0 + 4 * s, br.1 + (bh.saturating_sub(10 * s)) / 2, 10 * s, p.accent);
            text(fb, w, h, total, b"Known", br.0 + 16 * s, br.1 + bh.saturating_sub(line) / 2, p.accent, sc, bw.saturating_sub(18 * s));
            right_cursor = right_cursor.saturating_sub(4 * s);
        }

        let ssid_x = rr.0 + 28 * s;
        let ssid_max_w = right_cursor.saturating_sub(ssid_x);
        text_fit(fb, w, h, total, entry.ssid(), ssid_x, rr.1 + rr.3.saturating_sub(line) / 2, if is_pw_target { p.title } else { p.ink }, sc, ssid_max_w, entry.known);

        // Expand inline password entry drawer below selected unknown secured SSID
        if is_pw_target {
            if let Some(dr) = l.drawer {
                fill_round(fb, w, h, total, dr, 8 * s, p.card_bg);
                stroke_round(fb, w, h, total, dr, 8 * s, s, p.accent);
            }
            if let Some(pb) = l.password_box {
                fill_round(fb, w, h, total, pb, 5 * s, p.field_bg);
                stroke_round(fb, w, h, total, pb, 5 * s, s, p.accent);

                let px0 = pb.0 + 8 * s;
                let p_avail = pb.2.saturating_sub(16 * s);
                let py_mid = pb.1 + pb.3.saturating_sub(line) / 2;
                let plen = view.password_len as usize;

                if plen == 0 {
                    text(fb, w, h, total, b"Enter password...", px0, py_mid, p.dim, sc, p_avail);
                    fill_rect(fb, w, h, total, (px0, pb.1 + 5 * s, (2 * s).max(1), pb.3.saturating_sub(10 * s)), p.accent);
                } else if view.show_password {
                    let pw = view.visible_password();
                    // Keep the end of the passphrase visible if longer than input box.
                    let mut start = 0usize;
                    while start < pw.len()
                        && super::glyphs::ui_width(&pw[start..], sc).unwrap_or((pw.len() - start) * 7 * s) + 4 * s > p_avail
                    {
                        start += 1;
                    }
                    let slice = &pw[start..];
                    text(fb, w, h, total, slice, px0, py_mid, p.title, sc, p_avail);
                    let tw = super::glyphs::ui_width(slice, sc).unwrap_or(slice.len() * 7 * s);
                    fill_rect(fb, w, h, total, ((px0 + tw + 2 * s).min(pb.0 + pb.2 - 5 * s), pb.1 + 5 * s, (2 * s).max(1), pb.3.saturating_sub(10 * s)), p.accent);
                } else {
                    let dot_d = 5 * s;
                    let dot_step = 8 * s;
                    let max_dots = (p_avail.saturating_sub(4 * s) / dot_step).max(1);
                    let shown_dots = plen.min(max_dots);
                    let dy = pb.1 + (pb.3.saturating_sub(dot_d)) / 2;
                    for d_i in 0..shown_dots {
                        fill_circle(fb, w, h, total, px0 + d_i * dot_step, dy, dot_d, p.title);
                    }
                    let cx = (px0 + shown_dots * dot_step + 2 * s).min(pb.0 + pb.2 - 5 * s);
                    fill_rect(fb, w, h, total, (cx, pb.1 + 5 * s, (2 * s).max(1), pb.3.saturating_sub(10 * s)), p.accent);
                }
            }

            if let Some(eb) = l.eye_btn {
                let hovered = view.hover == HitTarget::EyeToggle;
                fill_round(fb, w, h, total, eb, 5 * s, if hovered { p.btn_hover } else { p.btn_bg });
                stroke_round(fb, w, h, total, eb, 5 * s, s, if hovered || view.show_password { p.accent } else { p.rim });
                draw_icon(fb, w, h, total, 2 /* eye */, eb.0 + 5 * s, eb.1 + (eb.3.saturating_sub(11 * s)) / 2, 11 * s, if view.show_password { p.accent } else { p.ink });
                let lbl: &[u8] = if view.show_password { b"Hide" } else { b"Show" };
                text(fb, w, h, total, lbl, eb.0 + 18 * s, eb.1 + eb.3.saturating_sub(line) / 2, p.ink, sc, eb.2.saturating_sub(20 * s));
            }

            if let Some(rb) = l.remember_btn {
                let csize = 13 * s;
                let cy_box = rb.1 + (rb.3.saturating_sub(csize)) / 2;
                let cbox = (rb.0, cy_box, csize, csize);
                if view.remember_checked {
                    fill_round(fb, w, h, total, cbox, 3 * s, p.accent);
                    draw_icon(fb, w, h, total, 5 /* check */, cbox.0 + 2 * s, cbox.1 + 2 * s, 9 * s, p.on_accent);
                } else {
                    fill_round(fb, w, h, total, cbox, 3 * s, p.field_bg);
                    stroke_round(fb, w, h, total, cbox, 3 * s, s, if view.hover == HitTarget::RememberToggle { p.accent } else { p.rim });
                }
                text(fb, w, h, total, b"Save network", rb.0 + csize + 6 * s, rb.1 + rb.3.saturating_sub(line) / 2, p.subtext, sc, rb.2.saturating_sub(csize + 6 * s));
            }

            if let Some(cb) = l.cancel_btn {
                let hovered = view.hover == HitTarget::CancelPassword;
                fill_round(fb, w, h, total, cb, 5 * s, if hovered { p.btn_hover } else { p.btn_bg });
                stroke_round(fb, w, h, total, cb, 5 * s, s, if hovered { p.accent } else { p.rim });
                let lbl = b"Cancel";
                let lw = super::glyphs::ui_width(lbl, sc).unwrap_or(0);
                text(fb, w, h, total, lbl, cb.0 + cb.2.saturating_sub(lw) / 2, cb.1 + cb.3.saturating_sub(line) / 2, p.ink, sc, cb.2);
            }

            if let Some(jb) = l.join_btn {
                let valid = view.password_len as usize >= wp::PASSWORD_MIN;
                let hovered = view.hover == HitTarget::JoinSubmit;
                let jbg = if valid {
                    if hovered { crate::chrome_candy::mix_u8(p.accent, 0x00FF_FFFF, 35) } else { p.accent }
                } else if hovered {
                    p.btn_hover
                } else {
                    p.btn_bg
                };
                fill_round(fb, w, h, total, jb, 5 * s, jbg);
                if !valid {
                    stroke_round(fb, w, h, total, jb, 5 * s, s, p.rim);
                }
                let lbl = b"Join";
                let lw = super::glyphs::ui_width(lbl, sc).unwrap_or(0);
                text(fb, w, h, total, lbl, jb.0 + jb.2.saturating_sub(lw) / 2, jb.1 + jb.3.saturating_sub(line) / 2, if valid { p.on_accent } else { p.subtext }, sc, jb.2);
            }

            if !view.status().is_empty() {
                if let (Some(dr), Some(pb)) = (l.drawer, l.password_box) {
                    let msg_y = pb.1 + pb.3 + 2 * s;
                    text_fit(fb, w, h, total, view.status(), dr.0 + 8 * s, msg_y, p.danger, sc, dr.2.saturating_sub(16 * s), false);
                }
            }
        }
    }

    // 7. Footer Divider + Network Preferences
    let pr = l.prefs_row;
    fill_rect(fb, w, h, total, (pr.0, pr.1.saturating_sub(3 * s), pr.2, s), p.divider);
    if view.hover == HitTarget::NetworkPrefs {
        fill_round(fb, w, h, total, pr, 6 * s, p.hover);
        fill_rect(fb, w, h, total, (pr.0 + 2 * s, pr.1 + 5 * s, 3 * s, pr.3.saturating_sub(10 * s)), p.accent);
    }
    draw_icon(fb, w, h, total, 4 /* settings */, pr.0 + 8 * s, pr.1 + (pr.3.saturating_sub(13 * s)) / 2, 13 * s, p.subtext);
    text(fb, w, h, total, b"Network Preferences...", pr.0 + 26 * s, pr.1 + pr.3.saturating_sub(line) / 2, p.ink, sc, pr.2.saturating_sub(42 * s));
    text(fb, w, h, total, b">", pr.0 + pr.2.saturating_sub(14 * s), pr.1 + pr.3.saturating_sub(line) / 2, p.subtext, sc, 12 * s);

    true
}
