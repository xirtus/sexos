// SPDX-License-Identifier: GPL-3.0-only
//! The window menu (slice S1): a right click on a title bar opens it. The Shell
//! decides what it says and where (`sex_pdx::window_menu::OP_WINDOW_MENU`);
//! Display draws it over the desktop with the layout both sides share, so the
//! Shell's clicks land on the rows drawn here. Only the Shell may send it.
use sex_pdx::window_menu::{Layout, Menu};

const BG: u32 = 0x0018_1825;
const RIM: u32 = 0x0045_475A;
const PICKED: u32 = 0x0031_3244;
const INK: u32 = 0x00CD_D6F4;
const CLOSE_INK: u32 = 0x00F3_8BA8;
const SHELL_PD: u32 = 3;

static mut SHOWN: Option<Menu> = None;

/// One `OP_WINDOW_MENU` call. True when the menu changed (shown, changed or hidden).
pub fn receive(caller: u32, word: u64) -> bool {
    if caller != SHELL_PD { return false; }
    let menu = if word == 0 { None } else {
        match Menu::decode(word) { Some(m) => Some(m), None => return false }
    };
    unsafe {
        if SHOWN == menu { return false; }
        SHOWN = menu;
    }
    static mut BUDGET: u32 = 32;
    unsafe {
        if BUDGET > 0 {
            BUDGET -= 1;
            match menu {
                Some(m) => { sex_pdx::serial_record!("[display.window_menu] shown=1 rows={} x={} y={} hover={}", m.rows(), m.x, m.y, m.hover.map_or(-1, |h| h as i32)); }
                None => { sex_pdx::serial_record!("[display.window_menu] shown=0"); }
            }
        }
    }
    true
}

fn fill(fb: *mut u32, w: usize, h: usize, total: usize, r: (usize, usize, usize, usize), c: u32) {
    for y in r.1..(r.1 + r.3).min(h) {
        for x in r.0..(r.0 + r.2).min(w) {
            let i = y * w + x;
            if i < total { unsafe { core::ptr::write_volatile(fb.add(i), c); } }
        }
    }
}

/// Text at (x0, y0) in `ink`, cut at `max_w`.
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

/// Draws the menu, if one shows (after the body passes, before the lock surface).
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, scale: u32) {
    let Some(menu) = (unsafe { SHOWN }) else { return };
    let Some(l) = Layout::new(&menu, w, h, scale) else { return };
    let scale = l.scale;
    let s = scale as usize;
    let (x, y, pw, ph) = l.panel;
    fill(fb, w, h, total, l.panel, RIM);
    fill(fb, w, h, total, (x + s, y + s, pw - 2 * s, ph - 2 * s), BG);
    let line = super::glyphs::UI_H * s;
    for i in 0..l.rows {
        let Some(item) = menu.item(i) else { break };
        let r = l.row(i);
        if menu.hover == Some(i as u8) {
            fill(fb, w, h, total, r, PICKED);
            fill(fb, w, h, total, (r.0, r.1 + 3 * s, 3 * s, r.3.saturating_sub(6 * s)), sex_pdx::desktop_style::FOCUS);
        }
        // A rule above Close sets it apart from the rest.
        if item == sex_pdx::window_menu::Item::Close { fill(fb, w, h, total, (r.0, r.1, r.2, s), RIM); }
        let ink = if item == sex_pdx::window_menu::Item::Close { CLOSE_INK } else { INK };
        let inset = 10 * s;
        let mid = r.1 + r.3.saturating_sub(line) / 2;
        text(fb, w, h, total, menu.label(item).as_bytes(), r.0 + inset, mid, ink, scale, r.2.saturating_sub(2 * inset));
    }
    static mut DRAW_BUDGET: u32 = 16;
    unsafe {
        if DRAW_BUDGET > 0 {
            DRAW_BUDGET -= 1;
            sex_pdx::serial_record!("[display.window_menu.draw] rows={} panel={}x{}+{}+{}", l.rows, pw, ph, x, y);
        }
    }
}
