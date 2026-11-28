// SPDX-License-Identifier: GPL-3.0-only
//! Frosted Candy Lavender renderer for SilkBar dropdown menus, status popovers,
//! and desktop/window right-click context menus.
//!
//! Shell drives state via `sex_pdx::silkbar_menus::OP_SILKBAR_MENU`; Display
//! renders using the shared `Layout` so pointer hits match drawn pixels.

use sex_pdx::silkbar_menus::{Layout, MenuKind, MenuState};
use sex_theme::{noir as nr, Mode, NoirAccent};
use crate::desk_theme::current as desk;

const SHELL_PD: u32 = 3;

static mut SHOWN: Option<MenuState> = None;

/// Handle `OP_SILKBAR_MENU` from Shell. Returns true when state changed.
pub fn receive(caller: u32, w0: u64, w1: u64) -> bool {
    if caller != SHELL_PD {
        return false;
    }
    let next = if w0 == 0 {
        None
    } else {
        match MenuState::decode(w0, w1) {
            Some(m) => Some(m),
            None => return false,
        }
    };
    unsafe {
        if SHOWN == next {
            return false;
        }
        SHOWN = next;
    }
    static mut BUDGET: u32 = 32;
    unsafe {
        if BUDGET > 0 {
            BUDGET -= 1;
            match next {
                Some(m) => {
                    sex_pdx::serial_record!(
                        "[display.silkbar_menu] shown=1 kind={} rows={} x={} y={} hover={}",
                        m.kind as u8,
                        m.row_count(),
                        m.x,
                        m.y,
                        m.hover.map_or(-1, |h| h as i32)
                    );
                }
                None => {
                    sex_pdx::serial_record!("[display.silkbar_menu] shown=0");
                }
            }
        }
    }
    true
}

/// Currently open menu kind, if any (used to highlight the active top-bar trigger).
pub fn active_kind() -> Option<MenuKind> {
    if crate::wifi_popover::is_open() {
        return Some(MenuKind::WifiPopover);
    }
    unsafe { SHOWN.map(|m| m.kind) }
}

/// Panel rectangle for `frost::glass_panels`.
pub fn glass_rect(w: usize, h: usize, scale: u32) -> Option<(usize, usize, usize, usize)> {
    if let Some(r) = crate::wifi_popover::glass_rect(w, h, scale) {
        return Some(r);
    }
    let m = (unsafe { SHOWN })?;
    Layout::new(&m, w, h, scale).map(|l| l.panel)
}

#[derive(Clone, Copy)]
struct CandyMenuColors {
    bg: u32,
    rim: u32,
    specular: u32,
    hover: u32,
    divider: u32,
    ink: u32,
    subtext: u32,
    danger: u32,
    accent: u32,
    btn_bg: u32,
}

fn colors(tint_idx: u8) -> CandyMenuColors {
    let d = desk();
    let accent = if d.mode == Mode::HighContrast {
        d.focus
    } else if d.accent == sex_theme::DEFAULT_ACCENT {
        NoirAccent::from_index(tint_idx).primary() & 0x00FF_FFFF
    } else {
        d.focus
    };
    match d.mode {
        Mode::HighContrast => CandyMenuColors {
            bg: d.menu,
            rim: d.menu_rim,
            specular: d.menu_rim,
            hover: d.menu_picked,
            divider: d.menu_rim,
            ink: d.menu_ink,
            subtext: d.menu_soft,
            danger: d.menu_close,
            accent,
            btn_bg: d.menu_button,
        },
        Mode::Dark => CandyMenuColors {
            bg: nr::DARK_MENU_BG & 0x00FF_FFFF,
            rim: nr::DARK_MENU_RIM & 0x00FF_FFFF,
            specular: nr::DARK_SPECULAR & 0x00FF_FFFF,
            hover: nr::DARK_MENU_HOVER & 0x00FF_FFFF,
            divider: nr::DARK_MENU_DIVIDER & 0x00FF_FFFF,
            ink: nr::DARK_TEXT & 0x00FF_FFFF,
            subtext: nr::DARK_MENU_SUBTEXT & 0x00FF_FFFF,
            danger: nr::DARK_MENU_DANGER & 0x00FF_FFFF,
            accent,
            btn_bg: nr::DARK_PILL_FILL & 0x00FF_FFFF,
        },
        Mode::Light => CandyMenuColors {
            bg: nr::LIGHT_MENU_BG & 0x00FF_FFFF,
            rim: nr::LIGHT_MENU_RIM & 0x00FF_FFFF,
            specular: nr::LIGHT_SPECULAR & 0x00FF_FFFF,
            hover: nr::LIGHT_MENU_HOVER & 0x00FF_FFFF,
            divider: nr::LIGHT_MENU_DIVIDER & 0x00FF_FFFF,
            ink: nr::LIGHT_TEXT & 0x00FF_FFFF,
            subtext: nr::LIGHT_MENU_SUBTEXT & 0x00FF_FFFF,
            danger: nr::LIGHT_MENU_DANGER & 0x00FF_FFFF,
            accent,
            btn_bg: nr::LIGHT_PILL_FILL & 0x00FF_FFFF,
        },
    }
}

