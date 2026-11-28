// SPDX-License-Identifier: GPL-3.0-only
//! Frosted macOS Noir / iOS Control Center Bento renderer for the Meta Super-Launcher
//! & Spotlight Quick-Settings Hub (`sex_pdx::meta_hub`).
//!
//! Shell drives all live state over `OP_META_HUB` (`0x302`); Display renders the
//! obsidian glass Bento card beside `SURFACE_ID_COMMAND_PALETTE` or Spotlight with
//! zero pixel overlap, using the shared `sex_pdx::meta_hub::Layout`.

use sex_pdx::meta_hub::{
    ACCENT_NAMES, DragSlider, HitTarget, Layout, MetaHubState, NIGHT_NAMES,
    POWER_BALANCED, POWER_GMODE, POWER_NAMES, QUICK_LAUNCH_COUNT, Receiver, Rect,
};
use sex_theme::{Mode, noir as nr};

use crate::chrome_candy::{mix_u8, noir_accent};
use crate::desk_theme::{current as desk, resolved_accent, resolved_fill_and_ink};

const SHELL_PD: u32 = 3;

const ICON_CAP: usize = 20;
const ICON_SLOTS: usize = 20;
const ICON_NAMES: [&str; ICON_SLOTS] = [
    "wifi",          // 0
    "wifi-off",      // 1
    "bluetooth",     // 2
    "power",         // 3
    "moon",          // 4
    "sun",           // 5
    "palette",       // 6
    "bell",          // 7
    "lock",          // 8
    "image",         // 9
    "volume",        // 10
    "volume-low",    // 11
    "volume-off",    // 12
    "play",          // 13
    "pause",         // 14
    "skip-previous", // 15
    "skip-next",     // 16
    "terminal",      // 17
    "settings",      // 18
    "chevron-right", // 19
];

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

static mut RECEIVER: Receiver = Receiver::new();

/// Handles an incoming `OP_META_HUB` PDX packet from Shell.
pub fn receive(caller: u32, arg0: u64, arg1: u64, arg2: u64) -> bool {
    if caller != SHELL_PD {
        return false;
    }
    let rx = unsafe { &mut *core::ptr::addr_of_mut!(RECEIVER) };
    let prev_vis = rx.current.visible;
    let Some(changed) = rx.receive(arg0, arg1, arg2) else {
        return false;
    };
    if rx.current.visible != prev_vis {
        static mut LOG_BUDGET: u32 = 32;
        unsafe {
            if LOG_BUDGET > 0 {
                LOG_BUDGET -= 1;
                sex_pdx::serial_record!(
                    "[display.meta_hub] visible={} mode={} wifi={} vol={} bright={}",
                    rx.current.visible as u8,
                    rx.current.mode,
                    rx.current.wifi_enabled as u8,
                    rx.current.volume_pct,
                    rx.current.brightness_pct
                );
            }
        }
    }
    changed
}

/// Outer frosted panel rectangle for `frost::glass_panels`, when visible & non-overlapping.
pub fn glass_rect(w: usize, h: usize, bar: usize, scale: u32) -> Option<Rect> {
    let st = unsafe { (*core::ptr::addr_of!(RECEIVER)).current };
    if !st.visible {
        return None;
    }
    Layout::compute(w, h, bar, scale, st.mode, st.anchor).map(|l| l.panel)
}

#[derive(Clone, Copy)]
struct Palette {
    bg: u32,
    tile_bg: u32,
    tile_hover: u32,
    trough: u32,
    rim: u32,
    specular: u32,
    divider: u32,
    ink: u32,
    subtext: u32,
    accent: u32,
    accent_fill: u32,
    on_accent: u32,
}

fn palette(st: &MetaHubState) -> Palette {
    let d = desk();
    let accent = if d.mode == Mode::HighContrast {
        d.focus
    } else if d.accent == sex_theme::DEFAULT_ACCENT {
        noir_accent(st.noir_accent)
    } else {
        resolved_accent()
    };
    let (accent_fill, on_accent) = if d.mode == Mode::HighContrast {
        (d.menu_send, d.menu_on_send)
    } else if d.accent == sex_theme::DEFAULT_ACCENT {
        let t = sex_theme::NoirAccent::from_index(st.noir_accent);
        (t.fill() & 0x00FF_FFFF, t.on_accent() & 0x00FF_FFFF)
    } else {
        resolved_fill_and_ink()
    };
    match d.mode {
        Mode::HighContrast => Palette {
            bg: d.menu,
            tile_bg: d.menu_button,
            tile_hover: d.menu_picked,
            trough: d.menu,
            rim: d.menu_rim,
            specular: d.menu_rim,
            divider: d.menu_rim,
            ink: d.menu_ink,
            subtext: d.menu_soft,
            accent,
            accent_fill,
            on_accent,
        },
        Mode::Dark => Palette {
            bg: 0x000B_0C10,
            tile_bg: 0x0016_1821,
            tile_hover: 0x0021_2533,
            trough: 0x0013_1620,
            rim: 0x002A_2D37,
            specular: nr::DARK_SPECULAR & 0x00FF_FFFF,
            divider: 0x0023_2734,
            ink: 0x00F1_F5F9,
            subtext: 0x0094_A3B8,
            accent,
            accent_fill,
            on_accent,
        },
        Mode::Light => Palette {
            bg: nr::LIGHT_MENU_BG & 0x00FF_FFFF,
            tile_bg: nr::LIGHT_SPOTLIGHT_TILE & 0x00FF_FFFF,
            tile_hover: nr::LIGHT_MENU_HOVER & 0x00FF_FFFF,
            trough: 0x00E2_E8F0,
            rim: nr::LIGHT_MENU_RIM & 0x00FF_FFFF,
            specular: nr::LIGHT_SPECULAR & 0x00FF_FFFF,
            divider: nr::LIGHT_MENU_DIVIDER & 0x00FF_FFFF,
            ink: nr::LIGHT_TEXT & 0x00FF_FFFF,
            subtext: nr::LIGHT_MENU_SUBTEXT & 0x00FF_FFFF,
            accent,
            accent_fill,
            on_accent,
        },
    }
}

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
                    core::ptr::write_volatile(p, mix_u8(under, tint, alpha));
                }
            }
        }
    }
}

