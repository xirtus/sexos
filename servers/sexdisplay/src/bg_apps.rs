// SPDX-License-Identifier: GPL-3.0-only
//! Frosted macOS Noir renderer for the top-right Background App Tray,
//! Open/Background Two-Section Switcher, App Quick-Action Popover,
//! Close vs Quit Confirmation Sheet, Force Quit Dialog, and About SexOS.

use sex_pdx::bg_apps::{
    BarLayout, HoverTarget, OverlayKind, OverlayLayout, Receiver, Rect, rect_contains,
};
use sex_pdx::brand;
use sex_theme::Mode;
use silkbar_model::{ModuleSlot, SilkBar};

use crate::chrome_candy::mix_u8;
use crate::desk_theme::current as desk;
use crate::frost;
use crate::scene::SceneSnapshot;

const SHELL_PD: u32 = 3;

static mut RX: Receiver = Receiver::new();

pub fn receive(caller: u32, a0: u64, a1: u64, a2: u64) -> bool {
    if caller != SHELL_PD {
        return false;
    }
    let rx = unsafe { &mut *core::ptr::addr_of_mut!(RX) };
    let changed = rx.push(a0, a1, a2);
    if changed {
        let s = &rx.shown;
        static mut BUDGET: u32 = 64;
        unsafe {
            if BUDGET > 0 {
                BUDGET -= 1;
                sex_pdx::serial_record!(
                    "[display.bg_apps] open={} parked={} overlay={} right_edge={}",
                    s.open_count,
                    s.parked_count,
                    s.overlay as u8,
                    s.bar_right_edge
                );
            }
        }
    }
    changed
}

fn bar_layout(bar: &SilkBar, scale: u32, bar_h: usize) -> BarLayout {
    let st = unsafe { &(*core::ptr::addr_of!(RX)).shown };
    let s = scale.max(1) as usize;
    let ws_last = if bar.layout[ModuleSlot::Workspace1 as usize].w > 0 {
        &bar.layout[ModuleSlot::Workspace8 as usize]
    } else {
        &bar.layout[ModuleSlot::Workspace0 as usize]
    };
    let ws_last_right = ws_last.x + ws_last.w;
    let chip0_x = bar.layout[ModuleSlot::Chip0 as usize].x;
    let fallback_right = chip0_x.saturating_sub(6 * s);
    let right_edge = if st.bar_right_edge == 0 || (st.bar_right_edge as usize) > chip0_x {
        fallback_right
    } else {
        st.bar_right_edge as usize
    };
    BarLayout::compute(bar_h, scale, ws_last_right, right_edge, st.parked_count as usize)
}

pub fn glass_rect(bar: &SilkBar, w: usize, h: usize, scale: u32, bar_h: usize) -> Option<Rect> {
    let st = unsafe { &(*core::ptr::addr_of!(RX)).shown };
    if st.overlay == OverlayKind::None {
        return None;
    }
    let bl = bar_layout(bar, scale, bar_h);
    OverlayLayout::compute(st, &bl, w, h, bar_h, scale).map(|o| o.panel())
}

#[derive(Clone, Copy)]
struct NoirColors {
    bg: u32,
    rim: u32,
    rim_accent: u32,
    specular: u32,
    hover: u32,
    card: u32,
    divider: u32,
    ink: u32,
    subtext: u32,
    rose: u32,
    on_rose: u32,
    danger: u32,
    danger_bg: u32,
    emerald: u32,
    btn_bg: u32,
}

