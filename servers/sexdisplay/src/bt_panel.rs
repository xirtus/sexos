// SPDX-License-Identifier: GPL-3.0-only
//! The Bluetooth panel (Cufflinks W2b): the Shell decides what it says and sends it as
//! lines (`sex_pdx::bt_ui::OP_PANEL`); Display draws it over the desktop under the bar
//! at the right, like the tray menus, with the layout both sides share
//! (`bt_ui::Panel`), so the Shell's clicks land on the rows drawn here. Only the Shell
//! may send it; text bytes outside printable ASCII were already made '?'.
use sex_pdx::bt_ui::{self as b, Assembler, Lines, Panel};

const BG: u32 = 0x0018_1825;
const RIM: u32 = 0x0045_475A;
const PICKED: u32 = 0x0031_3244;
const INK: u32 = 0x00CD_D6F4;
const BRIGHT: u32 = 0x00FF_FFFF;
const SOFT: u32 = 0x0093_99B2;

static mut ASSEMBLER: Assembler = Assembler::new();
static mut SHOWN: Option<(Lines, Option<usize>)> = None;

/// One `OP_PANEL` call. True when the panel changed (shown, changed or hidden).
pub fn receive(caller: u32, word: u64, a: u64, bytes: u64) -> bool {
    if caller != b::SHELL_PD { return false; }
    let assembler = unsafe { &mut *core::ptr::addr_of_mut!(ASSEMBLER) };
    let Some((lines, selected)) = assembler.push(word, a, bytes) else { return false };
    unsafe { SHOWN = (lines.count != 0).then_some((lines, selected)); }
    static mut BUDGET: u32 = 32;
    unsafe {
        if BUDGET > 0 {
            BUDGET -= 1;
            sex_pdx::serial_record!("[display.bt.panel] lines={} selected={}", lines.count, selected.map_or(-1, |s| s as i32));
        }
    }
    true
}

fn fill(fb: *mut u32, w: usize, h: usize, total: usize, r: b::Rect, c: u32) {
    for y in r.1..(r.1 + r.3).min(h) {
        for x in r.0..(r.0 + r.2).min(w) {
            let i = y * w + x;
            if i < total { unsafe { core::ptr::write_volatile(fb.add(i), c); } }
        }
    }
}

/// Text at (x0, y0) in `ink`, cut at `max_w`; returns its width.
fn text(fb: *mut u32, w: usize, h: usize, total: usize, t: &[u8], x0: usize, y0: usize, ink: u32, scale: u32, max_w: usize) -> usize {
    let width = super::glyphs::ui_width(t, scale).unwrap_or(0).min(max_w);
    let lh = super::glyphs::UI_H * scale as usize;
    for y in y0..(y0 + lh).min(h) {
        for x in x0..(x0 + width).min(w) {
            let at = y * w + x;
            if at >= total { return width; }
            let Some(alpha) = super::glyphs::ui_coverage(t, (x - x0) as i32, y - y0, scale) else { break };
            if alpha == 0 { continue; }
            unsafe {
                let p = fb.add(at);
                core::ptr::write_volatile(p, super::coverage_blend(ink, core::ptr::read_volatile(p), alpha));
            }
        }
    }
    width
}

/// Draws the panel, if one shows (after the body passes, before the lock surface).
pub fn draw(fb: *mut u32, w: usize, h: usize, total: usize, scale: u32, bar: usize) {
    let Some((lines, selected)) = (unsafe { SHOWN }) else { return };
    let s = scale.max(1) as usize;
    let Some(p) = Panel::new(w, bar, s, lines.count as usize) else { return };
    let (x, y, pw, ph) = p.panel;
    fill(fb, w, h, total, p.panel, RIM);
    fill(fb, w, h, total, (x + s, y + s, pw - 2 * s, ph - 2 * s), BG);
    let line = super::glyphs::UI_H * s;
    for i in 0..p.count {
        let r = p.row(i);
        let style = lines.style[i];
        if selected == Some(i) {
            fill(fb, w, h, total, r, PICKED);
            fill(fb, w, h, total, (r.0, r.1 + 3 * s, 3 * s, r.3.saturating_sub(6 * s)), sex_pdx::desktop_style::FOCUS);
        }
        if style == b::TITLE && i + 1 < p.count { fill(fb, w, h, total, (r.0, r.1 + r.3 - s, r.2, s), RIM); }
        let ink = match style { b::TITLE => BRIGHT, b::DIM => SOFT, _ => INK };
        let (left, right) = lines.columns(i);
        let mid = r.1 + r.3.saturating_sub(line) / 2;
        let inset = 8 * s;
        let right_w = super::glyphs::ui_width(right, scale).unwrap_or(0).min(r.2 / 2);
        text(fb, w, h, total, left, r.0 + inset, mid, ink, scale, r.2.saturating_sub(2 * inset + right_w + 8 * s));
        if !right.is_empty() {
            text(fb, w, h, total, right, r.0 + r.2 - inset - right_w, mid, if style == b::TITLE { INK } else { SOFT }, scale, right_w);
        }
    }
    static mut DRAW_BUDGET: u32 = 16;
    unsafe {
        if DRAW_BUDGET > 0 {
            DRAW_BUDGET -= 1;
            sex_pdx::serial_record!("[display.bt.panel.draw] lines={} panel={}x{}+{}+{}", lines.count, pw, ph, x, y);
        }
    }
}