fn fill_rounded(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, rad: usize, c: u32) {
    if r.2 == 0 || r.3 == 0 {
        return;
    }
    let rad = rad.min(r.2 / 2).min(r.3 / 2);
    let r2 = (rad * rad) as isize;
    for dy in 0..r.3 {
        let y = r.1 + dy;
        if y >= h {
            break;
        }
        let cy = if dy < rad {
            rad - dy
        } else if dy + rad >= r.3 {
            dy + rad + 1 - r.3
        } else {
            0
        };
        for dx in 0..r.2 {
            let x = r.0 + dx;
            if x >= w {
                break;
            }
            if cy > 0 {
                let cx = if dx < rad {
                    rad - dx
                } else if dx + rad >= r.2 {
                    dx + rad + 1 - r.2
                } else {
                    0
                };
                if cx > 0 && (cx * cx + cy * cy) as isize > r2 {
                    continue;
                }
            }
            put_px(fb, w, h, total, x, y, c);
        }
    }
}

/// Fills up to `fill_w` horizontal pixels clipped inside rounded capsule `r`.
fn fill_rounded_progress(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    r: Rect,
    rad: usize,
    fill_w: usize,
    c: u32,
) {
    if r.2 == 0 || r.3 == 0 || fill_w == 0 {
        return;
    }
    let fill_w = fill_w.min(r.2);
    let rad = rad.min(r.2 / 2).min(r.3 / 2);
    let r2 = (rad * rad) as isize;
    for dy in 0..r.3 {
        let y = r.1 + dy;
        if y >= h {
            break;
        }
        let cy = if dy < rad {
            rad - dy
        } else if dy + rad >= r.3 {
            dy + rad + 1 - r.3
        } else {
            0
        };
        for dx in 0..fill_w {
            let x = r.0 + dx;
            if x >= w {
                break;
            }
            if cy > 0 {
                let cx = if dx < rad {
                    rad - dx
                } else if dx + rad >= r.2 {
                    dx + rad + 1 - r.2
                } else {
                    0
                };
                if cx > 0 && (cx * cx + cy * cy) as isize > r2 {
                    continue;
                }
            }
            put_px(fb, w, h, total, x, y, c);
        }
    }
}

fn stroke_rounded(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, rad: usize, thick: usize, c: u32) {
    if r.2 == 0 || r.3 == 0 {
        return;
    }
    let rad = rad.min(r.2 / 2).min(r.3 / 2);
    let outer_r2 = (rad * rad) as isize;
    let inner_rad = rad.saturating_sub(thick);
    let inner_r2 = (inner_rad * inner_rad) as isize;
    for dy in 0..r.3 {
        let y = r.1 + dy;
        if y >= h {
            break;
        }
        let in_y_band = dy < thick || dy + thick >= r.3;
        let cy = if dy < rad {
            rad - dy
        } else if dy + rad >= r.3 {
            dy + rad + 1 - r.3
        } else {
            0
        };
        for dx in 0..r.2 {
            let x = r.0 + dx;
            if x >= w {
                break;
            }
            let in_x_band = dx < thick || dx + thick >= r.2;
            let cx = if dx < rad {
                rad - dx
            } else if dx + rad >= r.2 {
                dx + rad + 1 - r.2
            } else {
                0
            };
            if cx > 0 && cy > 0 {
                let d2 = (cx * cx + cy * cy) as isize;
                if d2 <= outer_r2 && d2 >= inner_r2 {
                    put_px(fb, w, h, total, x, y, c);
                }
            } else if in_x_band || in_y_band {
                put_px(fb, w, h, total, x, y, c);
            }
        }
    }
}

#[allow(static_mut_refs)]
fn draw_icon(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    slot: usize,
    x0: usize,
    y0: usize,
    side: usize,
    ink: u32,
) {
    let side = side.clamp(8, ICON_CAP);
    if slot >= ICON_SLOTS {
        return;
    }
    let cache = unsafe { &mut *core::ptr::addr_of_mut!(ICONS) };
    if cache.sides[slot] != side {
        let Some(data) = sex_icons::find(sex_icons::Kind::Line, ICON_NAMES[slot]) else {
            return;
        };
        let place = sex_icons::Place::square(side);
        let stroke = (1.8 * place.scale).max(1.2) / place.scale;
        let mask = &mut cache.masks[slot][..side * side];
        sex_icons::coverage(data, place, stroke, side, side, &mut cache.acc, mask);
        cache.sides[slot] = side;
    }
    let mask = &cache.masks[slot][..side * side];
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
            if a != 0 {
                blend_px(fb, w, h, total, px, py, ink, a);
            }
        }
    }
}