fn colors() -> NoirColors {
    let d = desk();
    match d.mode {
        Mode::HighContrast => NoirColors {
            bg: 0x0000_0000,
            rim: 0x00FF_FFFF,
            rim_accent: 0x00FF_E600,
            specular: 0x00FF_FFFF,
            hover: 0x0024_2424,
            card: 0x0010_1010,
            divider: 0x00FF_FFFF,
            ink: 0x00FF_FFFF,
            subtext: 0x00FF_FFFF,
            rose: 0x00FF_E600,
            on_rose: 0x0000_0000,
            danger: 0x00FF_6E6E,
            danger_bg: 0x0026_0000,
            emerald: 0x0000_FF88,
            btn_bg: 0x0018_1818,
        },
        Mode::Light => NoirColors {
            bg: 0x00F6_F5FA,
            rim: 0x00C8_C6D4,
            rim_accent: 0x00E1_1D48,
            specular: 0x00FF_FFFF,
            hover: 0x00E8_E6F0,
            card: 0x00ED_ECF4,
            divider: 0x00DB_D9E6,
            ink: 0x0014_141C,
            subtext: 0x0052_5064,
            rose: 0x00E1_1D48,
            on_rose: 0x00FF_FFFF,
            danger: 0x00BE_123C,
            danger_bg: 0x00FD_E8EE,
            emerald: 0x0005_9669,
            btn_bg: 0x00E4_E2EC,
        },
        Mode::Dark => NoirColors {
            bg: 0x0013_131B,
            rim: 0x003A_374C,
            rim_accent: 0x00F4_3F5E,
            specular: 0x0078_7492,
            hover: 0x0026_2434,
            card: 0x001B_1926,
            divider: 0x002A_2836,
            ink: 0x00F5_F5F8,
            subtext: 0x00A4_A2B8,
            rose: 0x00E1_1D48,
            on_rose: 0x00FF_FFFF,
            danger: 0x00FB_7185,
            danger_bg: 0x0033_1622,
            emerald: 0x0034_D399,
            btn_bg: 0x0023_212F,
        },
    }
}