fn fill(fb: *mut u32, w: usize, h: usize, total: usize, r: (usize, usize, usize, usize), c: u32) {
    for y in r.1..(r.1 + r.3).min(h) {
        for x in r.0..(r.0 + r.2).min(w) {
            let i = y * w + x;
            if i < total {
                unsafe { core::ptr::write_volatile(fb.add(i), c); }
            }
        }
    }
}

/// Blends a subtle lavender wash over the frosted glass interior for Candy Lavender harmony.
fn tint_glass(fb: *mut u32, w: usize, h: usize, total: usize, r: (usize, usize, usize, usize), tint: u32, alpha: u8) {
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

fn text_with_weight(fb: *mut u32, w: usize, h: usize, total: usize, t: &[u8], x0: usize, y0: usize, ink: u32, scale: u32, max_w: usize, body: bool) {
    let width = if body {
        super::glyphs::ui_body_width(t, scale)
    } else {
        super::glyphs::ui_width(t, scale)
    }.unwrap_or(0).min(max_w);
    let lh = super::glyphs::UI_H * scale as usize;
    for y in y0..(y0 + lh).min(h) {
        for x in x0..(x0 + width).min(w) {
            let at = y * w + x;
            if at >= total {
                return;
            }
            let cov = if body {
                super::glyphs::ui_body_coverage(t, (x - x0) as i32, y - y0, scale)
            } else {
                super::glyphs::ui_coverage(t, (x - x0) as i32, y - y0, scale)
            };
            let Some(alpha) = cov else { break };
            if alpha == 0 {
                continue;
            }
            unsafe {
                let p = fb.add(at);
                core::ptr::write_volatile(p, super::coverage_blend(ink, core::ptr::read_volatile(p), alpha));
            }
        }
    }
}

fn text(fb: *mut u32, w: usize, h: usize, total: usize, t: &[u8], x0: usize, y0: usize, ink: u32, scale: u32, max_w: usize) {
    text_with_weight(fb, w, h, total, t, x0, y0, ink, scale, max_w, false);
}

fn body_text(fb: *mut u32, w: usize, h: usize, total: usize, t: &[u8], x0: usize, y0: usize, ink: u32, scale: u32, max_w: usize) {
    text_with_weight(fb, w, h, total, t, x0, y0, ink, scale, max_w, true);
}

fn fmt_u32(mut v: u32, out: &mut [u8], pos: &mut usize) {
    let mut digits = [0u8; 10];
    let mut n = 0usize;
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    while n > 0 && *pos < out.len() {
        n -= 1;
        out[*pos] = digits[n];
        *pos += 1;
    }
}

fn put_bytes(s: &[u8], out: &mut [u8], pos: &mut usize) {
    for &b in s {
        if *pos < out.len() {
            out[*pos] = b;
            *pos += 1;
        }
    }
}

/// Renders the open SilkBar dropdown, status popover, or right-click context menu.
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, scale: u32, bar: usize, scene: &crate::scene::SceneSnapshot) {
    if crate::wifi_popover::draw(fb, w, h, total, scale, bar) {
        return;
    }
    let Some(m) = (unsafe { SHOWN }) else { return };
    let Some(l) = Layout::new(&m, w, h, scale) else { return };
    let sc = l.scale;
    let s = sc as usize;
    let c = colors(scene.bar.phase1.accent_tint_idx);
    let inner = crate::frost::inside(l.panel, s);

    // 1. Frosted obsidian glass backdrop + specular top rim + dark rim.
    crate::frost::glass_or(fb, w, h, total, crate::frost::Panel::SilkBarMenu, inner, sc, bar, c.bg);
    if desk().mode != Mode::HighContrast {
        tint_glass(fb, w, h, total, inner, c.bg, if desk().mode == Mode::Dark { 92 } else { 72 });
        // Subtle 1px top specular sheen inside the rim.
        tint_glass(fb, w, h, total, (inner.0, inner.1, inner.2, s), c.specular, if desk().mode == Mode::Dark { 36 } else { 75 });
    }
    crate::frost::rim(fb, w, h, total, l.panel, s, c.rim);

    let line = super::glyphs::UI_H * s;
    let inset = 10 * s;

    // 2. Special interactive rendering for Volume Popover.
    if m.kind == MenuKind::VolumePopover {
        let r0 = l.row_rect(0);
        let mut hdr = [0u8; 32];
        let mut hn = 0usize;
        if m.muted() {
            put_bytes(b"Volume: Muted", &mut hdr, &mut hn);
        } else {
            put_bytes(b"Volume: ", &mut hdr, &mut hn);
            fmt_u32(m.volume as u32, &mut hdr, &mut hn);
            put_bytes(b"%", &mut hdr, &mut hn);
        }
        let mid0 = r0.1 + r0.3.saturating_sub(line) / 2;
        text(fb, w, h, total, &hdr[..hn], r0.0 + inset, mid0, c.ink, sc, r0.2.saturating_sub(2 * inset));

        // Volume level bar
        let vb = l.vol_bar;
        fill(fb, w, h, total, vb, c.divider);
        let filled = vb.2 * (m.volume as usize) / 100;
        let fill_col = if m.muted() { c.subtext } else { c.accent };
        fill(fb, w, h, total, (vb.0, vb.1, filled, vb.3), fill_col);

        // - / Mute / + buttons
        let btns: [((usize, usize, usize, usize), &[u8], bool); 3] = [
            (l.vol_down, b"Vol -", false),
            (l.vol_mute, if m.muted() { b"Unmute" } else { b"Mute" }, m.muted()),
            (l.vol_up, b"Vol +", false),
        ];
        for (br, lbl, active) in btns {
            let bg = if active { c.danger } else { c.btn_bg };
            fill(fb, w, h, total, br, bg);
            crate::frost::rim(fb, w, h, total, br, s, c.rim);
            let tw = super::glyphs::ui_width(lbl, sc).unwrap_or(0);
            let tx = br.0 + br.2.saturating_sub(tw) / 2;
            let ty = br.1 + br.3.saturating_sub(line) / 2;
            let ink = if active { 0x00FF_FFFF } else { c.ink };
            text(fb, w, h, total, lbl, tx, ty, ink, sc, br.2);
        }
        return;
    }

    // 3. Standard rows for dropdowns, Wi-Fi/Battery/Clock popovers, and right-click context menus.
    for i in 0..l.rows {
        let Some(row) = m.row(i) else { break };
        let r = l.row_rect(i);
        if row.sep_before {
            fill(fb, w, h, total, (r.0 + 4 * s, r.1, r.2.saturating_sub(8 * s), s), c.divider);
        }
        let hovered = m.hover == Some(i as u8) && !row.header;
        if hovered {
            fill(fb, w, h, total, (r.0 + 2 * s, r.1 + s, r.2.saturating_sub(4 * s), r.3.saturating_sub(2 * s)), c.hover);
            fill(fb, w, h, total, (r.0 + 2 * s, r.1 + 4 * s, 3 * s, r.3.saturating_sub(8 * s)), c.accent);
        } else if row.active {
            fill(fb, w, h, total, (r.0 + 2 * s, r.1 + 4 * s, 3 * s, r.3.saturating_sub(8 * s)), c.accent);
        }

        let ink = if row.danger {
            c.danger
        } else if row.header {
            c.subtext
        } else {
            c.ink
        };

        let mid = r.1 + r.3.saturating_sub(line) / 2;
        let mut dyn_buf = [0u8; 40];
        let label_bytes: &[u8] = if m.kind == MenuKind::BatteryPopover && i == 0 {
            let mut n = 0usize;
            if m.battery <= 100 {
                put_bytes(b"Battery: ", &mut dyn_buf, &mut n);
                fmt_u32(m.battery as u32, &mut dyn_buf, &mut n);
                put_bytes(b"%", &mut dyn_buf, &mut n);
            } else {
                put_bytes(b"Power & Battery", &mut dyn_buf, &mut n);
            }
            &dyn_buf[..n]
        } else if m.kind == MenuKind::ClockPopover && i == 0 {
            let mut n = 0usize;
            if scene.bar.civil_enabled && scene.bar.civil_clock.seconds() != 0 {
                if let Some(g) = sex_time::SexTimestamp::from_unix(scene.bar.civil_clock.seconds() as i64, 0)
                    .and_then(sex_time::Gregorian::from_timestamp)
                {
                    let mname = sex_pdx::tray::MONTH_NAMES[(g.month.clamp(1, 12) - 1) as usize];
                    put_bytes(mname.as_bytes(), &mut dyn_buf, &mut n);
                    put_bytes(b" ", &mut dyn_buf, &mut n);
                    fmt_u32(g.day as u32, &mut dyn_buf, &mut n);
                    put_bytes(b", ", &mut dyn_buf, &mut n);
                    fmt_u32(g.year.max(0) as u32, &mut dyn_buf, &mut n);
                } else {
                    put_bytes(b"Date & Time", &mut dyn_buf, &mut n);
                }
            } else {
                put_bytes(b"SexOS System Clock", &mut dyn_buf, &mut n);
            }
            &dyn_buf[..n]
        } else if m.kind == MenuKind::ClockPopover && i == 2 {
            let mut n = 0usize;
            put_bytes(b"Uptime: ", &mut dyn_buf, &mut n);
            let hrs = m.uptime_mins / 60;
            let mins = m.uptime_mins % 60;
            if hrs > 0 {
                fmt_u32(hrs, &mut dyn_buf, &mut n);
                put_bytes(b"h ", &mut dyn_buf, &mut n);
            }
            fmt_u32(mins, &mut dyn_buf, &mut n);
            put_bytes(b"m", &mut dyn_buf, &mut n);
            &dyn_buf[..n]
        } else if row.active {
            let mut n = 0usize;
            put_bytes(b"* ", &mut dyn_buf, &mut n);
            put_bytes(row.label.as_bytes(), &mut dyn_buf, &mut n);
            &dyn_buf[..n]
        } else {
            row.label.as_bytes()
        };

        let sc_bytes = row.shortcut.as_bytes();
        let sc_w = if !sc_bytes.is_empty() {
            super::glyphs::ui_body_width(sc_bytes, sc).unwrap_or(0)
        } else {
            0
        };
        let label_max_w = if sc_w > 0 {
            r.2.saturating_sub(2 * inset + sc_w + 8 * s)
        } else {
            r.2.saturating_sub(2 * inset)
        };
        if row.header {
            body_text(fb, w, h, total, label_bytes, r.0 + inset, mid, ink, sc, label_max_w);
        } else {
            text(fb, w, h, total, label_bytes, r.0 + inset, mid, ink, sc, label_max_w);
        }
        if sc_w > 0 && r.2 > sc_w + 2 * inset {
            let sc_x = r.0 + r.2 - inset - sc_w;
            body_text(fb, w, h, total, sc_bytes, sc_x, mid, c.subtext, sc, sc_w);
        }
    }
}