/// Crisp procedural Microphone glyph (plus mute slash when muted).
fn draw_mic_icon(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    cx: usize,
    cy: usize,
    s: usize,
    ink: u32,
    muted: bool,
) {
    // Capsule head
    let cap_w = (5 * s).max(4);
    let cap_h = (8 * s).max(6);
    fill_rounded(fb, w, h, total, (cx.saturating_sub(cap_w / 2), cy.saturating_sub(cap_h / 2 + s), cap_w, cap_h), cap_w / 2, ink);
    // U-arc cradle + stem + base
    let cradle_w = cap_w + 4 * s;
    let cradle_x = cx.saturating_sub(cradle_w / 2);
    let cradle_top = cy.saturating_sub(s);
    let cradle_bot = cy + 4 * s;
    fill_rect(fb, w, h, total, (cradle_x, cradle_top, s.max(1), cradle_bot - cradle_top), ink);
    fill_rect(fb, w, h, total, (cradle_x + cradle_w - s.max(1), cradle_top, s.max(1), cradle_bot - cradle_top), ink);
    fill_rect(fb, w, h, total, (cradle_x, cradle_bot, cradle_w, s.max(1)), ink);
    fill_rect(fb, w, h, total, (cx.saturating_sub(s / 2), cradle_bot, s.max(1), 3 * s), ink);
    fill_rect(fb, w, h, total, (cx.saturating_sub(3 * s), cradle_bot + 3 * s, 6 * s, s.max(1)), ink);
    if muted {
        let span = (6 * s) as isize;
        for d in -span..=span {
            for t in 0..=s {
                let px = (cx as isize + d).max(0) as usize + t;
                let py = (cy as isize + d).max(0) as usize;
                put_px(fb, w, h, total, px, py, ink);
            }
        }
    }
}

/// Crisp shopping-bag vector mark for SexShop quick launcher.
fn draw_shop_icon(
    fb: *mut u32,
    w: usize,
    h: usize,
    total: usize,
    x0: usize,
    y0: usize,
    side: usize,
    ink: u32,
) {
    let bw = (side * 3 / 4).max(8);
    let bh = (side * 5 / 8).max(7);
    let bx = x0 + side.saturating_sub(bw) / 2;
    let by = y0 + side.saturating_sub(bh);
    let hw = (bw / 2).max(4);
    let hx = bx + bw.saturating_sub(hw) / 2;
    let hy = y0 + 1;
    let hh = by.saturating_sub(hy);
    stroke_rounded(fb, w, h, total, (hx, hy, hw, hh + 2), hw / 2, 1, ink);
    stroke_rounded(fb, w, h, total, (bx, by, bw, bh), 2, 1, ink);
    fill_rect(fb, w, h, total, (bx + 2, by + 2, bw.saturating_sub(4), bh.saturating_sub(4)), mix_u8(0x0010_1218, ink, 50));
}

fn draw_text_ui(
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
    let width = if semibold {
        super::glyphs::ui_width(t, scale)
    } else {
        super::glyphs::ui_body_width(t, scale)
    }
    .unwrap_or(0)
    .min(max_w);
    let lh = super::glyphs::UI_H * scale as usize;
    for y in y0..(y0 + lh).min(h) {
        for x in x0..(x0 + width).min(w) {
            let at = y * w + x;
            if at >= total {
                return;
            }
            let cov = if semibold {
                super::glyphs::ui_coverage(t, (x - x0) as i32, y - y0, scale)
            } else {
                super::glyphs::ui_body_coverage(t, (x - x0) as i32, y - y0, scale)
            };
            let Some(alpha) = cov else {
                break;
            };
            if alpha != 0 {
                blend_px(fb, w, h, total, x, y, ink, alpha);
            }
        }
    }
}

fn draw_caption(
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
    if scale >= 2 {
        draw_text_ui(fb, w, h, total, t, x0, y0, ink, 1, max_w, false);
        return;
    }
    let width = super::glyphs::caption_width(t).unwrap_or(0).min(max_w);
    let lh = super::glyphs::CAPTION_H;
    for y in y0..(y0 + lh).min(h) {
        for x in x0..(x0 + width).min(w) {
            let at = y * w + x;
            if at >= total {
                return;
            }
            let Some(alpha) = super::glyphs::caption_coverage(t, (x - x0) as i32, y - y0) else {
                break;
            };
            if alpha != 0 {
                blend_px(fb, w, h, total, x, y, ink, alpha);
            }
        }
    }
}

fn format_pct(v: u8, out: &mut [u8; 4]) -> usize {
    let v = v.min(100);
    if v >= 100 {
        out.copy_from_slice(b"100%");
        4
    } else if v >= 10 {
        out[0] = b'0' + (v / 10);
        out[1] = b'0' + (v % 10);
        out[2] = b'%';
        3
    } else {
        out[0] = b'0' + v;
        out[1] = b'%';
        2
    }
}