fn fill(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, c: u32) {
    for y in r.1..(r.1 + r.3).min(h) {
        for x in r.0..(r.0 + r.2).min(w) {
            let i = y * w + x;
            if i < total {
                unsafe { core::ptr::write_volatile(fb.add(i), c); }
            }
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

fn round_pill(fb: *mut u32, w: usize, h: usize, total: usize, r: Rect, rad: usize, fill_col: u32, rim_col: u32, border: usize) {
    if r.2 == 0 || r.3 == 0 {
        return;
    }
    let rad = rad.min(r.2 / 2).min(r.3 / 2);
    for yy in 0..r.3 {
        let y = r.1 + yy;
        if y >= h { break; }
        for xx in 0..r.2 {
            let x = r.0 + xx;
            if x >= w { break; }
            let dx = if xx < rad {
                rad - xx
            } else if xx >= r.2 - rad {
                xx - (r.2 - 1 - rad)
            } else {
                0
            };
            let dy = if yy < rad {
                rad - yy
            } else if yy >= r.3 - rad {
                yy - (r.3 - 1 - rad)
            } else {
                0
            };
            if rad > 0 && dx * dx + dy * dy > rad * rad {
                continue;
            }
            let on_rim = xx < border
                || xx >= r.2.saturating_sub(border)
                || yy < border
                || yy >= r.3.saturating_sub(border)
                || (rad > border && dx * dx + dy * dy > (rad - border) * (rad - border));
            let idx = y * w + x;
            if idx < total {
                unsafe { core::ptr::write_volatile(fb.add(idx), if on_rim { rim_col } else { fill_col }); }
            }
        }
    }
}

fn text(fb: *mut u32, w: usize, h: usize, total: usize, t: &[u8], x0: usize, y0: usize, ink: u32, scale: u32, max_w: usize) {
    let width = super::glyphs::ui_width(t, scale).unwrap_or(0).min(max_w);
    let lh = super::glyphs::UI_H * scale as usize;
    for y in y0..(y0 + lh).min(h) {
        for x in x0..(x0 + width).min(w) {
            let at = y * w + x;
            if at >= total { return; }
            let Some(alpha) = super::glyphs::ui_coverage(t, (x - x0) as i32, y - y0, scale) else { break };
            if alpha == 0 { continue; }
            unsafe {
                let p = fb.add(at);
                core::ptr::write_volatile(p, super::coverage_blend(ink, core::ptr::read_volatile(p), alpha));
            }
        }
    }
}

fn text_center(fb: *mut u32, w: usize, h: usize, total: usize, t: &[u8], r: Rect, ink: u32, scale: u32) {
    let tw = super::glyphs::ui_width(t, scale).unwrap_or(0).min(r.2);
    let lh = super::glyphs::UI_H * scale as usize;
    let tx = r.0 + r.2.saturating_sub(tw) / 2;
    let ty = r.1 + r.3.saturating_sub(lh) / 2;
    text(fb, w, h, total, t, tx, ty, ink, scale, r.2);
}

fn fmt_count(open: u8, parked: u8, out: &mut [u8; 8]) -> usize {
    let mut n = 0usize;
    let push_num = |mut v: u8, buf: &mut [u8; 8], p: &mut usize| {
        if v >= 10 {
            if *p < buf.len() { buf[*p] = b'0' + (v / 10).min(9); *p += 1; }
            v %= 10;
        }
        if *p < buf.len() { buf[*p] = b'0' + v; *p += 1; }
    };
    push_num(open, out, &mut n);
    if n < out.len() { out[n] = b'|'; n += 1; }
    push_num(parked, out, &mut n);
    n
}

fn round_pill_pixel(x: usize, y: usize, r: Rect, rad: usize, border: usize, fill_c: u32, rim_c: u32, under: u32) -> Option<u32> {
    if !rect_contains(r, x, y) {
        return None;
    }
    let xx = x - r.0;
    let yy = y - r.1;
    let rad = rad.min(r.2 / 2).min(r.3 / 2);
    let dx = if xx < rad {
        rad - xx
    } else if xx >= r.2 - rad {
        xx - (r.2 - 1 - rad)
    } else {
        0
    };
    let dy = if yy < rad {
        rad - yy
    } else if yy >= r.3 - rad {
        yy - (r.3 - 1 - rad)
    } else {
        0
    };
    if rad > 0 && dx * dx + dy * dy > rad * rad {
        return None;
    }
    let on_rim = xx < border
        || xx >= r.2.saturating_sub(border)
        || yy < border
        || yy >= r.3.saturating_sub(border)
        || (rad > border && dx * dx + dy * dy > (rad - border) * (rad - border));
    let _ = under;
    Some(if on_rim { rim_c } else { fill_c })
}

/// Top-bar background pixel for the Switcher pill and Parked App badges.
pub fn bar_bg_at(x: usize, y: usize, bar: &SilkBar, scene: &SceneSnapshot, body: u32) -> Option<u32> {
    let scale = scene.metrics.scale();
    let s = scale.max(1) as usize;
    let bar_h = scene.metrics.bar() as usize;
    let st = unsafe { &(*core::ptr::addr_of!(RX)).shown };
    let bl = bar_layout(bar, scale, bar_h);
    let c = colors();

    if bl.switcher_pill.2 > 0 && rect_contains(bl.switcher_pill, x, y) {
        let active = st.overlay == OverlayKind::Switcher || st.hover == HoverTarget::SwitcherPill;
        let fill_c = if active {
            mix_u8(c.card, c.rose, 78)
        } else if st.parked_count > 0 {
            mix_u8(c.card, c.rose, 28)
        } else {
            c.card
        };
        let rim_c = if active { c.rim_accent } else { c.rim };
        return round_pill_pixel(x, y, bl.switcher_pill, 6 * s, s, fill_c, rim_c, body);
    }

    for i in 0..bl.parked_count {
        let r = bl.parked_badges[i];
        if !rect_contains(r, x, y) {
            continue;
        }
        let app = &st.parked[i];
        let active = (st.overlay == OverlayKind::QuickAction && st.overlay_idx as usize == i)
            || st.hover == HoverTarget::ParkedBadge(i as u8);
        let accent = app.kind.badge_rgb();
        let fill_c = mix_u8(c.card, accent, if active { 112 } else { 68 });
        let rim_c = if active { c.rim_accent } else { mix_u8(c.rim, accent, 140) };
        if let Some(px) = round_pill_pixel(x, y, r, 6 * s, s, fill_c, rim_c, body) {
            // Live background dot in top-right corner of the badge.
            let dx = (x as isize) - ((r.0 + r.2.saturating_sub(4 * s)) as isize);
            let dy = (y as isize) - ((r.1 + 4 * s) as isize);
            let dr = (2 * s) as isize;
            if dx * dx + dy * dy <= dr * dr {
                return Some(c.emerald);
            }
            return Some(px);
        }
    }
    None
}

/// Top-bar foreground glyphs for the Switcher pill and Parked App badges.
pub fn bar_fg_at(x: usize, y: usize, bar: &SilkBar, scene: &SceneSnapshot, background: u32) -> Option<u32> {
    let scale = scene.metrics.scale();
    let s = scale.max(1) as usize;
    let bar_h = scene.metrics.bar() as usize;
    let st = unsafe { &(*core::ptr::addr_of!(RX)).shown };
    let bl = bar_layout(bar, scale, bar_h);
    let c = colors();

    if bl.switcher_pill.2 > 0 && rect_contains(bl.switcher_pill, x, y) {
        let r = bl.switcher_pill;
        // Left: mini stacked-window glyph inside the pill.
        let gx = r.0 + 7 * s;
        let gy = r.1 + 5 * s;
        let in_front_win = x >= gx + 2 * s && x < gx + 10 * s && y >= gy + 2 * s && y < gy + 10 * s;
        let front_edge = in_front_win && (x == gx + 2 * s || x + 1 == gx + 10 * s || y == gy + 2 * s || y + 1 == gy + 10 * s);
        if front_edge {
            return Some(c.ink);
        }
        let in_back_win = x >= gx && x < gx + 8 * s && y >= gy && y < gy + 8 * s && !in_front_win;
        let back_edge = in_back_win && (x == gx || x + 1 == gx + 8 * s || y == gy || y + 1 == gy + 8 * s);
        if back_edge {
            return Some(if st.parked_count > 0 { c.emerald } else { c.subtext });
        }

        let mut lbl = [0u8; 8];
        let n = fmt_count(st.open_count, st.parked_count, &mut lbl);
        let tw = super::glyphs::ui_width(&lbl[..n], scale).unwrap_or(0);
        let lh = super::glyphs::UI_H * s;
        let lx = gx + 14 * s + r.2.saturating_sub(21 * s + tw) / 2;
        let ly = r.1 + r.3.saturating_sub(lh) / 2;
        if x >= lx && x < lx + tw && y >= ly && y < ly + lh {
            if let Some(a) = super::glyphs::ui_coverage(&lbl[..n], (x - lx) as i32, y - ly, scale) {
                if a != 0 {
                    return Some(super::coverage_blend(c.ink, background, a));
                }
            }
        }
        return None;
    }

    for i in 0..bl.parked_count {
        let r = bl.parked_badges[i];
        if !rect_contains(r, x, y) {
            continue;
        }
        let mono = st.parked[i].badge_chars();
        let tw = super::glyphs::ui_width(&mono, scale).unwrap_or(0);
        let lh = super::glyphs::UI_H * s;
        let lx = r.0 + r.2.saturating_sub(tw) / 2;
        let ly = r.1 + r.3.saturating_sub(lh) / 2;
        if x >= lx && x < lx + tw && y >= ly && y < ly + lh {
            if let Some(a) = super::glyphs::ui_coverage(&mono, (x - lx) as i32, y - ly, scale) {
                if a != 0 {
                    return Some(super::coverage_blend(c.ink, background, a));
                }
            }
        }
    }
    None
}

fn draw_panel_shell(fb: *mut u32, w: usize, h: usize, total: usize, panel: Rect, s: usize, scale: u32, bar_h: usize, c: &NoirColors) {
    let inner = frost::inside(panel, s);
    frost::glass_or(fb, w, h, total, frost::Panel::BgApps, inner, scale, bar_h, c.bg);
    if desk().mode != Mode::HighContrast {
        tint_glass(fb, w, h, total, inner, c.bg, 96);
        tint_glass(fb, w, h, total, (inner.0, inner.1, inner.2, s), c.specular, 72);
    }
    frost::rim(fb, w, h, total, panel, s, c.rim);
}

fn draw_sculpted_lips(fb: *mut u32, w: usize, h: usize, total: usize, cx: usize, top: usize, mw: usize, bg: u32) {
    let mw_u = mw.min(256) as u32;
    if mw_u < 8 {
        return;
    }
    let mh_u = brand::height_for(mw_u);
    let x0 = cx.saturating_sub(mw_u as usize / 2);
    let mut row = [0u8; 256];
    for yy in 0..mh_u {
        let y = top + yy as usize;
        if y >= h { break; }
        brand::coverage_row(mw_u, yy, &mut row);
        for xx in 0..mw_u {
            let cov = row[xx as usize];
            if cov == 0 { continue; }
            let x = x0 + xx as usize;
            if x >= w { break; }
            let idx = y * w + x;
            if idx < total {
                let col = brand::color_at_sculpted(brand::Tone::Crimson, xx, yy, mw_u, mh_u, 48);
                let under = unsafe { core::ptr::read_volatile(fb.add(idx)) };
                let _ = bg;
                unsafe { core::ptr::write_volatile(fb.add(idx), brand::blend(col, under, cov)); }
            }
        }
    }
}

/// Draws the active Background App overlay / switcher / confirmation sheet.
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, bar: &SilkBar, scale: u32, bar_h: usize) {
    let st = unsafe { &(*core::ptr::addr_of!(RX)).shown };
    if st.overlay == OverlayKind::None {
        return;
    }
    let bl = bar_layout(bar, scale, bar_h);
    let Some(layout) = OverlayLayout::compute(st, &bl, w, h, bar_h, scale) else { return };
    let c = colors();

    match layout {
        OverlayLayout::QuickAction(l) => {
            let sc = l.scale;
            let s = sc as usize;
            let idx = (st.overlay_idx as usize).min((st.parked_count as usize).saturating_sub(1));
            let Some(app) = st.parked_slice().get(idx) else { return };
            draw_panel_shell(fb, w, h, total, l.panel, s, sc, bar_h, &c);

            // Header badge + Title + Running in Background status
            let badge = (l.header.0, l.header.1 + 4 * s, 26 * s, 26 * s);
            round_pill(fb, w, h, total, badge, 6 * s, mix_u8(c.card, app.kind.badge_rgb(), 96), app.kind.badge_rgb(), s);
            let mono = app.badge_chars();
            text_center(fb, w, h, total, &mono, badge, c.ink, sc);

            let tx = badge.0 + badge.2 + 8 * s;
            let tw = l.header.2.saturating_sub(badge.2 + 8 * s);
            text(fb, w, h, total, app.title_bytes(), tx, l.header.1 + 2 * s, c.ink, sc, tw);

            // Emerald live dot + status label
            let dot_r = (tx, l.header.1 + 20 * s, 6 * s, 6 * s);
            round_pill(fb, w, h, total, dot_r, 3 * s, c.emerald, c.emerald, 0);
            text(fb, w, h, total, b"Running in Background", tx + 10 * s, l.header.1 + 17 * s, c.subtext, sc, tw.saturating_sub(10 * s));

            // Contextual Quick Actions
            let actions = app.kind.quick_actions();
            let lh = super::glyphs::UI_H * s;
            for (qi, qr) in l.quick_rows.iter().enumerate() {
                let hovered = st.hover == HoverTarget::QuickRow(qi as u8);
                if hovered {
                    round_pill(fb, w, h, total, *qr, 5 * s, c.hover, c.rim, 0);
                    fill(fb, w, h, total, (qr.0, qr.1 + 4 * s, 3 * s, qr.3.saturating_sub(8 * s)), c.rose);
                }
                text(fb, w, h, total, actions[qi].as_bytes(), qr.0 + 10 * s, qr.1 + qr.3.saturating_sub(lh) / 2, c.ink, sc, qr.2.saturating_sub(14 * s));
            }

            // Divider before Reopen / Quit
            fill(fb, w, h, total, (l.header.0, l.reopen_btn.1.saturating_sub(5 * s), l.header.2, s), c.divider);

            let reopen_bg = if st.hover == HoverTarget::QuickRow(2) { mix_u8(c.rose, 0x00FF_FFFF, 28) } else { c.rose };
            round_pill(fb, w, h, total, l.reopen_btn, 6 * s, reopen_bg, c.rim_accent, s);
            text_center(fb, w, h, total, b"Reopen Window", l.reopen_btn, c.on_rose, sc);

            let quit_bg = if st.hover == HoverTarget::QuickRow(3) { mix_u8(c.danger_bg, c.danger, 48) } else { c.danger_bg };
            round_pill(fb, w, h, total, l.quit_btn, 6 * s, quit_bg, c.danger, s);
            let mut qbuf = [0u8; 24];
            let mut qn = 0usize;
            for &b in b"Quit " { if qn < qbuf.len() { qbuf[qn] = b; qn += 1; } }
            for &b in app.title_bytes() { if qn < qbuf.len() { qbuf[qn] = b; qn += 1; } }
            text_center(fb, w, h, total, &qbuf[..qn], l.quit_btn, c.danger, sc);
        }

        OverlayLayout::Switcher(l) => {
            let sc = l.scale;
            let s = sc as usize;
            let lh = super::glyphs::UI_H * s;
            draw_panel_shell(fb, w, h, total, l.panel, s, sc, bar_h, &c);
            let ix = l.panel.0 + 10 * s;
            let iw = l.panel.2.saturating_sub(20 * s);

            text(fb, w, h, total, b"Open & Background Apps", ix, l.panel.1 + 11 * s, c.ink, sc, iw);

            // Section 1: Open Windows
            text(fb, w, h, total, b"OPEN WINDOWS", ix, l.open_hdr_y + 2 * s, c.subtext, sc, iw);
            if l.open_visible == 0 {
                text(fb, w, h, total, b"No open windows", ix + 4 * s, l.open_hdr_y + 22 * s, c.subtext, sc, iw);
            }
            for i in 0..l.open_visible {
                let win = &st.open[i];
                let (focus_r, close_r) = l.open_rows[i];
                let hovered = st.hover == HoverTarget::SwitcherOpenFocus(i as u8);
                let bg_c = if hovered { c.hover } else if win.focused { mix_u8(c.card, c.rose, 26) } else { c.card };
                round_pill(fb, w, h, total, focus_r, 5 * s, bg_c, if win.focused { c.rim_accent } else { c.divider }, s);
                let dot = (focus_r.0 + 7 * s, focus_r.1 + (focus_r.3.saturating_sub(6 * s)) / 2, 6 * s, 6 * s);
                round_pill(fb, w, h, total, dot, 3 * s, if win.focused { c.rose } else { c.subtext }, c.subtext, 0);
                text(fb, w, h, total, win.title_bytes(), focus_r.0 + 18 * s, focus_r.1 + focus_r.3.saturating_sub(lh) / 2, c.ink, sc, focus_r.2.saturating_sub(24 * s));

                let close_hov = st.hover == HoverTarget::SwitcherOpenClose(i as u8);
                round_pill(fb, w, h, total, close_r, 4 * s, if close_hov { c.danger_bg } else { c.btn_bg }, if close_hov { c.danger } else { c.rim }, s);
                text_center(fb, w, h, total, b"x", close_r, if close_hov { c.danger } else { c.subtext }, sc);
            }

            // Section 2: Active in Background (Window Closed)
            fill(fb, w, h, total, (ix, l.bg_hdr_y.saturating_sub(5 * s), iw, s), c.divider);
            text(fb, w, h, total, b"ACTIVE IN BACKGROUND", ix, l.bg_hdr_y + 2 * s, c.subtext, sc, iw);
            if l.parked_visible == 0 {
                text(fb, w, h, total, b"No background apps", ix + 4 * s, l.bg_hdr_y + 22 * s, c.subtext, sc, iw);
            }
            for i in 0..l.parked_visible {
                let app = &st.parked[i];
                let (row_r, reopen_r, quit_r) = l.parked_rows[i];
                let badge = (row_r.0 + 2 * s, row_r.1 + 4 * s, 22 * s, 22 * s);
                round_pill(fb, w, h, total, badge, 5 * s, mix_u8(c.card, app.kind.badge_rgb(), 88), app.kind.badge_rgb(), s);
                text_center(fb, w, h, total, &app.badge_chars(), badge, c.ink, sc);

                let text_w = reopen_r.0.saturating_sub(badge.0 + badge.2 + 8 * s);
                text(fb, w, h, total, app.title_bytes(), badge.0 + badge.2 + 6 * s, row_r.1 + row_r.3.saturating_sub(lh) / 2, c.ink, sc, text_w);

                let rhov = st.hover == HoverTarget::SwitcherParkedReopen(i as u8);
                round_pill(fb, w, h, total, reopen_r, 5 * s, if rhov { mix_u8(c.rose, 0x00FF_FFFF, 28) } else { c.rose }, c.rim_accent, s);
                text_center(fb, w, h, total, b"Reopen", reopen_r, c.on_rose, sc);

                let qhov = st.hover == HoverTarget::SwitcherParkedQuit(i as u8);
                round_pill(fb, w, h, total, quit_r, 5 * s, if qhov { mix_u8(c.danger_bg, c.danger, 48) } else { c.danger_bg }, c.danger, s);
                text_center(fb, w, h, total, b"Quit", quit_r, c.danger, sc);
            }
        }

        OverlayLayout::CloseConfirm(l) => {
            let sc = l.scale;
            let s = sc as usize;
            let pad = 16 * s;
            draw_panel_shell(fb, w, h, total, l.panel, s, sc, bar_h, &c);
            let iw = l.panel.2.saturating_sub(2 * pad);
            let ix = l.panel.0 + pad;

            // Header: "Close [App] window or quit completely?"
            let mut title = [0u8; 48];
            let mut tn = 0usize;
            for &b in b"Close " { if tn < title.len() { title[tn] = b; tn += 1; } }
            for &b in st.confirm_title_bytes() { if tn < title.len() { title[tn] = b; tn += 1; } }
            for &b in b" window or quit completely?" { if tn < title.len() { title[tn] = b; tn += 1; } }
            text(fb, w, h, total, &title[..tn], ix, l.panel.1 + 18 * s, c.ink, sc, iw);
            text(fb, w, h, total, b"Keep in SilkBar to preserve background tasks.", ix, l.panel.1 + 42 * s, c.subtext, sc, iw);

            // Remember checkbox
            let cb_bg = if st.remember { c.rose } else { c.card };
            let cb_rim = if st.remember || st.hover == HoverTarget::CloseRemember { c.rim_accent } else { c.rim };
            round_pill(fb, w, h, total, l.checkbox, 4 * s, cb_bg, cb_rim, s);
            if st.remember {
                text_center(fb, w, h, total, b"x", l.checkbox, c.on_rose, sc);
            }
            let lh = super::glyphs::UI_H * s;
            text(
                fb,
                w,
                h,
                total,
                b"Remember my choice for this app",
                l.checkbox.0 + l.checkbox.2 + 8 * s,
                l.remember_row.1 + l.remember_row.3.saturating_sub(lh) / 2,
                c.ink,
                sc,
                l.remember_row.2.saturating_sub(l.checkbox.2 + 8 * s),
            );

            // Action Buttons
            let c_bg = if st.hover == HoverTarget::CloseCancel { c.hover } else { c.btn_bg };
            round_pill(fb, w, h, total, l.cancel_btn, 6 * s, c_bg, c.rim, s);
            text_center(fb, w, h, total, b"Cancel", l.cancel_btn, c.ink, sc);

            let q_bg = if st.hover == HoverTarget::CloseQuit { mix_u8(c.danger_bg, c.danger, 48) } else { c.danger_bg };
            round_pill(fb, w, h, total, l.quit_btn, 6 * s, q_bg, c.danger, s);
            text_center(fb, w, h, total, b"Quit App", l.quit_btn, c.danger, sc);

            let k_bg = if st.hover == HoverTarget::CloseKeep { mix_u8(c.rose, 0x00FF_FFFF, 28) } else { c.rose };
            round_pill(fb, w, h, total, l.keep_btn, 6 * s, k_bg, c.rim_accent, s);
            text_center(fb, w, h, total, b"Keep Running in SilkBar", l.keep_btn, c.on_rose, sc);
        }

        OverlayLayout::ForceQuit(l) => {
            let sc = l.scale;
            let s = sc as usize;
            let pad = 14 * s;
            let ix = l.panel.0 + pad;
            let iw = l.panel.2.saturating_sub(2 * pad);
            let lh = super::glyphs::UI_H * s;
            draw_panel_shell(fb, w, h, total, l.panel, s, sc, bar_h, &c);

            text(fb, w, h, total, b"Force Quit Applications", ix, l.panel.1 + 14 * s, c.ink, sc, iw);
            text(fb, w, h, total, b"Select a running app and choose Force Quit.", ix, l.panel.1 + 34 * s, c.subtext, sc, iw);

            if l.visible == 0 {
                text(fb, w, h, total, b"No running applications", ix, l.panel.1 + 66 * s, c.subtext, sc, iw);
            }
            let open_n = (st.open_count as usize).min(sex_pdx::bg_apps::MAX_OPEN);
            for i in 0..l.visible {
                let r = l.rows[i];
                let selected = st.overlay_idx as usize == i;
                let hov = st.hover == HoverTarget::ForceRow(i as u8);
                let bg = if selected { mix_u8(c.card, c.rose, 52) } else if hov { c.hover } else { c.card };
                round_pill(fb, w, h, total, r, 5 * s, bg, if selected { c.rim_accent } else { c.divider }, s);
                let (label, tag): (&[u8], &[u8]) = if i < open_n {
                    (st.open[i].title_bytes(), b"Open")
                } else {
                    let pi = i - open_n;
                    (st.parked[pi].title_bytes(), b"Background")
                };
                text(fb, w, h, total, label, r.0 + 10 * s, r.1 + r.3.saturating_sub(lh) / 2, c.ink, sc, r.2.saturating_sub(96 * s));
                text(fb, w, h, total, tag, r.0 + r.2.saturating_sub(82 * s), r.1 + r.3.saturating_sub(lh) / 2, if i < open_n { c.subtext } else { c.emerald }, sc, 76 * s);
            }

            let cbg = if st.hover == HoverTarget::ForceCloseBtn { c.hover } else { c.btn_bg };
            round_pill(fb, w, h, total, l.close_btn, 6 * s, cbg, c.rim, s);
            text_center(fb, w, h, total, b"Close", l.close_btn, c.ink, sc);

            let fbg = if st.hover == HoverTarget::ForceQuitBtn { mix_u8(c.danger, 0x00FF_FFFF, 28) } else { c.rose };
            round_pill(fb, w, h, total, l.force_quit_btn, 6 * s, fbg, c.rim_accent, s);
            text_center(fb, w, h, total, b"Force Quit", l.force_quit_btn, c.on_rose, sc);
        }

        OverlayLayout::AboutSexOs(l) => {
            let sc = l.scale;
            let s = sc as usize;
            let pad = 16 * s;
            let ix = l.panel.0 + pad;
            let iw = l.panel.2.saturating_sub(2 * pad);
            draw_panel_shell(fb, w, h, total, l.panel, s, sc, bar_h, &c);

            // Sculpted SexOS Lips Hero Emblem inside About This SexOS
            draw_sculpted_lips(fb, w, h, total, l.panel.0 + l.panel.2 / 2, l.panel.1 + 18 * s, 60 * s, c.bg);

            text_center(fb, w, h, total, b"SexOS", (ix, l.panel.1 + 72 * s, iw, 20 * s), c.ink, sc);
            text_center(fb, w, h, total, b"macOS Noir Edition | Capability OS", (ix, l.panel.1 + 96 * s, iw, 18 * s), c.subtext, sc);
            text_center(fb, w, h, total, b"SilkBar Background Lifecycle Active", (ix, l.panel.1 + 118 * s, iw, 18 * s), c.subtext, sc);

            let cbg = if st.hover == HoverTarget::AboutCloseBtn { c.hover } else { c.btn_bg };
            round_pill(fb, w, h, total, l.close_btn, 6 * s, cbg, c.rim, s);
            text_center(fb, w, h, total, b"Close", l.close_btn, c.ink, sc);

            let sbg = if st.hover == HoverTarget::AboutSettingsBtn { mix_u8(c.rose, 0x00FF_FFFF, 28) } else { c.rose };
            round_pill(fb, w, h, total, l.settings_btn, 6 * s, sbg, c.rim_accent, s);
            text_center(fb, w, h, total, b"System Settings...", l.settings_btn, c.on_rose, sc);
        }
    }
}