/// Renders the Frosted macOS Noir / iOS Control Center Bento Hub when visible.
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, scale: u32, bar: usize) {
    let st = unsafe { (*core::ptr::addr_of!(RECEIVER)).current };
    if !st.visible {
        return;
    }
    let Some(l) = Layout::compute(w, h, bar, scale, st.mode, st.anchor) else {
        return;
    };

    let sc = l.scale;
    let s = sc as usize;
    let p = palette(&st);
    let inner = crate::frost::inside(l.panel, s);
    let rad = (12 * s).min(l.panel.2 / 8);

    // 1. Subtle exterior shadow halo + Obsidian Frosted Glass backdrop + specular rim.
    if desk().mode != Mode::HighContrast {
        tint_glass(
            fb,
            w,
            h,
            total,
            (
                l.panel.0 + 2 * s,
                (l.panel.1 + l.panel.3).min(h),
                l.panel.2.saturating_sub(4 * s),
                (4 * s).min(h.saturating_sub(l.panel.1 + l.panel.3)),
            ),
            0x0000_0000,
            90,
        );
    }
    crate::frost::glass_or(fb, w, h, total, crate::frost::Panel::MetaHub, inner, sc, bar, p.bg);
    if desk().mode != Mode::HighContrast {
        tint_glass(
            fb,
            w,
            h,
            total,
            inner,
            p.bg,
            if desk().mode == Mode::Dark { 120 } else { 88 },
        );
        tint_glass(
            fb,
            w,
            h,
            total,
            (inner.0, inner.1, inner.2, s),
            p.specular,
            if desk().mode == Mode::Dark { 44 } else { 90 },
        );
    }
    crate::frost::rim(fb, w, h, total, l.panel, s, p.rim);

    let ui_h = super::glyphs::UI_H * s;
    let cap_h = if sc >= 2 { super::glyphs::UI_H } else { super::glyphs::CAPTION_H };
    let hov = st.hover_target;

    // 2. Header Strip: SexOS mark + "CONTROL HUB" + live Workspace pill + "SUPER" hint pill.
    let hdr = l.header;
    let badge_sz = (12 * s).min(hdr.3.saturating_sub(4 * s));
    let badge_y = hdr.1 + hdr.3.saturating_sub(badge_sz) / 2;
    fill_rounded(fb, w, h, total, (hdr.0, badge_y, badge_sz, badge_sz), badge_sz / 2, p.accent_fill);
    fill_rounded(
        fb,
        w,
        h,
        total,
        (hdr.0 + badge_sz / 4, badge_y + badge_sz / 4, badge_sz / 2, badge_sz / 2),
        badge_sz / 4,
        p.on_accent,
    );

    let title_x = hdr.0 + badge_sz + 7 * s;
    let title_y = hdr.1 + hdr.3.saturating_sub(ui_h) / 2;
    let title_bytes: &[u8] = if l.compact { b"HUB" } else { b"CONTROL HUB" };
    let title_max = l.space_pill.0.saturating_sub(title_x + 6 * s);
    draw_text_ui(fb, w, h, total, title_bytes, title_x, title_y, p.ink, sc, title_max, true);

    // Battery / window count subtle tag if room allows before Space pill
    let title_w = super::glyphs::ui_width(title_bytes, sc).unwrap_or(0);
    if !l.compact && st.battery_known && title_x + title_w + 40 * s < l.space_pill.0 {
        let mut btxt = [b' '; 6];
        let mut tmp = [0u8; 4];
        let bl = format_pct(st.battery_pct, &mut tmp);
        btxt[..bl].copy_from_slice(&tmp[..bl]);
        if st.charging && bl < 6 {
            btxt[bl] = b'+';
        }
        let bcol = if st.charging { 0x0034_D399 } else { p.subtext };
        let b_y = hdr.1 + hdr.3.saturating_sub(cap_h) / 2;
        draw_caption(fb, w, h, total, &btxt[..bl + st.charging as usize], title_x + title_w + 6 * s, b_y, bcol, sc, 44 * s);
    }

    // Space N pill
    let sp = l.space_pill;
    let sp_hov = hov == HitTarget::SpacePill.hover_code();
    let sp_bg = if sp_hov { p.tile_hover } else { p.tile_bg };
    fill_rounded(fb, w, h, total, sp, 8 * s, sp_bg);
    stroke_rounded(fb, w, h, total, sp, 8 * s, s, if sp_hov { p.accent } else { p.divider });
    let sp_txt = [b'S', b'p', b'a', b'c', b'e', b' ', b'1' + st.active_space.min(4)];
    let sp_w = if sc >= 2 {
        super::glyphs::ui_body_width(&sp_txt, 1).unwrap_or(0)
    } else {
        super::glyphs::caption_width(&sp_txt).unwrap_or(0)
    };
    draw_caption(
        fb,
        w,
        h,
        total,
        &sp_txt,
        sp.0 + sp.2.saturating_sub(sp_w) / 2,
        sp.1 + sp.3.saturating_sub(cap_h) / 2,
        p.ink,
        sc,
        sp.2.saturating_sub(4 * s),
    );

    // SUPER keycap hint pill on far right of Header
    let sup_x = sp.0 + sp.2 + 4 * s;
    let sup_w = (hdr.0 + hdr.2).saturating_sub(sup_x);
    if sup_w >= 26 * s {
        let sup_r = (sup_x, sp.1, sup_w, sp.3);
        fill_rounded(fb, w, h, total, sup_r, 8 * s, mix_u8(p.tile_bg, p.accent, 36));
        stroke_rounded(fb, w, h, total, sup_r, 8 * s, s, p.accent);
        let stxt = b"SUPER";
        let sw = if sc >= 2 {
            super::glyphs::ui_body_width(stxt, 1).unwrap_or(0)
        } else {
            super::glyphs::caption_width(stxt).unwrap_or(0)
        };
        draw_caption(
            fb,
            w,
            h,
            total,
            stxt,
            sup_r.0 + sup_r.2.saturating_sub(sw) / 2,
            sup_r.1 + sup_r.3.saturating_sub(cap_h) / 2,
            p.accent,
            sc,
            sup_r.2.saturating_sub(4 * s),
        );
    }

    // 3. 2x3 Quick-Toggle Bento Grid.
    let icon_side = (if l.compact { 15 * s } else { 18 * s }).clamp(12, ICON_CAP);
    let tile_rad = 10 * s;

    let draw_bento_tile = |rect: Rect,
                           active: bool,
                           active_fill: u32,
                           active_ink: u32,
                           hovered: bool,
                           icon_slot: usize,
                           title: &[u8],
                           sub: &[u8],
                           right_reserve: usize| {
        let bg = if active {
            if hovered { mix_u8(active_fill, 0x00FF_FFFF, 28) } else { active_fill }
        } else if hovered {
            p.tile_hover
        } else {
            p.tile_bg
        };
        fill_rounded(fb, w, h, total, rect, tile_rad, bg);
        if active && desk().mode != Mode::HighContrast {
            tint_glass(fb, w, h, total, (rect.0 + 4 * s, rect.1, rect.2.saturating_sub(8 * s), s), 0x00FF_FFFF, 36);
        }
        stroke_rounded(fb, w, h, total, rect, tile_rad, s, if hovered { p.accent } else if active { mix_u8(active_fill, 0x00FF_FFFF, 50) } else { p.divider });

        let fg = if active { active_ink } else { p.ink };
        let sub_fg = if active { mix_u8(active_ink, active_fill, 48) } else { p.subtext };

        let ix = rect.0 + (if l.compact { 7 * s } else { 10 * s });
        let iy = rect.1 + rect.3.saturating_sub(icon_side) / 2;
        draw_icon(fb, w, h, total, icon_slot, ix, iy, icon_side, fg);

        let tx = ix + icon_side + (if l.compact { 6 * s } else { 8 * s });
        let tmax = rect.0 + rect.2.saturating_sub(right_reserve + 4 * s) - tx.min(rect.0 + rect.2);
        let stack_h = ui_h + cap_h + s;
        let ty = rect.1 + rect.3.saturating_sub(stack_h) / 2;
        draw_text_ui(fb, w, h, total, title, tx, ty, fg, sc, tmax, true);
        draw_caption(fb, w, h, total, sub, tx, ty + ui_h + s, sub_fg, sc, tmax);
    };

    // Tile 0: Wi-Fi
    let wifi_active = st.wifi_enabled;
    let wifi_sub: &[u8] = if !wifi_active {
        b"Off"
    } else if st.wifi_connected && st.ssid_len > 0 {
        st.ssid_str().as_bytes()
    } else if st.wifi_connected {
        b"Connected"
    } else {
        b"On"
    };
    let wifi_fill = if st.wifi_connected { 0x000E_A5E9 } else { p.accent_fill };
    let wifi_ink = if st.wifi_connected { 0x00FF_FFFF } else { p.on_accent };
    let wifi_hov = hov == HitTarget::TileWifi.hover_code();
    draw_bento_tile(
        l.tiles[0],
        wifi_active,
        wifi_fill,
        wifi_ink,
        wifi_hov,
        if wifi_active { 0 } else { 1 },
        b"Wi-Fi",
        wifi_sub,
        l.tile_wifi_expand.2 + 2 * s,
    );
    // Small chevron button on Wi-Fi top-right for Scanner Popover
    let we = l.tile_wifi_expand;
    let we_hov = hov == HitTarget::TileWifiExpand.hover_code();
    let we_bg = if we_hov {
        p.tile_hover
    } else if wifi_active {
        mix_u8(wifi_fill, 0x0000_0000, 45)
    } else {
        mix_u8(p.tile_bg, 0x00FF_FFFF, 24)
    };
    fill_rounded(fb, w, h, total, we, we.3 / 2, we_bg);
    let chev_s = (12 * s).min(we.2.saturating_sub(2 * s)).max(8);
    draw_icon(
        fb,
        w,
        h,
        total,
        19,
        we.0 + we.2.saturating_sub(chev_s) / 2,
        we.1 + we.3.saturating_sub(chev_s) / 2,
        chev_s,
        if we_hov { p.accent } else if wifi_active { wifi_ink } else { p.ink },
    );

    // Tile 1: Bluetooth
    let bt_active = st.bt_enabled;
    let bt_sub: &[u8] = if !st.bt_available {
        if bt_active { b"On" } else { b"Ready" }
    } else if st.bt_connected_count > 0 {
        b"Connected"
    } else if bt_active {
        b"On"
    } else {
        b"Off"
    };
    draw_bento_tile(
        l.tiles[1],
        bt_active,
        0x003B_82F6,
        0x00FF_FFFF,
        hov == HitTarget::TileBluetooth.hover_code(),
        2,
        if l.compact { b"BT" } else { b"Bluetooth" },
        bt_sub,
        0,
    );

    // Tile 2: Power / Performance / G-Mode
    let pmode = (st.power_mode as usize).min(3);
    let p_active = st.power_mode != POWER_BALANCED;
    let (p_fill, p_ink) = match st.power_mode {
        0 => (0x0010_B981, 0x00FF_FFFF),       // Quiet Emerald
        2 => (0x00F5_9E0B, 0x000B_0C10),       // Performance Amber
        POWER_GMODE => (0x00E1_1D48, 0x00FF_FFFF), // G-Mode Crimson Turbo
        _ => (p.tile_bg, p.ink),
    };
    draw_bento_tile(
        l.tiles[2],
        p_active,
        p_fill,
        p_ink,
        hov == HitTarget::TilePower.hover_code(),
        3,
        b"Power",
        POWER_NAMES[pmode].as_bytes(),
        0,
    );

    // Tile 3: Night Light
    let nl = (st.night_light_level as usize).min(3);
    draw_bento_tile(
        l.tiles[3],
        nl > 0,
        0x00F5_9E0B,
        0x000B_0C10,
        hov == HitTarget::TileNightLight.hover_code(),
        4,
        if l.compact { b"Night" } else { b"Night Light" },
        NIGHT_NAMES[nl].as_bytes(),
        0,
    );

    // Tile 4: Noir Accent
    let acc_idx = (st.noir_accent as usize).min(3);
    let acc_col = noir_accent(st.noir_accent);
    draw_bento_tile(
        l.tiles[4],
        false,
        p.tile_bg,
        p.ink,
        hov == HitTarget::TileAccent.hover_code(),
        6,
        b"Accent",
        ACCENT_NAMES[acc_idx].as_bytes(),
        14 * s,
    );
    // Live accent swatch dot on right side of Accent tile
    let t4 = l.tiles[4];
    let dot_sz = (12 * s).min(t4.3 / 3);
    let dx = t4.0 + t4.2.saturating_sub(dot_sz + 8 * s);
    let dy = t4.1 + t4.3.saturating_sub(dot_sz) / 2;
    fill_rounded(fb, w, h, total, (dx, dy, dot_sz, dot_sz), dot_sz / 2, acc_col);
    stroke_rounded(fb, w, h, total, (dx, dy, dot_sz, dot_sz), dot_sz / 2, s, 0x00FF_FFFF);

    // Tile 5: Focus / DND + mini Lock & Snip utility buttons
    let t5 = l.tiles[5];
    let dnd_active = st.dnd_enabled;
    let dnd_hov = hov == HitTarget::TileDnd.hover_code();
    let dnd_bg = if dnd_active {
        0x008B_5CF6
    } else if dnd_hov {
        p.tile_hover
    } else {
        p.tile_bg
    };
    fill_rounded(fb, w, h, total, t5, tile_rad, dnd_bg);
    stroke_rounded(fb, w, h, total, t5, tile_rad, s, if dnd_hov { p.accent } else { p.divider });
    let dnd_fg = if dnd_active { 0x00FF_FFFF } else { p.ink };
    let dnd_sub_fg = if dnd_active { 0x00ED_E9FE } else { p.subtext };
    let dnd_ix = t5.0 + (if l.compact { 6 * s } else { 9 * s });
    let dnd_iy = t5.1 + t5.3.saturating_sub(icon_side) / 2;
    draw_icon(fb, w, h, total, 7, dnd_ix, dnd_iy, icon_side, dnd_fg);
    let dnd_tx = dnd_ix + icon_side + (if l.compact { 5 * s } else { 7 * s });
    let dnd_tmax = l.tile_dnd_toggle.0 + l.tile_dnd_toggle.2.saturating_sub(2 * s) - dnd_tx.min(l.tile_dnd_toggle.0 + l.tile_dnd_toggle.2);
    let stack_h = ui_h + cap_h + s;
    let dnd_ty = t5.1 + t5.3.saturating_sub(stack_h) / 2;
    draw_text_ui(fb, w, h, total, b"Focus", dnd_tx, dnd_ty, dnd_fg, sc, dnd_tmax, true);
    draw_caption(fb, w, h, total, if dnd_active { b"DND On" } else { b"Off" }, dnd_tx, dnd_ty + ui_h + s, dnd_sub_fg, sc, dnd_tmax);

    // Mini Lock & Snip buttons inside Focus card
    for &(br, bslot, bhov) in &[
        (l.btn_lock, 8usize, hov == HitTarget::BtnLock.hover_code()),
        (l.btn_shot, 9usize, hov == HitTarget::BtnShot.hover_code()),
    ] {
        let m_bg = if bhov {
            p.accent_fill
        } else if dnd_active {
            mix_u8(0x008B_5CF6, 0x0000_0000, 60)
        } else {
            mix_u8(p.tile_bg, 0x00FF_FFFF, 24)
        };
        let m_ink = if bhov { p.on_accent } else { dnd_fg };
        fill_rounded(fb, w, h, total, br, 6 * s, m_bg);
        stroke_rounded(fb, w, h, total, br, 6 * s, s, if bhov { p.accent } else { p.divider });
        let b_ico = br.3.saturating_sub(6 * s).clamp(10, ICON_CAP);
        draw_icon(
            fb,
            w,
            h,
            total,
            bslot,
            br.0 + br.2.saturating_sub(b_ico) / 2,
            br.1 + br.3.saturating_sub(b_ico) / 2,
            b_ico,
            m_ink,
        );
    }

    // 4. Display Brightness Pill Slider.
    let sb = l.slider_brightness;
    let sb_rad = sb.3 / 2;
    let sb_hov = hov == HitTarget::SliderBrightness.hover_code()
        || st.active_drag_slider == DragSlider::Brightness;
    fill_rounded(fb, w, h, total, sb, sb_rad, p.trough);
    let bright_fill_w = if st.brightness_pct == 0 {
        0
    } else {
        (sb.2 * st.brightness_pct.min(100) as usize / 100).max(sb.3 / 2)
    };
    let bright_bar_col = if desk().mode == Mode::HighContrast { p.accent_fill } else { 0x00F8_FAFC };
    fill_rounded_progress(fb, w, h, total, sb, sb_rad, bright_fill_w, bright_bar_col);
    stroke_rounded(fb, w, h, total, sb, sb_rad, s, if sb_hov { p.accent } else { p.rim });

    let sun_side = (16 * s).min(sb.3.saturating_sub(8 * s)).max(10);
    let sun_x = sb.0 + 10 * s;
    let sun_y = sb.1 + sb.3.saturating_sub(sun_side) / 2;
    let sun_on_fill = bright_fill_w > (sun_x - sb.0 + sun_side / 2);
    let sun_ink = if sun_on_fill && desk().mode != Mode::HighContrast {
        0x000F_172A
    } else {
        p.ink
    };
    draw_icon(fb, w, h, total, 5, sun_x, sun_y, sun_side, sun_ink);

    let mut pct_buf = [0u8; 4];
    let pct_n = format_pct(st.brightness_pct, &mut pct_buf);
    let pw = super::glyphs::ui_width(&pct_buf[..pct_n], sc).unwrap_or(0);
    let px_lbl = sb.0 + sb.2.saturating_sub(pw + 12 * s);
    let py_lbl = sb.1 + sb.3.saturating_sub(ui_h) / 2;
    let pct_on_fill = bright_fill_w > (px_lbl - sb.0 + pw / 2);
    let pct_ink = if pct_on_fill && desk().mode != Mode::HighContrast {
        0x000F_172A
    } else {
        p.ink
    };
    draw_text_ui(fb, w, h, total, &pct_buf[..pct_n], px_lbl, py_lbl, pct_ink, sc, pw + 4 * s, true);
    let disp_x = sun_x + sun_side + 8 * s;
    let disp_max = px_lbl.saturating_sub(disp_x + 4 * s);
    if disp_max >= 28 * s {
        let disp_on_fill = bright_fill_w > (disp_x - sb.0 + 16 * s);
        let d_ink = if disp_on_fill && desk().mode != Mode::HighContrast {
            0x001E_293B
        } else {
            p.subtext
        };
        draw_caption(fb, w, h, total, b"Display", disp_x, sb.1 + sb.3.saturating_sub(cap_h) / 2, d_ink, sc, disp_max);
    }

    // 5. Sound Master Volume Pill Slider + Speaker Mute Cap + Dedicated Mic Mute Pill.
    let vp = l.volume_pill;
    let vp_rad = vp.3 / 2;
    let vol_hov = hov == HitTarget::SliderVolume.hover_code()
        || st.active_drag_slider == DragSlider::Volume;
    let spk_hov = hov == HitTarget::BtnSpeakerMute.hover_code();
    fill_rounded(fb, w, h, total, vp, vp_rad, p.trough);

    let vol_effective = if st.speaker_muted { 0 } else { st.volume_pct.min(100) };
    let bar_fill_w = if vol_effective == 0 {
        0
    } else {
        (l.btn_speaker_mute.2 + l.slider_volume.2 * vol_effective as usize / 100).min(vp.2)
    };
    if bar_fill_w > 0 {
        fill_rounded_progress(fb, w, h, total, vp, vp_rad, bar_fill_w, p.accent_fill);
    }
    stroke_rounded(fb, w, h, total, vp, vp_rad, s, if vol_hov || spk_hov { p.accent } else { p.rim });

    // Speaker mute left cap highlight
    if spk_hov {
        fill_rounded(
            fb,
            w,
            h,
            total,
            (l.btn_speaker_mute.0 + 2 * s, l.btn_speaker_mute.1 + 2 * s, l.btn_speaker_mute.2.saturating_sub(4 * s), l.btn_speaker_mute.3.saturating_sub(4 * s)),
            vp_rad.saturating_sub(2 * s),
            mix_u8(p.trough, 0x00FF_FFFF, 35),
        );
    }
    let spk_slot = if st.speaker_muted || st.volume_pct == 0 {
        12
    } else if st.volume_pct < 45 {
        11
    } else {
        10
    };
    let spk_ink = if st.speaker_muted {
        0x00FB_7185
    } else if bar_fill_w > sun_side + 10 * s {
        p.on_accent
    } else {
        p.ink
    };
    draw_icon(
        fb,
        w,
        h,
        total,
        spk_slot,
        l.btn_speaker_mute.0 + l.btn_speaker_mute.2.saturating_sub(sun_side) / 2,
        l.btn_speaker_mute.1 + l.btn_speaker_mute.3.saturating_sub(sun_side) / 2,
        sun_side,
        spk_ink,
    );

    let vn = if st.speaker_muted {
        3
    } else {
        format_pct(st.volume_pct, &mut pct_buf)
    };
    let vbytes: &[u8] = if st.speaker_muted { b"Mute" } else { &pct_buf[..vn] };
    let vlen = if st.speaker_muted { 4 } else { vn };
    let vw = super::glyphs::ui_width(&vbytes[..vlen], sc).unwrap_or(0);
    let vx_lbl = vp.0 + vp.2.saturating_sub(vw + 12 * s);
    let vy_lbl = vp.1 + vp.3.saturating_sub(ui_h) / 2;
    let v_on_fill = bar_fill_w > (vx_lbl - vp.0 + vw / 2);
    let v_ink = if st.speaker_muted {
        0x00FB_7185
    } else if v_on_fill {
        p.on_accent
    } else {
        p.ink
    };
    draw_text_ui(fb, w, h, total, &vbytes[..vlen], vx_lbl, vy_lbl, v_ink, sc, vw + 4 * s, true);

    // Mic Mute pill
    let mm = l.btn_mic_mute;
    let mm_hov = hov == HitTarget::BtnMicMute.hover_code();
    let mm_bg = if st.mic_muted {
        0x00E1_1D48
    } else if mm_hov {
        p.tile_hover
    } else {
        p.tile_bg
    };
    let mm_ink = if st.mic_muted { 0x00FF_FFFF } else { p.ink };
    fill_rounded(fb, w, h, total, mm, mm.3 / 2, mm_bg);
    stroke_rounded(fb, w, h, total, mm, mm.3 / 2, s, if mm_hov { p.accent } else if st.mic_muted { 0x00FB_7185 } else { p.rim });
    draw_mic_icon(
        fb,
        w,
        h,
        total,
        mm.0 + mm.2 / 2,
        mm.1 + mm.3 / 2,
        s,
        mm_ink,
        st.mic_muted,
    );

    // 6. Media & Level Transport Bar.
    let mc = l.media_card;
    fill_rounded(fb, w, h, total, mc, 10 * s, p.tile_bg);
    stroke_rounded(fb, w, h, total, mc, 10 * s, s, p.divider);

    // Tactile 3-bar level indicator
    let eq_x = mc.0 + 10 * s;
    let eq_bot = mc.1 + mc.3.saturating_sub(9 * s);
    let eq_heights = if st.speaker_muted || st.volume_pct == 0 {
        [3 * s, 3 * s, 3 * s]
    } else if st.media_playing {
        [14 * s, 18 * s, 11 * s]
    } else {
        let scale_bar = (st.volume_pct as usize * 12 * s / 100).max(4 * s);
        [scale_bar * 2 / 3, scale_bar, scale_bar / 2]
    };
    let eq_col = if st.speaker_muted { 0x00FB_7185 } else { p.accent };
    for (bi, &bh) in eq_heights.iter().enumerate() {
        let bx = eq_x + bi * 4 * s;
        let clamped_h = bh.min(mc.3.saturating_sub(14 * s)).max(2 * s);
        fill_rounded(fb, w, h, total, (bx, eq_bot.saturating_sub(clamped_h), 3 * s, clamped_h), s, eq_col);
    }

    let m_tx = eq_x + 16 * s;
    let m_max = l.media_prev.0.saturating_sub(m_tx + 4 * s);
    let m_label: &[u8] = if st.media_playing { b"Playing" } else { b"Media" };
    draw_text_ui(fb, w, h, total, m_label, m_tx, mc.1 + mc.3.saturating_sub(ui_h) / 2, p.ink, sc, m_max, true);

    for &(mr, slot, hit_id, is_primary) in &[
        (l.media_prev, 15usize, HitTarget::MediaPrev.hover_code(), false),
        (l.media_play_pause, if st.media_playing { 14 } else { 13 }, HitTarget::MediaPlayPause.hover_code(), true),
        (l.media_next, 16usize, HitTarget::MediaNext.hover_code(), false),
    ] {
        let mh = hov == hit_id;
        let mbg = if is_primary {
            if mh { mix_u8(p.accent_fill, 0x00FF_FFFF, 30) } else { p.accent_fill }
        } else if mh {
            p.tile_hover
        } else {
            mix_u8(p.tile_bg, 0x00FF_FFFF, 18)
        };
        let mink = if is_primary { p.on_accent } else { p.ink };
        fill_rounded(fb, w, h, total, mr, mr.3 / 2, mbg);
        if mh {
            stroke_rounded(fb, w, h, total, mr, mr.3 / 2, s, p.accent);
        }
        let m_ico = mr.3.saturating_sub(8 * s).clamp(10, ICON_CAP);
        draw_icon(
            fb,
            w,
            h,
            total,
            slot,
            mr.0 + mr.2.saturating_sub(m_ico) / 2,
            mr.1 + mr.3.saturating_sub(m_ico) / 2,
            m_ico,
            mink,
        );
    }

    // 7. Super-Launcher Quick Actions Dock Row (6 tiles: Spindle, Settings, SexShop, Wi-Fi, Lock, Snip).
    let q_labels_full: [&[u8]; QUICK_LAUNCH_COUNT] = [b"Spindle", b"Prefs", b"Shop", b"Wi-Fi", b"Lock", b"Snip"];
    let q_labels_short: [&[u8]; QUICK_LAUNCH_COUNT] = [b"Term", b"Prefs", b"Shop", b"Wi-Fi", b"Lock", b"Snip"];
    for i in 0..QUICK_LAUNCH_COUNT {
        let qr = l.quick_actions[i];
        if qr.2 == 0 || qr.3 == 0 {
            continue;
        }
        let qh = hov == HitTarget::quick_launch(i).hover_code();
        let qbg = if qh { p.tile_hover } else { p.tile_bg };
        fill_rounded(fb, w, h, total, qr, 8 * s, qbg);
        stroke_rounded(fb, w, h, total, qr, 8 * s, s, if qh { p.accent } else { p.divider });

        let q_ico = (if l.compact { 14 * s } else { 16 * s }).clamp(10, ICON_CAP);
        let content_h = q_ico + 2 * s + cap_h;
        let qx = qr.0 + qr.2.saturating_sub(q_ico) / 2;
        let qy = qr.1 + qr.3.saturating_sub(content_h) / 2;
        let q_ink = if qh { p.accent } else { p.ink };

        match i {
            0 => draw_icon(fb, w, h, total, 17, qx, qy, q_ico, q_ink),
            1 => draw_icon(fb, w, h, total, 18, qx, qy, q_ico, q_ink),
            2 => draw_shop_icon(fb, w, h, total, qx, qy, q_ico, q_ink),
            3 => draw_icon(fb, w, h, total, 0, qx, qy, q_ico, q_ink),
            4 => draw_icon(fb, w, h, total, 8, qx, qy, q_ico, q_ink),
            _ => draw_icon(fb, w, h, total, 9, qx, qy, q_ico, q_ink),
        }

        let lbl = if l.compact { q_labels_short[i] } else { q_labels_full[i] };
        let lw = if sc >= 2 {
            super::glyphs::ui_body_width(lbl, 1).unwrap_or(0)
        } else {
            super::glyphs::caption_width(lbl).unwrap_or(0)
        };
        let lmax = qr.2.saturating_sub(4 * s);
        let lx = qr.0 + qr.2.saturating_sub(lw.min(lmax)) / 2;
        let ly = qy + q_ico + 2 * s;
        draw_caption(fb, w, h, total, lbl, lx, ly, if qh { p.ink } else { p.subtext }, sc, lmax);
    }

    let _ = rad;
}
